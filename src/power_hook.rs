//! Native power-mode hook for AOD.
//!
//! ColorOS asks SurfaceFlinger for `POWER_MODE_DOZE_SUSPEND` about a second
//! after the screen enters doze, which makes this panel stop showing the AOD.
//! The framework reaches the composer through
//! `SurfaceComposerClient::setDisplayPowerMode` imported by
//! `libandroid_runtime.so`. The import's GOT slot is redirected to a small
//! generator that rewrites mode 3 (`DOZE_SUSPEND`) to mode 1 (`DOZE`) and
//! tail-calls the original libgui implementation.
//!
//! Patching the GOT instead of the libgui entry point keeps the original
//! function usable: the stub still has to call it, and the entry point is
//! overwritten by a direct branch in the existing brightness hook.

use std::io;

use crate::elf::{Elf, R_AARCH64_GLOB_DAT, R_AARCH64_JUMP_SLOT};
use crate::hwc_hook::{peek64, poke_bytes, runtime_address, MapLine};
use crate::logging::{log_d, log_e};

/// Offset of the power stub inside the gui payload code page.
pub const STUB_POWER: u64 = 0x500;
/// Offset of the original `setDisplayPowerMode` address literal.
pub const POWER_ORIG: u64 = 0x51c;
/// Size of the stub including the address literal (0x500..0x524).
pub const STUB_LEN: usize = 0x24;

/// A resolved GOT slot plus the original function it must keep calling.
pub struct PowerTarget {
    /// Path of the library that owns the GOT slots (`libandroid_runtime.so`).
    pub lib_path: String,
    /// Runtime addresses of the imported symbol's GOT entries.
    pub got_slots: Vec<u64>,
    /// Runtime address of the libgui implementation.
    pub original: u64,
    /// Mangled name of the imported symbol, for logging.
    pub import_name: String,
}

/// AArch64 generator:
///
/// ```text
/// 0x500  bti  c
/// 0x504  cmp  w1, #3
/// 0x508  b.ne 0x514
/// 0x50c  mov  w1, #1
/// 0x510  ldr  x16, [pc, #12]   ; literal at 0x51c
/// 0x514  br   x16
/// 0x518  nop
/// 0x51c  .quad original
/// ```
///
/// The `int mode` argument is in `w1`; `x0` and every other argument are
/// preserved. `bti c` makes the indirect GOT branch a valid BTI landing pad
/// on devices that enforce branch target identification.
pub fn stub_bytes(original: u64) -> [u8; STUB_LEN] {
    let mut out = [0u8; STUB_LEN];
    let insns: [(usize, u32); 7] = [
        (0x00, 0xd503_245f), // bti c
        (0x04, 0x7100_0c3f), // cmp w1, #3
        (0x08, 0x5400_0061), // b.ne +12
        (0x0c, 0x5280_0021), // mov w1, #1
        (0x10, 0x5800_0070), // ldr x16, [pc, #12]
        (0x14, 0xd61f_0200), // br x16
        (0x18, 0xd503_201f), // nop
    ];
    for (off, insn) in insns {
        out[off..off + 4].copy_from_slice(&insn.to_le_bytes());
    }
    out[0x1c..0x24].copy_from_slice(&original.to_le_bytes());
    out
}

/// Resolve the `libandroid_runtime.so` import and the libgui implementation.
///
/// `maps` and `gui_elf` describe the target process and its `libgui.so`.
pub fn locate(maps: &[MapLine], gui_elf: &Elf, gui_path: &str) -> Option<PowerTarget> {
    let runtime_path = maps
        .iter()
        .find(|m| m.path.ends_with("/libandroid_runtime.so"))
        .map(|m| m.path.clone())?;

    let data = match std::fs::read(&runtime_path) {
        Ok(d) => d,
        Err(e) => {
            log_e(&format!(
                "[PowerHook] cannot read {}: {}",
                runtime_path, e
            ));
            return None;
        }
    };
    let runtime_elf = match Elf::parse(&data) {
        Ok(e) => e,
        Err(e) => {
            log_e(&format!("[PowerHook] cannot parse {}: {}", runtime_path, e));
            return None;
        }
    };

    log_d(&format!(
        "[PowerHook] {} parsed, {} relocation(s)",
        runtime_path,
        runtime_elf.relocations.len()
    ));
    let all_relocs = runtime_elf.find_import_got("setDisplayPowerMode");
    if all_relocs.is_empty() {
        log_e(&format!(
            "[PowerHook] {} has no setDisplayPowerMode GOT import",
            runtime_path
        ));
        return None;
    }
    // Prefer the SurfaceComposerClient import over any AIDL proxy import.
    let mut relocs: Vec<_> = all_relocs
        .iter()
        .filter(|r| r.symbol.contains("SurfaceComposerClient19setDisplayPowerMode"))
        .copied()
        .collect();
    if relocs.is_empty() {
        relocs = all_relocs;
    }

    let import_name = relocs[0].symbol.clone();
    let power_sym = match gui_elf
        .find_exact(&import_name)
        .or_else(|| gui_elf.find("SurfaceComposerClient19setDisplayPowerMode"))
        .or_else(|| gui_elf.find("setDisplayPowerMode"))
    {
        Some(s) if s.value != 0 => s,
        _ => {
            log_e(&format!(
                "[PowerHook] {} has no setDisplayPowerMode definition for {}",
                gui_path, import_name
            ));
            return None;
        }
    };
    let original = match runtime_address(maps, gui_elf, gui_path, power_sym.value) {
        Some(a) => a,
        None => {
            log_e(&format!(
                "[PowerHook] cannot map {} symbol {:#x}",
                gui_path, power_sym.value
            ));
            return None;
        }
    };

    let mut got_slots = Vec::new();
    for r in &relocs {
        match runtime_address(maps, &runtime_elf, &runtime_path, r.offset) {
            Some(addr) => {
                log_d(&format!(
                    "[PowerHook] import {} type {} GOT {:#x}",
                    r.symbol, r.r_type, addr
                ));
                got_slots.push(addr);
            }
            None => log_e(&format!(
                "[PowerHook] cannot map GOT slot {:#x} of {}",
                r.offset, runtime_path
            )),
        }
    }
    if got_slots.is_empty() {
        return None;
    }

    log_d(&format!(
        "[PowerHook] {} imports {} ({} slot(s)), original {:#x} in {}",
        runtime_path,
        import_name,
        got_slots.len(),
        original,
        gui_path
    ));

    Some(PowerTarget {
        lib_path: runtime_path,
        got_slots,
        original,
        import_name,
    })
}

/// Install (or re-install) the power stub and redirect every GOT slot.
pub fn install(tid: i32, code_page: u64, target: &PowerTarget) -> io::Result<()> {
    let stub = stub_bytes(target.original);
    if !poke_bytes(tid, code_page + STUB_POWER, &stub) {
        return Err(io::Error::new(
            io::ErrorKind::Other,
            "power stub write failed",
        ));
    }

    let stub_addr = code_page + STUB_POWER;
    let mut patched = 0usize;
    for slot in &target.got_slots {
        let old = peek64(tid, *slot).unwrap_or(0);
        if !poke_bytes(tid, *slot, &stub_addr.to_le_bytes()) {
            return Err(io::Error::new(
                io::ErrorKind::Other,
                format!("GOT slot {:#x} write failed", slot),
            ));
        }
        patched += 1;
        log_d(&format!(
            "[PowerHook] GOT {:#x}: {:#x} -> {:#x} ({} -> {})",
            slot, old, stub_addr, target.import_name, "DOZE"
        ));
    }
    if patched == 0 {
        return Err(io::Error::new(
            io::ErrorKind::Other,
            "no power GOT slot was patched",
        ));
    }

    log_d(&format!(
        "[PowerHook] installed in {}: {} GOT slot(s), stub {:#x}, original {:#x}",
        target.lib_path, patched, stub_addr, target.original
    ));
    Ok(())
}

/// True when a GOT slot already points at our stub.
pub fn is_installed(tid: i32, code_page: u64, target: &PowerTarget) -> bool {
    let stub_addr = code_page + STUB_POWER;
    target
        .got_slots
        .iter()
        .any(|slot| peek64(tid, *slot) == Some(stub_addr))
}

/// Expose the relocation kinds that are accepted for function imports.
pub fn is_function_import(r_type: u32) -> bool {
    r_type == R_AARCH64_JUMP_SLOT || r_type == R_AARCH64_GLOB_DAT
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stub_rewrites_doze_suspend_to_doze() {
        let bytes = stub_bytes(0x1122_3344_5566_7788);
        let u32_at = |off: usize| {
            u32::from_le_bytes([
                bytes[off],
                bytes[off + 1],
                bytes[off + 2],
                bytes[off + 3],
            ])
        };
        assert_eq!(u32_at(0x00), 0xd503_245f); // bti c
        assert_eq!(u32_at(0x04), 0x7100_0c3f); // cmp w1, #3
        assert_eq!(u32_at(0x08), 0x5400_0061); // b.ne +12
        assert_eq!(u32_at(0x0c), 0x5280_0021); // mov w1, #1
        assert_eq!(u32_at(0x10), 0x5800_0070); // ldr x16, [pc, #12]
        assert_eq!(u32_at(0x14), 0xd61f_0200); // br x16
        assert_eq!(u32_at(0x18), 0xd503_201f); // nop
        assert_eq!(
            u64::from_le_bytes(bytes[0x1c..0x24].try_into().unwrap()),
            0x1122_3344_5566_7788
        );
    }

    #[test]
    fn stub_layout_ends_at_0x524() {
        assert_eq!(STUB_POWER + STUB_LEN as u64, 0x524);
        assert_eq!(POWER_ORIG, STUB_POWER + 0x1c);
        assert!(is_function_import(R_AARCH64_JUMP_SLOT));
        assert!(is_function_import(R_AARCH64_GLOB_DAT));
        assert!(!is_function_import(1027)); // R_AARCH64_RELATIVE
    }
}
