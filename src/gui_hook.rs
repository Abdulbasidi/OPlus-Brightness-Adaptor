use std::io;

use crate::elf::Elf;
use crate::gui_payload::{
    DPTR, FLOATS, PATHBUF, PAYLOAD, STATE, STATE_LEN, STUB_SET, STUB_SUPPORT,
};
use crate::hwc_hook::{
    discover_backlight, encode_branch, find_free_holes, find_svc_gadget, poke_bytes, poke32,
    peek32, read_maps, remote_syscall, runtime_address, seize_and_stop, Backlight, InstallReport,
    MapLine, AT_FDCWD, B_OPCODE, B_RANGE, MAP_PRIVATE, MAP_PRIVATE_ANON, NR_MMAP, NR_MUNMAP,
    NR_OPENAT, O_RDONLY_CLOEXEC, O_WRONLY_CLOEXEC, PAGE_SIZE, PROT_RW, PROT_RX,
};
use crate::logging::log_d;

extern "C" {
    fn ptrace(
        request: std::os::raw::c_int,
        pid: std::os::raw::c_int,
        addr: *mut std::os::raw::c_void,
        data: *mut std::os::raw::c_void,
    ) -> std::os::raw::c_long;
}

const PTRACE_CONT: std::os::raw::c_int = 7;

pub struct GuiTarget {
    pub pid: i32,
    pub lib_path: String,
    pub set_brightness: u64,
    pub get_support: u64,
}

pub fn find_system_server() -> Option<(i32, String)> {
    for entry in std::fs::read_dir("/proc").ok()? {
        let entry = match entry.ok() {
            Some(e) => e,
            None => continue,
        };
        let name = entry.file_name();
        let name = name.to_string_lossy();
        let pid: i32 = match name.parse() {
            Ok(p) => p,
            Err(_) => continue,
        };
        let cmdline = std::fs::read(format!("/proc/{}/cmdline", pid)).unwrap_or_default();
        let cmdline = String::from_utf8_lossy(&cmdline).replace('\0', " ");
        if !cmdline.contains("system_server") {
            continue;
        }
        log_d(&format!("[GuiHook] candidate system_server pid {} ({})", pid, cmdline.trim()));
        let maps = match read_maps(pid) {
            Ok(m) => m,
            Err(_) => continue,
        };
        for m in &maps {
            if m.path.ends_with("/libgui.so") {
                return Some((pid, m.path.clone()));
            }
        }
    }
    None
}

fn resolve_symbols(maps: &[MapLine], elf: &Elf, lib_path: &str) -> Option<(u64, u64)> {
    let set = elf.find("SurfaceComposerClient20setDisplayBrightness")?;
    let sup = elf.find("SurfaceComposerClient27getDisplayBrightnessSupport")?;
    if set.value == 0 || sup.value == 0 {
        return None;
    }
    let set_addr = runtime_address(maps, elf, lib_path, set.value)?;
    let sup_addr = runtime_address(maps, elf, lib_path, sup.value)?;
    Some((set_addr, sup_addr))
}

pub fn locate() -> io::Result<GuiTarget> {
    let (pid, lib_path) = find_system_server().ok_or_else(|| {
        io::Error::new(io::ErrorKind::NotFound, "system_server process not found")
    })?;
    let data = std::fs::read(&lib_path)?;
    let elf = Elf::parse(&data).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
    let maps = read_maps(pid)?;
    let (set_brightness, get_support) = resolve_symbols(&maps, &elf, &lib_path).ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::NotFound,
            "SurfaceComposerClient symbols not found",
        )
    })?;
    Ok(GuiTarget {
        pid,
        lib_path,
        set_brightness,
        get_support,
    })
}

fn already_patched(tid: i32, target: &GuiTarget) -> Option<u64> {
    let word = peek32(tid, target.get_support)?;
    if (word & 0xfc00_0000) != B_OPCODE {
        return None;
    }
    let imm = (word & 0x03ff_ffff) as i32;
    let imm = if imm & 0x0200_0000 != 0 {
        imm - 0x0400_0000
    } else {
        imm
    };
    let dest = (target.get_support as i64 + (imm as i64) * 4) as u64;
    let sig = peek32(tid, dest)?;
    let expect = u32::from_le_bytes([PAYLOAD[0], PAYLOAD[1], PAYLOAD[2], PAYLOAD[3]]);
    if sig == expect {
        Some(dest)
    } else {
        None
    }
}

fn map_pages(
    tid: i32,
    svc: u64,
    target: &GuiTarget,
    hint: u64,
) -> io::Result<(u64, u64)> {
    let regs = crate::hwc_hook::get_regs(tid)
        .ok_or_else(|| io::Error::new(io::ErrorKind::Other, "cannot read registers"))?;
    let path = target.lib_path.as_bytes();
    if path.len() >= 512 {
        return Err(io::Error::new(io::ErrorKind::InvalidInput, "library path too long"));
    }
    let path_addr = regs.sp - 1024;
    if !poke_bytes(tid, path_addr, path) || !poke32(tid, path_addr + path.len() as u64, 0) {
        return Err(io::Error::new(io::ErrorKind::Other, "stack path write failed"));
    }

    let lib_fd = remote_syscall(
        tid,
        svc,
        NR_OPENAT,
        [AT_FDCWD, path_addr, O_RDONLY_CLOEXEC, 0, 0, 0],
    )
    .ok_or_else(|| io::Error::new(io::ErrorKind::Other, "remote openat failed"))? as i64;
    if lib_fd < 0 {
        return Err(io::Error::new(
            io::ErrorKind::Other,
            format!("remote openat failed: errno {}", -lib_fd),
        ));
    }

    let maps = read_maps(target.pid)?;
    let holes = find_free_holes(&maps, target.set_brightness, B_RANGE as u64);
    let mut code_page = 0u64;
    for candidate in holes.iter().take(16) {
        let page = remote_syscall(
            tid,
            svc,
            NR_MMAP,
            [
                *candidate,
                PAGE_SIZE as u64,
                PROT_RX,
                MAP_PRIVATE,
                lib_fd as u64,
                0,
            ],
        )
        .unwrap_or(0);
        if page == 0 || page > i64::MAX as u64 {
            continue;
        }
        let distance = (page as i64 - target.set_brightness as i64).abs();
        if distance <= B_RANGE {
            code_page = page;
            break;
        }
        let _ = remote_syscall(tid, svc, NR_MUNMAP, [page, PAGE_SIZE as u64, 0, 0, 0, 0]);
    }
    if code_page == 0 {
        return Err(io::Error::new(
            io::ErrorKind::Other,
            "no code page within branch reach",
        ));
    }

    let state_page = remote_syscall(
        tid,
        svc,
        NR_MMAP,
        [
            hint,
            PAGE_SIZE as u64,
            PROT_RW,
            MAP_PRIVATE_ANON,
            u64::MAX,
            0,
        ],
    )
    .ok_or_else(|| io::Error::new(io::ErrorKind::Other, "remote state mmap failed"))?;
    if state_page == 0 || state_page > i64::MAX as u64 {
        return Err(io::Error::new(
            io::ErrorKind::Other,
            format!("remote mmap failed: errno {}", -(state_page as i64)),
        ));
    }

    Ok((code_page, state_page))
}

fn peek64(pid: i32, addr: u64) -> Option<u64> {
    let lo = peek32(pid, addr)? as u64;
    let hi = peek32(pid, addr + 4)? as u64;
    Some(lo | (hi << 32))
}

fn inject(
    tid: i32,
    svc: u64,
    target: &GuiTarget,
    backlight: &Backlight,
) -> io::Result<InstallReport> {
    if let Some(code_page) = already_patched(tid, target) {
        let state_page = peek64(tid, code_page + DPTR as u64).unwrap_or(0);
        let support_calls = peek32(tid, state_page + 0x1c).unwrap_or(0);
        let set_calls = peek32(tid, state_page + 0x20).unwrap_or(0);
        let last_val = peek32(tid, state_page + 0x0c).unwrap_or(0);
        let wresult = peek64(tid, state_page + 0x18).unwrap_or(0);
        log_d(&format!(
            "[GuiHook] system_server already patched (entry {:#x} -> {:#x}, state={:#x}): support_calls={} set_calls={} last_val={} wresult={}",
            target.get_support, code_page, state_page, support_calls, set_calls, last_val, wresult
        ));
        return Ok(InstallReport {
            pid: target.pid,
            lib_path: target.lib_path.clone(),
            page: code_page,
            state_page: 0,
            get_support: target.get_support,
            set_brightness: target.set_brightness,
            led_fd: -1,
            min_bri: backlight.min,
            max_bri: backlight.max,
        });
    }

    let hint = target.set_brightness & !0xfffff;
    let (code_page, state_page) = map_pages(tid, svc, target, hint)?;
    log_d(&format!(
        "[GuiHook] code page {:#x}, state page {:#x}",
        code_page, state_page
    ));

    if !poke_bytes(tid, code_page, PAYLOAD) {
        return Err(io::Error::new(io::ErrorKind::Other, "payload write failed"));
    }
    let node = backlight.node.as_bytes();
    if !poke_bytes(tid, state_page + PATHBUF as u64, node)
        || !poke32(tid, state_page + PATHBUF as u64 + node.len() as u64, 0)
    {
        return Err(io::Error::new(io::ErrorKind::Other, "path write failed"));
    }

    let fd = remote_syscall(
        tid,
        svc,
        NR_OPENAT,
        [
            AT_FDCWD,
            state_page + PATHBUF as u64,
            O_WRONLY_CLOEXEC,
            0,
            0,
            0,
        ],
    )
    .ok_or_else(|| io::Error::new(io::ErrorKind::Other, "remote openat failed"))? as i32;
    if fd < 0 {
        return Err(io::Error::new(
            io::ErrorKind::Other,
            format!("remote openat failed: errno {}", -fd),
        ));
    }

    let mut state = [0u8; STATE_LEN];
    state[0..4].copy_from_slice(&fd.to_le_bytes());
    state[4..8].copy_from_slice(&backlight.min.to_le_bytes());
    state[8..12].copy_from_slice(&backlight.max.to_le_bytes());
    state[12..16].copy_from_slice(&(-1i32).to_le_bytes());
    if !poke_bytes(tid, state_page + STATE as u64, &state) {
        return Err(io::Error::new(io::ErrorKind::Other, "state write failed"));
    }
    let mut floats = [0u8; 8];
    floats[0..4].copy_from_slice(&1.0f32.to_le_bytes());
    floats[4..8].copy_from_slice(&0.0f32.to_le_bytes());
    let _ = poke_bytes(tid, state_page + FLOATS as u64, &floats);

    if !poke_bytes(tid, code_page + DPTR as u64, &state_page.to_le_bytes()) {
        return Err(io::Error::new(io::ErrorKind::Other, "dptr write failed"));
    }

    let branch_support = encode_branch(target.get_support, code_page + STUB_SUPPORT as u64)
        .ok_or_else(|| io::Error::new(io::ErrorKind::Other, "support stub out of reach"))?;
    let branch_set = encode_branch(target.set_brightness, code_page + STUB_SET as u64)
        .ok_or_else(|| io::Error::new(io::ErrorKind::Other, "set stub out of reach"))?;

    if !poke32(tid, target.get_support, branch_support)
        || !poke32(tid, target.set_brightness, branch_set)
    {
        return Err(io::Error::new(io::ErrorKind::Other, "entry point patch failed"));
    }

    log_d(&format!(
        "[GuiHook] patched system_server pid {} ({}): support={:#x} set={:#x} code={:#x} state={:#x} fd={} range={}..{}",
        target.pid,
        target.lib_path,
        target.get_support,
        target.set_brightness,
        code_page,
        state_page,
        fd,
        backlight.min,
        backlight.max
    ));

    Ok(InstallReport {
        pid: target.pid,
        lib_path: target.lib_path.clone(),
        page: code_page,
        state_page,
        get_support: target.get_support,
        set_brightness: target.set_brightness,
        led_fd: fd,
        min_bri: backlight.min,
        max_bri: backlight.max,
    })
}

pub fn install() -> io::Result<InstallReport> {
    let target = locate()?;
    let backlight = discover_backlight()?;
    let tid = seize_and_stop(target.pid)
        .ok_or_else(|| io::Error::new(io::ErrorKind::Other, "cannot stop system_server thread"))?;
    let maps = read_maps(target.pid)?;
    let svc = find_svc_gadget(&maps)
        .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "no svc gadget in target"))?;

    let res = inject(tid, svc, &target, &backlight);
    unsafe { ptrace(PTRACE_CONT, tid, std::ptr::null_mut(), std::ptr::null_mut()) };
    res
}

#[cfg(test)]
mod tests {
    use crate::gui_payload::{DPTR, PAYLOAD, STUB_SET, STUB_SUPPORT};

    #[test]
    fn gui_payload_layout_matches() {
        assert_eq!(PAYLOAD.len(), 1032);
        assert_eq!(STUB_SUPPORT, 0x000);
        assert_eq!(STUB_SET, 0x200);
        assert_eq!(DPTR, 0x400);
    }

    #[test]
    fn gui_payload_stub_a_encodings() {
        let u32_at = |off: usize| {
            u32::from_le_bytes([
                PAYLOAD[off],
                PAYLOAD[off + 1],
                PAYLOAD[off + 2],
                PAYLOAD[off + 3],
            ])
        };
        assert_eq!(u32_at(0x000), 0x1000_2005);
        assert_eq!(u32_at(0x004), 0xf940_00a4);
        assert_eq!(u32_at(0x014), 0x5280_0020);
        assert_eq!(u32_at(0x018), 0xd65f_03c0);
    }

    #[test]
    fn gui_payload_stub_b_encodings() {
        let u32_at = |off: usize| {
            u32::from_le_bytes([
                PAYLOAD[off],
                PAYLOAD[off + 1],
                PAYLOAD[off + 2],
                PAYLOAD[off + 3],
            ])
        };
        assert_eq!(u32_at(0x200), 0x1000_1005);
        assert_eq!(u32_at(0x204), 0xf940_00a4);
        assert_eq!(u32_at(0x208), 0xb400_07a1);
    }
}
