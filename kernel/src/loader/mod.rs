//! Executable Binary Loader Subsystem for KntonOS.
//!
//! Provides parsing, segment mapping, System V AMD64 ABI user stack setup,
//! and process execution for standard ELF64 binaries.

pub mod elf;
pub mod dynlink;

pub use dynlink::{
    apply_relocations, compute_relocation, extract_string, find_symbol_in_symtab,
    resolve_symbol_in_libraries, resolve_symbol_value, test_dynamic_linker_subsystem,
    test_dynamic_parser_and_symbols, test_relocation_engine,
    test_shared_library_loading_and_got_plt, DynamicLinker, DynamicSectionInfo, DynLinkError,
    Elf64Dyn, Elf64Rela, Elf64Sym, ExportedSymbol, SharedLibrary, DYNAMIC_LINKER, DT_BIND_NOW, DT_DEBUG, DT_FINI, DT_FINI_ARRAY,
    DT_FINI_ARRAYSZ, DT_FLAGS, DT_HASH, DT_INIT, DT_INIT_ARRAY, DT_INIT_ARRAYSZ, DT_JMPREL,
    DT_NEEDED, DT_NULL, DT_PLTGOT, DT_PLTREL, DT_PLTRELSZ, DT_REL, DT_RELA, DT_RELAENT,
    DT_RELASZ, DT_RELENT, DT_RELSZ, DT_RPATH, DT_SONAME, DT_STRSZ, DT_STRTAB, DT_SYMBOLIC,
    DT_SYMENT, DT_SYMTAB, DT_TEXTREL, R_X86_64_16, R_X86_64_32, R_X86_64_32S, R_X86_64_64,
    R_X86_64_8, R_X86_64_COPY, R_X86_64_GLOB_DAT, R_X86_64_GOT32, R_X86_64_GOTPCREL,
    R_X86_64_JUMP_SLOT, R_X86_64_NONE, R_X86_64_PC16, R_X86_64_PC32, R_X86_64_PC8,
    R_X86_64_PLT32, R_X86_64_RELATIVE, SHN_UNDEF, STB_GLOBAL, STB_LOCAL, STB_WEAK, STT_COMMON,
    STT_FILE, STT_FUNC, STT_NOTYPE, STT_OBJECT, STT_SECTION, STT_TLS,
};

pub use elf::{
    test_elf_parser, test_elf_segment_mapping, Elf64Auxv, Elf64Ehdr, Elf64Phdr, ElfError,
    ElfLoader, ElfProcess, ElfSpawnError, ElfStackInitializer, ElfStackSetup, LoadedElfImage,
    ParsedElf64, ProcessManager, AT_BASE, AT_CLKTCK, AT_EGID, AT_ENTRY, AT_EUID, AT_EXECFD,
    AT_FLAGS, AT_GID, AT_HWCAP, AT_IGNORE, AT_NOTELF, AT_NULL, AT_PAGESZ, AT_PHDR, AT_PHENT,
    AT_PHNUM, AT_PLATFORM, AT_RANDOM, AT_UID, ELF_CLASS_64, ELF_DATA_2LSB, ELF_MAGIC,
    ELF_OSABI_SYSV, EM_X86_64, ET_DYN, ET_EXEC, PF_R, PF_W, PF_X, PT_DYNAMIC, PT_INTERP,
    PT_LOAD,
};

/// Initializes the ELF64 executable loader subsystem.
pub fn init() {
    crate::serial_write_str("[KNT-ELF-INIT] Initializing ELF64 executable loader subsystem...\r\n");
}

/// Verification test suite exercising ELF64 header validation, segment mapping,
/// System V ABI stack formatting, and user process execution.
pub fn test_loader_subsystem() -> bool {
    crate::serial_write_str("[KNT-ELF-TEST] Running ELF64 loader and userland execution test suite...\r\n");

    // 1. Verify ELF64 header and program header table parser
    if !test_elf_parser() {
        crate::serial_write_str("[KNT-PANIC] ELF64 parser unit test failed!\r\n");
        return false;
    }

    // 2. Verify PT_LOAD segment mapping and BSS zero-fill
    if !test_elf_segment_mapping() {
        crate::serial_write_str("[KNT-PANIC] ELF64 segment mapping test failed!\r\n");
        return false;
    }

    // 3. Verify End-to-End ELF user process spawning and System V stack setup
    {
        let mut mock_elf = [0u8; 256];
        mock_elf[0..4].copy_from_slice(&ELF_MAGIC);
        mock_elf[4] = ELF_CLASS_64;
        mock_elf[5] = ELF_DATA_2LSB;
        mock_elf[6] = 1; // EV_CURRENT
        mock_elf[7] = ELF_OSABI_SYSV;

        let ehdr_ptr = mock_elf.as_mut_ptr() as *mut Elf64Ehdr;
        unsafe {
            (*ehdr_ptr).e_type = ET_EXEC;
            (*ehdr_ptr).e_machine = EM_X86_64;
            (*ehdr_ptr).e_version = 1;
            (*ehdr_ptr).e_entry = 0x0040_0000;
            (*ehdr_ptr).e_phoff = 64;
            (*ehdr_ptr).e_ehsize = 64;
            (*ehdr_ptr).e_phentsize = 56;
            (*ehdr_ptr).e_phnum = 1;
        }

        let phdr_ptr = unsafe { mock_elf.as_mut_ptr().add(64) as *mut Elf64Phdr };
        unsafe {
            (*phdr_ptr).p_type = PT_LOAD;
            (*phdr_ptr).p_flags = PF_R | PF_W | PF_X;
            (*phdr_ptr).p_offset = 128;
            (*phdr_ptr).p_vaddr = 0x0040_0000;
            (*phdr_ptr).p_filesz = 64;
            (*phdr_ptr).p_memsz = 4096;
            (*phdr_ptr).p_align = 4096;
        }

        let process = match ProcessManager::spawn_process("init.elf", &mock_elf, &["init.elf", "--daemon"]) {
            Ok(p) => p,
            Err(_) => {
                crate::serial_write_str("[KNT-PANIC] Failed to spawn ELF user process!\r\n");
                return false;
            }
        };

        if process.pcb.pid == 0 || process.main_thread_id == 0 {
            crate::serial_write_str("[KNT-PANIC] Spawned ELF process has invalid PID or TID!\r\n");
            return false;
        }

        // Yield CPU to allow the spawned ELF user thread to execute and complete its syscall
        crate::process::scheduler::yield_now();
    }

    // 4. Verify Dynamic Linker & Shared Library Subsystem
    if !dynlink::test_dynamic_linker_subsystem() {
        crate::serial_write_str("[KNT-PANIC] Dynamic linker subsystem self-test failed!\r\n");
        return false;
    }

    crate::serial_write_str("[KNT-ELF-PASS] ELF64 executable loader and userland execution verified\r\n");
    true
}
