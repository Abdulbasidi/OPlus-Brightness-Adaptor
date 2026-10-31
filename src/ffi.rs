#![allow(dead_code, unused_imports)]

use std::os::raw::{c_char, c_int, c_uchar, c_void};

#[cfg(target_os = "android")]
unsafe extern "C" {
    pub(crate) fn __system_property_get(name: *const c_uchar, value: *mut c_uchar) -> c_int;
    pub(crate) fn __system_property_set(name: *const c_uchar, value: *const c_uchar) -> c_int;
    pub(crate) fn __system_property_find(name: *const c_char) -> *const c_void;
    pub(crate) fn __system_property_read_callback(
        pi: *const c_void,
        callback: extern "C" fn(cookie: *mut c_void, name: *const c_char, value: *const c_char, serial: u32),
        cookie: *mut c_void,
    );
    pub(crate) fn __system_property_wait(
        pi: *const c_void,
        old_serial: u32,
        new_serial_ptr: *mut u32,
        relative_timeout: *const c_void,
    ) -> bool;
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

#[cfg(not(target_os = "android"))]
pub(crate) unsafe fn __system_property_find(_name: *const c_char) -> *const c_void {
    std::ptr::null()
}

#[cfg(not(target_os = "android"))]
pub(crate) unsafe fn __system_property_read_callback(
    _pi: *const c_void,
    _callback: extern "C" fn(*mut c_void, *const c_char, *const c_char, u32),
    _cookie: *mut c_void,
) {}

#[cfg(not(target_os = "android"))]
pub(crate) unsafe fn __system_property_wait(
    _pi: *const c_void,
    _old_serial: u32,
    _new_serial_ptr: *mut u32,
    _relative_timeout: *const c_void,
) -> bool {
    false
}
