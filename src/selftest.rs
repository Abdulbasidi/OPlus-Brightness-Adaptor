// Self-test for the composer-side payload blob.
//
// The payload cannot be exercised inside the composer on a stock policy:
// `hal_graphics_composer_default` has no `execmem`, so no executable page
// can be mapped there. The adaptor's own domain does have it, and the blob
// is position independent, so the same bytes can be run here to prove the
// machine code works: the support stub reports support, the set stub scales
// the normalized float and writes the panel level.
//
// The page layout matches the injection layout exactly - the payload starts
// at offset 0 of the page and every internal reference is page relative.

use std::io;
use std::os::raw::{c_int, c_void};

use crate::hwc_payload::{
    DPTR, FLOATS, PATHBUF, PAYLOAD, STATE, STATE_LEN, STUB_SET, STUB_SUPPORT, WRESULT,
};

extern "C" {
    fn mmap(
        addr: *mut c_void,
        length: usize,
        prot: c_int,
        flags: c_int,
        fd: c_int,
        offset: i64,
    ) -> *mut c_void;
    fn munmap(addr: *mut c_void, length: usize) -> c_int;
    fn close(fd: c_int) -> c_int;
}

const PROT_READ: c_int = 0x1;
const PROT_WRITE: c_int = 0x2;
const PROT_EXEC: c_int = 0x4;
const MAP_PRIVATE: c_int = 0x02;
const MAP_ANONYMOUS: c_int = 0x20;

const PAGE_SIZE: usize = 4096;

/// Result of one self-test run.
pub struct SelfTestReport {
    pub page: usize,
    pub state_page: usize,
    pub support_result: i32,
    pub support_flag: bool,
    pub set_result: i32,
    pub set_value: i32,
    pub off_result: i32,
    pub off_value: i32,
    pub node_after_set: i32,
    pub node_after_off: i32,
    pub write_result: i32,
}

/// Run the payload blob in this process and verify its behaviour.
pub fn run() -> io::Result<SelfTestReport> {
    let backlight = crate::hwc_hook::discover_backlight()?;

    let page = unsafe {
        mmap(
            std::ptr::null_mut(),
            PAGE_SIZE,
            PROT_READ | PROT_WRITE | PROT_EXEC,
            MAP_PRIVATE | MAP_ANONYMOUS,
            -1,
            0,
        )
    };
    if page.is_null() || page as isize == -1 {
        return Err(io::Error::last_os_error());
    }
    let page_usize = page as usize;

    // The state page is a separate writable mapping, exactly as in the
    // injected case.
    let state = unsafe {
        mmap(
            std::ptr::null_mut(),
            PAGE_SIZE,
            PROT_READ | PROT_WRITE,
            MAP_PRIVATE | MAP_ANONYMOUS,
            -1,
            0,
        )
    };
    if state.is_null() || state as isize == -1 {
        return Err(io::Error::last_os_error());
    }
    let state_usize = state as usize;

    unsafe {
        std::ptr::copy_nonoverlapping(PAYLOAD.as_ptr(), page as *mut u8, PAYLOAD.len());
    }

    // The path string lives in the state page, exactly as in the injected case.
    let node = backlight.node.clone();
    let node_bytes = node.as_bytes();
    if node_bytes.len() >= 256 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "backlight path too long for the page buffer",
        ));
    }
    unsafe {
        std::ptr::copy_nonoverlapping(
            node_bytes.as_ptr(),
            (state_usize + PATHBUF) as *mut u8,
            node_bytes.len(),
        );
        *((state_usize + PATHBUF + node_bytes.len()) as *mut u8) = 0;
    }

    // Open the node from this process. In the injected case the composer
    // opens it, so the write happens under the composer's own label.
    let path_c = format!("{}\0", node);
    let fd = unsafe { libc::open(path_c.as_ptr() as *const libc::c_char, libc::O_WRONLY | libc::O_CLOEXEC) };
    if fd < 0 {
        unsafe { munmap(page, PAGE_SIZE) };
        return Err(io::Error::last_os_error());
    }

    let mut data = [0u8; STATE_LEN];
    data[0..4].copy_from_slice(&fd.to_le_bytes());
    data[4..8].copy_from_slice(&backlight.min.to_le_bytes());
    data[8..12].copy_from_slice(&backlight.max.to_le_bytes());
    data[12..16].copy_from_slice(&(-1i32).to_le_bytes());
    unsafe {
        std::ptr::copy_nonoverlapping(
            data.as_ptr(),
            (state_usize + STATE) as *mut u8,
            STATE_LEN,
        );
        let mut floats = [0u8; 8];
        floats[0..4].copy_from_slice(&1.0f32.to_le_bytes());
        floats[4..8].copy_from_slice(&0.0f32.to_le_bytes());
        std::ptr::copy_nonoverlapping(
            floats.as_ptr(),
            (state_usize + FLOATS) as *mut u8,
            floats.len(),
        );
        // Point the stub at the state page.
        std::ptr::copy_nonoverlapping(
            (state_usize as u64).to_le_bytes().as_ptr(),
            (page_usize + DPTR) as *mut u8,
            8,
        );
    }

    type SupportFn = unsafe extern "C" fn(u64, u64, *mut u8) -> i32;
    type SetFn = unsafe extern "C" fn(u64, u64, f32) -> i32;

    let support_fn: SupportFn = unsafe { std::mem::transmute(page_usize + STUB_SUPPORT) };
    let set_fn: SetFn = unsafe { std::mem::transmute(page_usize + STUB_SET) };

    let mut flag: u8 = 0;
    let support_result = unsafe { support_fn(0, 0, &mut flag) };

    // Half scale: min + (max - min) * 0.5, truncated.
    let expected = backlight.min + ((backlight.max - backlight.min) as f32 * 0.5) as i32;
    let set_result = unsafe { set_fn(0, 0, 0.5f32) };
    let node_after_set = read_node(&node);

    // A negative value means "off" and must produce a zero write.
    let off_result = unsafe { set_fn(0, 0, -1.0f32) };
    let node_after_off = read_node(&node);

    let write_result = unsafe {
        let p = (state_usize + WRESULT) as *const i32;
        std::ptr::read_unaligned(p)
    };

    unsafe {
        close(fd);
        munmap(page, PAGE_SIZE);
        munmap(state, PAGE_SIZE);
    }

    Ok(SelfTestReport {
        page: page_usize,
        state_page: state_usize,
        support_result,
        support_flag: flag != 0,
        set_result,
        set_value: expected,
        off_result,
        off_value: 0,
        node_after_set,
        node_after_off,
        write_result,
    })
}

fn read_node(node: &str) -> i32 {
    std::fs::read_to_string(node)
        .ok()
        .and_then(|s| s.trim().parse::<i32>().ok())
        .unwrap_or(-1)
}
