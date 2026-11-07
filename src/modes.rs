use std::fs::OpenOptions;
use std::os::unix::io::AsRawFd;
use std::sync::{Arc, RwLock};

use crate::constants::SERVICE_NAME;
use crate::logging::{log_d, log_e};
use crate::paths::{
    bright_path, display_type_prop, is_oplus_panel_prop, persist_bright_mode_prop,
    persist_dbg, persist_lux_aod_prop,
};
use crate::properties::{get_prop, get_prop_int};
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

    let (service, _inner) = PanelService::new(hw_min, hw_max, Arc::clone(&range_arc), file_fd);

    if let Err(err) = service.register() {
        log_e(&format!(
            "[DisplayAdaptor] Failed to register service {}: {:?}",
            SERVICE_NAME, err
        ));
        std::process::exit(1);
    }
    log_d(&format!("[DisplayAdaptor] Registered {}", SERVICE_NAME));

    // Inject brightness interceptor into system_server (libgui.so) or the
    // composer process (hwcomposer.*.so). The target process writes the
    // panel node directly, eliminating the need for property listeners.
    std::thread::spawn(move || {
        let mut installed_pid = 0;
        loop {
            if installed_pid == 0
                || !std::path::Path::new(&format!("/proc/{}", installed_pid)).exists()
            {
                match crate::gui_hook::install().or_else(|_| crate::hwc_hook::install()) {
                    Ok(report) => {
                        log_d(&format!(
                            "[Hook] installed: pid={} page={:#x} fd={} range={}..{}",
                            report.pid, report.page, report.led_fd, report.min_bri, report.max_bri
                        ));
                        installed_pid = report.pid;
                    }
                    Err(e) => {
                        log_e(&format!("[Hook] install failed: {}", e));
                        std::thread::sleep(std::time::Duration::from_secs(1));
                        continue;
                    }
                }
            }
            std::thread::sleep(std::time::Duration::from_secs(2));
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
