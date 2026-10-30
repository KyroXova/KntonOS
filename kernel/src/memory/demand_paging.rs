//! Virtual Memory Demand Paging, Virtual Memory Areas (VMA), and Page Fault Resolution.
//!
//! Provides the core architectural abstraction for userland and kernel virtual memory:
//! - `PageFaultErrorCode`: x86-64 architectural exception error code bit decoder.
//! - `VmFlags`: Memory protection, allocation, and Copy-on-Write bitmask flags.
//! - `VmArea`: Descriptor for contiguous virtual address ranges.

use super::pmm::PAGE_SIZE;
use super::vmm::{VirtAddr, PageTableFlags};

/// x86-64 Architectural Page Fault Exception Error Code (CR2 contains faulting linear address).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[repr(transparent)]
pub struct PageFaultErrorCode(pub u64);

impl PageFaultErrorCode {
    pub const PRESENT: u64 = 1 << 0;          // Bit 0: 0 = non-present page, 1 = protection violation
    pub const WRITE: u64 = 1 << 1;            // Bit 1: 0 = read access, 1 = write access
    pub const USER: u64 = 1 << 2;             // Bit 2: 0 = supervisor/kernel, 1 = userland (CPL 3)
    pub const RESERVED_WRITE: u64 = 1 << 3;   // Bit 3: 1 = reserved bits overwritten in paging structures
    pub const INSTRUCTION_FETCH: u64 = 1 << 4;// Bit 4: 1 = instruction fetch (NX / Execute Disable)
    pub const PROTECTION_KEY: u64 = 1 << 5;   // Bit 5: 1 = protection key violation
    pub const SHADOW_STACK: u64 = 1 << 6;     // Bit 6: 1 = shadow stack access fault
    pub const SGX: u64 = 1 << 15;             // Bit 15: 1 = SGX access control violation

    #[inline(always)]
    pub const fn new(raw: u64) -> Self {
        Self(raw)
    }

    #[inline(always)]
    pub const fn raw(&self) -> u64 {
        self.0
    }

    /// True if the fault was caused by a page-protection violation; false if non-present.
    #[inline(always)]
    pub const fn is_present(&self) -> bool {
        (self.0 & Self::PRESENT) != 0
    }

    /// True if the fault was caused by a memory write access.
    #[inline(always)]
    pub const fn is_write(&self) -> bool {
        (self.0 & Self::WRITE) != 0
    }

    /// True if the processor was executing in user mode (CPL 3) when the fault occurred.
    #[inline(always)]
    pub const fn is_user(&self) -> bool {
        (self.0 & Self::USER) != 0
    }

    /// True if reserved bits in paging structures were detected as set.
    #[inline(always)]
    pub const fn is_reserved_write(&self) -> bool {
        (self.0 & Self::RESERVED_WRITE) != 0
    }

    /// True if the fault was triggered by an instruction fetch on a NO_EXECUTE page.
    #[inline(always)]
    pub const fn is_instruction_fetch(&self) -> bool {
        (self.0 & Self::INSTRUCTION_FETCH) != 0
    }

    /// True if the fault represents an instruction fetch violation.
    #[inline(always)]
    pub const fn is_nx_violation(&self) -> bool {
        self.is_instruction_fetch()
    }
}

/// Bitfield flags describing permissions, storage backing, and sharing semantics for a VMA.
pub struct VmFlags;

impl VmFlags {
    pub const VM_NONE: u64 = 0;
    pub const VM_READ: u64 = 1 << 0;       // Page is readable
    pub const VM_WRITE: u64 = 1 << 1;      // Page is writable
    pub const VM_EXEC: u64 = 1 << 2;       // Page is executable
    pub const VM_SHARED: u64 = 1 << 3;     // Changes shared across mapped processes
    pub const VM_ANONYMOUS: u64 = 1 << 4;  // Zero-initialized memory (not backed by file)
    pub const VM_COW: u64 = 1 << 5;        // Copy-on-Write enabled
    pub const VM_USER: u64 = 1 << 6;       // Userland accessible
}

/// Virtual Memory Area (VMA) describing a contiguous, uniformly-protected region of address space.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VmArea {
    pub start: VirtAddr,
    pub end: VirtAddr,
    pub flags: u64,
}

impl VmArea {
    /// Constructs a new Virtual Memory Area spanning [start, end) with given flags.
    pub fn new(start: VirtAddr, end: VirtAddr, flags: u64) -> Self {
        assert!(start.0 <= end.0, "VMA start address must be <= end address");
        Self { start, end, flags }
    }

    /// Tests if a virtual address falls within [start, end).
    #[inline(always)]
    pub fn contains(&self, addr: VirtAddr) -> bool {
        addr.0 >= self.start.0 && addr.0 < self.end.0
    }

    /// Total byte length of this VMA.
    #[inline(always)]
    pub fn len(&self) -> usize {
        self.end.0.saturating_sub(self.start.0)
    }

    /// Total number of 4 KiB virtual pages in this VMA.
    #[inline(always)]
    pub fn page_count(&self) -> usize {
        (self.len() + PAGE_SIZE - 1) / PAGE_SIZE
    }

    #[inline(always)]
    pub fn is_readable(&self) -> bool {
        (self.flags & VmFlags::VM_READ) != 0
    }

    #[inline(always)]
    pub fn is_writable(&self) -> bool {
        (self.flags & VmFlags::VM_WRITE) != 0
    }

    #[inline(always)]
    pub fn is_executable(&self) -> bool {
        (self.flags & VmFlags::VM_EXEC) != 0
    }

    #[inline(always)]
    pub fn is_shared(&self) -> bool {
        (self.flags & VmFlags::VM_SHARED) != 0
    }

    #[inline(always)]
    pub fn is_anonymous(&self) -> bool {
        (self.flags & VmFlags::VM_ANONYMOUS) != 0
    }

    #[inline(always)]
    pub fn is_cow(&self) -> bool {
        (self.flags & VmFlags::VM_COW) != 0
    }

    #[inline(always)]
    pub fn is_user(&self) -> bool {
        (self.flags & VmFlags::VM_USER) != 0
    }

    /// Translates VmArea permission flags to x86-64 PageTableFlags.
    pub fn to_page_table_flags(&self) -> u64 {
        let mut flags = PageTableFlags::PRESENT;
        if self.is_writable() && !self.is_cow() {
            flags |= PageTableFlags::WRITABLE;
        }
        if self.is_user() {
            flags |= PageTableFlags::USER_ACCESSIBLE;
        }
        if !self.is_executable() {
            flags |= PageTableFlags::NO_EXECUTE;
        }
        if self.is_cow() {
            flags |= PageTableFlags::COW;
        }
        flags
    }
}

/// Unit test verifying VMA region descriptor manipulation and page fault error code decoding.
pub fn test_vma_and_error_codes() -> bool {
    // 1. Test PageFaultErrorCode decoding
    let code_read_not_present = PageFaultErrorCode::new(0x0); // Read, not present, kernel
    assert!(!code_read_not_present.is_present());
    assert!(!code_read_not_present.is_write());
    assert!(!code_read_not_present.is_user());

    let code_user_write_fault = PageFaultErrorCode::new(
        PageFaultErrorCode::PRESENT | PageFaultErrorCode::WRITE | PageFaultErrorCode::USER,
    );
    assert!(code_user_write_fault.is_present());
    assert!(code_user_write_fault.is_write());
    assert!(code_user_write_fault.is_user());
    assert!(!code_user_write_fault.is_instruction_fetch());

    let code_nx_fault = PageFaultErrorCode::new(
        PageFaultErrorCode::PRESENT | PageFaultErrorCode::INSTRUCTION_FETCH,
    );
    assert!(code_nx_fault.is_present());
    assert!(code_nx_fault.is_instruction_fetch());
    assert!(code_nx_fault.is_nx_violation());

    // 2. Test VmArea calculations
    let start = VirtAddr::new(0x0000_7FFF_0000_0000);
    let end = VirtAddr::new(0x0000_7FFF_0004_0000); // 256 KiB = 64 pages
    let vma = VmArea::new(
        start,
        end,
        VmFlags::VM_READ | VmFlags::VM_WRITE | VmFlags::VM_ANONYMOUS | VmFlags::VM_USER,
    );

    assert_eq!(vma.len(), 0x40000);
    assert_eq!(vma.page_count(), 64);
    assert!(vma.contains(start));
    assert!(vma.contains(VirtAddr::new(0x0000_7FFF_0001_0000)));
    assert!(!vma.contains(end)); // End is exclusive
    assert!(!vma.contains(VirtAddr::new(0x0000_7FFF_0005_0000)));

    assert!(vma.is_readable());
    assert!(vma.is_writable());
    assert!(vma.is_anonymous());
    assert!(vma.is_user());
    assert!(!vma.is_cow());
    assert!(!vma.is_executable());

    // Check mapping to hardware page table flags
    let pte_flags = vma.to_page_table_flags();
    assert!((pte_flags & PageTableFlags::PRESENT) != 0);
    assert!((pte_flags & PageTableFlags::WRITABLE) != 0);
    assert!((pte_flags & PageTableFlags::USER_ACCESSIBLE) != 0);
    assert!((pte_flags & PageTableFlags::NO_EXECUTE) != 0);
    assert!((pte_flags & PageTableFlags::COW) == 0);

    true
}
