//! Physical Memory Compactor & Two-Pointer Migration Scanner.
//!
//! Implements memory compaction for buddy and slab allocators:
//! - Physical memory fragmentation index calculation (0.0 = contiguous, 1.0 = fragmented).
//! - Movable page frame identification (unpinned userland/anonymous frames vs pinned kernel/DMA).
//! - Two-pointer migration scanner:
//!   * `migrate_pfn` scans upwards from the bottom of the zone searching for movable pages.
//!   * `free_pfn` scans downwards from the top of the zone searching for free target frames.
//! - Pairs candidates for atomic physical frame migration, defragmentation, and swapping.

use core::sync::atomic::{fence, Ordering};
use super::thp::PAGE_SIZE_4KB;

/// Maximum number of tracked 4KB physical page frames per compaction zone (e.g. 2048 frames = 8 MiB).
pub const MAX_ZONE_FRAMES: usize = 2048;

/// Maximum number of migration candidate pairs buffered in a single compaction pass.
pub const MAX_MIGRATION_PAIRS: usize = 256;

/// Page frame allocation and mobility state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PageFrameStatus {
    /// Frame is currently free and available as a migration target.
    Free,
    /// Frame is allocated to an unpinned userland mapping and is safe to relocate.
    Movable,
    /// Frame is pinned by kernel, DMA, MMIO, or active page table leaf and cannot move.
    Unmovable,
}

/// Metadata descriptor for a single 4KB physical page frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PageDescriptor {
    pub pfn: usize,
    pub status: PageFrameStatus,
    pub owner_pid: u32,
    pub virtual_address: usize,
    pub pinned: bool,
    pub dirty: bool,
}

impl PageDescriptor {
    pub const fn empty(pfn: usize) -> Self {
        Self {
            pfn,
            status: PageFrameStatus::Free,
            owner_pid: 0,
            virtual_address: 0,
            pinned: false,
            dirty: false,
        }
    }

    /// Returns true if this frame can be migrated by the compactor.
    #[inline]
    pub fn is_movable(&self) -> bool {
        self.status == PageFrameStatus::Movable && !self.pinned
    }

    /// Returns true if this frame is free and eligible as a migration destination.
    #[inline]
    pub fn is_free(&self) -> bool {
        self.status == PageFrameStatus::Free
    }
}

/// Candidate pair of (source, destination) physical page frames for migration.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct MigrationPair {
    pub source_pfn: usize,
    pub target_pfn: usize,
    pub virtual_address: usize,
    pub owner_pid: u32,
}

impl MigrationPair {
    pub const fn new(source_pfn: usize, target_pfn: usize, virtual_address: usize, owner_pid: u32) -> Self {
        Self {
            source_pfn,
            target_pfn,
            virtual_address,
            owner_pid,
        }
    }
}

/// Error conditions encountered during frame migration.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CompactionError {
    SourceNotMovable,
    TargetNotFree,
    OutOfBounds,
    IdenticalFrames,
    PinnedFrame,
}

/// Compaction engine telemetry and operation counters.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct CompactorStats {
    pub pages_scanned: usize,
    pub pages_migrated: usize,
    pub pages_failed: usize,
    pub bytes_moved: usize,
    pub compaction_passes: usize,
}

/// Compaction pass outcome metrics.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CompactionResult {
    pub pairs_scanned: usize,
    pub pages_migrated: usize,
    pub initial_fragmentation: f32,
    pub final_fragmentation: f32,
}

/// Two-pointer compaction scanner traversing physical address space.
#[derive(Debug, Clone, Copy)]
pub struct TwoPointerScanner {
    pub zone_start_pfn: usize,
    pub zone_end_pfn: usize,
    pub migrate_pfn: usize,
    pub free_pfn: usize,
    pub completed: bool,
}

impl TwoPointerScanner {
    pub fn new(zone_start_pfn: usize, frame_count: usize) -> Self {
        let zone_end_pfn = zone_start_pfn + frame_count;
        Self {
            zone_start_pfn,
            zone_end_pfn,
            migrate_pfn: zone_start_pfn,
            free_pfn: zone_end_pfn,
            completed: false,
        }
    }

    pub fn reset(&mut self) {
        self.migrate_pfn = self.zone_start_pfn;
        self.free_pfn = self.zone_end_pfn;
        self.completed = false;
    }
}

/// Physical memory compaction zone managing frame tracking, fragmentation evaluation,
/// two-pointer migration discovery, and atomic frame swapping.
pub struct CompactionZone {
    pub base_pfn: usize,
    pub total_frames: usize,
    pub frames: [PageDescriptor; MAX_ZONE_FRAMES],
    pub scanner: TwoPointerScanner,
    pub stats: CompactorStats,
}

impl CompactionZone {
    /// Initializes a compaction zone starting at `base_pfn` spanning `total_frames`.
    pub fn new(base_pfn: usize, total_frames: usize) -> Self {
        let count = total_frames.min(MAX_ZONE_FRAMES);
        let mut frames = [PageDescriptor::empty(0); MAX_ZONE_FRAMES];
        for i in 0..count {
            frames[i] = PageDescriptor::empty(base_pfn + i);
        }

        Self {
            base_pfn,
            total_frames: count,
            frames,
            scanner: TwoPointerScanner::new(base_pfn, count),
            stats: CompactorStats::default(),
        }
    }

    /// Marks a physical frame as occupied and movable with reverse mapping info.
    pub fn set_frame_movable(&mut self, pfn: usize, owner_pid: u32, vaddr: usize) -> bool {
        if pfn < self.base_pfn || pfn >= self.base_pfn + self.total_frames {
            return false;
        }
        let idx = pfn - self.base_pfn;
        self.frames[idx].status = PageFrameStatus::Movable;
        self.frames[idx].owner_pid = owner_pid;
        self.frames[idx].virtual_address = vaddr;
        self.frames[idx].pinned = false;
        true
    }

    /// Marks a physical frame as unmovable (pinned kernel memory, DMA buffer, page table).
    pub fn set_frame_unmovable(&mut self, pfn: usize) -> bool {
        if pfn < self.base_pfn || pfn >= self.base_pfn + self.total_frames {
            return false;
        }
        let idx = pfn - self.base_pfn;
        self.frames[idx].status = PageFrameStatus::Unmovable;
        self.frames[idx].pinned = true;
        true
    }

    /// Marks a physical frame as free.
    pub fn set_frame_free(&mut self, pfn: usize) -> bool {
        if pfn < self.base_pfn || pfn >= self.base_pfn + self.total_frames {
            return false;
        }
        let idx = pfn - self.base_pfn;
        self.frames[idx] = PageDescriptor::empty(pfn);
        true
    }

    /// Counts free and movable frames in the compaction zone.
    pub fn count_frame_types(&self) -> (usize, usize, usize) {
        let mut free = 0;
        let mut movable = 0;
        let mut unmovable = 0;
        for i in 0..self.total_frames {
            match self.frames[i].status {
                PageFrameStatus::Free => free += 1,
                PageFrameStatus::Movable => {
                    if self.frames[i].pinned {
                        unmovable += 1;
                    } else {
                        movable += 1;
                    }
                }
                PageFrameStatus::Unmovable => unmovable += 1,
            }
        }
        (free, movable, unmovable)
    }

    /// Calculates external memory fragmentation index for a requested contiguous block order.
    /// `target_block_pages`: Contiguous chunk size (e.g. 512 pages for 2MB huge page).
    /// Returns index from 0.0 (no fragmentation, completely contiguous) to 1.0 (severe fragmentation).
    pub fn calculate_fragmentation_index(&self, target_block_pages: usize) -> f32 {
        let (free_total, _, _) = self.count_frame_types();
        if free_total == 0 {
            return 0.0;
        }

        let mut free_in_large_blocks = 0;
        let mut current_run = 0;

        for i in 0..self.total_frames {
            if self.frames[i].is_free() {
                current_run += 1;
            } else {
                if current_run >= target_block_pages {
                    free_in_large_blocks += current_run;
                }
                current_run = 0;
            }
        }
        if current_run >= target_block_pages {
            free_in_large_blocks += current_run;
        }

        if free_in_large_blocks >= free_total {
            return 0.0;
        }

        let fragmented_free = free_total - free_in_large_blocks;
        (fragmented_free as f32) / (free_total as f32)
    }

    /// Performs one step of the two-pointer compaction scanner:
    /// - Advances `migrate_pfn` upwards to locate the next movable page.
    /// - Advances `free_pfn` downwards to locate the next free destination frame.
    /// Returns `Some(MigrationPair)` if a candidate pair is found where `migrate_pfn < free_pfn`.
    pub fn scan_next_migration_pair(&mut self) -> Option<MigrationPair> {
        if self.scanner.completed {
            return None;
        }

        let base = self.base_pfn;

        while self.scanner.migrate_pfn < self.scanner.free_pfn {
            let m_idx = self.scanner.migrate_pfn - base;
            if m_idx >= self.total_frames {
                self.scanner.completed = true;
                return None;
            }

            if !self.frames[m_idx].is_movable() {
                self.scanner.migrate_pfn += 1;
                continue;
            }

            if self.scanner.free_pfn <= self.scanner.migrate_pfn {
                self.scanner.completed = true;
                return None;
            }

            let f_idx = self.scanner.free_pfn - 1 - base;
            if f_idx >= self.total_frames {
                self.scanner.free_pfn -= 1;
                continue;
            }

            if !self.frames[f_idx].is_free() {
                self.scanner.free_pfn -= 1;
                continue;
            }

            let src_pfn = self.scanner.migrate_pfn;
            let dst_pfn = self.scanner.free_pfn - 1;

            if src_pfn >= dst_pfn {
                self.scanner.completed = true;
                return None;
            }

            let pair = MigrationPair::new(
                src_pfn,
                dst_pfn,
                self.frames[m_idx].virtual_address,
                self.frames[m_idx].owner_pid,
            );

            self.scanner.migrate_pfn += 1;
            self.scanner.free_pfn -= 1;

            return Some(pair);
        }

        self.scanner.completed = true;
        None
    }

    /// Scans a batch of migration candidate pairs up to `max_pairs`.
    pub fn scan_compaction_candidates(
        &mut self,
        max_pairs: usize,
        out_pairs: &mut [MigrationPair],
    ) -> usize {
        let mut count = 0;
        let cap = max_pairs.min(out_pairs.len());

        while count < cap {
            match self.scan_next_migration_pair() {
                Some(pair) => {
                    out_pairs[count] = pair;
                    count += 1;
                }
                None => break,
            }
        }
        count
    }

    /// Executes the migration of a single physical page frame from source to target.
    /// Memory barriers guarantee data store consistency prior to reverse mapping updates.
    pub fn migrate_page_frame(&mut self, pair: MigrationPair) -> Result<(), CompactionError> {
        if pair.source_pfn == pair.target_pfn {
            return Err(CompactionError::IdenticalFrames);
        }

        if pair.source_pfn < self.base_pfn || pair.source_pfn >= self.base_pfn + self.total_frames {
            return Err(CompactionError::OutOfBounds);
        }
        if pair.target_pfn < self.base_pfn || pair.target_pfn >= self.base_pfn + self.total_frames {
            return Err(CompactionError::OutOfBounds);
        }

        let src_idx = pair.source_pfn - self.base_pfn;
        let dst_idx = pair.target_pfn - self.base_pfn;

        if !self.frames[src_idx].is_movable() {
            self.stats.pages_failed += 1;
            return Err(CompactionError::SourceNotMovable);
        }
        if !self.frames[dst_idx].is_free() {
            self.stats.pages_failed += 1;
            return Err(CompactionError::TargetNotFree);
        }

        // Memory barrier ensuring prior writes to the source frame are committed
        fence(Ordering::SeqCst);

        // Copy frame descriptor metadata to destination
        let src_meta = self.frames[src_idx];
        self.frames[dst_idx].status = PageFrameStatus::Movable;
        self.frames[dst_idx].owner_pid = src_meta.owner_pid;
        self.frames[dst_idx].virtual_address = src_meta.virtual_address;
        self.frames[dst_idx].pinned = false;
        self.frames[dst_idx].dirty = src_meta.dirty;

        // Reset source frame to free
        self.frames[src_idx] = PageDescriptor::empty(pair.source_pfn);

        // Memory barrier guaranteeing page table modifications see new frame contents
        fence(Ordering::SeqCst);

        self.stats.pages_migrated += 1;
        self.stats.bytes_moved += PAGE_SIZE_4KB;

        Ok(())
    }

    /// Swaps two physical page frames atomically.
    pub fn swap_page_frames(&mut self, pfn_a: usize, pfn_b: usize) -> Result<(), CompactionError> {
        if pfn_a == pfn_b {
            return Err(CompactionError::IdenticalFrames);
        }
        if pfn_a < self.base_pfn || pfn_a >= self.base_pfn + self.total_frames {
            return Err(CompactionError::OutOfBounds);
        }
        if pfn_b < self.base_pfn || pfn_b >= self.base_pfn + self.total_frames {
            return Err(CompactionError::OutOfBounds);
        }

        let idx_a = pfn_a - self.base_pfn;
        let idx_b = pfn_b - self.base_pfn;

        if self.frames[idx_a].pinned || self.frames[idx_b].pinned {
            return Err(CompactionError::PinnedFrame);
        }

        fence(Ordering::SeqCst);

        let meta_a = self.frames[idx_a];
        let meta_b = self.frames[idx_b];

        self.frames[idx_a].status = meta_b.status;
        self.frames[idx_a].owner_pid = meta_b.owner_pid;
        self.frames[idx_a].virtual_address = meta_b.virtual_address;
        self.frames[idx_a].dirty = meta_b.dirty;

        self.frames[idx_b].status = meta_a.status;
        self.frames[idx_b].owner_pid = meta_a.owner_pid;
        self.frames[idx_b].virtual_address = meta_a.virtual_address;
        self.frames[idx_b].dirty = meta_a.dirty;

        fence(Ordering::SeqCst);

        self.stats.pages_migrated += 2;
        self.stats.bytes_moved += PAGE_SIZE_4KB * 2;

        Ok(())
    }

    /// Runs a full compaction pass: scans migration pairs and moves pages.
    pub fn compact_zone(&mut self, target_block_pages: usize, max_migrations: usize) -> CompactionResult {
        let initial_frag = self.calculate_fragmentation_index(target_block_pages);
        self.scanner.reset();

        let mut pairs = [MigrationPair::default(); MAX_MIGRATION_PAIRS];
        let pairs_found = self.scan_compaction_candidates(max_migrations, &mut pairs);

        let mut migrated = 0;
        for i in 0..pairs_found {
            if self.migrate_page_frame(pairs[i]).is_ok() {
                migrated += 1;
            }
        }

        let final_frag = self.calculate_fragmentation_index(target_block_pages);
        self.stats.compaction_passes += 1;

        CompactionResult {
            pairs_scanned: pairs_found,
            pages_migrated: migrated,
            initial_fragmentation: initial_frag,
            final_fragmentation: final_frag,
        }
    }
}

// -----------------------------------------------------------------------------
// Unit Tests
// -----------------------------------------------------------------------------

/// Unit test verifying physical memory fragmentation index and two-pointer scanner.
pub fn test_two_pointer_compactor_scanner() -> bool {
    let base_pfn = 0x2000;
    let total_frames = 100;
    let mut zone = CompactionZone::new(base_pfn, total_frames);

    let initial_frag = zone.calculate_fragmentation_index(20);
    if initial_frag != 0.0 {
        return false;
    }

    for i in 0..total_frames {
        if i % 2 == 1 {
            zone.set_frame_movable(base_pfn + i, 10, 0x0040_0000 + i * PAGE_SIZE_4KB);
        }
    }

    let (free_cnt, mov_cnt, unmov_cnt) = zone.count_frame_types();
    if free_cnt != 50 || mov_cnt != 50 || unmov_cnt != 0 {
        return false;
    }

    let frag_index = zone.calculate_fragmentation_index(2);
    if frag_index != 1.0 {
        return false;
    }

    let mut pairs = [MigrationPair::default(); MAX_MIGRATION_PAIRS];
    let pairs_found = zone.scan_compaction_candidates(50, &mut pairs);

    if pairs_found != 25 {
        return false;
    }

    if pairs[0].source_pfn != base_pfn + 1 || pairs[0].target_pfn != base_pfn + 98 {
        return false;
    }

    if pairs[24].source_pfn != base_pfn + 49 || pairs[24].target_pfn != base_pfn + 50 {
        return false;
    }

    if !zone.scanner.completed || zone.scan_next_migration_pair().is_some() {
        return false;
    }

    true
}

/// Unit test verifying movable page frame migration and atomic frame swapping.
pub fn test_frame_migration_and_atomic_swap() -> bool {
    let base_pfn = 0x3000;
    let total_frames = 100;
    let mut zone = CompactionZone::new(base_pfn, total_frames);

    // Setup checkerboard fragmentation: 50 movable (odd), 50 free (even)
    for i in 0..total_frames {
        if i % 2 == 1 {
            zone.set_frame_movable(base_pfn + i, 42, 0x0080_0000 + i * PAGE_SIZE_4KB);
        }
    }

    // Initial 20-page fragmentation is 1.0
    let init_frag = zone.calculate_fragmentation_index(20);
    if init_frag != 1.0 {
        return false;
    }

    // Execute compaction pass for up to 30 migrations
    let result = zone.compact_zone(20, 30);
    if result.pages_migrated != 25 {
        return false;
    }
    // After 25 migrations, the first 50 frames (0..49) are completely free,
    // forming a contiguous run of 50 free pages!
    if result.final_fragmentation != 0.0 {
        return false;
    }

    // Verify first 50 frames are all free
    for i in 0..50 {
        if !zone.frames[i].is_free() {
            return false;
        }
    }

    // Verify upper 50 frames are all occupied/movable
    for i in 50..100 {
        if !zone.frames[i].is_movable() || zone.frames[i].owner_pid != 42 {
            return false;
        }
    }

    // Verify atomic frame swapping
    let pfn_x = base_pfn + 60; // Movable (PID 42)
    let pfn_y = base_pfn + 10; // Free
    if zone.swap_page_frames(pfn_x, pfn_y).is_err() {
        return false;
    }

    if !zone.frames[60].is_free() || !zone.frames[10].is_movable() || zone.frames[10].owner_pid != 42 {
        return false;
    }

    // Verify stats
    if zone.stats.pages_migrated != 27 || zone.stats.compaction_passes != 1 {
        return false;
    }

    true
}
