#![allow(dead_code)]

// file paths & property keys
pub(crate) fn min_bright_path() -> &'static str { "/sys/class/leds/lcd-backlight/min_brightness" }
pub(crate) fn max_bright_path() -> &'static str { "/sys/class/leds/lcd-backlight/max_hw_brightness" }
pub(crate) fn bright_path() -> &'static str { "/sys/class/leds/lcd-backlight/brightness" }
pub(crate) fn sys_prop_max() -> &'static str { "sys.oplus.multibrightness" }
pub(crate) fn sys_prop_min() -> &'static str { "sys.oplus.multibrightness.min" }
pub(crate) fn persist_max() -> &'static str { "persist.sys.rianixia.multibrightness.max" }
pub(crate) fn persist_min() -> &'static str { "persist.sys.rianixia.multibrightness.min" }
pub(crate) fn log_tag() -> &'static str { "Xia-DisplayAdaptor" }
pub(crate) fn persist_dbg() -> &'static str { "persist.sys.rianixia.display-debug" } //set true for debug logs
pub(crate) fn oplus_bright_path() -> &'static str { "/data/addon/oplus_display/oplus_brightness" } // add for OS14 and under
pub(crate) fn persist_oplus_min() -> &'static str { "persist.sys.rianixia-display.min" }  // add for OS14 and under
pub(crate) fn persist_oplus_max() -> &'static str { "persist.sys.rianixia-display.max" } // add for OS14 and under
pub(crate) fn is_oplus_panel_prop() -> &'static str { "persist.sys.rianixia.is-displaypanel.support" } // add for OS14 and under
pub(crate) fn persist_custom_devmax_prop() -> &'static str { "persist.sys.rianixia.custom.devmax.brightness" } // adjust device max value for scaling
pub(crate) fn persist_custom_devmin_prop() -> &'static str { "persist.sys.rianixia.custom.devmin.brightness" } // adjust device min value for scaling
pub(crate) fn display_type_prop() -> &'static str { "persist.sys.rianixia.display.type" } // value = IPS or AMOLED (usually not needed)
pub(crate) fn persist_hw_min() -> &'static str { "persist.sys.rianixia.hw_min" } 
pub(crate) fn persist_hw_max() -> &'static str { "persist.sys.rianixia.hw_max" }
pub(crate) fn persist_bright_mode_prop() -> &'static str { "persist.sys.rianixia.brightness.mode" } // 0=Curved, 1=Linear, 2=Custom
pub(crate) fn persist_lux_aod_prop() -> &'static str { "persist.sys.rianixia.oplus.lux_aod" } // for lux aod logic
pub(crate) fn persist_lux_aod_brightness_prop() -> &'static str { "persist.sys.rianixia.oplus.lux_aod.brightness" } // New prop for overriding 1
