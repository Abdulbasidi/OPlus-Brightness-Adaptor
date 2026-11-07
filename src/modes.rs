use std::fs::OpenOptions;
use std::os::unix::io::AsRawFd;
use std::sync::{Arc, RwLock};

use crate::constants::{BRIGHTNESS_PROP, SCREEN_STATE_PROP, SERVICE_NAME};
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
        if let Some(v) = parse_fp_type(&val_str) {
            return v;
        }
    }
    crate::constants::FP_TYPE_UNSET
}

/// Parse an FP_TYPE property value. `-1` (FP_TYPE_UNSET) is accepted so consumers
/// fall back to the Oplus feature configuration instead of reading a hardcoded type.
pub fn parse_fp_type(val_str: &str) -> Option<i32> {
    let trimmed = val_str.trim();
    let parsed = if let Some(hex) = trimmed.strip_prefix("0x").or_else(|| trimmed.strip_prefix("0X")) {
        i32::from_str_radix(hex, 16).ok()
    } else {
        trimmed.parse::<i32>().ok()
    };
    parsed.filter(|&v| v > 0 || v == crate::constants::FP_TYPE_UNSET)
}

// main runner
pub fn run() {
    // Diagnostic mode: install the composer hook and exit without registering
    // the Binder service. Used to verify the injection on a device without
    // disturbing the running adaptor.
    if std::env::args().any(|a| a == "--selftest") {
        match crate::selftest::run() {
            Ok(r) => log_d(&format!(
                "[HwcSelftest] page={:#x} state={:#x} support_ret={} support={} set_ret={} set_val={} node_set={} off_ret={} node_off={} write_ret={}",
                r.page, r.state_page, r.support_result, r.support_flag, r.set_result, r.set_value,
                r.node_after_set, r.off_result, r.node_after_off, r.write_result
            )),
            Err(e) => log_e(&format!("[HwcSelftest] failed: {}", e)),
        }
        return;
    }

    if std::env::args().any(|a| a == "--gui-hook") {
        match crate::gui_hook::install() {
            Ok(report) => log_d(&format!(
                "[GuiHook] installed: pid={} page={:#x} fd={} range={}..{}",
                report.pid, report.page, report.led_fd, report.min_bri, report.max_bri
            )),
            Err(e) => log_e(&format!("[GuiHook] install failed: {}", e)),
        }
        return;
    }

    if std::env::args().any(|a| a == "--hook-only") {
        let res = crate::gui_hook::install().or_else(|_| crate::hwc_hook::install());
        match res {
            Ok(report) => log_d(&format!(
                "[Hook] installed: pid={} page={:#x} fd={} range={}..{}",
                report.pid, report.page, report.led_fd, report.min_bri, report.max_bri
            )),
            Err(e) => log_e(&format!("[Hook] install failed: {}", e)),
        }
        return;
    }

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

    if let Err(err) = service.register() {
        log_e(&format!(
            "[DisplayAdaptor] Failed to register service {}: {:?}",
            SERVICE_NAME, err
        ));
        std::process::exit(1);
    }
    log_d(&format!("[DisplayAdaptor] Registered {}", SERVICE_NAME));

    // Install the composer hook once. The composer process is located at
    // runtime, the two HWC2 entry points are resolved from the library's own
    // symbol table, and the payload is patched into the composer process. The
    // patched composer writes the panel node itself, so the adaptor needs no
    // brightness property, no Light HAL and no background thread at all.
    std::thread::spawn(move || {
        // Retry until system_server comes up in late-init.
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(120);
        loop {
            let res = crate::gui_hook::install().or_else(|_| crate::hwc_hook::install());
            match res {
                Ok(report) => {
                    log_d(&format!(
                        "[Hook] installed: pid={} page={:#x} fd={} range={}..{}",
                        report.pid, report.page, report.led_fd, report.min_bri, report.max_bri
                    ));
                    return;
                }
                Err(e) => {
                    if std::time::Instant::now() >= deadline {
                        log_e(&format!("[Hook] install failed: {}", e));
                        return;
                    }
                    std::thread::sleep(std::time::Duration::from_secs(1));
                }
            }
        }
    });

    // Brightness input. The composer hook is the primary path, but the
    // framework only produces an HWC2 brightness transaction on ports whose
    // composer exposes the V2_3+ API. Where it does not (this port declares
    // 2.1, and the service runs on libhwc2on1adapter), the framework's only
    // brightness output is this property, so the same value is applied here.
    // The wait is a futex sleep on the property slot: the thread is woken by
    // the kernel when the value changes and never polls.
    let inner_watcher = Arc::clone(&inner);
    std::thread::spawn(move || {
        log_d(&format!(
            "[DisplayAdaptor] Starting futex event listener for {}",
            BRIGHTNESS_PROP
        ));

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
            if trimmed.is_empty() {
                continue;
            }
            if let Ok(f) = trimmed.parse::<f32>() {
                let raw = f.round() as i32;
                let hw_invalid = inner_watcher
                    .last_hw_val
                    .lock()
                    .map(|v| *v < 0)
                    .unwrap_or(false);
                if raw > 0 && (raw != last_raw || hw_invalid) {
                    last_raw = raw;
                    inner_watcher.apply_brightness(raw);
                }
            }
        }
    });

    // Event-driven kernel futex listener for boot completion.
    // When sys.boot_completed reaches "1", force fetches and restores the brightness
    // so the panel is never left in a black screen state after reboot.
    let inner_boot = Arc::clone(&inner);
    std::thread::spawn(move || {
        log_d("[DisplayAdaptor] Starting kernel futex listener for sys.boot_completed");

        let mut pi = find_prop("sys.boot_completed");
        let mut global_serial = 0u32;
        while pi.is_none() {
            if let Some(s) = wait_any_prop(global_serial) {
                global_serial = s;
            }
            pi = find_prop("sys.boot_completed");
        }
        let pi = pi.unwrap();

        let mut pv = read_prop_pi(pi);
        let mut serial = pv.serial;
        while pv.value.trim() != "1" {
            if let Some(new_serial) = wait_prop_pi(pi, serial) {
                serial = new_serial;
                pv = read_prop_pi(pi);
            }
        }

        log_d("[DisplayAdaptor] sys.boot_completed=1 received, forcing brightness restore");
        std::thread::sleep(std::time::Duration::from_millis(300));
        inner_boot.force_restore_brightness("boot_completed_initial");

        // Follow-up restore after 1.2s to guarantee steady-state brightness after keyguard transition
        std::thread::sleep(std::time::Duration::from_millis(1200));
        inner_boot.force_restore_brightness("boot_completed_delayed");
    });

    // Event-driven kernel futex listener for screen state transitions (Screen ON / OFF / DOZE).
    // The Oplus DisplayPowerController writes `debug.tracing.screen_state`:
    // "1" = Screen OFF / Doze
    // "2" = Screen ON
    // When the screen wakes back up ("2"), we force restore brightness immediately so the panel
    // never stays stuck at 0 or dim even if debug.tracing.screen_brightness was unchanged.
    let inner_state = Arc::clone(&inner);
    std::thread::spawn(move || {
        log_d("[DisplayAdaptor] Starting kernel futex listener for screen_state");

        let mut pi = find_prop(SCREEN_STATE_PROP);
        let mut global_serial = 0u32;
        while pi.is_none() {
            if let Some(s) = wait_any_prop(global_serial) {
                global_serial = s;
            }
            pi = find_prop(SCREEN_STATE_PROP);
        }
        let pi = pi.unwrap();

        let initial = read_prop_pi(pi);
        let mut serial = initial.serial;
        let mut last_state = initial.value.trim().to_string();

        // Apply the state the screen is actually in at startup, so the panel is
        // not left lit while the screen is off.
        let initial_state: i32 = last_state
            .parse()
            .unwrap_or(crate::constants::SCREEN_STATE_ON);
        inner_state.apply_screen_state(initial_state, dbg_on());

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

            let state = pv.value.trim();
            if state != last_state {
                log_d(&format!(
                    "[DisplayAdaptor] screen_state transitioned: {} -> {}",
                    last_state, state
                ));
                last_state = state.to_string();

                let state_val: i32 = state.parse().unwrap_or(crate::constants::SCREEN_STATE_ON);
                inner_state.apply_screen_state(state_val, dbg_on());
            }
        }
    });

    if let Err(err) = rsbinder::ProcessState::join_thread_pool() {
        log_e(&format!("[DisplayAdaptor] Binder thread pool exited: {:?}", err));
        std::process::exit(1);
    }
}

#[cfg(test)]
mod tests {
    use super::parse_fp_type;
    use crate::constants::FP_TYPE_UNSET;

    #[test]
    fn fp_type_minus_one_is_accepted() {
        assert_eq!(parse_fp_type("-1"), Some(FP_TYPE_UNSET));
        assert_eq!(parse_fp_type(" -1 "), Some(FP_TYPE_UNSET));
    }

    #[test]
    fn fp_type_positive_and_hex_are_accepted() {
        assert_eq!(parse_fp_type("4"), Some(4));
        assert_eq!(parse_fp_type("0x10"), Some(16));
    }

    #[test]
    fn fp_type_zero_and_negative_other_than_unset_are_rejected() {
        assert_eq!(parse_fp_type("0"), None);
        assert_eq!(parse_fp_type("-2"), None);
        assert_eq!(parse_fp_type("abc"), None);
    }
}
