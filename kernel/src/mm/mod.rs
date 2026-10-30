//! Memory Management Architecture (MM) - Transparent Huge Pages & NUMA Subsystem.
//!
//! Subsystem providing 2MB Transparent Huge Pages (THP), large page collapse/split,
//! zero-fill optimization, AMD 3D V-Cache topology detection, and NUMA core affinity.

pub mod thp;

pub use thp::{
    HugePageAllocator, HugePageEntry, HugePageFlags, HugePhysFrame, PageDirectoryTable,
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
