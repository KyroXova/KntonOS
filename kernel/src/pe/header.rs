//! PE32+ (64-bit) Executable and COFF header structures per Microsoft Open PE/COFF specification.

pub const IMAGE_DOS_SIGNATURE: u16 = 0x5A4D;       // "MZ"
pub const IMAGE_NT_SIGNATURE: u32 = 0x0000_4550;   // "PE\0\0"
pub const IMAGE_FILE_MACHINE_AMD64: u16 = 0x8664;  // x86-64
pub const IMAGE_NT_OPTIONAL_HDR64_MAGIC: u16 = 0x020B; // PE32+ (64-bit)

#[repr(C, packed)]
#[derive(Copy, Clone, Debug)]
pub struct ImageDosHeader {
    pub e_magic: u16,      // Magic number ("MZ")
    pub e_cblp: u16,
    pub e_cp: u16,
    pub e_crlc: u16,
    pub e_cparhdr: u16,
    pub e_minalloc: u16,
    pub e_maxalloc: u16,
    pub e_ss: u16,
    pub e_sp: u16,
    pub e_csum: u16,
    pub e_ip: u16,
    pub e_cs: u16,
    pub e_lfarlc: u16,
    pub e_ovno: u16,
    pub e_res: [u16; 4],
    pub e_oemid: u16,
    pub e_oeminfo: u16,
    pub e_res2: [u16; 10],
    pub e_lfanew: i32,     // File address of new exe header (NT Header offset)
}

#[repr(C, packed)]
#[derive(Copy, Clone, Debug)]
pub struct ImageFileHeader {
    pub machine: u16,
    pub number_of_sections: u16,
    pub time_date_stamp: u32,
    pub pointer_to_symbol_table: u32,
    pub number_of_symbols: u32,
    pub size_of_optional_header: u16,
    pub characteristics: u16,
}

#[repr(C, packed)]
#[derive(Copy, Clone, Debug, Default)]
pub struct ImageDataDirectory {
    pub virtual_address: u32,
    pub size: u32,
}

#[repr(C, packed)]
#[derive(Copy, Clone, Debug)]
pub struct ImageOptionalHeader64 {
    pub magic: u16,
    pub major_linker_version: u8,
    pub minor_linker_version: u8,
    pub size_of_code: u32,
    pub size_of_initialized_data: u32,
    pub size_of_uninitialized_data: u32,
    pub address_of_entry_point: u32,   // RVA of entry point
    pub base_of_code: u32,
    pub image_base: u64,               // Preferred load address
    pub section_alignment: u32,
    pub file_alignment: u32,
    pub major_operating_system_version: u16,
    pub minor_operating_system_version: u16,
    pub major_image_version: u16,
    pub minor_image_version: u16,
    pub major_subsystem_version: u16,
    pub minor_subsystem_version: u16,
    pub win32_version_value: u32,
    pub size_of_image: u32,            // Total virtual memory size required
    pub size_of_headers: u32,
    pub check_sum: u32,
    pub subsystem: u16,                // 2 = Windows GUI, 3 = Windows CUI (Console)
    pub dll_characteristics: u16,
    pub size_of_stack_reserve: u64,
    pub size_of_stack_commit: u64,
    pub size_of_heap_reserve: u64,
    pub size_of_heap_commit: u64,
    pub loader_flags: u32,
    pub number_of_rva_and_sizes: u32,
    pub data_directory: [ImageDataDirectory; 16],
}

#[repr(C, packed)]
#[derive(Copy, Clone, Debug)]
pub struct ImageSectionHeader {
    pub name: [u8; 8],
    pub virtual_size: u32,
    pub virtual_address: u32,          // RVA of section
    pub size_of_raw_data: u32,
    pub pointer_to_raw_data: u32,      // File offset
    pub pointer_to_relocations: u32,
    pub pointer_to_linenumbers: u32,
    pub number_of_relocations: u16,
    pub number_of_linenumbers: u16,
    pub characteristics: u32,          // Flags (e.g. MEM_EXECUTE, MEM_READ, MEM_WRITE)
}
