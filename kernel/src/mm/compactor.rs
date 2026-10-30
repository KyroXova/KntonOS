//! Physical Memory Compactor & Two-Pointer Migration Scanner.
//!
//! Implements memory compaction for buddy and slab allocators:
//! - Physical memory fragmentation index calculation (0.0 = contiguous, 1.0 = fragmented).
//! - Movable page frame identification (unpinned userland/anonymous frames vs pinned kernel/DMA).
//! - Two-pointer migration scanner:
//!   * `migrate_pfn` scans upwards from the bottom of the zone searching for movable pages.
//!   * `free_pfn` scans downwards from the top of the zone searching for free target frames.
//! - Pairs candidates for atomic physical frame migration and defragmentation.

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
/// and two-pointer migration discovery.
pub struct CompactionZone {
    pub base_pfn: usize,
    pub total_frames: usize,
    pub frames: [PageDescriptor; MAX_ZONE_FRAMES],
    pub scanner: TwoPointerScanner,
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
            return 0.0; // No free memory to fragment
        }

        // Count free pages that are part of contiguous free blocks >= target_block_pages
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

        // Fragmentation formula: (Free_Total - Free_in_Large_Blocks) / Free_Total
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
            // 1. Advance migrate_pfn upwards to find movable page
            let m_idx = self.scanner.migrate_pfn - base;
            if m_idx >= self.total_frames {
                self.scanner.completed = true;
                return None;
            }

            if !self.frames[m_idx].is_movable() {
                self.scanner.migrate_pfn += 1;
                continue;
            }

            // 2. Advance free_pfn downwards to find free page
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

            // Found movable page at migrate_pfn and free frame at free_pfn - 1
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

            // Step both pointers
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
}

// -----------------------------------------------------------------------------
// Unit Tests
// -----------------------------------------------------------------------------

/// Unit test verifying physical memory fragmentation index and two-pointer scanner.
pub fn test_two_pointer_compactor_scanner() -> bool {
    let base_pfn = 0x2000; // e.g. 32 MiB physical (0x0200_0000 / 4096)
    let total_frames = 100;
    let mut zone = CompactionZone::new(base_pfn, total_frames);

    // Initial state: all 100 frames are free.
    // 100 contiguous free frames. If target block size is 20, fragmentation should be 0.0
    let initial_frag = zone.calculate_fragmentation_index(20);
    if initial_frag != 0.0 {
        return false;
    }

    // Pattern alternating: free, movable, free, movable... (checkerboard fragmentation)
    for i in 0..total_frames {
        if i % 2 == 1 {
            zone.set_frame_movable(base_pfn + i, 10, 0x0040_0000 + i * PAGE_SIZE_4KB);
        }
    }

    let (free_cnt, mov_cnt, unmov_cnt) = zone.count_frame_types();
    if free_cnt != 50 || mov_cnt != 50 || unmov_cnt != 0 {
        return false;
    }

    // Since largest contiguous free block is 1 page, asking for 2 contiguous pages should yield 1.0 (100% fragmented)
    let frag_index = zone.calculate_fragmentation_index(2);
    if frag_index != 1.0 {
        return false;
    }

    // Run Two-Pointer Migration Scanner
    // Migrate scanner moves up from base_pfn (finding odd frames: 1, 3, 5...)
    // Free scanner moves down from base_pfn + 100 (finding even frames: 98, 96, 94...)
    let mut pairs = [MigrationPair::default(); MAX_MIGRATION_PAIRS];
    let pairs_found = zone.scan_compaction_candidates(50, &mut pairs);

    // In a 100-frame zone with 50 movable and 50 free interleaved,
    // the two pointers will meet near index 50.
    // Movable pages in the lower half (PFN 1, 3, 5 ... 49 = 25 pages) will be paired
    // with free frames in the upper half (PFN 98, 96, 94 ... 50 = 25 pages).
    if pairs_found != 25 {
        return false;
    }

    // Verify first pair: source is 1st movable page (base + 1), target is highest free page (base + 98)
    if pairs[0].source_pfn != base_pfn + 1 || pairs[0].target_pfn != base_pfn + 98 {
        return false;
    }

    // Verify last pair: source is base + 49, target is base + 50
    if pairs[24].source_pfn != base_pfn + 49 || pairs[24].target_pfn != base_pfn + 50 {
        return false;
    }

    // Verify scanner marked completed
    if !zone.scanner.completed || zone.scan_next_migration_pair().is_some() {
        return false;
    }

    true
}
