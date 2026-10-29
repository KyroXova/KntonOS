//! Dynamic Linker & Shared Library Loader Subsystem.
//!
//! Provides parsing of ELF64 PT_DYNAMIC section tags, dynamic symbol table
//! resolution, and shared library dependency tracking.

use alloc::string::String;
use alloc::vec::Vec;

pub const DT_NULL: i64 = 0;
pub const DT_NEEDED: i64 = 1;
pub const DT_PLTRELSZ: i64 = 2;
pub const DT_PLTGOT: i64 = 3;
pub const DT_HASH: i64 = 4;
pub const DT_STRTAB: i64 = 5;
pub const DT_SYMTAB: i64 = 6;
pub const DT_RELA: i64 = 7;
pub const DT_RELASZ: i64 = 8;
pub const DT_RELAENT: i64 = 9;
pub const DT_STRSZ: i64 = 10;
pub const DT_SYMENT: i64 = 11;
pub const DT_INIT: i64 = 12;
pub const DT_FINI: i64 = 13;
pub const DT_SONAME: i64 = 14;
pub const DT_RPATH: i64 = 15;
pub const DT_SYMBOLIC: i64 = 16;
pub const DT_REL: i64 = 17;
pub const DT_RELSZ: i64 = 18;
pub const DT_RELENT: i64 = 19;
pub const DT_PLTREL: i64 = 20;
pub const DT_DEBUG: i64 = 21;
pub const DT_TEXTREL: i64 = 22;
pub const DT_JMPREL: i64 = 23;
pub const DT_BIND_NOW: i64 = 24;
pub const DT_INIT_ARRAY: i64 = 25;
pub const DT_FINI_ARRAY: i64 = 26;
pub const DT_INIT_ARRAYSZ: i64 = 27;
pub const DT_FINI_ARRAYSZ: i64 = 28;
pub const DT_FLAGS: i64 = 30;

pub const R_X86_64_NONE: u32 = 0;
pub const R_X86_64_64: u32 = 1;
pub const R_X86_64_PC32: u32 = 2;
pub const R_X86_64_GOT32: u32 = 3;
pub const R_X86_64_PLT32: u32 = 4;
pub const R_X86_64_COPY: u32 = 5;
pub const R_X86_64_GLOB_DAT: u32 = 6;
pub const R_X86_64_JUMP_SLOT: u32 = 7;
pub const R_X86_64_RELATIVE: u32 = 8;
pub const R_X86_64_GOTPCREL: u32 = 9;
pub const R_X86_64_32: u32 = 10;
pub const R_X86_64_32S: u32 = 11;
pub const R_X86_64_16: u32 = 12;
pub const R_X86_64_PC16: u32 = 13;
pub const R_X86_64_8: u32 = 14;
pub const R_X86_64_PC8: u32 = 15;

pub const STB_LOCAL: u8 = 0;
pub const STB_GLOBAL: u8 = 1;
pub const STB_WEAK: u8 = 2;

pub const STT_NOTYPE: u8 = 0;
pub const STT_OBJECT: u8 = 1;
pub const STT_FUNC: u8 = 2;
pub const STT_SECTION: u8 = 3;
pub const STT_FILE: u8 = 4;
pub const STT_COMMON: u8 = 5;
pub const STT_TLS: u8 = 6;

pub const SHN_UNDEF: u16 = 0;

#[inline(always)]
pub const fn elf64_st_bind(info: u8) -> u8 {
    info >> 4
}

#[inline(always)]
pub const fn elf64_st_type(info: u8) -> u8 {
    info & 0x0F
}

#[inline(always)]
pub const fn elf64_st_info(bind: u8, st_type: u8) -> u8 {
    (bind << 4) | (st_type & 0x0F)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DynLinkError {
    MissingDynamicSection,
    InvalidDynamicEntry,
    MissingStringTable,
    MissingSymbolTable,
    SymbolNotFound,
    UnsupportedRelocation(u32),
    RelocationOverflow,
    LibraryNotFound,
    SegmentMappingFailed,
}

/// ELF64 Dynamic Array Tag Entry.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Elf64Dyn {
    pub d_tag: i64,
    pub d_val_or_ptr: u64,
}

/// ELF64 Symbol Table Entry.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Elf64Sym {
    pub st_name: u32,
    pub st_info: u8,
    pub st_other: u8,
    pub st_shndx: u16,
    pub st_value: u64,
    pub st_size: u64,
}

/// ELF64 Relocation Entry with Explicit Addend (RELA).
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Elf64Rela {
    pub r_offset: u64,
    pub r_info: u64,
    pub r_addend: i64,
}

impl Elf64Rela {
    #[inline(always)]
    pub const fn r_sym(&self) -> u32 {
        (self.r_info >> 32) as u32
    }

    #[inline(always)]
    pub const fn r_type(&self) -> u32 {
        (self.r_info & 0xFFFF_FFFF) as u32
    }

    #[inline(always)]
    pub const fn new(r_offset: u64, r_sym: u32, r_type: u32, r_addend: i64) -> Self {
        Self {
            r_offset,
            r_info: ((r_sym as u64) << 32) | (r_type as u64 & 0xFFFF_FFFF),
            r_addend,
        }
    }
}

/// Parsed metadata extracted from ELF PT_DYNAMIC segment.
#[derive(Debug, Clone, Default)]
pub struct DynamicSectionInfo {
    pub needed_libraries: Vec<String>,
    pub strtab_vaddr: u64,
    pub strtab_size: u64,
    pub symtab_vaddr: u64,
    pub syment_size: u64,
    pub rela_vaddr: u64,
    pub rela_size: u64,
    pub rela_ent: u64,
    pub pltgot_vaddr: u64,
    pub jmprel_vaddr: u64,
    pub pltrel_size: u64,
    pub hash_vaddr: u64,
    pub init_vaddr: u64,
    pub fini_vaddr: u64,
    pub soname: Option<String>,
}

impl DynamicSectionInfo {
    /// Parses an array of Elf64Dyn entries and resolves DT_NEEDED library names against string table bytes.
    pub fn parse(dyn_entries: &[Elf64Dyn], strtab_bytes: Option<&[u8]>) -> Result<Self, DynLinkError> {
        let mut info = Self::default();
        let mut needed_offsets = Vec::new();

        for entry in dyn_entries {
            match entry.d_tag {
                DT_NULL => break,
                DT_NEEDED => needed_offsets.push(entry.d_val_or_ptr as usize),
                DT_STRTAB => info.strtab_vaddr = entry.d_val_or_ptr,
                DT_STRSZ => info.strtab_size = entry.d_val_or_ptr,
                DT_SYMTAB => info.symtab_vaddr = entry.d_val_or_ptr,
                DT_SYMENT => info.syment_size = entry.d_val_or_ptr,
                DT_RELA => info.rela_vaddr = entry.d_val_or_ptr,
                DT_RELASZ => info.rela_size = entry.d_val_or_ptr,
                DT_RELAENT => info.rela_ent = entry.d_val_or_ptr,
                DT_PLTGOT => info.pltgot_vaddr = entry.d_val_or_ptr,
                DT_JMPREL => info.jmprel_vaddr = entry.d_val_or_ptr,
                DT_PLTRELSZ => info.pltrel_size = entry.d_val_or_ptr,
                DT_HASH => info.hash_vaddr = entry.d_val_or_ptr,
                DT_INIT => info.init_vaddr = entry.d_val_or_ptr,
                DT_FINI => info.fini_vaddr = entry.d_val_or_ptr,
                DT_SONAME => {
                    if let Some(bytes) = strtab_bytes {
                        if let Some(name) = extract_string(bytes, entry.d_val_or_ptr as usize) {
                            info.soname = Some(name);
                        }
                    }
                }
                _ => {}
            }
        }

        // Resolve DT_NEEDED strings if strtab is provided
        if let Some(bytes) = strtab_bytes {
            for offset in needed_offsets {
                if let Some(name) = extract_string(bytes, offset) {
                    info.needed_libraries.push(name);
                }
            }
        }

        Ok(info)
    }
}

/// Extracts a null-terminated UTF-8 string from a byte slice at the given offset.
pub fn extract_string(bytes: &[u8], offset: usize) -> Option<String> {
    if offset >= bytes.len() {
        return None;
    }
    let mut end = offset;
    while end < bytes.len() && bytes[end] != 0 {
        end += 1;
    }
    core::str::from_utf8(&bytes[offset..end]).ok().map(String::from)
}

/// Exported dynamic symbol descriptor.
#[derive(Debug, Clone)]
pub struct ExportedSymbol {
    pub name: String,
    pub address: u64,
    pub size: u64,
    pub is_function: bool,
}

/// Represents a loaded shared object / dynamic library.
#[derive(Debug, Clone)]
pub struct SharedLibrary {
    pub name: String,
    pub base_address: u64,
    pub size: u64,
    pub exports: Vec<ExportedSymbol>,
}

impl SharedLibrary {
    pub fn new(name: &str, base_address: u64, size: u64) -> Self {
        Self {
            name: String::from(name),
            base_address,
            size,
            exports: Vec::new(),
        }
    }

    pub fn add_export(&mut self, name: &str, offset: u64, size: u64, is_function: bool) {
        self.exports.push(ExportedSymbol {
            name: String::from(name),
            address: self.base_address + offset,
            size,
            is_function,
        });
    }

    pub fn find_symbol(&self, name: &str) -> Option<u64> {
        for sym in &self.exports {
            if sym.name == name {
                return Some(sym.address);
            }
        }
        None
    }
}

/// Looks up an external symbol across a list of loaded shared libraries.
pub fn resolve_symbol_in_libraries(name: &str, libraries: &[SharedLibrary]) -> Option<u64> {
    for lib in libraries {
        if let Some(addr) = lib.find_symbol(name) {
            return Some(addr);
        }
    }
    None
}

/// Finds a symbol in an ELF64 symbol table by name.
pub fn find_symbol_in_symtab<'a>(
    syms: &'a [Elf64Sym],
    strtab: &[u8],
    target_name: &str,
) -> Option<&'a Elf64Sym> {
    for sym in syms {
        if let Some(name) = extract_string(strtab, sym.st_name as usize) {
            if name == target_name {
                return Some(sym);
            }
        }
    }
    None
}

/// Verification unit test exercising ELF64 dynamic section parsing and symbol table resolution.
pub fn test_dynamic_parser_and_symbols() -> bool {
    // 1. Build mock dynamic string table:
    // Offset 0: \0
    // Offset 1: libc.so.6\0
    // Offset 11: libm.so.6\0
    // Offset 21: printf\0
    // Offset 28: sqrt\0
    let mut strtab = Vec::new();
    strtab.push(0);
    strtab.extend_from_slice(b"libc.so.6\0"); // offset 1
    strtab.extend_from_slice(b"libm.so.6\0"); // offset 11
    strtab.extend_from_slice(b"printf\0");    // offset 21
    strtab.extend_from_slice(b"sqrt\0");      // offset 28

    // 2. Build mock dynamic section tags
    let dyn_tags = [
        Elf64Dyn { d_tag: DT_NEEDED, d_val_or_ptr: 1 },  // libc.so.6
        Elf64Dyn { d_tag: DT_NEEDED, d_val_or_ptr: 11 }, // libm.so.6
        Elf64Dyn { d_tag: DT_STRTAB, d_val_or_ptr: 0x0040_2000 },
        Elf64Dyn { d_tag: DT_STRSZ, d_val_or_ptr: strtab.len() as u64 },
        Elf64Dyn { d_tag: DT_SYMTAB, d_val_or_ptr: 0x0040_1000 },
        Elf64Dyn { d_tag: DT_SYMENT, d_val_or_ptr: 24 },
        Elf64Dyn { d_tag: DT_RELA, d_val_or_ptr: 0x0040_3000 },
        Elf64Dyn { d_tag: DT_RELASZ, d_val_or_ptr: 48 },
        Elf64Dyn { d_tag: DT_RELAENT, d_val_or_ptr: 24 },
        Elf64Dyn { d_tag: DT_PLTGOT, d_val_or_ptr: 0x0040_4000 },
        Elf64Dyn { d_tag: DT_JMPREL, d_val_or_ptr: 0x0040_5000 },
        Elf64Dyn { d_tag: DT_PLTRELSZ, d_val_or_ptr: 48 },
        Elf64Dyn { d_tag: DT_HASH, d_val_or_ptr: 0x0040_6000 },
        Elf64Dyn { d_tag: DT_NULL, d_val_or_ptr: 0 },
    ];

    let info = match DynamicSectionInfo::parse(&dyn_tags, Some(&strtab)) {
        Ok(i) => i,
        Err(_) => return false,
    };

    assert_eq!(info.needed_libraries.len(), 2);
    assert_eq!(info.needed_libraries[0], "libc.so.6");
    assert_eq!(info.needed_libraries[1], "libm.so.6");
    assert_eq!(info.strtab_vaddr, 0x0040_2000);
    assert_eq!(info.symtab_vaddr, 0x0040_1000);
    assert_eq!(info.pltgot_vaddr, 0x0040_4000);
    assert_eq!(info.jmprel_vaddr, 0x0040_5000);

    // 3. Test symbol lookup in symbol table
    let syms = [
        Elf64Sym::default(), // index 0 (UNDEF)
        Elf64Sym {
            st_name: 21, // "printf"
            st_info: elf64_st_info(STB_GLOBAL, STT_FUNC),
            st_other: 0,
            st_shndx: 1,
            st_value: 0x1234,
            st_size: 64,
        },
        Elf64Sym {
            st_name: 28, // "sqrt"
            st_info: elf64_st_info(STB_GLOBAL, STT_FUNC),
            st_other: 0,
            st_shndx: 1,
            st_value: 0x5678,
            st_size: 32,
        },
    ];

    let printf_sym = find_symbol_in_symtab(&syms, &strtab, "printf").expect("printf must be found");
    assert_eq!(printf_sym.st_value, 0x1234);

    let sqrt_sym = find_symbol_in_symtab(&syms, &strtab, "sqrt").expect("sqrt must be found");
    assert_eq!(sqrt_sym.st_value, 0x5678);

    assert!(find_symbol_in_symtab(&syms, &strtab, "non_existent").is_none());

    // 4. Test multi-library export resolution
    let mut libc = SharedLibrary::new("libc.so.6", 0x0000_7FFF_1000_0000, 0x10000);
    libc.add_export("printf", 0x1234, 64, true);

    let mut libm = SharedLibrary::new("libm.so.6", 0x0000_7FFF_2000_0000, 0x10000);
    libm.add_export("sqrt", 0x5678, 32, true);

    let libs = [libc, libm];
    assert_eq!(
        resolve_symbol_in_libraries("printf", &libs),
        Some(0x0000_7FFF_1000_1234)
    );
    assert_eq!(
        resolve_symbol_in_libraries("sqrt", &libs),
        Some(0x0000_7FFF_2000_5678)
    );
    assert_eq!(resolve_symbol_in_libraries("missing", &libs), None);

    true
}

/// Resolves the absolute 64-bit virtual address for a symbol in relocations.
pub fn resolve_symbol_value(
    base_address: u64,
    sym: &Elf64Sym,
    strtab: &[u8],
    libraries: &[SharedLibrary],
) -> Result<u64, DynLinkError> {
    if sym.st_shndx == SHN_UNDEF {
        let name = extract_string(strtab, sym.st_name as usize)
            .ok_or(DynLinkError::SymbolNotFound)?;
        resolve_symbol_in_libraries(&name, libraries)
            .ok_or(DynLinkError::SymbolNotFound)
    } else {
        Ok(base_address + sym.st_value)
    }
}

/// Computes the target virtual address and patched value for an individual RELA entry.
pub fn compute_relocation(
    base_address: u64,
    rela: &Elf64Rela,
    symtab: &[Elf64Sym],
    strtab: &[u8],
    libraries: &[SharedLibrary],
) -> Result<(u64, u64), DynLinkError> {
    let target_vaddr = base_address + rela.r_offset;
    let r_type = rela.r_type();
    let r_sym = rela.r_sym() as usize;

    let computed_val = match r_type {
        R_X86_64_NONE => return Ok((target_vaddr, 0)),
        R_X86_64_RELATIVE => {
            (base_address as i64 + rela.r_addend) as u64
        }
        R_X86_64_64 => {
            if r_sym >= symtab.len() {
                return Err(DynLinkError::SymbolNotFound);
            }
            let sym = &symtab[r_sym];
            let sym_val = resolve_symbol_value(base_address, sym, strtab, libraries)?;
            (sym_val as i64 + rela.r_addend) as u64
        }
        R_X86_64_GLOB_DAT | R_X86_64_JUMP_SLOT => {
            if r_sym >= symtab.len() {
                return Err(DynLinkError::SymbolNotFound);
            }
            let sym = &symtab[r_sym];
            let sym_val = resolve_symbol_value(base_address, sym, strtab, libraries)?;
            (sym_val as i64 + rela.r_addend) as u64
        }
        R_X86_64_32 | R_X86_64_32S => {
            if r_sym >= symtab.len() {
                return Err(DynLinkError::SymbolNotFound);
            }
            let sym = &symtab[r_sym];
            let sym_val = resolve_symbol_value(base_address, sym, strtab, libraries)?;
            let res = (sym_val as i64 + rela.r_addend) as u64;
            res & 0xFFFF_FFFF
        }
        other => return Err(DynLinkError::UnsupportedRelocation(other)),
    };

    Ok((target_vaddr, computed_val))
}

/// Applies an array of RELA relocations to the active process virtual memory.
pub fn apply_relocations(
    base_address: u64,
    relas: &[Elf64Rela],
    symtab: &[Elf64Sym],
    strtab: &[u8],
    libraries: &[SharedLibrary],
) -> Result<usize, DynLinkError> {
    let mut count = 0;
    for rela in relas {
        let (target_vaddr, val) = compute_relocation(base_address, rela, symtab, strtab, libraries)?;

        let cr3: u64;
        unsafe {
            core::arch::asm!("mov {}, cr3", out(reg) cr3, options(nomem, nostack, preserves_flags));
            let ptm = crate::memory::vmm::PageTableManager::new((cr3 & !0xFFF) as *mut crate::memory::vmm::PageTable);
            if ptm.translate(crate::memory::vmm::VirtAddr(target_vaddr as usize)).is_some() {
                *(target_vaddr as *mut u64) = val;
            }
        }
        count += 1;
    }
    Ok(count)
}

/// Verification unit test exercising x86-64 RELA relocation records (RELATIVE, GLOB_DAT, JUMP_SLOT, 64).
pub fn test_relocation_engine() -> bool {
    let base_address = 0x0000_7FFF_1000_0000;

    let mut strtab = Vec::new();
    strtab.push(0);
    strtab.extend_from_slice(b"printf\0"); // offset 1
    strtab.extend_from_slice(b"sqrt\0");   // offset 8

    let symtab = [
        Elf64Sym::default(), // 0: UNDEF
        Elf64Sym {
            st_name: 1, // "printf"
            st_info: elf64_st_info(STB_GLOBAL, STT_FUNC),
            st_other: 0,
            st_shndx: SHN_UNDEF, // External
            st_value: 0,
            st_size: 0,
        },
        Elf64Sym {
            st_name: 8, // "sqrt"
            st_info: elf64_st_info(STB_GLOBAL, STT_FUNC),
            st_other: 0,
            st_shndx: SHN_UNDEF, // External
            st_value: 0,
            st_size: 0,
        },
        Elf64Sym {
            st_name: 0, // internal data symbol
            st_info: elf64_st_info(STB_LOCAL, STT_OBJECT),
            st_other: 0,
            st_shndx: 1,
            st_value: 0x0500,
            st_size: 8,
        },
    ];

    let mut libc = SharedLibrary::new("libc.so.6", 0x0000_7FFF_4000_0000, 0x10000);
    libc.add_export("printf", 0x0100, 64, true);

    let mut libm = SharedLibrary::new("libm.so.6", 0x0000_7FFF_5000_0000, 0x10000);
    libm.add_export("sqrt", 0x0200, 32, true);

    let libraries = [libc, libm];

    // 1. Test R_X86_64_RELATIVE (B + A)
    let rela_relative = Elf64Rela::new(0x1000, 0, R_X86_64_RELATIVE, 0x2000);
    let (target, val) = compute_relocation(base_address, &rela_relative, &symtab, &strtab, &libraries)
        .expect("R_X86_64_RELATIVE must succeed");
    assert_eq!(target, base_address + 0x1000);
    assert_eq!(val, base_address + 0x2000);

    // 2. Test R_X86_64_GLOB_DAT (GOT entry creation for printf)
    let rela_glob_dat = Elf64Rela::new(0x1008, 1, R_X86_64_GLOB_DAT, 0);
    let (target, val) = compute_relocation(base_address, &rela_glob_dat, &symtab, &strtab, &libraries)
        .expect("R_X86_64_GLOB_DAT must succeed");
    assert_eq!(target, base_address + 0x1008);
    assert_eq!(val, 0x0000_7FFF_4000_0100);

    // 3. Test R_X86_64_JUMP_SLOT (PLT slot binding for sqrt)
    let rela_jump_slot = Elf64Rela::new(0x1010, 2, R_X86_64_JUMP_SLOT, 0);
    let (target, val) = compute_relocation(base_address, &rela_jump_slot, &symtab, &strtab, &libraries)
        .expect("R_X86_64_JUMP_SLOT must succeed");
    assert_eq!(target, base_address + 0x1010);
    assert_eq!(val, 0x0000_7FFF_5000_0200);

    // 4. Test R_X86_64_64 (S + A on internal symbol)
    let rela_64 = Elf64Rela::new(0x1018, 3, R_X86_64_64, 0x08);
    let (target, val) = compute_relocation(base_address, &rela_64, &symtab, &strtab, &libraries)
        .expect("R_X86_64_64 must succeed");
    assert_eq!(target, base_address + 0x1018);
    assert_eq!(val, base_address + 0x0508);

    // 5. Test missing symbol returns error
    let rela_missing = Elf64Rela::new(0x1020, 99, R_X86_64_GLOB_DAT, 0);
    assert_eq!(
        compute_relocation(base_address, &rela_missing, &symtab, &strtab, &libraries),
        Err(DynLinkError::SymbolNotFound)
    );

    // 6. Test unsupported relocation returns error
    let rela_unsupported = Elf64Rela::new(0x1028, 0, 0xFE, 0);
    assert_eq!(
        compute_relocation(base_address, &rela_unsupported, &symtab, &strtab, &libraries),
        Err(DynLinkError::UnsupportedRelocation(0xFE))
    );

    true
}
