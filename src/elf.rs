// Minimal ELF64 parser: enough to resolve symbols by name from an on-disk
// shared object, without any external crate.

const SHT_SYMTAB: u32 = 2;
const SHT_DYNSYM: u32 = 11;
const SHT_STRTAB: u32 = 3;
const SHT_RELA: u32 = 4;

/// `R_AARCH64_JUMP_SLOT`: a PLT entry's GOT slot.
pub const R_AARCH64_JUMP_SLOT: u32 = 1026;
/// `R_AARCH64_GLOB_DAT`: a non-PLT GOT slot.
pub const R_AARCH64_GLOB_DAT: u32 = 1025;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Symbol {
    pub name: String,
    /// Virtual address (st_value) of the symbol in the ELF image.
    pub value: u64,
    /// st_size, 0 when unknown.
    pub size: u64,
    /// True when the symbol came from .dynsym.
    pub dynamic: bool,
}

#[derive(Clone, Debug)]
pub struct Segment {
    pub vaddr: u64,
    pub offset: u64,
    pub filesz: u64,
    pub memsz: u64,
    pub flags: u32,
}

/// One `Elf64_Rela` entry with its symbol name resolved when present.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Relocation {
    /// Virtual address of the relocated slot (the GOT slot for imports).
    pub offset: u64,
    /// `ELF64_R_TYPE(r_info)`.
    pub r_type: u32,
    /// Name of the referenced dynamic symbol, empty for relative relocations.
    pub symbol: String,
}

pub struct Elf {
    pub symbols: Vec<Symbol>,
    pub segments: Vec<Segment>,
    pub relocations: Vec<Relocation>,
}

fn rd_u16(b: &[u8], off: usize) -> Option<u16> {
    let s = b.get(off..off + 2)?;
    Some(u16::from_le_bytes([s[0], s[1]]))
}

fn rd_u32(b: &[u8], off: usize) -> Option<u32> {
    let s = b.get(off..off + 4)?;
    Some(u32::from_le_bytes([s[0], s[1], s[2], s[3]]))
}

fn rd_u64(b: &[u8], off: usize) -> Option<u64> {
    let s = b.get(off..off + 8)?;
    let mut a = [0u8; 8];
    a.copy_from_slice(s);
    Some(u64::from_le_bytes(a))
}

fn cstr(b: &[u8], off: usize) -> String {
    let end = b[off..].iter().position(|&c| c == 0).unwrap_or(0);
    String::from_utf8_lossy(&b[off..off + end]).into_owned()
}

impl Elf {
    pub fn parse(data: &[u8]) -> Result<Elf, String> {
        if data.len() < 64 || &data[0..4] != b"\x7fELF" {
            return Err("not an ELF file".into());
        }
        if data[4] != 2 || data[5] != 1 {
            return Err("only 64-bit little-endian ELF is supported".into());
        }

        let e_shoff = rd_u64(data, 0x28).ok_or("truncated ELF header")?;
        let e_shentsize = rd_u16(data, 0x3a).ok_or("truncated ELF header")? as usize;
        let e_shnum = rd_u16(data, 0x3c).ok_or("truncated ELF header")? as usize;
        let _e_shstrndx = rd_u16(data, 0x3e).ok_or("truncated ELF header")? as usize;
        let e_phoff = rd_u64(data, 0x20).ok_or("truncated ELF header")?;
        let e_phentsize = rd_u16(data, 0x36).ok_or("truncated ELF header")? as usize;
        let e_phnum = rd_u16(data, 0x38).ok_or("truncated ELF header")? as usize;

        // Section headers.
        struct Sh {
            sh_type: u32,
            sh_offset: u64,
            sh_size: u64,
            sh_link: u32,
            sh_entsize: u64,
        }
        let mut sections = Vec::new();
        for i in 0..e_shnum {
            let base = (e_shoff as usize) + i * e_shentsize;
            let sh_type = rd_u32(data, base + 4).ok_or("truncated section header")?;
            let sh_flags = rd_u32(data, base + 8).unwrap_or(0);
            let _ = sh_flags;
            let sh_offset = rd_u64(data, base + 0x18).ok_or("truncated section header")?;
            let sh_size = rd_u64(data, base + 0x20).ok_or("truncated section header")?;
            let sh_link = rd_u32(data, base + 0x28).ok_or("truncated section header")?;
            let sh_entsize = rd_u64(data, base + 0x38).ok_or("truncated section header")?;
            sections.push(Sh { sh_type, sh_offset, sh_size, sh_link, sh_entsize });
        }

        // Program headers.
        let mut segments = Vec::new();
        for i in 0..e_phnum {
            let base = (e_phoff as usize) + i * e_phentsize;
            let p_type = rd_u32(data, base).ok_or("truncated program header")?;
            if p_type != 1 {
                continue; // PT_LOAD
            }
            let p_flags = rd_u32(data, base + 4).unwrap_or(0);
            let p_offset = rd_u64(data, base + 8).ok_or("truncated program header")?;
            let p_vaddr = rd_u64(data, base + 0x10).ok_or("truncated program header")?;
            let p_filesz = rd_u64(data, base + 0x20).ok_or("truncated program header")?;
            let p_memsz = rd_u64(data, base + 0x28).ok_or("truncated program header")?;
            segments.push(Segment { vaddr: p_vaddr, offset: p_offset, filesz: p_filesz, memsz: p_memsz, flags: p_flags });
        }

        // Symbols.
        let mut symbols = Vec::new();
        for (idx, sh) in sections.iter().enumerate() {
            if sh.sh_type != SHT_SYMTAB && sh.sh_type != SHT_DYNSYM {
                continue;
            }
            let strtab = match sections.get(sh.sh_link as usize) {
                Some(s) if s.sh_type == SHT_STRTAB => s,
                _ => continue,
            };
            let entsize = if sh.sh_entsize == 0 { 24 } else { sh.sh_entsize } as usize;
            let count = (sh.sh_size as usize) / entsize;
            for i in 0..count {
                let base = (sh.sh_offset as usize) + i * entsize;
                let st_name = rd_u32(data, base).unwrap_or(0);
                let st_info = data.get(base + 4).copied().unwrap_or(0);
                let st_shndx = rd_u16(data, base + 6).unwrap_or(0);
                let st_value = rd_u64(data, base + 8).unwrap_or(0);
                let st_size = rd_u64(data, base + 16).unwrap_or(0);
                if st_name == 0 || st_shndx == 0 {
                    continue; // undefined / local
                }
                if st_info & 0xf != 1 && st_info & 0xf != 2 {
                    continue; // STT_OBJECT / STT_FUNC only
                }
                let name_off = (strtab.sh_offset as usize) + st_name as usize;
                if name_off >= data.len() {
                    continue;
                }
                let name = cstr(data, name_off);
                if name.is_empty() {
                    continue;
                }
                symbols.push(Symbol {
                    name,
                    value: st_value,
                    size: st_size,
                    dynamic: sh.sh_type == SHT_DYNSYM,
                });
            }
            let _ = idx;
        }

        // Relocations. Only SHT_RELA is used on aarch64; the `.rela.plt` and
        // `.rela.dyn` sections both link to the dynamic symbol table.
        let mut relocations = Vec::new();
        for sh in &sections {
            if sh.sh_type != SHT_RELA {
                continue;
            }
            let symtab = match sections.get(sh.sh_link as usize) {
                Some(s) if s.sh_type == SHT_DYNSYM || s.sh_type == SHT_SYMTAB => s,
                _ => continue,
            };
            let strtab = match sections.get(symtab.sh_link as usize) {
                Some(s) if s.sh_type == SHT_STRTAB => s,
                _ => continue,
            };
            let entsize = if sh.sh_entsize == 0 { 24 } else { sh.sh_entsize } as usize;
            if entsize < 24 {
                continue;
            }
            let count = (sh.sh_size as usize) / entsize;
            let sym_entsize = if symtab.sh_entsize == 0 { 24 } else { symtab.sh_entsize } as usize;
            for i in 0..count {
                let base = (sh.sh_offset as usize) + i * entsize;
                let r_offset = rd_u64(data, base).unwrap_or(0);
                let r_info = rd_u64(data, base + 8).unwrap_or(0);
                let sym_idx = (r_info >> 32) as usize;
                let r_type = (r_info & 0xffff_ffff) as u32;
                let symbol = if sym_idx != 0 && sym_entsize >= 24 {
                    let sym_off = (symtab.sh_offset as usize) + sym_idx * sym_entsize;
                    let st_name = rd_u32(data, sym_off).unwrap_or(0) as usize;
                    let name_off = (strtab.sh_offset as usize).saturating_add(st_name);
                    if st_name != 0 && name_off < data.len() {
                        cstr(data, name_off)
                    } else {
                        String::new()
                    }
                } else {
                    String::new()
                };
                relocations.push(Relocation { offset: r_offset, r_type, symbol });
            }
        }

        Ok(Elf { symbols, segments, relocations })
    }

    /// First symbol whose name contains `needle`, preferring dynamic symbols.
    pub fn find(&self, needle: &str) -> Option<&Symbol> {
        let dyn_hit = self.symbols.iter().find(|s| s.dynamic && s.name.contains(needle));
        dyn_hit.or_else(|| self.symbols.iter().find(|s| s.name.contains(needle)))
    }

    /// Symbol with exactly this mangled name, preferring dynamic symbols.
    pub fn find_exact(&self, name: &str) -> Option<&Symbol> {
        let dyn_hit = self.symbols.iter().find(|s| s.dynamic && s.name == name);
        dyn_hit.or_else(|| self.symbols.iter().find(|s| s.name == name))
    }

    /// All symbols whose name contains `needle`.
    pub fn find_all(&self, needle: &str) -> Vec<&Symbol> {
        self.symbols.iter().filter(|s| s.name.contains(needle)).collect()
    }

    /// GOT relocations that import a symbol whose name contains `needle`.
    ///
    /// Only function/data imports (`JUMP_SLOT` and `GLOB_DAT`) are returned;
    /// relative relocations never carry a symbol name.
    pub fn find_import_got(&self, needle: &str) -> Vec<&Relocation> {
        self.relocations
            .iter()
            .filter(|r| {
                (r.r_type == R_AARCH64_JUMP_SLOT || r.r_type == R_AARCH64_GLOB_DAT)
                    && r.symbol.contains(needle)
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Build a tiny but valid ELF64 with one .dynsym/.dynstr pair.
    ///
    /// The field offsets follow the ELF64 spec: `sh_type` at +0x04,
    /// `sh_offset` at +0x18, `sh_size` at +0x20, `sh_link` at +0x28 and
    /// `sh_entsize` at +0x38; the same for the program headers.
    fn build_elf(sym_name: &str, value: u64, size: u64) -> Vec<u8> {
        let mut data = vec![0u8; 0x400];
        data[0..4].copy_from_slice(b"\x7fELF");
        data[4] = 2; // ELFCLASS64
        data[5] = 1; // ELFDATA2LSB
        data[6] = 1; // EV_CURRENT
        data[0x20..0x28].copy_from_slice(&0x40u64.to_le_bytes()); // e_phoff
        data[0x28..0x30].copy_from_slice(&0x100u64.to_le_bytes()); // e_shoff
        data[0x34..0x36].copy_from_slice(&0x40u16.to_le_bytes()); // e_phentsize
        data[0x38..0x3a].copy_from_slice(&1u16.to_le_bytes()); // e_phnum
        data[0x3a..0x3c].copy_from_slice(&0x40u16.to_le_bytes()); // e_shentsize
        data[0x3c..0x3e].copy_from_slice(&3u16.to_le_bytes()); // e_shnum
        data[0x3e..0x40].copy_from_slice(&2u16.to_le_bytes()); // e_shstrndx

        // PT_LOAD at 0x40
        data[0x40..0x44].copy_from_slice(&1u32.to_le_bytes()); // p_type = PT_LOAD
        data[0x44..0x48].copy_from_slice(&5u32.to_le_bytes()); // p_flags = R+X
        data[0x48..0x50].copy_from_slice(&0u64.to_le_bytes()); // p_offset
        data[0x50..0x58].copy_from_slice(&0u64.to_le_bytes()); // p_vaddr
        data[0x60..0x68].copy_from_slice(&0x1000u64.to_le_bytes()); // p_filesz
        data[0x68..0x70].copy_from_slice(&0x1000u64.to_le_bytes()); // p_memsz

        // Section headers at 0x100: [0]=null, [1]=dynsym, [2]=dynstr
        let mut sh = |base: usize, sh_type: u32, offset: u64, size: u64, link: u32, entsize: u64| {
            data[base + 0x04..base + 0x08].copy_from_slice(&sh_type.to_le_bytes());
            data[base + 0x18..base + 0x20].copy_from_slice(&offset.to_le_bytes());
            data[base + 0x20..base + 0x28].copy_from_slice(&size.to_le_bytes());
            data[base + 0x28..base + 0x2c].copy_from_slice(&link.to_le_bytes());
            data[base + 0x38..base + 0x40].copy_from_slice(&entsize.to_le_bytes());
        };
        sh(0x100, 0, 0, 0, 0, 0);
        sh(0x140, SHT_DYNSYM, 0x200, 24, 2, 24);
        sh(0x180, SHT_STRTAB, 0x300, 64, 0, 0);

        // dynstr
        data[0x300..0x300 + sym_name.len()].copy_from_slice(sym_name.as_bytes());
        data[0x300 + sym_name.len()] = 0;

        // dynsym entry
        data[0x200..0x204].copy_from_slice(&1u32.to_le_bytes()); // st_name
        data[0x204] = 0x12; // GLOBAL FUNC
        data[0x206..0x208].copy_from_slice(&1u16.to_le_bytes()); // st_shndx
        data[0x208..0x210].copy_from_slice(&value.to_le_bytes());
        data[0x210..0x218].copy_from_slice(&size.to_le_bytes());
        data
    }

    #[test]
    fn parses_dynamic_symbol() {
        let data = build_elf("_ZN11HWCMediator20displaySetBrightnessEP11hwc2_devicemf", 0x78d68, 8);
        let elf = Elf::parse(&data).expect("parse");
        let sym = elf.find("displaySetBrightness").expect("symbol found");
        assert_eq!(sym.value, 0x78d68);
        assert_eq!(sym.size, 8);
        assert!(sym.dynamic);
    }

    #[test]
    fn finds_by_mangled_substring() {
        let data = build_elf("_ZN11HWCMediator27displayGetBrightnessSupportEP11hwc2_devicemPb", 0x745e8, 360);
        let elf = Elf::parse(&data).expect("parse");
        let sym = elf.find("displayGetBrightnessSupport").expect("symbol found");
        assert_eq!(sym.value, 0x745e8);
        assert_eq!(elf.find("displaySetBrightness"), None);
    }

    /// Add a `.rela.plt`-like section that references the single dynamic
    /// symbol built by `build_elf`.
    fn with_rela(mut data: Vec<u8>, r_offset: u64, r_type: u32) -> Vec<u8> {
        data.resize(0x600, 0);
        data[0x3c..0x3e].copy_from_slice(&4u16.to_le_bytes()); // e_shnum
        let base = 0x1c0;
        data[base + 0x04..base + 0x08].copy_from_slice(&SHT_RELA.to_le_bytes());
        data[base + 0x18..base + 0x20].copy_from_slice(&0x400u64.to_le_bytes());
        data[base + 0x20..base + 0x28].copy_from_slice(&24u64.to_le_bytes());
        data[base + 0x28..base + 0x2c].copy_from_slice(&1u32.to_le_bytes()); // .dynsym
        data[base + 0x38..base + 0x40].copy_from_slice(&24u64.to_le_bytes());

        data[0x400..0x408].copy_from_slice(&r_offset.to_le_bytes());
        let r_info = (1u64 << 32) | r_type as u64;
        data[0x408..0x410].copy_from_slice(&r_info.to_le_bytes());
        data
    }

    #[test]
    fn parses_plt_relocation() {
        let data = build_elf("_ZN7android21SurfaceComposerClient19setDisplayPowerModeERKNS_2spINS_7IBinderEEEi", 0x1234, 8);
        let data = with_rela(data, 0x3f00, R_AARCH64_JUMP_SLOT);
        let elf = Elf::parse(&data).expect("parse");
        let relocs = elf.find_import_got("setDisplayPowerMode");
        assert_eq!(relocs.len(), 1);
        assert_eq!(relocs[0].offset, 0x3f00);
        assert_eq!(relocs[0].r_type, R_AARCH64_JUMP_SLOT);
        assert!(relocs[0].symbol.contains("setDisplayPowerMode"));
        assert!(elf.find_import_got("setDisplayBrightness").is_empty());
    }

    #[test]
    fn finds_glob_dat_relocation() {
        let data = build_elf("_ZN7android21SurfaceComposerClient19setDisplayPowerModeERKNS_2spINS_7IBinderEEEi", 0x1234, 8);
        let data = with_rela(data, 0x3f08, R_AARCH64_GLOB_DAT);
        let elf = Elf::parse(&data).expect("parse");
        let relocs = elf.find_import_got("setDisplayPowerMode");
        assert_eq!(relocs.len(), 1);
        assert_eq!(relocs[0].r_type, R_AARCH64_GLOB_DAT);
    }

    #[test]
    fn rejects_non_elf() {
        assert!(Elf::parse(b"not an elf").is_err());
    }

    #[test]
    fn parses_load_segments() {
        let data = build_elf("foo", 0x10, 4);
        let elf = Elf::parse(&data).expect("parse");
        assert_eq!(elf.segments.len(), 1);
        assert_eq!(elf.segments[0].filesz, 0x1000);
        assert_eq!(elf.segments[0].flags, 5);
    }
}
