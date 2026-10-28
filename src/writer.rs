use std::io;
use crate::logging::{log_d, log_e};

// brightness write function for sysfs
pub(crate) fn write_brightness(fd: i32, val: i32, last_val: &mut i32, dbg: bool) {
    if *last_val == val {
        return;
    }
    if dbg {
        log_d(&format!("[DisplayAdaptor] Writing brightness: {} -> {}", *last_val, val));
    }

    let s = format!("{}\n", val);
    let bytes = s.as_bytes();

    unsafe {
        libc::lseek(fd, 0, libc::SEEK_SET);
        let result = libc::write(fd, bytes.as_ptr() as *const _, bytes.len());
        if result < 0 {
            if dbg {
                log_e(&format!(
                    "[DisplayAdaptor] Write failed for value {}: {}",
                    val,
                    io::Error::last_os_error()
                ));
            }
        } else {
            *last_val = val;
        }
    }
}
