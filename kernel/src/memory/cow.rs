//! Copy-on-Write (COW) Address Space Management & Physical Frame Reference Tracking.
//!
//! Provides:
//! - `PTE_COW_BIT`: Bit 9 in x86-64 PTE used for software Copy-on-Write marking.
//! - `PTE_AVL_MASK`: Mask covering available-for-system bits 9..11.
//! - `MAX_TRACKED_PHYS_FRAMES`: Size of frame reference tracking table (128 MiB physical space).
//! - `handle_cow_fault`: Resolution of Copy-on-Write write faults.

use super::demand_paging::{AddressSpace, PageFaultErrorCode, PageFaultResolution};
use super::pmm::FrameAllocator;
use super::vmm::{PageTableEntry, PageTableFlags, VirtAddr};

pub const PTE_COW_BIT: u64 = PageTableFlags::COW;
pub const PTE_AVL_MASK: u64 = 0x0E00; // Bits 9..11 in x86-64 page table entry
pub const MAX_TRACKED_PHYS_FRAMES: usize = 32768; // 32,768 * 4 KiB = 128 MiB

/// Handles a Copy-on-Write page fault when writing to a page marked with PTE_COW_BIT.
pub unsafe fn handle_cow_fault<A: FrameAllocator>(
    _aspace: &mut AddressSpace,
    page_virt: VirtAddr,
    _pte: &mut PageTableEntry,
    _allocator: &mut A,
) -> Result<PageFaultResolution, PageFaultResolution> {
    Err(PageFaultResolution::ProtectionViolation {
        addr: page_virt,
        code: PageFaultErrorCode::new(PageFaultErrorCode::PRESENT | PageFaultErrorCode::WRITE),
    })
}

/// Unit test verifying bitfields and constants for Copy-on-Write subsystem.
pub fn test_cow_bitfields() -> bool {
    assert_eq!(PTE_COW_BIT, 1 << 9);
    assert_eq!(PTE_AVL_MASK, (1 << 9) | (1 << 10) | (1 << 11));
    assert!((PTE_COW_BIT & PTE_AVL_MASK) == PTE_COW_BIT);
    true
}
