use std::ffi::{CString, CStr};
use std::os::raw::{c_char, c_uchar};
use crate::ffi::{__system_property_get, __system_property_set};

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
