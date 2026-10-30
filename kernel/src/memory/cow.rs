//! Copy-on-Write (COW) Address Space Management & Physical Frame Reference Tracking.
//!
//! Provides:
//! - `PTE_COW_BIT`: Bit 9 in x86-64 PTE used for software Copy-on-Write marking.
//! - `PTE_AVL_MASK`: Mask covering available-for-system bits 9..11.
//! - `MAX_TRACKED_PHYS_FRAMES`: Size of frame reference tracking table (128 MiB physical space).

use super::vmm::PageTableFlags;

pub const PTE_COW_BIT: u64 = PageTableFlags::COW;
pub const PTE_AVL_MASK: u64 = 0x0E00; // Bits 9..11 in x86-64 page table entry
pub const MAX_TRACKED_PHYS_FRAMES: usize = 32768; // 32,768 * 4 KiB = 128 MiB

/// Unit test verifying bitfields and constants for Copy-on-Write subsystem.
pub fn test_cow_bitfields() -> bool {
    assert_eq!(PTE_COW_BIT, 1 << 9);
    assert_eq!(PTE_AVL_MASK, (1 << 9) | (1 << 10) | (1 << 11));
    assert!((PTE_COW_BIT & PTE_AVL_MASK) == PTE_COW_BIT);
    true
}
