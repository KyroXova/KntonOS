//! Executable Binary Loader Subsystem for KntonOS.
//!
//! Provides parsing, section/segment mapping, and process bootstrap
//! for standard ELF64 (System V AMD64 ABI) binaries.

pub mod elf;

pub use elf::{
    test_elf_parser, test_elf_segment_mapping, Elf64Ehdr, Elf64Phdr, ElfError, ElfLoader,
    LoadedElfImage, ParsedElf64, ELF_CLASS_64, ELF_DATA_2LSB, ELF_MAGIC, ELF_OSABI_SYSV,
    EM_X86_64, ET_DYN, ET_EXEC, PF_R, PF_W, PF_X, PT_DYNAMIC, PT_INTERP, PT_LOAD,
};
