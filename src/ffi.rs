#![allow(dead_code, unused_imports)]

use std::os::raw::{c_char, c_int, c_uchar};

#[cfg(target_os = "android")]
unsafe extern "C" {
    pub(crate) fn __system_property_get(name: *const c_uchar, value: *mut c_uchar) -> c_int;
    pub(crate) fn __system_property_set(name: *const c_uchar, value: *const c_uchar) -> c_int;
    pub(crate) fn __android_log_print(
        prio: c_int,
        tag: *const c_char,
        fmt: *const c_char,
        ...
    ) -> c_int;
}

#[cfg(not(target_os = "android"))]
pub(crate) unsafe fn __system_property_get(_name: *const c_uchar, _value: *mut c_uchar) -> c_int {
    0
}

#[cfg(not(target_os = "android"))]
pub(crate) unsafe fn __system_property_set(_name: *const c_uchar, _value: *const c_uchar) -> c_int {
    0
}
