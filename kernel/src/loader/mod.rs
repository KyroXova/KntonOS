//! Executable Binary Loader Subsystem for KntonOS.
//!
//! Provides parsing, segment mapping, System V AMD64 ABI user stack setup,
//! and process execution for standard ELF64 binaries.

pub mod elf;

pub use elf::{
    test_elf_parser, test_elf_segment_mapping, Elf64Auxv, Elf64Ehdr, Elf64Phdr, ElfError,
    ElfLoader, ElfProcess, ElfSpawnError, ElfStackInitializer, ElfStackSetup, LoadedElfImage,
    ParsedElf64, ProcessManager, AT_BASE, AT_CLKTCK, AT_EGID, AT_ENTRY, AT_EUID, AT_EXECFD,
    AT_FLAGS, AT_GID, AT_HWCAP, AT_IGNORE, AT_NOTELF, AT_NULL, AT_PAGESZ, AT_PHDR, AT_PHENT,
    AT_PHNUM, AT_PLATFORM, AT_RANDOM, AT_UID, ELF_CLASS_64, ELF_DATA_2LSB, ELF_MAGIC,
    ELF_OSABI_SYSV, EM_X86_64, ET_DYN, ET_EXEC, PF_R, PF_W, PF_X, PT_DYNAMIC, PT_INTERP,
    PT_LOAD,
};
