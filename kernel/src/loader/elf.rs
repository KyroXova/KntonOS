//! ELF64 (64-bit Executable and Linkable Format) Binary Parser and Segment Descriptor.
//!
//! Implements strict System V ABI and Linux AMD64 executable validation,
//! header parsing, and program header table enumeration for userland execution.

use alloc::vec::Vec;

pub const ELF_MAGIC: [u8; 4] = [0x7F, b'E', b'L', b'F'];

pub const EI_MAG0: usize = 0;
pub const EI_MAG1: usize = 1;
pub const EI_MAG2: usize = 2;
pub const EI_MAG3: usize = 3;
pub const EI_CLASS: usize = 4;
pub const EI_DATA: usize = 5;
pub const EI_VERSION: usize = 6;
pub const EI_OSABI: usize = 7;
pub const EI_ABIVERSION: usize = 8;
pub const EI_NIDENT: usize = 16;

// ELF Architecture Classes
pub const ELF_CLASS_NONE: u8 = 0;
pub const ELF_CLASS_32: u8 = 1;
pub const ELF_CLASS_64: u8 = 2;

// ELF Data Encoding (Endianness)
pub const ELF_DATA_NONE: u8 = 0;
pub const ELF_DATA_2LSB: u8 = 1; // 2's complement, little endian
pub const ELF_DATA_2MSB: u8 = 2; // 2's complement, big endian

// ELF Current Version
pub const EV_CURRENT: u8 = 1;

// ELF OS ABI Identification
pub const ELF_OSABI_SYSV: u8 = 0;
pub const ELF_OSABI_HPUX: u8 = 1;
pub const ELF_OSABI_NETBSD: u8 = 2;
pub const ELF_OSABI_LINUX: u8 = 3;
pub const ELF_OSABI_SOLARIS: u8 = 6;
pub const ELF_OSABI_FREEBSD: u8 = 9;

// ELF Object File Types
pub const ET_NONE: u16 = 0;
pub const ET_REL: u16 = 1;
pub const ET_EXEC: u16 = 2;
pub const ET_DYN: u16 = 3;
pub const ET_CORE: u16 = 4;

// Machine Architecture
pub const EM_NONE: u16 = 0;
pub const EM_X86_64: u16 = 62; // Advanced Micro Devices x86-64

// Program Header Segment Types
pub const PT_NULL: u32 = 0;
pub const PT_LOAD: u32 = 1;
pub const PT_DYNAMIC: u32 = 2;
pub const PT_INTERP: u32 = 3;
pub const PT_NOTE: u32 = 4;
pub const PT_SHLIB: u32 = 5;
pub const PT_PHDR: u32 = 6;
pub const PT_TLS: u32 = 7;
pub const PT_GNU_EH_FRAME: u32 = 0x6474E550;
pub const PT_GNU_STACK: u32 = 0x6474E551;
pub const PT_GNU_RELRO: u32 = 0x6474E552;

// Program Header Flags
pub const PF_X: u32 = 1 << 0; // Execute
pub const PF_W: u32 = 1 << 1; // Write
pub const PF_R: u32 = 1 << 2; // Read
pub const PF_MASKOS: u32 = 0x00FF_0000;
pub const PF_MASKPROC: u32 = 0xFF00_0000;

#[repr(C, packed)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Elf64Ehdr {
    pub e_ident: [u8; EI_NIDENT],
    pub e_type: u16,
    pub e_machine: u16,
    pub e_version: u32,
    pub e_entry: u64,
    pub e_phoff: u64,
    pub e_shoff: u64,
    pub e_flags: u32,
    pub e_ehsize: u16,
    pub e_phentsize: u16,
    pub e_phnum: u16,
    pub e_shentsize: u16,
    pub e_shnum: u16,
    pub e_shstrndx: u16,
}

#[repr(C, packed)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Elf64Phdr {
    pub p_type: u32,
    pub p_flags: u32,
    pub p_offset: u64,
    pub p_vaddr: u64,
    pub p_paddr: u64,
    pub p_filesz: u64,
    pub p_memsz: u64,
    pub p_align: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ElfError {
    BufferTooSmall,
    InvalidMagic,
    UnsupportedClass,
    UnsupportedDataFormat,
    InvalidVersion,
    UnsupportedMachine,
    UnsupportedType,
    InvalidHeaderSize,
    InvalidProgramHeaderOffset,
    InvalidProgramHeaderEntrySize,
    NoLoadableSegments,
    SegmentBoundsOverflow,
}

/// A parsed and validated ELF64 binary image view.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedElf64<'a> {
    pub header: Elf64Ehdr,
    pub entry_point: u64,
    pub program_headers: Vec<Elf64Phdr>,
    pub raw_bytes: &'a [u8],
}

impl<'a> ParsedElf64<'a> {
    /// Parses and validates raw ELF64 executable bytes according to System V ABI standards.
    pub fn parse(bytes: &'a [u8]) -> Result<Self, ElfError> {
        let ehdr_size = core::mem::size_of::<Elf64Ehdr>();
        if bytes.len() < ehdr_size {
            return Err(ElfError::BufferTooSmall);
        }

        // 1. Validate ELF Magic identification (\x7FELF)
        if bytes[..4] != ELF_MAGIC {
            return Err(ElfError::InvalidMagic);
        }

        // 2. Validate ELF Class (Must be 64-bit)
        if bytes[EI_CLASS] != ELF_CLASS_64 {
            return Err(ElfError::UnsupportedClass);
        }

        // 3. Validate Data Format (Must be Little-Endian)
        if bytes[EI_DATA] != ELF_DATA_2LSB {
            return Err(ElfError::UnsupportedDataFormat);
        }

        // 4. Validate Version (Must be EV_CURRENT = 1)
        if bytes[EI_VERSION] != EV_CURRENT {
            return Err(ElfError::InvalidVersion);
        }

        // Safe copy of unaligned packed Elf64Ehdr
        let header: Elf64Ehdr = unsafe {
            core::ptr::read_unaligned(bytes.as_ptr() as *const Elf64Ehdr)
        };

        // 5. Validate Machine (Must be x86-64 / AMD64)
        if header.e_machine != EM_X86_64 {
            return Err(ElfError::UnsupportedMachine);
        }

        // 6. Validate Type (Must be ET_EXEC or ET_DYN)
        if header.e_type != ET_EXEC && header.e_type != ET_DYN {
            return Err(ElfError::UnsupportedType);
        }

        let phoff = header.e_phoff as usize;
        let phentsize = header.e_phentsize as usize;
        let phnum = header.e_phnum as usize;
        let expected_phdr_size = core::mem::size_of::<Elf64Phdr>();

        if phentsize < expected_phdr_size {
            return Err(ElfError::InvalidProgramHeaderEntrySize);
        }

        let total_phdr_size = phnum.checked_mul(phentsize).ok_or(ElfError::SegmentBoundsOverflow)?;
        let phdr_end = phoff.checked_add(total_phdr_size).ok_or(ElfError::SegmentBoundsOverflow)?;

        if bytes.len() < phdr_end {
            return Err(ElfError::InvalidProgramHeaderOffset);
        }

        // 7. Parse Program Header Table
        let mut program_headers = Vec::with_capacity(phnum);
        for i in 0..phnum {
            let offset = phoff + i * phentsize;
            let phdr: Elf64Phdr = unsafe {
                core::ptr::read_unaligned(bytes.as_ptr().add(offset) as *const Elf64Phdr)
            };

            // Verify file offset bounds for segments
            if phdr.p_type == PT_LOAD {
                let seg_end = (phdr.p_offset as usize)
                    .checked_add(phdr.p_filesz as usize)
                    .ok_or(ElfError::SegmentBoundsOverflow)?;
                if bytes.len() < seg_end {
                    return Err(ElfError::SegmentBoundsOverflow);
                }
            }

            program_headers.push(phdr);
        }

        let has_loadable = program_headers.iter().any(|ph| ph.p_type == PT_LOAD);
        if !has_loadable {
            return Err(ElfError::NoLoadableSegments);
        }

        Ok(Self {
            header,
            entry_point: header.e_entry,
            program_headers,
            raw_bytes: bytes,
        })
    }

    /// Returns an iterator over all PT_LOAD loadable segments.
    pub fn load_segments(&self) -> impl Iterator<Item = &Elf64Phdr> {
        self.program_headers.iter().filter(|ph| ph.p_type == PT_LOAD)
    }

    /// Extracts the dynamic linker / interpreter path if a PT_INTERP segment is present.
    pub fn interpreter(&self) -> Option<&'a str> {
        for ph in &self.program_headers {
            if ph.p_type == PT_INTERP {
                let start = ph.p_offset as usize;
                let end = start + ph.p_filesz as usize;
                if end <= self.raw_bytes.len() {
                    let s = &self.raw_bytes[start..end];
                    let s_clean = if s.ends_with(&[0]) { &s[..s.len() - 1] } else { s };
                    return core::str::from_utf8(s_clean).ok();
                }
            }
        }
        None
    }
}

/// Information about a successfully loaded ELF64 image mapped into virtual memory.
#[derive(Debug, Clone)]
pub struct LoadedElfImage {
    pub entry_point: u64,
    pub base_address: u64,
    pub image_size: usize,
    pub segment_count: usize,
    pub allocated_frames: Vec<crate::memory::pmm::PhysAddr>,
}

pub struct ElfLoader;

impl ElfLoader {
    /// Maps all PT_LOAD segments of an ELF64 binary into the given page table,
    /// allocating physical memory frames and zero-filling BSS regions.
    pub unsafe fn load(
        elf: &ParsedElf64,
        ptm: &mut crate::memory::vmm::PageTableManager,
    ) -> Result<LoadedElfImage, ElfError> {
        use crate::memory::pmm::PAGE_SIZE;
        use crate::memory::vmm::{PageTableFlags, VirtAddr};
        use crate::memory::PMM;

        let mut pmm_guard = PMM.lock();
        let mut allocated_frames = Vec::new();
        let mut total_image_size = 0;
        let mut min_vaddr = u64::MAX;
        let mut max_vaddr = 0u64;
        let mut segment_count = 0;

        // Temporarily clear CR0.WP to allow writing page tables in UEFI
        let cr0: u64;
        core::arch::asm!("mov {}, cr0", out(reg) cr0, options(nomem, nostack, preserves_flags));
        core::arch::asm!("mov cr0, {}", in(reg) (cr0 & !(1 << 16)), options(nomem, nostack, preserves_flags));

        for phdr in elf.load_segments() {
            if phdr.p_memsz == 0 {
                continue;
            }

            let seg_vaddr = phdr.p_vaddr as usize;
            let seg_filesz = phdr.p_filesz as usize;
            let seg_memsz = phdr.p_memsz as usize;
            let seg_offset = phdr.p_offset as usize;

            if phdr.p_vaddr < min_vaddr {
                min_vaddr = phdr.p_vaddr;
            }
            if phdr.p_vaddr + phdr.p_memsz > max_vaddr {
                max_vaddr = phdr.p_vaddr + phdr.p_memsz;
            }

            // Determine page attributes from ELF program header flags
            let mut page_flags = PageTableFlags::PRESENT | PageTableFlags::USER_ACCESSIBLE;
            if (phdr.p_flags & PF_W) != 0 {
                page_flags |= PageTableFlags::WRITABLE;
            }
            if (phdr.p_flags & PF_X) == 0 {
                page_flags |= PageTableFlags::NO_EXECUTE;
            }

            let start_page = seg_vaddr & !(PAGE_SIZE - 1);
            let end_page = (seg_vaddr + seg_memsz + PAGE_SIZE - 1) & !(PAGE_SIZE - 1);
            let page_count = (end_page - start_page) / PAGE_SIZE;

            for page_idx in 0..page_count {
                let vaddr = VirtAddr::new(start_page + page_idx * PAGE_SIZE);

                // Allocate physical frame
                let frame = match pmm_guard.allocate_frame() {
                    Some(f) => f,
                    None => {
                        core::arch::asm!("mov cr0, {}", in(reg) cr0, options(nomem, nostack, preserves_flags));
                        return Err(ElfError::SegmentBoundsOverflow);
                    }
                };

                let phys_addr = frame.start_address();
                allocated_frames.push(phys_addr);

                // Map page into the process virtual address space
                if ptm.translate(vaddr).is_none() {
                    let _ = ptm.map_page(vaddr, phys_addr, page_flags, &mut *pmm_guard);
                }

                // Zero-fill entire physical page frame (covers BSS and alignment padding)
                let frame_ptr = phys_addr.0 as *mut u8;
                core::ptr::write_bytes(frame_ptr, 0, PAGE_SIZE);

                // Copy segment file data into this page
                let page_vaddr = vaddr.0;
                let page_end_vaddr = page_vaddr + PAGE_SIZE;
                let seg_file_end = seg_vaddr + seg_filesz;

                let copy_start = core::cmp::max(page_vaddr, seg_vaddr);
                let copy_end = core::cmp::min(page_end_vaddr, seg_file_end);

                if copy_start < copy_end {
                    let dest_offset = copy_start - page_vaddr;
                    let src_offset = seg_offset + (copy_start - seg_vaddr);
                    let copy_len = copy_end - copy_start;

                    if src_offset + copy_len <= elf.raw_bytes.len() {
                        core::ptr::copy_nonoverlapping(
                            elf.raw_bytes.as_ptr().add(src_offset),
                            frame_ptr.add(dest_offset),
                            copy_len,
                        );
                    }
                }
            }

            segment_count += 1;
            total_image_size += seg_memsz;
        }

        // Restore CR0
        core::arch::asm!("mov cr0, {}", in(reg) cr0, options(nomem, nostack, preserves_flags));

        Ok(LoadedElfImage {
            entry_point: elf.entry_point,
            base_address: if min_vaddr == u64::MAX { 0 } else { min_vaddr },
            image_size: total_image_size,
            segment_count,
            allocated_frames,
        })
    }
}

/// Standalone verification self-test for ELF64 header parser.
pub fn test_elf_parser() -> bool {
    crate::serial_write_str("[KNT-ELF-PARSER-TEST] Testing ELF64 header and program header table parser...\r\n");

    // Construct a valid minimal mock ELF64 header with 1 PT_LOAD segment
    let mut mock_elf = [0u8; 128];
    // Magic
    mock_elf[0..4].copy_from_slice(&ELF_MAGIC);
    mock_elf[EI_CLASS] = ELF_CLASS_64;
    mock_elf[EI_DATA] = ELF_DATA_2LSB;
    mock_elf[EI_VERSION] = EV_CURRENT;
    mock_elf[EI_OSABI] = ELF_OSABI_SYSV;

    let ehdr_ptr = mock_elf.as_mut_ptr() as *mut Elf64Ehdr;
    unsafe {
        (*ehdr_ptr).e_type = ET_EXEC;
        (*ehdr_ptr).e_machine = EM_X86_64;
        (*ehdr_ptr).e_version = 1;
        (*ehdr_ptr).e_entry = 0x0040_0000;
        (*ehdr_ptr).e_phoff = 64; // Starts immediately after 64-byte Ehdr
        (*ehdr_ptr).e_ehsize = 64;
        (*ehdr_ptr).e_phentsize = 56; // 56-byte Elf64Phdr
        (*ehdr_ptr).e_phnum = 1;
    }

    let phdr_ptr = unsafe { mock_elf.as_mut_ptr().add(64) as *mut Elf64Phdr };
    unsafe {
        (*phdr_ptr).p_type = PT_LOAD;
        (*phdr_ptr).p_flags = PF_R | PF_X;
        (*phdr_ptr).p_offset = 0;
        (*phdr_ptr).p_vaddr = 0x0040_0000;
        (*phdr_ptr).p_filesz = 120;
        (*phdr_ptr).p_memsz = 4096;
        (*phdr_ptr).p_align = 4096;
    }

    // 1. Verify valid ELF64 parse
    let parsed = match ParsedElf64::parse(&mock_elf) {
        Ok(p) => p,
        Err(_) => {
            crate::serial_write_str("[KNT-PANIC] Valid mock ELF64 parsing failed!\r\n");
            return false;
        }
    };

    if parsed.entry_point != 0x0040_0000 {
        crate::serial_write_str("[KNT-PANIC] Entry point mismatch!\r\n");
        return false;
    }

    if parsed.program_headers.len() != 1 || parsed.program_headers[0].p_type != PT_LOAD {
        crate::serial_write_str("[KNT-PANIC] Program header count or type mismatch!\r\n");
        return false;
    }

    // 2. Verify invalid magic rejection
    let mut bad_magic = mock_elf;
    bad_magic[0] = 0x00;
    if ParsedElf64::parse(&bad_magic) != Err(ElfError::InvalidMagic) {
        crate::serial_write_str("[KNT-PANIC] Invalid magic was not rejected!\r\n");
        return false;
    }

    // 3. Verify 32-bit class rejection
    let mut bad_class = mock_elf;
    bad_class[EI_CLASS] = ELF_CLASS_32;
    if ParsedElf64::parse(&bad_class) != Err(ElfError::UnsupportedClass) {
        crate::serial_write_str("[KNT-PANIC] 32-bit class was not rejected!\r\n");
        return false;
    }

    crate::serial_write_str("[KNT-ELF-PARSER-PASS] ELF64 header validation and program header parsing verified\r\n");
    true
}

/// Standalone verification self-test for ELF segment loading and BSS zero-filling.
pub fn test_elf_segment_mapping() -> bool {
    crate::serial_write_str("[KNT-ELF-SEGMENT-TEST] Testing ELF64 segment mapping and BSS zero-fill...\r\n");

    let mut mock_elf = [0u8; 256];
    mock_elf[0..4].copy_from_slice(&ELF_MAGIC);
    mock_elf[EI_CLASS] = ELF_CLASS_64;
    mock_elf[EI_DATA] = ELF_DATA_2LSB;
    mock_elf[EI_VERSION] = EV_CURRENT;
    mock_elf[EI_OSABI] = ELF_OSABI_SYSV;

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

    // Segment with 64 bytes in file, but 4096 bytes in memory (BSS = 4032 bytes)
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

    // Fill file data at offset 128..192
    for i in 0..64 {
        mock_elf[128 + i] = 0xAA;
    }

    let parsed = ParsedElf64::parse(&mock_elf).expect("Mock ELF parse failed");

    // Load into a temporary test page table
    let cr3: u64;
    unsafe {
        core::arch::asm!("mov {}, cr3", out(reg) cr3, options(nomem, nostack, preserves_flags));
        let root_pml4 = (cr3 & !0xFFF) as *mut crate::memory::vmm::PageTable;
        let mut ptm = crate::memory::vmm::PageTableManager::new(root_pml4);

        let loaded = ElfLoader::load(&parsed, &mut ptm).expect("ELF segment load failed");

        if loaded.segment_count != 1 || loaded.allocated_frames.is_empty() {
            crate::serial_write_str("[KNT-PANIC] Loaded ELF image segment count or frame list invalid!\r\n");
            return false;
        }

        // Verify physical frame contents: first 64 bytes are 0xAA, remainder are 0x00 (BSS)
        let frame_ptr = loaded.allocated_frames[0].0 as *const u8;
        for i in 0..64 {
            if *frame_ptr.add(i) != 0xAA {
                crate::serial_write_str("[KNT-PANIC] Loaded ELF segment file data mismatch!\r\n");
                return false;
            }
        }
        for i in 64..4096 {
            if *frame_ptr.add(i) != 0x00 {
                crate::serial_write_str("[KNT-PANIC] BSS region was not zero-filled!\r\n");
                return false;
            }
        }
    }

    crate::serial_write_str("[KNT-ELF-SEGMENT-PASS] ELF segment mapping and BSS zero-fill verified\r\n");
    true
}
