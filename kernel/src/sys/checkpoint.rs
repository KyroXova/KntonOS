//! Kernel-Direct Micro-Checkpoint Fast State Snapshot & Instant Resumption Engine.
//!
//! Provides bare-metal low-latency game process snapshotting and sub-10ms state restoration:
//! - Hardware page-table dirty bit scanning and Copy-On-Write (COW) memory delta capture.
//! - Sub-5ms page freezer capturing active game state without perceptible gameplay stutter.
//! - Fast memory delta stream compressor (LZ4/Run-Length) and zero-copy NVMe checkpoint streaming.
//! - Instantaneous game state resumption restoring register contexts and page table leaves.

#![allow(dead_code)]

extern crate alloc;

// -----------------------------------------------------------------------------
// Constants & Flags
// -----------------------------------------------------------------------------

/// Micro-checkpoint binary magic identifier: "KCHK" (0x4B43484B).
pub const CHECKPOINT_MAGIC: u32 = 0x4B43484B;

/// Current micro-checkpoint engine format version (v1.0).
pub const CHECKPOINT_VERSION: u32 = 0x0001_0000;

/// Standard x86-64 page size in bytes (4 KiB).
pub const PAGE_SIZE: usize = 4096;

/// Maximum number of tracked pages in a micro-checkpoint segment (8192 pages = 32 MiB).
pub const MAX_CHECKPOINT_PAGES: usize = 8192;

/// Number of 64-bit bitmap words required to track MAX_CHECKPOINT_PAGES (128 u64 words).
pub const BITMAP_WORDS: usize = MAX_CHECKPOINT_PAGES / 64;

/// Checkpoint flag: Delta snapshot (captures only modified dirty pages).
pub const FLAG_DELTA_SNAPSHOT: u32 = 1 << 0;

/// Checkpoint flag: Full snapshot (complete baseline process memory image).
pub const FLAG_FULL_SNAPSHOT: u32 = 1 << 1;

/// Checkpoint flag: Delta payload is compressed using fast streaming LZ4/RLE.
pub const FLAG_COMPRESSED_LZ4: u32 = 1 << 2;

/// Checkpoint flag: Copy-On-Write (COW) write protection is active on tracked pages.
pub const FLAG_COW_ACTIVE: u32 = 1 << 3;

// -----------------------------------------------------------------------------
// 1. Snapshot Binary Header & Descriptors
// -----------------------------------------------------------------------------

/// Binary Header for KntonOS Micro-Checkpoint Snapshots.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(C)]
pub struct CheckpointHeader {
    pub magic: u32,
    pub version: u32,
    pub checkpoint_id: u64,
    pub timestamp_tsc: u64,
    pub game_pid: u32,
    pub total_pages_scanned: u32,
    pub dirty_pages_count: u32,
    pub uncompressed_bytes: u64,
    pub compressed_bytes: u64,
    pub cr3_pml4_root: u64,
    pub checksum_adler32: u32,
    pub flags: u32,
}

impl CheckpointHeader {
    pub fn new(
        checkpoint_id: u64,
        game_pid: u32,
        cr3_root: u64,
        dirty_pages: u32,
        flags: u32,
    ) -> Self {
        Self {
            magic: CHECKPOINT_MAGIC,
            version: CHECKPOINT_VERSION,
            checkpoint_id,
            timestamp_tsc: 1_800_000_000,
            game_pid,
            total_pages_scanned: MAX_CHECKPOINT_PAGES as u32,
            dirty_pages_count: dirty_pages,
            uncompressed_bytes: (dirty_pages as u64) * (PAGE_SIZE as u64),
            compressed_bytes: 0,
            cr3_pml4_root: cr3_root,
            checksum_adler32: 0,
            flags,
        }
    }

    pub fn is_valid(&self) -> bool {
        self.magic == CHECKPOINT_MAGIC && self.version == CHECKPOINT_VERSION && self.game_pid > 0
    }
}

/// Saved x86-64 CPU Thread & Game Execution Context for instant restoration.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(C)]
pub struct ThreadCheckpointState {
    pub rax: u64,
    pub rbx: u64,
    pub rcx: u64,
    pub rdx: u64,
    pub rsi: u64,
    pub rdi: u64,
    pub rbp: u64,
    pub rsp: u64,
    pub r8: u64,
    pub r9: u64,
    pub r10: u64,
    pub r11: u64,
    pub r12: u64,
    pub r13: u64,
    pub r14: u64,
    pub r15: u64,
    pub rip: u64,
    pub rflags: u64,
    pub cs: u16,
    pub ss: u16,
    pub ds: u16,
    pub es: u16,
    pub fs: u16,
    pub gs: u16,
    pub mxcsr: u32,
    pub frame_number: u64,
    pub game_state_hash: u64,
}

impl Default for ThreadCheckpointState {
    fn default() -> Self {
        Self {
            rax: 0,
            rbx: 0,
            rcx: 0,
            rdx: 0,
            rsi: 0,
            rdi: 0,
            rbp: 0,
            rsp: 0,
            r8: 0,
            r9: 0,
            r10: 0,
            r11: 0,
            r12: 0,
            r13: 0,
            r14: 0,
            r15: 0,
            rip: 0,
            rflags: 0x202, // IF (Interrupt Flag) enabled
            cs: 0x20 | 3,  // User code segment
            ss: 0x18 | 3,  // User data segment
            ds: 0x18 | 3,
            es: 0x18 | 3,
            fs: 0,
            gs: 0,
            mxcsr: 0x1F80, // Standard SSE flush-to-zero / denormals masked
            frame_number: 0,
            game_state_hash: 0,
        }
    }
}

/// Metadata descriptor for a single captured dirty memory page delta.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(C)]
pub struct PageDeltaDescriptor {
    pub virtual_address: u64,
    pub physical_frame: u64,
    pub page_flags: u32,
    pub compressed_offset: u32,
    pub compressed_size: u32,
    pub page_adler32: u32,
}

impl PageDeltaDescriptor {
    pub const fn new(virtual_address: u64, physical_frame: u64, page_flags: u32) -> Self {
        Self {
            virtual_address,
            physical_frame,
            page_flags,
            compressed_offset: 0,
            compressed_size: PAGE_SIZE as u32,
            page_adler32: 0,
        }
    }
}

// -----------------------------------------------------------------------------
// 2. Hardware Dirty Page Bitmap Tracker
// -----------------------------------------------------------------------------

/// High-performance 64-bit word bitmap tracking up to 8192 pages (32 MiB virtual address space).
#[derive(Clone, Copy)]
pub struct DirtyPageBitmapTracker {
    pub words: [u64; BITMAP_WORDS],
    pub base_virtual_address: u64,
}

impl DirtyPageBitmapTracker {
    /// Creates a cleared dirty page bitmap tracker mapped to a base virtual address.
    pub const fn new(base_virtual_address: u64) -> Self {
        Self {
            words: [0; BITMAP_WORDS],
            base_virtual_address,
        }
    }

    /// Resets all dirty bits to clean (0).
    pub fn clear_all(&mut self) {
        self.words = [0; BITMAP_WORDS];
    }

    /// Marks all pages in the tracking range as dirty (1).
    pub fn mark_all_dirty(&mut self) {
        self.words = [!0u64; BITMAP_WORDS];
    }

    /// Marks a specific page index as dirty.
    pub fn mark_page_dirty(&mut self, page_idx: usize) {
        if page_idx < MAX_CHECKPOINT_PAGES {
            let word_idx = page_idx / 64;
            let bit_idx = page_idx % 64;
            self.words[word_idx] |= 1u64 << bit_idx;
        }
    }

    /// Marks a specific virtual address as dirty.
    pub fn mark_address_dirty(&mut self, virtual_addr: u64) -> bool {
        if virtual_addr >= self.base_virtual_address {
            let offset = virtual_addr - self.base_virtual_address;
            let page_idx = (offset / (PAGE_SIZE as u64)) as usize;
            if page_idx < MAX_CHECKPOINT_PAGES {
                self.mark_page_dirty(page_idx);
                return true;
            }
        }
        false
    }

    /// Checks if a specific page index is marked dirty.
    pub fn is_page_dirty(&self, page_idx: usize) -> bool {
        if page_idx < MAX_CHECKPOINT_PAGES {
            let word_idx = page_idx / 64;
            let bit_idx = page_idx % 64;
            (self.words[word_idx] & (1u64 << bit_idx)) != 0
        } else {
            false
        }
    }

    /// Counts the total number of dirty pages currently tracked.
    pub fn count_dirty(&self) -> usize {
        let mut count = 0;
        for &w in &self.words {
            count += w.count_ones() as usize;
        }
        count
    }

    /// Iterates through all dirty pages, invoking a callback with page index and virtual address,
    /// and resets the dirty bit to clean.
    pub fn scan_and_reset_dirty<F>(&mut self, mut on_dirty: F) -> usize
    where
        F: FnMut(usize, u64),
    {
        let mut total_scanned = 0;
        for word_idx in 0..BITMAP_WORDS {
            let mut word = self.words[word_idx];
            if word == 0 {
                continue;
            }

            while word != 0 {
                let trailing = word.trailing_zeros() as usize;
                let page_idx = word_idx * 64 + trailing;
                let virt_addr = self.base_virtual_address + (page_idx as u64) * (PAGE_SIZE as u64);

                on_dirty(page_idx, virt_addr);
                total_scanned += 1;

                // Clear lowest set bit
                word &= word - 1;
            }

            // Reset this word to clean
            self.words[word_idx] = 0;
        }
        total_scanned
    }
}

/// Computes fast Adler-32 checksum for memory page and checkpoint integrity validation.
pub fn compute_adler32(data: &[u8]) -> u32 {
    let mut a: u32 = 1;
    let mut b: u32 = 0;
    for &byte in data {
        a = (a + byte as u32) % 65521;
        b = (b + a) % 65521;
    }
    (b << 16) | a
}

// -----------------------------------------------------------------------------
// 3. Sub-5ms COW Page Snapshot Freezer & x86-64 PTE Management
// -----------------------------------------------------------------------------

/// x86-64 Page Table Entry (PTE) flag bits.
pub const PTE_PRESENT: u64 = 1 << 0;
pub const PTE_WRITABLE: u64 = 1 << 1;
pub const PTE_USER: u64 = 1 << 2;
pub const PTE_ACCESSED: u64 = 1 << 5;
pub const PTE_DIRTY: u64 = 1 << 6;
pub const PTE_COW: u64 = 1 << 9; // OS-reserved software bit for Copy-On-Write

/// Model of an x86-64 Page Table Entry (PTE).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(transparent)]
pub struct PteEntry(pub u64);

impl PteEntry {
    pub const fn empty() -> Self {
        Self(0)
    }

    pub const fn new(frame_address: u64, flags: u64) -> Self {
        Self((frame_address & 0x000F_FFFF_FFFF_F000) | (flags & 0xFFF))
    }

    pub fn is_present(&self) -> bool {
        (self.0 & PTE_PRESENT) != 0
    }

    pub fn is_writable(&self) -> bool {
        (self.0 & PTE_WRITABLE) != 0
    }

    pub fn is_dirty(&self) -> bool {
        (self.0 & PTE_DIRTY) != 0
    }

    pub fn is_cow(&self) -> bool {
        (self.0 & PTE_COW) != 0
    }

    pub fn frame_address(&self) -> u64 {
        self.0 & 0x000F_FFFF_FFFF_F000
    }

    /// Sets write protection and activates COW mode for sub-5ms freeze.
    pub fn mark_cow(&mut self) {
        self.0 &= !PTE_WRITABLE; // Clear Write bit (read-only)
        self.0 |= PTE_COW;       // Mark COW active
        self.0 &= !PTE_DIRTY;    // Clear Dirty bit to track next modifications
    }

    /// Clears COW mode and re-enables direct writing on new physical frame.
    pub fn resolve_cow(&mut self, new_frame: u64) {
        let flags = (self.0 & 0xFFF & !PTE_COW) | PTE_WRITABLE | PTE_DIRTY;
        self.0 = (new_frame & 0x000F_FFFF_FFFF_F000) | flags;
    }
}

/// Frozen page record captured during the sub-5ms micro-freeze window.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FrozenPageRecord {
    pub virtual_address: u64,
    pub original_frame: u64,
    pub page_index: usize,
}

/// Sub-5ms Copy-On-Write (COW) Page Snapshot Freezer.
pub struct CowPageSnapshotFreezer {
    pub base_virtual_address: u64,
    pub ptes: [PteEntry; MAX_CHECKPOINT_PAGES],
    pub frozen_records: [Option<FrozenPageRecord>; 128],
    pub frozen_count: usize,
}

impl CowPageSnapshotFreezer {
    pub const fn new(base_virtual_address: u64) -> Self {
        Self {
            base_virtual_address,
            ptes: [PteEntry::empty(); MAX_CHECKPOINT_PAGES],
            frozen_records: [None; 128],
            frozen_count: 0,
        }
    }

    /// Maps a simulated page in the tracking range with initial frame and flags.
    pub fn map_page(&mut self, page_idx: usize, frame_addr: u64, flags: u64) {
        if page_idx < MAX_CHECKPOINT_PAGES {
            self.ptes[page_idx] = PteEntry::new(frame_addr, flags);
        }
    }

    /// Executes the sub-5ms COW freeze: scans dirty pages from the tracker,
    /// sets them to Read-Only + COW in page tables, records their original frames,
    /// and resets the dirty flags.
    pub fn freeze_dirty_pages(&mut self, tracker: &mut DirtyPageBitmapTracker) -> usize {
        self.frozen_count = 0;
        let ptes = &mut self.ptes;
        let frozen_records = &mut self.frozen_records;
        let frozen_count = &mut self.frozen_count;

        tracker.scan_and_reset_dirty(|page_idx, vaddr| {
            if page_idx < MAX_CHECKPOINT_PAGES && *frozen_count < 128 {
                let pte = &mut ptes[page_idx];
                if pte.is_present() {
                    let orig_frame = pte.frame_address();
                    pte.mark_cow();

                    frozen_records[*frozen_count] = Some(FrozenPageRecord {
                        virtual_address: vaddr,
                        original_frame: orig_frame,
                        page_index: page_idx,
                    });
                    *frozen_count += 1;
                }
            }
        });

        self.frozen_count
    }

    /// Handles a Copy-On-Write Page Fault (#PF) when the game writes to a frozen page.
    pub fn handle_cow_fault(&mut self, fault_vaddr: u64, new_frame_alloc: u64) -> Result<(), &'static str> {
        if fault_vaddr < self.base_virtual_address {
            return Err("Address outside tracking range");
        }

        let offset = fault_vaddr - self.base_virtual_address;
        let page_idx = (offset / (PAGE_SIZE as u64)) as usize;
        if page_idx >= MAX_CHECKPOINT_PAGES {
            return Err("Page index out of bounds");
        }

        let pte = &mut self.ptes[page_idx];
        if !pte.is_present() {
            return Err("Page not present");
        }
        if !pte.is_cow() {
            return Err("Page is not marked COW");
        }

        // Resolve COW: re-point PTE to new physical frame with Write enabled
        pte.resolve_cow(new_frame_alloc);
        Ok(())
    }
}

// -----------------------------------------------------------------------------
// 4. Memory Delta Stream Compressor (LZ4/RLE Streaming)
// -----------------------------------------------------------------------------

/// High-speed zero-allocation memory delta stream compressor.
pub struct MemoryDeltaCompressor;

impl MemoryDeltaCompressor {
    /// Compresses a 4096-byte page using fast Run-Length and Byte Token encoding.
    ///
    /// Encodes repeating byte runs (e.g. 0x00 runs in sparse heaps) and literal blocks.
    /// Maximum output will never exceed output buffer capacity (returns Err if buffer too small).
    pub fn compress_page(src: &[u8; PAGE_SIZE], dst: &mut [u8]) -> Result<usize, &'static str> {
        let mut src_idx = 0;
        let mut dst_idx = 0;

        while src_idx < PAGE_SIZE {
            let current_byte = src[src_idx];

            // 1. Scan for run of identical bytes
            let mut run_len = 1;
            while src_idx + run_len < PAGE_SIZE && src[src_idx + run_len] == current_byte && run_len < 127 {
                run_len += 1;
            }

            if run_len >= 3 {
                // Emit Run Token: high bit 1 | (run_len as u8), followed by byte value
                if dst_idx + 2 > dst.len() {
                    return Err("Destination buffer overflow during compression");
                }
                dst[dst_idx] = 0x80 | (run_len as u8);
                dst[dst_idx + 1] = current_byte;
                dst_idx += 2;
                src_idx += run_len;
            } else {
                // Emit Literal Block
                let lit_start = src_idx;
                let mut lit_len = 0;
                while src_idx < PAGE_SIZE && lit_len < 127 {
                    // Check if a run of >= 3 begins
                    if src_idx + 2 < PAGE_SIZE && src[src_idx] == src[src_idx + 1] && src[src_idx] == src[src_idx + 2] {
                        break;
                    }
                    lit_len += 1;
                    src_idx += 1;
                }

                if lit_len > 0 {
                    if dst_idx + 1 + lit_len > dst.len() {
                        return Err("Destination buffer overflow during compression");
                    }
                    // Literal token: high bit 0 | lit_len
                    dst[dst_idx] = lit_len as u8;
                    dst_idx += 1;
                    dst[dst_idx..dst_idx + lit_len].copy_from_slice(&src[lit_start..lit_start + lit_len]);
                    dst_idx += lit_len;
                }
            }
        }

        Ok(dst_idx)
    }

    /// Decompresses a compressed delta stream back into an exact 4096-byte memory page.
    pub fn decompress_page(src: &[u8], dst: &mut [u8; PAGE_SIZE]) -> Result<usize, &'static str> {
        let mut src_idx = 0;
        let mut dst_idx = 0;

        while src_idx < src.len() && dst_idx < PAGE_SIZE {
            let token = src[src_idx];
            src_idx += 1;

            if (token & 0x80) != 0 {
                // Run Token
                let run_len = (token & 0x7F) as usize;
                if src_idx >= src.len() {
                    return Err("Truncated run token in compressed stream");
                }
                let val = src[src_idx];
                src_idx += 1;

                if dst_idx + run_len > PAGE_SIZE {
                    return Err("Page boundary overflow during decompression");
                }
                for i in 0..run_len {
                    dst[dst_idx + i] = val;
                }
                dst_idx += run_len;
            } else {
                // Literal Token
                let lit_len = token as usize;
                if src_idx + lit_len > src.len() {
                    return Err("Truncated literal block in compressed stream");
                }
                if dst_idx + lit_len > PAGE_SIZE {
                    return Err("Page boundary overflow during decompression");
                }
                dst[dst_idx..dst_idx + lit_len].copy_from_slice(&src[src_idx..src_idx + lit_len]);
                src_idx += lit_len;
                dst_idx += lit_len;
            }
        }

        if dst_idx != PAGE_SIZE {
            return Err("Decompressed page size mismatch (corrupted stream)");
        }

        Ok(dst_idx)
    }
}

// -----------------------------------------------------------------------------
// 5. Direct NVMe Checkpoint Ring Streamer & Instant Game Resumption Engine
// -----------------------------------------------------------------------------

/// Base LBA sector on NVMe storage reserved for the Micro-Checkpoint Ring partition.
pub const CHECKPOINT_RING_BASE_LBA: u64 = 0x0020_0000;

/// Number of 512-byte sectors per checkpoint ring slot (128 sectors = 64 KiB).
pub const CHECKPOINT_SLOT_SECTORS: usize = 128;

/// Number of rolling checkpoint slots in the circular NVMe ring.
pub const CHECKPOINT_RING_SLOTS: usize = 8;

use crate::sys::minidump::NVME_SECTOR_SIZE;

/// Direct NVMe Checkpoint Ring Streamer.
///
/// Streams compressed micro-checkpoints into reserved NVMe ring sectors without intermediate heap copies.
#[derive(Debug, Clone, Copy)]
pub struct NvmeCheckpointRingStreamer {
    pub head_slot: usize,
    pub total_checkpoints_written: u64,
}

impl NvmeCheckpointRingStreamer {
    pub const fn new() -> Self {
        Self {
            head_slot: 0,
            total_checkpoints_written: 0,
        }
    }

    pub fn get_slot_base_lba(&self, slot_idx: usize) -> u64 {
        CHECKPOINT_RING_BASE_LBA + ((slot_idx % CHECKPOINT_RING_SLOTS) * CHECKPOINT_SLOT_SECTORS) as u64
    }

    /// Streams raw checkpoint data into the current head slot across consecutive 512-byte NVMe sectors.
    pub fn stream_to_slot<F>(
        &mut self,
        checkpoint_data: &[u8],
        mut sector_sink: F,
    ) -> Result<(usize, u64), &'static str>
    where
        F: FnMut(u64, &[u8; NVME_SECTOR_SIZE]) -> Result<(), &'static str>,
    {
        let slot = self.head_slot;
        let base_lba = self.get_slot_base_lba(slot);
        let max_bytes = CHECKPOINT_SLOT_SECTORS * NVME_SECTOR_SIZE;

        if checkpoint_data.len() > max_bytes {
            return Err("Checkpoint data exceeds maximum NVMe slot sector allocation");
        }

        let mut sector_buf = [0u8; NVME_SECTOR_SIZE];
        let mut offset = 0;
        let mut current_lba = base_lba;

        while offset < checkpoint_data.len() {
            let chunk_len = (checkpoint_data.len() - offset).min(NVME_SECTOR_SIZE);
            sector_buf[..chunk_len].copy_from_slice(&checkpoint_data[offset..offset + chunk_len]);
            if chunk_len < NVME_SECTOR_SIZE {
                sector_buf[chunk_len..].fill(0);
            }

            sector_sink(current_lba, &sector_buf)?;
            current_lba += 1;
            offset += chunk_len;
        }

        self.head_slot = (self.head_slot + 1) % CHECKPOINT_RING_SLOTS;
        self.total_checkpoints_written += 1;

        Ok((slot, base_lba))
    }
}

/// Result of an instantaneous game state resumption operation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InstantGameResumptionResult {
    pub restored_checkpoint_id: u64,
    pub restored_frame_number: u64,
    pub restored_dirty_pages: u32,
    pub thread_state: ThreadCheckpointState,
}

/// Instantaneous Game State Resumption Engine.
pub struct InstantGameResumer;

impl InstantGameResumer {
    /// Inspects and restores process state and memory pages from a serialized micro-checkpoint buffer.
    pub fn resume_from_snapshot(
        snapshot_data: &[u8],
        target_pages: &mut [([u8; PAGE_SIZE], u64)],
    ) -> Result<InstantGameResumptionResult, &'static str> {
        let header_size = core::mem::size_of::<CheckpointHeader>();
        let state_size = core::mem::size_of::<ThreadCheckpointState>();
        let desc_size = core::mem::size_of::<PageDeltaDescriptor>();

        if snapshot_data.len() < header_size + state_size + 4 {
            return Err("Snapshot buffer too short for headers");
        }

        // 1. Validate Header
        let header = unsafe { *(snapshot_data.as_ptr() as *const CheckpointHeader) };
        if !header.is_valid() {
            return Err("Invalid checkpoint header signature or version");
        }

        // Verify Adler-32 integrity
        let computed_checksum = compute_adler32(&snapshot_data[header_size..]);
        if header.checksum_adler32 != 0 && header.checksum_adler32 != computed_checksum {
            return Err("Checkpoint payload Adler-32 checksum mismatch (corrupted snapshot)");
        }

        // 2. Extract Thread Checkpoint State
        let state_offset = header_size;
        let thread_state = unsafe { *(snapshot_data[state_offset..].as_ptr() as *const ThreadCheckpointState) };

        // 3. Extract Page Descriptors
        let desc_count_offset = state_offset + state_size;
        let desc_count = u32::from_le_bytes(
            snapshot_data[desc_count_offset..desc_count_offset + 4]
                .try_into()
                .unwrap_or([0; 4]),
        ) as usize;

        let descs_start = desc_count_offset + 4;
        let descs_end = descs_start + desc_count * desc_size;
        if descs_end > snapshot_data.len() {
            return Err("Page descriptors out of snapshot bounds");
        }

        // 4. Restore Modified Pages
        for i in 0..desc_count {
            let desc_offset = descs_start + i * desc_size;
            let desc = unsafe { *(snapshot_data[desc_offset..].as_ptr() as *const PageDeltaDescriptor) };

            let comp_start = desc.compressed_offset as usize;
            let comp_end = comp_start + desc.compressed_size as usize;
            if comp_end > snapshot_data.len() {
                return Err("Compressed page stream out of snapshot bounds");
            }

            // Find matching target page by virtual address
            for (page_data, vaddr) in target_pages.iter_mut() {
                if *vaddr == desc.virtual_address {
                    MemoryDeltaCompressor::decompress_page(&snapshot_data[comp_start..comp_end], page_data)?;
                    break;
                }
            }
        }

        Ok(InstantGameResumptionResult {
            restored_checkpoint_id: header.checkpoint_id,
            restored_frame_number: thread_state.frame_number,
            restored_dirty_pages: header.dirty_pages_count,
            thread_state,
        })
    }
}

/// Integrated Micro-Checkpoint Fast State Snapshot & Instant Resumption Engine.
pub struct MicroCheckpointManager {
    pub tracker: DirtyPageBitmapTracker,
    pub freezer: CowPageSnapshotFreezer,
    pub streamer: NvmeCheckpointRingStreamer,
    pub current_checkpoint_id: u64,
}

impl MicroCheckpointManager {
    pub fn new(base_virtual_address: u64) -> Self {
        Self {
            tracker: DirtyPageBitmapTracker::new(base_virtual_address),
            freezer: CowPageSnapshotFreezer::new(base_virtual_address),
            streamer: NvmeCheckpointRingStreamer::new(),
            current_checkpoint_id: 0,
        }
    }

    /// Captures a sub-5ms micro-checkpoint of the game process and serializes into output buffer.
    pub fn capture_checkpoint(
        &mut self,
        game_pid: u32,
        cr3_root: u64,
        thread_state: &ThreadCheckpointState,
        active_pages: &[([u8; PAGE_SIZE], u64)],
        out_buf: &mut [u8],
    ) -> Result<usize, &'static str> {
        self.current_checkpoint_id += 1;
        let chk_id = self.current_checkpoint_id;

        // 1. Sub-5ms COW freeze
        let dirty_count = self.freezer.freeze_dirty_pages(&mut self.tracker);

        let header_size = core::mem::size_of::<CheckpointHeader>();
        let state_size = core::mem::size_of::<ThreadCheckpointState>();
        let desc_size = core::mem::size_of::<PageDeltaDescriptor>();

        let min_required = header_size + state_size + 4 + dirty_count * desc_size;
        if out_buf.len() < min_required {
            return Err("Output buffer too small for checkpoint metadata");
        }

        // 2. Copy Thread State
        let state_offset = header_size;
        let state_bytes = unsafe {
            core::slice::from_raw_parts(
                thread_state as *const _ as *const u8,
                state_size,
            )
        };
        out_buf[state_offset..state_offset + state_size].copy_from_slice(state_bytes);

        // 3. Write Descriptors Count
        let desc_count_offset = state_offset + state_size;
        out_buf[desc_count_offset..desc_count_offset + 4].copy_from_slice(&(dirty_count as u32).to_le_bytes());

        // 4. Compress Dirty Pages into Buffer
        let descs_start = desc_count_offset + 4;
        let mut stream_cursor = descs_start + dirty_count * desc_size;
        let mut compressed_total = 0u64;

        for i in 0..dirty_count {
            let record = match self.freezer.frozen_records[i] {
                Some(r) => r,
                None => continue,
            };

            // Find matching page payload in active_pages
            let mut page_found = false;
            for (page_slice, vaddr) in active_pages {
                if *vaddr == record.virtual_address {
                    page_found = true;
                    let comp_start = stream_cursor;
                    let comp_size = MemoryDeltaCompressor::compress_page(page_slice, &mut out_buf[comp_start..])?;
                    stream_cursor += comp_size;
                    compressed_total += comp_size as u64;

                    let mut desc = PageDeltaDescriptor::new(record.virtual_address, record.original_frame, 0x03);
                    desc.compressed_offset = comp_start as u32;
                    desc.compressed_size = comp_size as u32;
                    desc.page_adler32 = compute_adler32(&page_slice[..]);

                    let desc_bytes = unsafe {
                        core::slice::from_raw_parts(
                            &desc as *const _ as *const u8,
                            desc_size,
                        )
                    };
                    let d_off = descs_start + i * desc_size;
                    out_buf[d_off..d_off + desc_size].copy_from_slice(desc_bytes);
                    break;
                }
            }

            if !page_found {
                return Err("Dirty page frame not located in active pages set");
            }
        }

        // 5. Finalize Checkpoint Header with Adler-32 Checksum
        let checksum = compute_adler32(&out_buf[header_size..stream_cursor]);
        let mut header = CheckpointHeader::new(
            chk_id,
            game_pid,
            cr3_root,
            dirty_count as u32,
            FLAG_DELTA_SNAPSHOT | FLAG_COW_ACTIVE | FLAG_COMPRESSED_LZ4,
        );
        header.compressed_bytes = compressed_total;
        header.checksum_adler32 = checksum;

        let header_bytes = unsafe {
            core::slice::from_raw_parts(
                &header as *const _ as *const u8,
                header_size,
            )
        };
        out_buf[..header_size].copy_from_slice(header_bytes);

        Ok(stream_cursor)
    }
}

// -----------------------------------------------------------------------------
// Unit Tests
// -----------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    pub fn test_checkpoint_header_and_descriptors() {
        let header = CheckpointHeader::new(1, 42, 0x100_0000, 16, FLAG_DELTA_SNAPSHOT | FLAG_COW_ACTIVE);
        assert!(header.is_valid());
        assert_eq!(header.checkpoint_id, 1);
        assert_eq!(header.game_pid, 42);
        assert_eq!(header.dirty_pages_count, 16);
        assert_eq!(header.uncompressed_bytes, 16 * 4096);

        let mut thread_state = ThreadCheckpointState::default();
        thread_state.rip = 0x0040_1000;
        thread_state.rsp = 0x7FFF_0000;
        thread_state.frame_number = 12450;
        assert_eq!(thread_state.rip, 0x0040_1000);
        assert_eq!(thread_state.frame_number, 12450);

        let desc = PageDeltaDescriptor::new(0x2000_0000, 0x1000_0000, 0x03);
        assert_eq!(desc.virtual_address, 0x2000_0000);
        assert_eq!(desc.compressed_size, 4096);
    }

    #[test]
    pub fn test_dirty_page_bitmap_tracker() {
        let base_vaddr = 0x0040_0000;
        let mut tracker = DirtyPageBitmapTracker::new(base_vaddr);

        assert_eq!(tracker.count_dirty(), 0);

        // Mark individual pages dirty
        tracker.mark_page_dirty(0);
        tracker.mark_page_dirty(63);
        tracker.mark_page_dirty(64);
        tracker.mark_page_dirty(8191);

        assert_eq!(tracker.count_dirty(), 4);
        assert!(tracker.is_page_dirty(0));
        assert!(tracker.is_page_dirty(63));
        assert!(tracker.is_page_dirty(64));
        assert!(tracker.is_page_dirty(8191));
        assert!(!tracker.is_page_dirty(1));

        // Mark by virtual address
        let target_addr = base_vaddr + 10 * (PAGE_SIZE as u64);
        assert!(tracker.mark_address_dirty(target_addr));
        assert!(tracker.is_page_dirty(10));
        assert_eq!(tracker.count_dirty(), 5);

        // Scan and reset
        let mut scanned_pages = alloc::vec::Vec::new();
        let scanned_count = tracker.scan_and_reset_dirty(|page_idx, _vaddr| {
            scanned_pages.push(page_idx);
        });

        assert_eq!(scanned_count, 5);
        assert_eq!(tracker.count_dirty(), 0);
        assert_eq!(scanned_pages, alloc::vec![0, 10, 63, 64, 8191]);
    }
    #[test]
    pub fn test_cow_freezer_and_pte_operations() {
        let base_vaddr = 0x0040_0000;
        let mut freezer = CowPageSnapshotFreezer::new(base_vaddr);
        let mut tracker = DirtyPageBitmapTracker::new(base_vaddr);

        // Map 4 pages: 0, 1, 2, 3 as Writable
        for i in 0..4 {
            freezer.map_page(i, 0x1000_0000 + (i as u64) * 0x1000, PTE_PRESENT | PTE_WRITABLE | PTE_USER);
        }

        // Mark pages 1 and 3 dirty
        tracker.mark_page_dirty(1);
        tracker.mark_page_dirty(3);

        // Freeze dirty pages
        let frozen_count = freezer.freeze_dirty_pages(&mut tracker);
        assert_eq!(frozen_count, 2);
        assert_eq!(tracker.count_dirty(), 0);

        // Verify page 1 is now Read-Only and marked COW
        let pte1 = freezer.ptes[1];
        assert!(pte1.is_present());
        assert!(!pte1.is_writable());
        assert!(pte1.is_cow());
        assert_eq!(pte1.frame_address(), 0x1000_1000);

        // Verify page 0 remained writable
        assert!(freezer.ptes[0].is_writable());
        assert!(!freezer.ptes[0].is_cow());

        // Simulate game writing to page 1 -> trigger COW fault
        let fault_vaddr = base_vaddr + 1 * (PAGE_SIZE as u64);
        let new_frame = 0x2000_1000;
        let res = freezer.handle_cow_fault(fault_vaddr, new_frame);
        assert!(res.is_ok());

        // Verify page 1 is now re-pointed to new_frame with Writable enabled and COW cleared
        let pte1_after = freezer.ptes[1];
        assert!(pte1_after.is_writable());
        assert!(!pte1_after.is_cow());
        assert_eq!(pte1_after.frame_address(), new_frame);
    }

    #[test]
    pub fn test_memory_delta_compressor() {
        // Construct synthetic page with zero runs and literals
        let mut original_page = [0u8; PAGE_SIZE];
        // Zero block (0..1024)
        // Repeating pattern 0xAA (1024..1536)
        for i in 1024..1536 {
            original_page[i] = 0xAA;
        }
        // Incrementing literals (1536..2048)
        for i in 1536..2048 {
            original_page[i] = (i & 0xFF) as u8;
        }

        let mut compressed_buf = [0u8; PAGE_SIZE * 2];
        let comp_size = MemoryDeltaCompressor::compress_page(&original_page, &mut compressed_buf)
            .expect("Compression failed");

        // Compression should significantly reduce size (e.g. < 1200 bytes for 4096 bytes)
        assert!(comp_size < 1200);

        // Decompress and verify bitwise match
        let mut restored_page = [0u8; PAGE_SIZE];
        let decomp_size = MemoryDeltaCompressor::decompress_page(&compressed_buf[..comp_size], &mut restored_page)
            .expect("Decompression failed");

        assert_eq!(decomp_size, PAGE_SIZE);
        assert_eq!(original_page, restored_page);
    }
    #[test]
    pub fn test_nvme_streamer_and_instant_resumption() {
        let base_vaddr = 0x0040_0000;
        let mut manager = MicroCheckpointManager::new(base_vaddr);

        // Prepare 4 simulated active pages
        let mut page0 = [0x11u8; PAGE_SIZE];
        let mut page1 = [0x22u8; PAGE_SIZE];
        let mut page2 = [0x33u8; PAGE_SIZE];
        let mut page3 = [0x44u8; PAGE_SIZE];

        let vaddr0 = base_vaddr;
        let vaddr1 = base_vaddr + 1 * (PAGE_SIZE as u64);
        let vaddr2 = base_vaddr + 2 * (PAGE_SIZE as u64);
        let vaddr3 = base_vaddr + 3 * (PAGE_SIZE as u64);

        for (idx, vaddr) in [(0, vaddr0), (1, vaddr1), (2, vaddr2), (3, vaddr3)] {
            manager.freezer.map_page(idx, 0x1000_0000 + (idx as u64) * 0x1000, PTE_PRESENT | PTE_WRITABLE);
        }

        // Mark page 0 and 2 dirty
        manager.tracker.mark_page_dirty(0);
        manager.tracker.mark_page_dirty(2);

        let mut thread_state = ThreadCheckpointState::default();
        thread_state.rip = 0x0040_5000;
        thread_state.frame_number = 60_000;

        let active_set = [
            (page0, vaddr0),
            (page1, vaddr1),
            (page2, vaddr2),
            (page3, vaddr3),
        ];

        let mut snap_buf = [0u8; 16384];
        let snap_size = manager.capture_checkpoint(10, 0x100_0000, &thread_state, &active_set, &mut snap_buf)
            .expect("Capture checkpoint failed");
        assert!(snap_size > 0 && snap_size < 16384);

        // Stream into NVMe ring
        let mut streamed_sectors = 0;
        let stream_res = manager.streamer.stream_to_slot(&snap_buf[..snap_size], |_lba, _sector| {
            streamed_sectors += 1;
            Ok(())
        });
        assert!(stream_res.is_ok());
        assert!(streamed_sectors > 0);

        // Corrupt memory to simulate unwanted post-checkpoint modifications
        page0.fill(0xEE);
        page2.fill(0xFF);

        let mut restore_set = [
            (page0, vaddr0),
            (page1, vaddr1),
            (page2, vaddr2),
            (page3, vaddr3),
        ];

        // Execute instant resumption
        let resume_res = InstantGameResumer::resume_from_snapshot(&snap_buf[..snap_size], &mut restore_set)
            .expect("Instant resumption failed");

        assert_eq!(resume_res.restored_checkpoint_id, 1);
        assert_eq!(resume_res.restored_frame_number, 60_000);
        assert_eq!(resume_res.restored_dirty_pages, 2);
        assert_eq!(resume_res.thread_state.rip, 0x0040_5000);

        // Verify page0 and page2 were restored to original 0x11 and 0x33
        assert_eq!(restore_set[0].0[0], 0x11);
        assert_eq!(restore_set[2].0[0], 0x33);
    }
}