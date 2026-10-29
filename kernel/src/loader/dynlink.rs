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
