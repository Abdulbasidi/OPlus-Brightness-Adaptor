#![allow(unused_imports)]

use std::ffi::CString;
use std::os::raw::c_int;
use crate::paths::log_tag;
use crate::constants::{LOG_DEBUG, LOG_ERROR};

#[cfg(target_os = "android")]
pub(crate) fn log_write(level: c_int, msg: &str) {
    let tag = CString::new(log_tag()).unwrap();
    let fmt = CString::new("%s").unwrap();
    let c_msg = CString::new(msg).unwrap();
    unsafe { crate::ffi::__android_log_print(level, tag.as_ptr(), fmt.as_ptr(), c_msg.as_ptr()) };
}

#[cfg(not(target_os = "android"))]
pub(crate) fn log_write(level: c_int, msg: &str) {
    if level >= LOG_ERROR {
        eprintln!("[{}] {}", log_tag(), msg);
    } else {
        println!("[{}] {}", log_tag(), msg);
    }
}

pub(crate) fn log_d(msg: &str) { log_write(LOG_DEBUG, msg); }
pub(crate) fn log_e(msg: &str) { log_write(LOG_ERROR, msg); }
