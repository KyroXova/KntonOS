//! Virtual Memory Demand Paging, Virtual Memory Areas (VMA), and Page Fault Resolution.
//!
//! Provides the core architectural abstraction for userland and kernel virtual memory:
//! - `PageFaultErrorCode`: x86-64 architectural exception error code bit decoder.
//! - `VmFlags`: Memory protection, allocation, and Copy-on-Write bitmask flags.
//! - `VmArea`: Descriptor for contiguous virtual address ranges.
//! - `AddressSpace`: Process virtual address space containing VMAs and root PML4.
//! - `handle_page_fault`: Core demand paging allocation and page fault resolution engine.

use super::pmm::{FrameAllocator, PhysAddr, PAGE_SIZE};
use super::vmm::{invlpg, PageTable, PageTableFlags, PageTableManager, VirtAddr};
use alloc::vec::Vec;

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

/// Represents an isolated process virtual address space with a root PML4 table and set of VMAs.
pub struct AddressSpace {
    pub root_pml4: *mut PageTable,
    pub vmas: Vec<VmArea>,
}

impl AddressSpace {
    /// Creates a new AddressSpace bound to a root PML4 table.
    pub fn new(root_pml4: *mut PageTable) -> Self {
        Self {
            root_pml4,
            vmas: Vec::new(),
        }
    }

    /// Adds a Virtual Memory Area to the address space if it does not overlap with existing VMAs.
    pub fn add_vma(&mut self, new_vma: VmArea) -> bool {
        if new_vma.len() == 0 {
            return false;
        }

        // Check for overlaps
        for vma in &self.vmas {
            if !(new_vma.end.0 <= vma.start.0 || new_vma.start.0 >= vma.end.0) {
                return false; // Overlap detected
            }
        }

        self.vmas.push(new_vma);
        true
    }

    /// Finds the VMA containing the target virtual address.
    pub fn find_vma(&self, addr: VirtAddr) -> Option<&VmArea> {
        self.vmas.iter().find(|vma| vma.contains(addr))
    }

    /// Finds a mutable reference to the VMA containing the target virtual address.
    pub fn find_vma_mut(&mut self, addr: VirtAddr) -> Option<&mut VmArea> {
        self.vmas.iter_mut().find(|vma| vma.contains(addr))
    }

    /// Returns a PageTableManager bound to this address space's root PML4 table.
    pub unsafe fn page_table_manager(&self) -> PageTableManager {
        PageTableManager::new(self.root_pml4)
    }
}

/// Result of evaluating and attempting to resolve a page fault.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PageFaultResolution {
    /// Non-present anonymous page was demand allocated and zero-filled.
    ResolvedDemandPage {
        virt: VirtAddr,
        phys: PhysAddr,
    },
    /// Writable access to Copy-on-Write page was resolved via page duplication or unsharing.
    ResolvedCow {
        virt: VirtAddr,
        new_phys: PhysAddr,
    },
    /// Address does not fall within any registered Virtual Memory Area (SIGSEGV).
    SegmentationFault {
        addr: VirtAddr,
        code: PageFaultErrorCode,
    },
    /// Access violated VMA permissions (e.g. write to read-only non-COW, or execute on non-executable).
    ProtectionViolation {
        addr: VirtAddr,
        code: PageFaultErrorCode,
    },
    /// Out of physical memory to satisfy demand allocation or COW duplication.
    OutOfMemory,
}

/// Core demand paging & page fault handler.
/// Evaluates the faulting address and error code against process address space VMAs and page tables.
pub unsafe fn handle_page_fault<A: FrameAllocator>(
    aspace: &mut AddressSpace,
    fault_addr: VirtAddr,
    error_code: PageFaultErrorCode,
    allocator: &mut A,
) -> Result<PageFaultResolution, PageFaultResolution> {
    // 1. Locate the Virtual Memory Area containing the faulting address
    let vma = match aspace.find_vma(fault_addr) {
        Some(v) => *v,
        None => {
            return Err(PageFaultResolution::SegmentationFault {
                addr: fault_addr,
                code: error_code,
            });
        }
    };

    // 2. Validate architectural permissions
    if error_code.is_instruction_fetch() && !vma.is_executable() {
        return Err(PageFaultResolution::ProtectionViolation {
            addr: fault_addr,
            code: error_code,
        });
    }

    if error_code.is_user() && !vma.is_user() {
        return Err(PageFaultResolution::ProtectionViolation {
            addr: fault_addr,
            code: error_code,
        });
    }

    let page_virt = VirtAddr::new(fault_addr.0 & !0xFFF);

    // 3. Handle non-present page fault (Demand Paging)
    if !error_code.is_present() {
        if vma.is_anonymous() {
            // Allocate a fresh 4 KiB physical frame
            let frame = allocator.allocate_frame().ok_or(PageFaultResolution::OutOfMemory)?;
            let phys_addr = frame.start_address();

            // Zero-fill the 4096-byte frame
            let slice = core::slice::from_raw_parts_mut(phys_addr.0 as *mut u8, PAGE_SIZE);
            slice.fill(0);

            // Map the page in the address space with VMA permissions
            let mut pt_mgr = aspace.page_table_manager();
            let flags = vma.to_page_table_flags();

            if let Err(_) = pt_mgr.map_page(page_virt, phys_addr, flags, allocator) {
                return Err(PageFaultResolution::OutOfMemory);
            }

            invlpg(page_virt);

            return Ok(PageFaultResolution::ResolvedDemandPage {
                virt: page_virt,
                phys: phys_addr,
            });
        } else {
            return Err(PageFaultResolution::SegmentationFault {
                addr: fault_addr,
                code: error_code,
            });
        }
    }

    // 4. Handle protection violation on write: Check for Copy-on-Write
    if error_code.is_write() {
        let mut pt_mgr = aspace.page_table_manager();
        if let Some(pte) = pt_mgr.get_pte_mut(page_virt) {
            if pte.is_cow() && vma.is_writable() {
                // Defer to cow::handle_cow_fault
                return super::cow::handle_cow_fault(aspace, page_virt, pte, allocator);
            }
        }

        return Err(PageFaultResolution::ProtectionViolation {
            addr: fault_addr,
            code: error_code,
        });
    }

    Err(PageFaultResolution::ProtectionViolation {
        addr: fault_addr,
        code: error_code,
    })
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

/// Unit test verifying demand paging allocation engine and anonymous zero-page fault resolution.
pub fn test_demand_paging_engine() -> bool {
    unsafe {
        static mut TEST_ROOT_PML4: PageTable = PageTable::new();
        let pml4_ptr = core::ptr::addr_of_mut!(TEST_ROOT_PML4);
        (*pml4_ptr).zero();

        let mut aspace = AddressSpace::new(pml4_ptr);
        let mut pmm = super::PMM.lock();

        let vma_start = VirtAddr::new(0x0000_7000_0000_0000);
        let vma_end = VirtAddr::new(0x0000_7000_0002_0000); // 128 KiB
        let vma = VmArea::new(
            vma_start,
            vma_end,
            VmFlags::VM_READ | VmFlags::VM_WRITE | VmFlags::VM_ANONYMOUS | VmFlags::VM_USER,
        );
        assert!(aspace.add_vma(vma));

        // 1. Fault at unmapped address inside anonymous VMA (offset 0x1000)
        let fault_addr = VirtAddr::new(0x0000_7000_0000_1000);
        let error_code = PageFaultErrorCode::new(PageFaultErrorCode::WRITE | PageFaultErrorCode::USER); // Present=0

        let res = handle_page_fault(&mut aspace, fault_addr, error_code, &mut *pmm);
        match res {
            Ok(PageFaultResolution::ResolvedDemandPage { virt, phys }) => {
                assert_eq!(virt, fault_addr);
                assert!(phys.0 > 0);

                // Verify the page is now mapped and translated accurately
                let pt_mgr = aspace.page_table_manager();
                let translated = pt_mgr.translate(fault_addr).expect("Demand page must be translated");
                assert_eq!(translated, phys);

                // Verify the physical memory is zero-filled
                let slice = core::slice::from_raw_parts(phys.0 as *const u8, PAGE_SIZE);
                for &byte in slice {
                    assert_eq!(byte, 0, "Demand allocated page must be zero-filled");
                }
            }
            _ => panic!("Expected ResolvedDemandPage"),
        }

        // 2. Fault at address outside any VMA must produce SegmentationFault
        let segv_addr = VirtAddr::new(0x0000_1000_0000_0000);
        let segv_res = handle_page_fault(&mut aspace, segv_addr, error_code, &mut *pmm);
        match segv_res {
            Err(PageFaultResolution::SegmentationFault { addr, .. }) => {
                assert_eq!(addr, segv_addr);
            }
            _ => panic!("Expected SegmentationFault for unmapped address"),
        }

        // 3. Write fault to a Read-Only VMA must produce ProtectionViolation
        let ro_start = VirtAddr::new(0x0000_6000_0000_0000);
        let ro_end = VirtAddr::new(0x0000_6000_0001_0000);
        let ro_vma = VmArea::new(ro_start, ro_end, VmFlags::VM_READ | VmFlags::VM_ANONYMOUS);
        assert!(aspace.add_vma(ro_vma));

        let ro_fault_addr = VirtAddr::new(0x0000_6000_0000_0000);
        let ro_write_code = PageFaultErrorCode::new(PageFaultErrorCode::PRESENT | PageFaultErrorCode::WRITE);
        let ro_res = handle_page_fault(&mut aspace, ro_fault_addr, ro_write_code, &mut *pmm);
        match ro_res {
            Err(PageFaultResolution::ProtectionViolation { addr, .. }) => {
                assert_eq!(addr, ro_fault_addr);
            }
            _ => panic!("Expected ProtectionViolation for write to read-only VMA"),
        }

        true
    }
}


/// Comprehensive Demand Paging & Copy-on-Write (COW) verification suite emitting [KNT-MEM-COW-PASS].
pub fn test_cow_and_demand_paging() -> bool {
    crate::serial_write_str("[KNT-MEM-TEST] Executing Demand Paging & COW Fork verification...\r\n");

    // 1. Verify VMA structures and x86-64 page fault error code decoder
    if !test_vma_and_error_codes() {
        crate::serial_write_str("[KNT-PANIC] VMA descriptors or page fault error code decoder failed!\r\n");
        return false;
    }

    // 2. Verify COW bitfield definitions
    if !super::cow::test_cow_bitfields() {
        crate::serial_write_str("[KNT-PANIC] Copy-on-Write PTE bitfield definitions failed!\r\n");
        return false;
    }

    // 3. Verify Demand Paging allocation engine and zero-page fault resolution
    if !test_demand_paging_engine() {
        crate::serial_write_str("[KNT-PANIC] Demand paging allocation engine check failed!\r\n");
        return false;
    }

    // 4. Verify COW address space cloning and physical frame reference counter
    if !super::cow::test_cow_fork_and_reference_counter() {
        crate::serial_write_str("[KNT-PANIC] COW address space fork & refcounter check failed!\r\n");
        return false;
    }

    // 5. Multi-page, multi-process memory isolation and demand zeroing verification
    unsafe {
        static mut PARENT_PML4: PageTable = PageTable::new();
        static mut CHILD_PML4: PageTable = PageTable::new();
        let parent_ptr = core::ptr::addr_of_mut!(PARENT_PML4);
        let child_ptr = core::ptr::addr_of_mut!(CHILD_PML4);
        (*parent_ptr).zero();
        (*child_ptr).zero();

        let mut parent = AddressSpace::new(parent_ptr);
        let mut pmm = super::PMM.lock();

        let vma = VmArea::new(
            VirtAddr::new(0x0000_7500_0000_0000),
            VirtAddr::new(0x0000_7500_0003_0000), // 3 pages: 0x0000, 0x1000, 0x2000
            VmFlags::VM_READ | VmFlags::VM_WRITE | VmFlags::VM_ANONYMOUS | VmFlags::VM_USER,
        );
        assert!(parent.add_vma(vma));

        let page0 = VirtAddr::new(0x0000_7500_0000_0000);
        let page1 = VirtAddr::new(0x0000_7500_0000_1000);
        let page2 = VirtAddr::new(0x0000_7500_0000_2000);

        let demand_code = PageFaultErrorCode::new(PageFaultErrorCode::WRITE | PageFaultErrorCode::USER);
        let cow_code = PageFaultErrorCode::new(PageFaultErrorCode::PRESENT | PageFaultErrorCode::WRITE | PageFaultErrorCode::USER);

        // Demand allocate page 0 and page 2 in parent
        let res0 = handle_page_fault(&mut parent, page0, demand_code, &mut *pmm).expect("Demand page0 failed");
        let res2 = handle_page_fault(&mut parent, page2, demand_code, &mut *pmm).expect("Demand page2 failed");

        let phys0 = match res0 { PageFaultResolution::ResolvedDemandPage { phys, .. } => phys, _ => panic!() };
        let phys2 = match res2 { PageFaultResolution::ResolvedDemandPage { phys, .. } => phys, _ => panic!() };

        // Write pattern data
        let slice0 = core::slice::from_raw_parts_mut(phys0.0 as *mut u32, PAGE_SIZE / 4);
        slice0[0] = 0x11223344;

        let slice2 = core::slice::from_raw_parts_mut(phys2.0 as *mut u32, PAGE_SIZE / 4);
        slice2[0] = 0x55667788;

        super::cow::FRAME_REF_COUNTER.lock().set_ref(phys0, 1);
        super::cow::FRAME_REF_COUNTER.lock().set_ref(phys2, 1);

        // Fork into child
        let mut child = super::cow::fork_address_space(&mut parent, child_ptr, &mut *pmm).expect("Fork failed");

        // Both pages must be COW in parent and child
        assert_eq!(super::cow::FRAME_REF_COUNTER.lock().get_ref(phys0), 2);
        assert_eq!(super::cow::FRAME_REF_COUNTER.lock().get_ref(phys2), 2);

        // Child writes to page 0 -> triggers COW
        let child_cow0 = handle_page_fault(&mut child, page0, cow_code, &mut *pmm).expect("Child COW page0 failed");
        let child_phys0 = match child_cow0 { PageFaultResolution::ResolvedCow { new_phys, .. } => new_phys, _ => panic!() };
        assert_ne!(child_phys0, phys0);

        let child_slice0 = core::slice::from_raw_parts_mut(child_phys0.0 as *mut u32, PAGE_SIZE / 4);
        assert_eq!(child_slice0[0], 0x11223344);
        child_slice0[0] = 0x99AABBCC;

        // Verify parent memory isolation
        let parent_slice0 = core::slice::from_raw_parts(phys0.0 as *const u32, PAGE_SIZE / 4);
        assert_eq!(parent_slice0[0], 0x11223344, "Parent memory must remain isolated");

        // Child writes to page 1 (unmapped in both) -> triggers Demand Allocation
        let child_demand1 = handle_page_fault(&mut child, page1, demand_code, &mut *pmm).expect("Child demand page1 failed");
        let child_phys1 = match child_demand1 { PageFaultResolution::ResolvedDemandPage { phys, .. } => phys, _ => panic!() };

        let child_slice1 = core::slice::from_raw_parts_mut(child_phys1.0 as *mut u32, PAGE_SIZE / 4);
        assert_eq!(child_slice1[0], 0, "Demand allocated page must be zeroed");
        child_slice1[0] = 0xDEADBEEF;

        // Verify page 1 is STILL unmapped in parent
        assert!(parent.page_table_manager().get_pte(page1).is_none(), "Parent page 1 must remain unmapped");

        // Parent writes to page 2 (shared with child) -> Parent gets new frame
        let parent_cow2 = handle_page_fault(&mut parent, page2, cow_code, &mut *pmm).expect("Parent COW page2 failed");
        let parent_phys2 = match parent_cow2 { PageFaultResolution::ResolvedCow { new_phys, .. } => new_phys, _ => panic!() };
        assert_ne!(parent_phys2, phys2);

        let parent_slice2 = core::slice::from_raw_parts_mut(parent_phys2.0 as *mut u32, PAGE_SIZE / 4);
        assert_eq!(parent_slice2[0], 0x55667788);
        parent_slice2[0] = 0xCAFEBABE;

        // Now child is sole owner of original phys2
        assert_eq!(super::cow::FRAME_REF_COUNTER.lock().get_ref(phys2), 1);

        // Child writes to page 2 -> must be unshared in-place
        let child_cow2 = handle_page_fault(&mut child, page2, cow_code, &mut *pmm).expect("Child in-place COW page2 failed");
        match child_cow2 {
            PageFaultResolution::ResolvedCow { new_phys, .. } => assert_eq!(new_phys, phys2),
            _ => panic!("Expected in-place unshare"),
        }

        let child_slice2 = core::slice::from_raw_parts(phys2.0 as *const u32, PAGE_SIZE / 4);
        assert_eq!(child_slice2[0], 0x55667788, "Child must preserve original page 2 content");
    }

    crate::serial_write_str("[KNT-MEM-COW-PASS] Demand paging, COW fork engine, and page fault recovery verified\r\n");
    true
}
