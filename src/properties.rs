use std::ffi::{CString, CStr};
use std::os::raw::{c_char, c_uchar, c_void};
use crate::ffi::{
    __system_property_find, __system_property_get, __system_property_read_callback,
    __system_property_set, __system_property_wait,
};

pub struct PropValue {
    pub value: String,
    pub serial: u32,
}

extern "C" fn read_callback_trampoline(
    cookie: *mut c_void,
    _name: *const c_char,
    value: *const c_char,
    serial: u32,
) {
    unsafe {
        let pv = &mut *(cookie as *mut PropValue);
        pv.serial = serial;
        if !value.is_null() {
            let s = CStr::from_ptr(value).to_string_lossy();
            pv.value = s.into_owned();
        }
    }
}

pub(crate) fn find_prop(name: &str) -> Option<*const c_void> {
    let c_name = CString::new(name).ok()?;
    let pi = unsafe { __system_property_find(c_name.as_ptr()) };
    if pi.is_null() {
        None
    } else {
        Some(pi)
    }
}

pub(crate) fn read_prop_pi(pi: *const c_void) -> PropValue {
    let mut pv = PropValue {
        value: String::new(),
        serial: 0,
    };
    unsafe {
        __system_property_read_callback(
            pi,
            read_callback_trampoline,
            &mut pv as *mut PropValue as *mut c_void,
        );
    }
    pv
}

pub(crate) fn wait_prop_pi(pi: *const c_void, old_serial: u32) -> Option<u32> {
    let mut new_serial = 0u32;
    let ok = unsafe {
        __system_property_wait(pi, old_serial, &mut new_serial, std::ptr::null())
    };
    if ok {
        Some(new_serial)
    } else {
        None
    }
}

pub(crate) fn wait_any_prop(old_global_serial: u32) -> Option<u32> {
    let mut new_serial = 0u32;
    let ok = unsafe {
        __system_property_wait(std::ptr::null(), old_global_serial, &mut new_serial, std::ptr::null())
    };
    if ok {
        Some(new_serial)
    } else {
        None
    }
}

// system property utilities
pub(crate) fn get_prop(key: &str) -> Option<String> {
    const PROP_VALUE_MAX: usize = 92;
    let c_key = CString::new(key).ok()?;
    let mut buffer = vec![0u8; PROP_VALUE_MAX];
    let len = unsafe { __system_property_get(c_key.as_ptr() as *const c_uchar, buffer.as_mut_ptr() as *mut c_uchar) };
    if len > 0 {
        let c_str = unsafe { CStr::from_ptr(buffer.as_ptr() as *const c_char) };
        Some(c_str.to_string_lossy().into_owned())
    } else { None }
}
pub(crate) fn get_prop_int(key: &str) -> Option<i32> { get_prop(key)?.parse::<i32>().ok() }
pub(crate) fn set_prop(key: &str, val: &str) -> bool {
    let c_key = CString::new(key).ok().unwrap();
    let c_val = CString::new(val).ok().unwrap();
    unsafe { __system_property_set(c_key.as_ptr() as *const c_uchar, c_val.as_ptr() as *const c_uchar) == 0 }
}
