//! Memory Management Architecture (MM) - Transparent Huge Pages & NUMA Subsystem.
//!
//! Subsystem providing 2MB Transparent Huge Pages (THP), large page collapse/split,
//! zero-fill optimization, AMD 3D V-Cache topology detection, and NUMA core affinity.

pub mod thp;

pub use thp::{
    HugePageAllocator, HugePageEntry, HugePageFlags, HugePhysFrame, HugeZeroPool, PageDirectoryTable,
    PageTable4KB, PageTableEntry4KB, ThpMode, ThpStats, TransparentHugePageDaemon,
    VirtualAddressComponents, OFFSET_MASK_2MB, PAGES_PER_2MB, PAGE_SIZE_2MB, PAGE_SIZE_4KB,
};

/// Unit test verifying 2MB physical allocator and PML4/PDPT/PD translation structures.
pub fn verify_thp_allocator_and_structures() -> bool {
    let mut allocator = HugePageAllocator::new();
    let base_phys = 0x0200_0000; // 32 MiB physical (2MB aligned)

    if !allocator.init(base_phys, 16) {
        return false;
    }

    if allocator.total_pages() != 16 || allocator.free_pages() != 16 {
        return false;
    }

    // Allocate 1st huge page
    let frame1 = match allocator.allocate_2mb_frame() {
        Some(f) => f,
        None => return false,
    };

    if frame1.base_address() != base_phys {
        return false;
    }

    // Allocate 2nd huge page
    let frame2 = match allocator.allocate_2mb_frame() {
        Some(f) => f,
        None => return false,
    };

    if frame2.base_address() != base_phys + PAGE_SIZE_2MB {
        return false;
    }

    if allocator.allocated_pages() != 2 || allocator.free_pages() != 14 {
        return false;
    }

    // Verify Page Directory 2MB mapping and translation
    let mut pd = PageDirectoryTable::new();
    let test_vaddr = 0x0000_7FFF_8020_4000; // Virtual address with offset 0x4000
    let flags = HugePageFlags::PRESENT | HugePageFlags::WRITABLE | HugePageFlags::USER_ACCESSIBLE;

    if pd.map_2mb(test_vaddr, frame1.base_address(), flags).is_err() {
        return false;
    }

    let translated = match pd.translate_2mb(test_vaddr) {
        Some(p) => p,
        None => return false,
    };

    if translated != frame1.base_address() + 0x4000 {
        return false;
    }

    // Unmap and verify
    let unmapped = match pd.unmap_2mb(test_vaddr) {
        Some(f) => f,
        None => return false,
    };

    if unmapped != frame1 {
        return false;
    }

    if pd.translate_2mb(test_vaddr).is_some() {
        return false;
    }

    // Deallocate and check pool counters
    if !allocator.deallocate_2mb_frame(frame1) || !allocator.deallocate_2mb_frame(frame2) {
        return false;
    }

    if allocator.free_pages() != 16 || allocator.allocated_pages() != 0 {
        return false;
    }

    true
}

/// Unit test verifying THP collapse daemon, split engine, and pre-zeroed pool optimization.
pub fn verify_thp_collapse_split_and_zero_pool() -> bool {
    let mut allocator = HugePageAllocator::new();
    let base_phys = 0x0400_0000; // 64 MiB physical (2MB aligned)

    if !allocator.init(base_phys, 16) {
        return false;
    }

    let mut daemon = TransparentHugePageDaemon::new(ThpMode::Always);

    // 1. Replenish pre-zeroed pool with 4 frames
    let replenished = daemon.zero_pool.replenish(&mut allocator);
    if replenished != 8 || daemon.zero_pool.available() != 8 {
        return false;
    }

    // 2. Build a simulated 512-entry 4KB Page Table
    let mut pt = PageTable4KB::new();
    let pt_flags = HugePageFlags::PRESENT | HugePageFlags::WRITABLE | HugePageFlags::USER_ACCESSIBLE;
    let mock_base = 0x0100_0000;
    for i in 0..PAGES_PER_2MB {
        pt.entries[i] = PageTableEntry4KB::new(mock_base + i * PAGE_SIZE_4KB, pt_flags);
    }

    if pt.present_count() != 512 || !pt.has_uniform_permissions() {
        return false;
    }

    // 3. Scan and collapse into a 2MB huge page
    let mut pd = PageDirectoryTable::new();
    let vaddr_collapse = 0x0000_7FFF_A000_0000;

    let huge_frame = match daemon.scan_and_collapse(&mut pd, &pt, vaddr_collapse, &mut allocator) {
        Ok(f) => f,
        Err(_) => return false,
    };

    if daemon.stats.pages_collapsed != 1 || daemon.stats.zero_pool_hits != 1 {
        return false;
    }

    // Validate that PD maps this address as 2MB huge page
    let trans_mid = match pd.translate_2mb(vaddr_collapse + 0x1000) {
        Some(addr) => addr,
        None => return false,
    };
    if trans_mid != huge_frame.base_address() + 0x1000 {
        return false;
    }

    // 4. Split the 2MB huge page back into 512 4KB entries
    let split_pt = match daemon.split_huge_page(&mut pd, vaddr_collapse, &mut allocator) {
        Ok(t) => t,
        Err(_) => return false,
    };

    if daemon.stats.pages_split != 1 {
        return false;
    }

    // PD entry should now be unmapped
    if pd.translate_2mb(vaddr_collapse).is_some() {
        return false;
    }

    // Verify reconstructed 4KB PTEs
    if split_pt.present_count() != 512 {
        return false;
    }
    if split_pt.entries[0].physical_address() != huge_frame.base_address() {
        return false;
    }
    if split_pt.entries[511].physical_address() != huge_frame.base_address() + 511 * PAGE_SIZE_4KB {
        return false;
    }

    true
}
