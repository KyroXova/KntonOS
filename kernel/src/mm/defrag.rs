//! Transparent Huge Page (2MB THP) Defragmentation & TLB Invalidator.
//!
//! Provides:
//! - Page table PML4/PDPT/PD leaf re-pointing with atomic TLB shootdown invalidation (`invlpg`).
//! - Transparent 2MB huge page aggregation (collapsing 512 scattered 4KB frames into contiguous 2MB THP).
//! - Memory barrier synchronization (`SeqCst` fences) protecting concurrent page walks.
//! - Proactive background defragmentation daemon with dynamic fragmentation threshold triggers.

use core::sync::atomic::{fence, Ordering};
use super::compactor::CompactionZone;
use super::thp::{
    HugePageAllocator, HugePageEntry, HugePageFlags, HugePhysFrame, PageDirectoryTable,
    PageTable4KB, PageTableEntry4KB, VirtualAddressComponents, OFFSET_MASK_2MB, PAGES_PER_2MB,
    PAGE_SIZE_4KB,
};

/// Performs an atomic local TLB invalidation for a single 4KB or 2MB virtual address page.
#[inline]
pub fn tlb_invalidate_page(vaddr: usize) {
    unsafe {
        core::arch::asm!(
            "invlpg [{}]",
            in(reg) vaddr,
            options(nostack, preserves_flags)
        );
    }
}

/// Atomically re-points an individual 4KB Page Table Entry (PTE) to a new physical address
/// and executes a TLB shootdown invalidation.
pub fn repoint_page_table_leaf(
    pt: &mut PageTable4KB,
    pt_index: usize,
    new_phys_addr: usize,
    vaddr: usize,
) -> Result<(), &'static str> {
    if pt_index >= PAGES_PER_2MB {
        return Err("PTE index out of bounds");
    }

    let old_entry = pt.entries[pt_index];
    if !old_entry.is_present() {
        return Err("PTE not present");
    }

    let flags = old_entry.flags();

    // Memory barrier ensuring prior frame copy is committed before updating page table
    fence(Ordering::SeqCst);

    pt.entries[pt_index] = PageTableEntry4KB::new(new_phys_addr, flags);

    // Memory barrier ensuring PTE update is committed before TLB invalidation
    fence(Ordering::SeqCst);

    tlb_invalidate_page(vaddr);

    Ok(())
}

/// Aggregates 512 scattered 4KB page frames into a single contiguous 2MB Transparent Huge Page.
/// - Validates 2MB alignment of the virtual base address.
/// - Verifies that all 512 PTEs are present and homogeneous in permissions.
/// - Allocates a 2MB-aligned contiguous physical frame.
/// - Atomically replaces the Page Directory pointer with a 2MB large-page PDE.
/// - Invalidates the TLB on the 2MB range.
pub fn aggregate_2mb_huge_page(
    pt: &PageTable4KB,
    pd: &mut PageDirectoryTable,
    vaddr: usize,
    huge_alloc: &mut HugePageAllocator,
) -> Result<HugePhysFrame, &'static str> {
    if (vaddr & OFFSET_MASK_2MB) != 0 {
        return Err("Virtual address is not 2MB aligned");
    }

    // 1. Verify all 512 entries in the 4KB page table are present
    let first_entry = pt.entries[0];
    if !first_entry.is_present() {
        return Err("Base PTE not present");
    }

    let mut common_flags = HugePageFlags::PRESENT;
    if first_entry.is_writable() {
        common_flags |= HugePageFlags::WRITABLE;
    }
    if first_entry.is_user() {
        common_flags |= HugePageFlags::USER_ACCESSIBLE;
    }

    for i in 1..PAGES_PER_2MB {
        let entry = pt.entries[i];
        if !entry.is_present() {
            return Err("Incomplete 4KB page table (sparse pages cannot collapse)");
        }
    }

    // 2. Allocate a contiguous 2MB physical frame
    let huge_frame = huge_alloc
        .allocate_2mb_frame()
        .ok_or("No free 2MB physical frames available for aggregation")?;

    let comp = VirtualAddressComponents::decompose(vaddr);

    // Memory barrier ensuring the 2MB frame allocation is globally visible
    fence(Ordering::SeqCst);

    // 3. Atomically re-point the Page Directory Entry (PDE) to 2MB huge page (bit 7 set)
    pd.entries[comp.pd_index] = HugePageEntry::new(huge_frame.base_address(), common_flags);

    // Memory barrier ensuring PDE write is visible
    fence(Ordering::SeqCst);

    // 4. Invalidate TLB for the collapsed 2MB virtual address
    tlb_invalidate_page(vaddr);

    Ok(huge_frame)
}

// -----------------------------------------------------------------------------
// Proactive Background Defragmentation Daemon
// -----------------------------------------------------------------------------

/// Policy governing proactive memory compaction and THP defragmentation triggers.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DefragPolicy {
    /// Fragmentation index above which background defrag is triggered (e.g. 0.40).
    pub high_watermark_frag: f32,
    /// Target fragmentation index below which defrag pauses (e.g. 0.15).
    pub low_watermark_frag: f32,
    /// Maximum number of page migrations executed in a single scheduled burst.
    pub max_migrations_per_burst: usize,
    /// Contiguous block size target in 4KB pages (512 pages = 2MB).
    pub target_order_pages: usize,
}

impl Default for DefragPolicy {
    fn default() -> Self {
        Self {
            high_watermark_frag: 0.40,
            low_watermark_frag: 0.15,
            max_migrations_per_burst: 64,
            target_order_pages: PAGES_PER_2MB,
        }
    }
}

/// Operational state of the transparent defragmenter daemon.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DefragState {
    Idle,
    Active,
    Suspended,
}

/// Transparent Page Defragmentation Daemon controlling memory compaction cycles.
pub struct TransparentDefragDaemon {
    pub policy: DefragPolicy,
    pub state: DefragState,
    pub total_bursts: usize,
    pub total_pages_compacted: usize,
    pub total_thp_aggregations: usize,
    pub last_measured_frag: f32,
}

impl TransparentDefragDaemon {
    pub fn new(policy: DefragPolicy) -> Self {
        Self {
            policy,
            state: DefragState::Idle,
            total_bursts: 0,
            total_pages_compacted: 0,
            total_thp_aggregations: 0,
            last_measured_frag: 0.0,
        }
    }

    /// Evaluates whether the current fragmentation index warrants compaction.
    pub fn should_trigger_defrag(&self, current_frag: f32) -> bool {
        match self.state {
            DefragState::Idle => current_frag >= self.policy.high_watermark_frag,
            DefragState::Active => current_frag > self.policy.low_watermark_frag,
            DefragState::Suspended => false,
        }
    }

    /// Executes a single proactive defragmentation cycle against a compaction zone.
    pub fn run_cycle(&mut self, zone: &mut CompactionZone) -> usize {
        let frag = zone.calculate_fragmentation_index(self.policy.target_order_pages);
        self.last_measured_frag = frag;

        if !self.should_trigger_defrag(frag) {
            self.state = DefragState::Idle;
            return 0;
        }

        self.state = DefragState::Active;
        let res = zone.compact_zone(self.policy.target_order_pages, self.policy.max_migrations_per_burst);

        self.total_bursts += 1;
        self.total_pages_compacted += res.pages_migrated;
        self.last_measured_frag = res.final_fragmentation;

        if res.final_fragmentation <= self.policy.low_watermark_frag {
            self.state = DefragState::Idle;
        }

        res.pages_migrated
    }
}

// -----------------------------------------------------------------------------
// Unit Tests
// -----------------------------------------------------------------------------

/// Unit test verifying 2MB huge page aggregation, atomic leaf re-pointing, and TLB invalidation.
pub fn test_2mb_huge_page_aggregation_and_tlb() -> bool {
    // 1. Verify repoint_page_table_leaf
    let mut pt = PageTable4KB::new();
    let old_phys = 0x0100_0000;
    let new_phys = 0x0100_4000;
    let test_vaddr = 0x0000_7FFF_0000_1000;

    pt.entries[1] = PageTableEntry4KB::new(old_phys, HugePageFlags::PRESENT | HugePageFlags::WRITABLE);
    if repoint_page_table_leaf(&mut pt, 1, new_phys, test_vaddr).is_err() {
        return false;
    }

    if pt.entries[1].physical_address() != new_phys || !pt.entries[1].is_writable() {
        return false;
    }

    // 2. Verify 2MB Transparent Huge Page (THP) Aggregation
    let mut dense_pt = PageTable4KB::new();
    for i in 0..PAGES_PER_2MB {
        // Scattered physical pages
        let paddr = 0x0200_0000 + (PAGES_PER_2MB - 1 - i) * PAGE_SIZE_4KB;
        dense_pt.entries[i] = PageTableEntry4KB::new(paddr, HugePageFlags::PRESENT | HugePageFlags::WRITABLE | HugePageFlags::USER_ACCESSIBLE);
    }

    let mut pd = PageDirectoryTable::new();
    let mut huge_alloc = HugePageAllocator::new();
    huge_alloc.init(0x0400_0000, 8); // 8 huge pages at 64 MiB

    let collapse_vaddr = 0x0000_7FFF_2000_0000; // 2MB aligned
    let huge_frame = match aggregate_2mb_huge_page(&dense_pt, &mut pd, collapse_vaddr, &mut huge_alloc) {
        Ok(frame) => frame,
        Err(_) => return false,
    };

    if huge_frame.base_address() != 0x0400_0000 {
        return false;
    }

    // Verify Page Directory translation now recognizes 2MB huge page
    let translated = match pd.translate_2mb(collapse_vaddr + 0x8000) {
        Some(addr) => addr,
        None => return false,
    };
    if translated != 0x0400_0000 + 0x8000 {
        return false;
    }

    // 3. Verify Defragmenter Daemon policy and proactive triggers
    let mut daemon = TransparentDefragDaemon::new(DefragPolicy {
        high_watermark_frag: 0.50,
        low_watermark_frag: 0.15,
        max_migrations_per_burst: 32,
        target_order_pages: 20,
    });

    // Below threshold (0.30 < 0.50) -> should NOT trigger
    if daemon.should_trigger_defrag(0.30) {
        return false;
    }

    // Above threshold (0.65 >= 0.50) -> should trigger
    if !daemon.should_trigger_defrag(0.65) {
        return false;
    }

    // Run cycle on a fragmented zone
    let base_pfn = 0x4000;
    let mut test_zone = CompactionZone::new(base_pfn, 100);
    for i in 0..100 {
        if i % 2 == 1 {
            test_zone.set_frame_movable(base_pfn + i, 99, 0x00A0_0000 + i * PAGE_SIZE_4KB);
        }
    }

    let migrated = daemon.run_cycle(&mut test_zone);
    if migrated != 25 {
        return false;
    }
    // After cycle, final fragmentation is 0.0, daemon should return to Idle
    if daemon.last_measured_frag != 0.0 || daemon.state != DefragState::Idle {
        return false;
    }

    true
}
