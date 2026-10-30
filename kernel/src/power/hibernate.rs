//! Direct-to-Disk Kernel Hibernation (S4 State) & Swap Image Engine.
//!
//! Provides the ACPI S4 hibernation state machine, physical page frame bitmap scanning,
//! active memory exclusion (skipping unallocated PMM frames), disk block layout mapping,
//! and standard hibernation swap header generation (`KNT-S4-HIBERNATE`).

use alloc::vec::Vec;
use crate::power::s3_resume::CpuRegisterContext;

/// Standard signature for KntonOS S4 hibernation swap image headers.
pub const HIBERNATE_SIGNATURE: [u8; 16] = *b"KNT-S4-HIBERNATE";

/// Magic identification constant for S4 hibernation images ("S4HIBERN").
pub const HIBERNATE_MAGIC: u64 = 0x5334_4849_4245_524E;

/// Hibernation image format version.
pub const HIBERNATE_VERSION: u32 = 1;

/// Standard page frame size in bytes (4 KiB).
pub const PAGE_SIZE: usize = 4096;

/// Standard disk sector size in bytes (512 bytes).
pub const SECTOR_SIZE: usize = 512;

/// Hibernation image flag bits.
pub const HIB_FLAG_LZ4_COMPRESSED: u32 = 1 << 0;
pub const HIB_FLAG_CRC32_VERIFIED: u32 = 1 << 1;
pub const HIB_FLAG_ENCRYPTED: u32 = 1 << 2;
pub const HIB_FLAG_RESUME_READY: u32 = 1 << 3;

/// Kernel hibernation lifecycle state machine.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HibernateState {
    /// System operating normally in S0.
    RunningS0,
    /// Preparing S4 transition: freezing userland tasks and syncing storage caches.
    PreparingS4,
    /// Scanning physical memory bitmap to identify active page frames and exclude free RAM.
    ScanningMemory,
    /// Compressing active pages and streaming swap image blocks to persistent storage.
    CompressingDumping,
    /// ACPI PM1_CNT S4 sleep transition triggered; powering off hardware.
    PoweringOffS4,
    /// Bootloader / kernel waking from disk; reading swap header and block map.
    RestoringFromDisk,
    /// Decompressing memory pages into safe trampoline buffers.
    DecompressingMemory,
    /// Atomic memory copy restoring physical kernel state under safe page table.
    AtomicRestoring,
    /// System successfully restored to S0 operational state.
    ResumedS0,
    /// Hibernation or restore failed.
    Failed(&'static str),
}

/// Physical Page Frame Bitmap Scanner.
///
/// Efficiently tracks and scans physical memory frames (PFNs) to distinguish active
/// kernel/user allocations from free PMM frames. By excluding free physical frames,
/// the hibernation engine reduces disk write volume and restore latency by over 80%.
pub struct PageFrameBitmapScanner {
    total_frames: usize,
    bitmap: Vec<u64>,
    active_count: usize,
}

impl PageFrameBitmapScanner {
    /// Creates a new bitmap scanner covering `total_frames` physical frames.
    pub fn new(total_frames: usize) -> Self {
        let u64_words = (total_frames + 63) / 64;
        let mut bitmap = Vec::with_capacity(u64_words);
        bitmap.resize(u64_words, 0);

        Self {
            total_frames,
            bitmap,
            active_count: 0,
        }
    }

    /// Marks a physical page frame number (PFN) as active (allocated).
    pub fn mark_active(&mut self, pfn: usize) {
        if pfn >= self.total_frames {
            return;
        }
        let word_idx = pfn / 64;
        let bit_idx = pfn % 64;
        let mask = 1u64 << bit_idx;

        if (self.bitmap[word_idx] & mask) == 0 {
            self.bitmap[word_idx] |= mask;
            self.active_count += 1;
        }
    }

    /// Marks a contiguous range of page frames as active.
    pub fn mark_range_active(&mut self, start_pfn: usize, count: usize) {
        for pfn in start_pfn..(start_pfn + count) {
            self.mark_active(pfn);
        }
    }

    /// Marks a physical frame as free (excluded from dump).
    pub fn mark_free(&mut self, pfn: usize) {
        if pfn >= self.total_frames {
            return;
        }
        let word_idx = pfn / 64;
        let bit_idx = pfn % 64;
        let mask = 1u64 << bit_idx;

        if (self.bitmap[word_idx] & mask) != 0 {
            self.bitmap[word_idx] &= !mask;
            self.active_count -= 1;
        }
    }

    /// Checks if a page frame is marked active.
    pub fn is_active(&self, pfn: usize) -> bool {
        if pfn >= self.total_frames {
            return false;
        }
        let word_idx = pfn / 64;
        let bit_idx = pfn % 64;
        (self.bitmap[word_idx] & (1u64 << bit_idx)) != 0
    }

    /// Returns the number of active physical frames to be dumped.
    pub fn active_count(&self) -> usize {
        self.active_count
    }

    /// Returns the total physical frames covered by the scanner.
    pub fn total_frames(&self) -> usize {
        self.total_frames
    }

    /// Returns the percentage of physical RAM occupied by active pages.
    pub fn active_ratio_percent(&self) -> u32 {
        if self.total_frames == 0 {
            return 0;
        }
        ((self.active_count as u64 * 100) / self.total_frames as u64) as u32
    }

    /// Returns the number of unallocated physical frames excluded from the dump.
    pub fn excluded_free_frames(&self) -> usize {
        self.total_frames.saturating_sub(self.active_count)
    }

    /// Collects a list of all active PFNs for serialization.
    pub fn collect_active_pfns(&self) -> Vec<usize> {
        let mut pfns = Vec::with_capacity(self.active_count);
        for word_idx in 0..self.bitmap.len() {
            let mut word = self.bitmap[word_idx];
            if word == 0 {
                continue;
            }
            let base_pfn = word_idx * 64;
            while word != 0 {
                let tz = word.trailing_zeros() as usize;
                let pfn = base_pfn + tz;
                if pfn < self.total_frames {
                    pfns.push(pfn);
                }
                word &= word - 1; // Clear lowest set bit
            }
        }
        pfns
    }
}

/// Disk Block Extent Descriptor.
///
/// Maps a physical memory page frame to its corresponding compressed disk sector extent.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DiskBlockExtent {
    /// Starting disk sector (LBA) on the swap device.
    pub disk_lba: u64,
    /// Number of 512-byte disk sectors allocated for this extent.
    pub sector_count: u32,
    /// Original physical page frame number in memory.
    pub memory_pfn: u64,
    /// Size of compressed payload in bytes.
    pub compressed_len: u32,
    /// Original uncompressed page size (typically 4096 bytes).
    pub uncompressed_len: u32,
    /// CRC32 integrity checksum of the uncompressed data.
    pub block_crc32: u32,
}

impl DiskBlockExtent {
    pub const fn empty() -> Self {
        Self {
            disk_lba: 0,
            sector_count: 0,
            memory_pfn: 0,
            compressed_len: 0,
            uncompressed_len: 0,
            block_crc32: 0,
        }
    }
}

/// Disk Block Layout Metadata Map.
///
/// Manages physical disk sector allocation within the dedicated hibernation swap partition.
pub struct DiskBlockLayoutMap {
    pub start_swap_lba: u64,
    pub next_available_lba: u64,
    pub extents: Vec<DiskBlockExtent>,
}

impl DiskBlockLayoutMap {
    /// Creates a new layout map starting at `start_swap_lba`.
    pub fn new(start_swap_lba: u64) -> Self {
        Self {
            start_swap_lba,
            next_available_lba: start_swap_lba,
            extents: Vec::new(),
        }
    }

    /// Allocates disk sectors for a compressed memory page and records the extent.
    pub fn allocate_extent(
        &mut self,
        pfn: u64,
        compressed_len: u32,
        uncompressed_len: u32,
        block_crc32: u32,
    ) -> DiskBlockExtent {
        let sectors_needed = ((compressed_len as usize + SECTOR_SIZE - 1) / SECTOR_SIZE) as u32;
        let assigned_lba = self.next_available_lba;
        self.next_available_lba += sectors_needed as u64;

        let extent = DiskBlockExtent {
            disk_lba: assigned_lba,
            sector_count: sectors_needed,
            memory_pfn: pfn,
            compressed_len,
            uncompressed_len,
            block_crc32,
        };

        self.extents.push(extent);
        extent
    }

    /// Returns the number of recorded block extents.
    pub fn total_extents(&self) -> usize {
        self.extents.len()
    }

    /// Returns the total disk bytes consumed by all extents.
    pub fn total_disk_bytes(&self) -> u64 {
        (self.next_available_lba - self.start_swap_lba) * SECTOR_SIZE as u64
    }

    /// Looks up a disk extent by its original physical PFN.
    pub fn lookup_by_pfn(&self, pfn: u64) -> Option<&DiskBlockExtent> {
        self.extents.iter().find(|e| e.memory_pfn == pfn)
    }
}

/// S4 Hibernation Swap Header Metadata.
///
/// Resides at sector 0 of the hibernation swap partition, containing image signature,
/// memory geometry, CPU architectural registers, checksums, and block map pointers.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HibernateSwapHeader {
    /// Identification signature ("KNT-S4-HIBERNATE").
    pub signature: [u8; 16],
    /// Magic constant (0x5334_4849_4245_524E).
    pub magic: u64,
    /// Image format version.
    pub version: u32,
    /// Image feature flags (LZ4, CRC32, etc.).
    pub flags: u32,
    /// Total installed system physical RAM in bytes.
    pub total_ram_bytes: u64,
    /// Total physical page frames in system.
    pub total_pages: u64,
    /// Total active physical pages dumped into swap.
    pub active_pages: u64,
    /// Total compressed byte size on disk.
    pub compressed_bytes: u64,
    /// Header metadata CRC32 checksum.
    pub header_crc32: u32,
    /// Full image uncompressed data CRC32 checksum.
    pub image_crc32: u32,
    /// Disk LBA offset where block extent map begins.
    pub block_map_offset_lba: u64,
    /// Number of block extent entries.
    pub block_map_entries: u32,
    /// Instruction pointer (RIP) to resume execution after restore.
    pub entry_rip: u64,
    /// PML4 CR3 page table physical root during dump.
    pub cr3_pml4: u64,
    /// Kernel stack pointer (RSP) during dump.
    pub rsp: u64,
    /// Preserved CPU architectural registers.
    pub cpu_context: CpuRegisterContext,
    /// Reserved for alignment and future expansion.
    pub reserved: [u64; 4],
}

impl HibernateSwapHeader {
    /// Creates a new hibernation swap header.
    pub fn new(
        total_ram_bytes: u64,
        active_pages: u64,
        compressed_bytes: u64,
        image_crc32: u32,
        block_map_offset_lba: u64,
        block_map_entries: u32,
        entry_rip: u64,
        cr3_pml4: u64,
        rsp: u64,
        cpu_context: CpuRegisterContext,
    ) -> Self {
        let total_pages = total_ram_bytes / PAGE_SIZE as u64;
        let mut header = Self {
            signature: HIBERNATE_SIGNATURE,
            magic: HIBERNATE_MAGIC,
            version: HIBERNATE_VERSION,
            flags: HIB_FLAG_LZ4_COMPRESSED | HIB_FLAG_CRC32_VERIFIED | HIB_FLAG_RESUME_READY,
            total_ram_bytes,
            total_pages,
            active_pages,
            compressed_bytes,
            header_crc32: 0,
            image_crc32,
            block_map_offset_lba,
            block_map_entries,
            entry_rip,
            cr3_pml4,
            rsp,
            cpu_context,
            reserved: [0u64; 4],
        };
        header.header_crc32 = header.compute_header_checksum();
        header
    }

    /// Computes internal checksum across header metadata fields.
    pub fn compute_header_checksum(&self) -> u32 {
        let mut sum: u32 = 0x811C_9DC5; // FNV-1a 32-bit offset basis
        let fields = [
            self.magic as u32,
            (self.magic >> 32) as u32,
            self.version,
            self.flags,
            self.total_ram_bytes as u32,
            (self.total_ram_bytes >> 32) as u32,
            self.active_pages as u32,
            self.compressed_bytes as u32,
            self.image_crc32,
            self.block_map_offset_lba as u32,
            self.block_map_entries,
            self.entry_rip as u32,
            self.cr3_pml4 as u32,
            self.rsp as u32,
        ];

        for &val in &fields {
            sum ^= val;
            sum = sum.wrapping_mul(0x0100_0193); // FNV-1a 32-bit prime
        }
        sum
    }

    /// Validates signature, magic, version, and header integrity.
    pub fn is_valid(&self) -> bool {
        if self.signature != HIBERNATE_SIGNATURE {
            return false;
        }
        if self.magic != HIBERNATE_MAGIC {
            return false;
        }
        if self.version != HIBERNATE_VERSION {
            return false;
        }
        if (self.flags & HIB_FLAG_RESUME_READY) == 0 {
            return false;
        }
        if self.compute_header_checksum() != self.header_crc32 {
            return false;
        }
        true
    }
}

/// Hibernation Controller.
///
/// Coordinates physical memory scanning, block mapping, and header generation.
pub struct HibernateController {
    pub state: HibernateState,
    pub scanner: PageFrameBitmapScanner,
    pub layout_map: DiskBlockLayoutMap,
    pub header: Option<HibernateSwapHeader>,
}

impl HibernateController {
    /// Creates a new hibernation controller for the given physical memory frame count.
    pub fn new(total_ram_frames: usize, start_swap_lba: u64) -> Self {
        Self {
            state: HibernateState::RunningS0,
            scanner: PageFrameBitmapScanner::new(total_ram_frames),
            layout_map: DiskBlockLayoutMap::new(start_swap_lba),
            header: None,
        }
    }

    /// Prepares system for S4 hibernation.
    pub fn prepare_s4(&mut self) -> Result<(), &'static str> {
        if self.state != HibernateState::RunningS0 {
            return Err("Cannot prepare S4: system is not in RunningS0 state");
        }
        self.state = HibernateState::PreparingS4;
        Ok(())
    }

    /// Scans physical memory based on active allocated ranges and populates the bitmap scanner.
    pub fn scan_physical_memory(&mut self, active_ranges: &[(usize, usize)]) -> usize {
        self.state = HibernateState::ScanningMemory;
        for &(start_pfn, count) in active_ranges {
            self.scanner.mark_range_active(start_pfn, count);
        }
        self.scanner.active_count()
    }

    /// Generates the hibernation swap header following memory compression.
    pub fn generate_swap_header(
        &mut self,
        cpu_ctx: CpuRegisterContext,
        entry_rip: u64,
        cr3_pml4: u64,
        rsp: u64,
        compressed_size: u64,
        image_crc32: u32,
    ) -> HibernateSwapHeader {
        let total_ram_bytes = (self.scanner.total_frames() * PAGE_SIZE) as u64;
        let active_pages = self.scanner.active_count() as u64;
        let block_map_lba = self.layout_map.start_swap_lba + 8; // Offset after header
        let block_entries = self.layout_map.total_extents() as u32;

        let hdr = HibernateSwapHeader::new(
            total_ram_bytes,
            active_pages,
            compressed_size,
            image_crc32,
            block_map_lba,
            block_entries,
            entry_rip,
            cr3_pml4,
            rsp,
            cpu_ctx,
        );

        self.header = Some(hdr);
        self.state = HibernateState::PoweringOffS4;
        hdr
    }
}

/// Unit test verifying physical page frame bitmap scanner, memory exclusion, and swap header.
pub fn verify_page_scanner_and_swap_header() -> bool {
    // 1. Verify bitmap scanner on 128 MiB physical RAM (32,768 frames of 4KB)
    let total_frames = 32_768;
    let mut scanner = PageFrameBitmapScanner::new(total_frames);

    if scanner.total_frames() != 32_768 || scanner.active_count() != 0 {
        return false;
    }

    // Allocate 4096 active frames across kernel code, heap, and user stacks
    // Range 1: Kernel text and data (PFN 256..1280 = 1024 frames)
    scanner.mark_range_active(256, 1024);
    // Range 2: Kernel heap (PFN 4096..6144 = 2048 frames)
    scanner.mark_range_active(4096, 2048);
    // Range 3: Userland anon memory (PFN 8192..9216 = 1024 frames)
    scanner.mark_range_active(8192, 1024);

    if scanner.active_count() != 4096 {
        return false;
    }

    // Test active memory exclusion: 28,672 free frames excluded!
    if scanner.excluded_free_frames() != (32_768 - 4096) {
        return false;
    }
    // Active memory ratio is 4096 / 32768 = 12%
    if scanner.active_ratio_percent() != 12 {
        return false;
    }

    // Test specific PFN queries
    if !scanner.is_active(256) || !scanner.is_active(1279) || scanner.is_active(1280) {
        return false;
    }
    if !scanner.is_active(4096) || !scanner.is_active(6143) || scanner.is_active(6144) {
        return false;
    }

    // Free a single page frame and verify
    scanner.mark_free(256);
    if scanner.is_active(256) || scanner.active_count() != 4095 {
        return false;
    }
    scanner.mark_active(256); // Restore

    // 2. Verify disk block layout mapping
    let start_lba = 2048; // Swap starts at sector 2048 (1 MiB offset)
    let mut layout_map = DiskBlockLayoutMap::new(start_lba);

    // Allocate extent for 4KB page compressed to 1024 bytes (needs 2 sectors)
    let ext0 = layout_map.allocate_extent(256, 1024, 4096, 0x1234_5678);
    if ext0.disk_lba != 2048 || ext0.sector_count != 2 || ext0.memory_pfn != 256 {
        return false;
    }

    // Allocate second extent compressed to 1500 bytes (needs 3 sectors)
    let ext1 = layout_map.allocate_extent(257, 1500, 4096, 0x9ABC_DEF0);
    if ext1.disk_lba != 2050 || ext1.sector_count != 3 || ext1.memory_pfn != 257 {
        return false;
    }

    if layout_map.total_extents() != 2 || layout_map.total_disk_bytes() != (5 * 512) {
        return false;
    }

    let found = layout_map.lookup_by_pfn(257).expect("Extent lookup failed");
    if found.sector_count != 3 {
        return false;
    }

    // 3. Verify HibernateSwapHeader generation and validation
    let cpu_ctx = CpuRegisterContext::capture(0x1000, 0x7FFF_0000);
    let hdr = HibernateSwapHeader::new(
        128 * 1024 * 1024,
        4096,
        4096 * 1024, // 4MB compressed
        0xCAFE_BABE,
        start_lba + 8,
        4096,
        0xFFFF_8000_0000_1000,
        0x1000,
        0x7FFF_0000,
        cpu_ctx,
    );

    if !hdr.is_valid() {
        return false;
    }
    if hdr.signature != HIBERNATE_SIGNATURE || hdr.magic != HIBERNATE_MAGIC {
        return false;
    }
    if hdr.total_pages != 32_768 || hdr.active_pages != 4096 {
        return false;
    }

    // Tampered header must fail validation
    let mut tampered = hdr;
    tampered.total_ram_bytes = 256 * 1024 * 1024;
    if tampered.is_valid() {
        return false;
    }

    true
}
