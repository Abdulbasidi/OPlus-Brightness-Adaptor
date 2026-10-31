use std::fs::OpenOptions;
use std::os::unix::io::AsRawFd;
use std::sync::{Arc, RwLock};

use crate::constants::{SERVICE_NAME, BRIGHTNESS_PROP};
use crate::light::Lights;
use crate::logging::{log_d, log_e};
use crate::paths::{
    bright_path, display_type_prop, is_oplus_panel_prop, persist_bright_mode_prop,
    persist_dbg, persist_lux_aod_prop,
};
use crate::properties::{
    find_prop, get_prop, get_prop_int, read_prop_pi, wait_any_prop, wait_prop_pi,
};
use crate::range::BrightnessRange;
use crate::service::PanelService;
use crate::state::ensure_addon_dir;
use crate::utils::{get_max_brightness, get_min_brightness};

// debug check
pub fn dbg_on() -> bool {
    get_prop(persist_dbg()).as_deref() == Some("true")
}

// mode checks
pub fn is_oplus_panel_mode() -> bool {
    get_prop(is_oplus_panel_prop()).as_deref() == Some("true")
}

pub fn is_ips_mode() -> bool {
    get_prop(display_type_prop()).as_deref() == Some("IPS")
}

pub fn is_lux_aod_mode() -> bool {
    get_prop(persist_lux_aod_prop()).as_deref() == Some("true")
}

// 0 = Curved, 1 = Linear, 2 = Custom
pub fn get_brightness_mode() -> i32 {
    get_prop_int(persist_bright_mode_prop()).unwrap_or(0)
}

pub fn get_fp_type() -> i32 {
    if let Some(val_str) = get_prop("ro.vendor.display.fp_type")
        .or_else(|| get_prop(crate::paths::persist_fp_type()))
    {
        let trimmed = val_str.trim();
        let parsed = if let Some(hex) = trimmed.strip_prefix("0x").or_else(|| trimmed.strip_prefix("0X")) {
            i32::from_str_radix(hex, 16).ok()
        } else {
            trimmed.parse::<i32>().ok()
        };
        if let Some(v) = parsed {
            if v > 0 {
                return v;
            }
        }
    }
    4
}

// main runner
pub fn run() {
    let dbg = dbg_on();
    log_d("[DisplayAdaptor] Starting vendor.xia.display.adaptor-V6@1.0-service AIDL HAL...");

    ensure_addon_dir();

    if let Err(err) = rsbinder::ProcessState::init_default() {
        log_e(&format!("[DisplayAdaptor] Failed to init ProcessState: {}", err));
        std::process::exit(1);
    }
    rsbinder::ProcessState::start_thread_pool();

    let hw_min = get_min_brightness(dbg);
    let hw_max = get_max_brightness(dbg);
    if dbg {
        log_d(&format!(
            "[DisplayAdaptor] Detected hardware brightness limits: min={}, max={}",
            hw_min, hw_max
        ));
    }

    let mut range = BrightnessRange::init();
    range.refresh_range();
    if dbg {
        log_d(&format!(
            "[DisplayAdaptor] Initialized input range: min={}, max={}",
            range.min, range.max
        ));
    }
    let range_arc = Arc::new(RwLock::new(range));

    let file_fd = match OpenOptions::new().write(true).open(bright_path()) {
        Ok(file) => {
            let fd = file.as_raw_fd();
            std::mem::forget(file);
            Some(fd)
        }
        Err(e) => {
            log_e(&format!(
                "[DisplayAdaptor] Could not open sysfs brightness file {}: {}",
                bright_path(),
                e
            ));
            None
        }
    };

    let (service, inner) = PanelService::new(hw_min, hw_max, Arc::clone(&range_arc), file_fd);
    let light_conn = Arc::clone(&inner.lights);

    if let Err(err) = service.register() {
        log_e(&format!(
            "[DisplayAdaptor] Failed to register service {}: {:?}",
            SERVICE_NAME, err
        ));
        std::process::exit(1);
    }
    log_d(&format!("[DisplayAdaptor] Registered {}", SERVICE_NAME));

    if let Some(lights) = Lights::connect() {
        if dbg {
            match lights.lights() {
                Ok(list) => {
                    log_d(&format!("[DisplayAdaptor] Light HAL exposes {} lights", list.len()));
                    if let Some(backlight) = list.iter().find(|l| l.is_backlight()) {
                        log_d(&format!(
                            "[DisplayAdaptor] Backlight light id={}, ordinal={}",
                            backlight.id, backlight.ordinal
                        ));
                    }
                }
                Err(e) => log_e(&format!("[DisplayAdaptor] Light HAL query failed: {:?}", e)),
            }
        }
        let _ = light_conn.set(lights);
    } else if dbg {
        log_d("[DisplayAdaptor] Light HAL unavailable; using direct sysfs control");
    }

    // Event-driven kernel futex property watcher.
    // Replaces 50ms polling loop with Bionic's __system_property_wait futex sleep.
    // The Oplus framework writes its computed 11-bit brightness to debug.tracing.screen_brightness.
    // When property_service writes the slot, kernel issues FUTEX_WAKE and wakes this thread instantly.
    let inner_watcher = Arc::clone(&inner);
    std::thread::spawn(move || {
        log_d("[DisplayAdaptor] Starting kernel futex property event listener for brightness");

        let mut pi = find_prop(BRIGHTNESS_PROP);
        let mut global_serial = 0u32;
        while pi.is_none() {
            if let Some(s) = wait_any_prop(global_serial) {
                global_serial = s;
            }
            pi = find_prop(BRIGHTNESS_PROP);
        }
        let pi = pi.unwrap();

        let initial = read_prop_pi(pi);
        let mut serial = initial.serial;
        if let Ok(f) = initial.value.trim().parse::<f32>() {
            let raw = f.round() as i32;
            if raw > 0 {
                inner_watcher.apply_brightness(raw);
            }
        }

        let mut last_raw = -1;
        loop {
            let new_serial = match wait_prop_pi(pi, serial) {
                Some(s) => s,
                None => continue,
            };
            serial = new_serial;

            let pv = read_prop_pi(pi);
            if pv.serial != 0 {
                serial = pv.serial;
            }

            let trimmed = pv.value.trim();
            if !trimmed.is_empty() {
                if let Ok(f) = trimmed.parse::<f32>() {
                    let raw = f.round() as i32;
                    if raw > 0 && raw != last_raw {
                        last_raw = raw;
                        inner_watcher.apply_brightness(raw);
                    }
                }
            }
        }
    });

    if let Err(err) = rsbinder::ProcessState::join_thread_pool() {
        log_e(&format!("[DisplayAdaptor] Binder thread pool exited: {:?}", err));
        std::process::exit(1);
    }
}
