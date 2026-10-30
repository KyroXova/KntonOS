//! Copy-on-Write (COW) Address Space Management & Physical Frame Reference Tracking.
//!
//! Implements:
//! - `FrameRefCounter`: Physical frame reference table tracking shared COW mappings.
//! - `fork_address_space`: Duplication of parent address space into child with COW write-protection.
//! - `handle_cow_fault`: Resolution of Copy-on-Write write faults (page copy or in-place unsharing).

use super::demand_paging::{AddressSpace, PageFaultErrorCode, PageFaultResolution, VmArea, VmFlags};
use super::pmm::{FrameAllocator, PhysAddr, PAGE_SIZE};
use super::spinlock::Spinlock;
use super::vmm::{invlpg, PageTable, PageTableEntry, PageTableFlags, VirtAddr, VmmError};

pub const PTE_COW_BIT: u64 = PageTableFlags::COW;
pub const PTE_AVL_MASK: u64 = 0x0E00; // Bits 9..11 in x86-64 page table entry
pub const MAX_TRACKED_PHYS_FRAMES: usize = 32768; // 32,768 * 4 KiB = 128 MiB physical space

/// Global physical frame reference table tracking COW frame sharing across all processes.
pub struct FrameRefCounter {
    counts: [u16; MAX_TRACKED_PHYS_FRAMES],
}

impl FrameRefCounter {
    pub const fn new() -> Self {
        Self {
            counts: [0; MAX_TRACKED_PHYS_FRAMES],
        }
    }

    #[inline(always)]
    fn frame_index(phys: PhysAddr) -> Option<usize> {
        let idx = phys.0 / PAGE_SIZE;
        if idx < MAX_TRACKED_PHYS_FRAMES {
            Some(idx)
        } else {
            None
        }
    }

    /// Returns the active reference count for the physical frame.
    pub fn get_ref(&self, phys: PhysAddr) -> u16 {
        if let Some(idx) = Self::frame_index(phys) {
            self.counts[idx]
        } else {
            1
        }
    }

    /// Sets the reference count for the physical frame.
    pub fn set_ref(&mut self, phys: PhysAddr, count: u16) {
        if let Some(idx) = Self::frame_index(phys) {
            self.counts[idx] = count;
        }
    }

    /// Increments the reference count for the physical frame and returns the new count.
    pub fn inc_ref(&mut self, phys: PhysAddr) -> u16 {
        if let Some(idx) = Self::frame_index(phys) {
            self.counts[idx] = self.counts[idx].saturating_add(1);
            self.counts[idx]
        } else {
            1
        }
    }

    /// Decrements the reference count for the physical frame and returns the remaining count.
    pub fn dec_ref(&mut self, phys: PhysAddr) -> u16 {
        if let Some(idx) = Self::frame_index(phys) {
            if self.counts[idx] > 0 {
                self.counts[idx] -= 1;
            }
            self.counts[idx]
        } else {
            0
        }
    }
}

/// Global Physical Frame Reference Counter singleton.
pub static FRAME_REF_COUNTER: Spinlock<FrameRefCounter> = Spinlock::new(FrameRefCounter::new());

/// Clones an address space for process fork using Copy-on-Write (COW).
///
/// Marks all writable mapped pages as Read-Only with PTE_COW_BIT in both parent and child,
/// and increments physical frame reference counts.
pub unsafe fn fork_address_space<A: FrameAllocator>(
    parent: &mut AddressSpace,
    child_pml4: *mut PageTable,
    allocator: &mut A,
) -> Result<AddressSpace, VmmError> {
    let mut child = AddressSpace::new(child_pml4);
    let mut frame_refs = FRAME_REF_COUNTER.lock();
    let parent_vmas = parent.vmas.clone();

    for vma in &parent_vmas {
        let mut child_vma = *vma;
        if vma.is_writable() {
            child_vma.flags |= VmFlags::VM_COW;
        }
        child.vmas.push(child_vma);

        // Iterate through all 4 KiB virtual pages within the VMA
        let mut vaddr_num = vma.start.0;
        while vaddr_num < vma.end.0 {
            let virt = VirtAddr::new(vaddr_num);

            // Read parent PTE
            let mut parent_pt_mgr = parent.page_table_manager();
            if let Some(pte) = parent_pt_mgr.get_pte_mut(virt) {
                if pte.is_present() {
                    let phys = pte.addr();
                    let mut flags = pte.flags();

                    if pte.is_writable() || vma.is_writable() {
                        // Mark both parent and child as Read-Only with COW bit set
                        flags &= !PageTableFlags::WRITABLE;
                        flags |= PageTableFlags::COW;
                        pte.set(phys, flags);
                        invlpg(virt);
                    }

                    // Map in child page table with identical flags
                    let mut child_pt_mgr = child.page_table_manager();
                    child_pt_mgr.map_page(virt, phys, flags, allocator)?;

                    // Increment frame refcount (or initialize to 2 if first share)
                    let current_ref = frame_refs.get_ref(phys);
                    if current_ref == 0 {
                        frame_refs.set_ref(phys, 2);
                    } else {
                        frame_refs.inc_ref(phys);
                    }
                }
            }

            vaddr_num += PAGE_SIZE;
        }
    }

    // Mark parent VMAs as COW if writable
    for vma in &mut parent.vmas {
        if vma.is_writable() {
            vma.flags |= VmFlags::VM_COW;
        }
    }

    Ok(child)
}

/// Handles a Copy-on-Write page fault when writing to a page marked with PTE_COW_BIT.
pub unsafe fn handle_cow_fault<A: FrameAllocator>(
    _aspace: &mut AddressSpace,
    page_virt: VirtAddr,
    pte: &mut PageTableEntry,
    allocator: &mut A,
) -> Result<PageFaultResolution, PageFaultResolution> {
    let old_phys = pte.addr();
    let mut ref_lock = FRAME_REF_COUNTER.lock();
    let refcount = ref_lock.get_ref(old_phys);

    if refcount > 1 {
        // Shared frame: allocate fresh frame, duplicate memory, and remap writable
        let new_frame = allocator.allocate_frame().ok_or(PageFaultResolution::OutOfMemory)?;
        let new_phys = new_frame.start_address();

        // Copy 4096 bytes from old_phys to new_phys
        let src = core::slice::from_raw_parts(old_phys.0 as *const u8, PAGE_SIZE);
        let dst = core::slice::from_raw_parts_mut(new_phys.0 as *mut u8, PAGE_SIZE);
        dst.copy_from_slice(src);

        // Update PTE: remap to new_phys, make writable, clear COW bit
        let new_flags = (pte.flags() | PageTableFlags::WRITABLE) & !PageTableFlags::COW;
        pte.set(new_phys, new_flags);

        // Update frame refcounts
        ref_lock.dec_ref(old_phys);
        ref_lock.set_ref(new_phys, 1);
        drop(ref_lock);

        invlpg(page_virt);

        Ok(PageFaultResolution::ResolvedCow {
            virt: page_virt,
            new_phys,
        })
    } else {
        // Sole owner (refcount <= 1): break COW in-place without duplicating frame
        ref_lock.set_ref(old_phys, 1);
        drop(ref_lock);

        pte.set_cow(false);
        pte.set_writable(true);

        invlpg(page_virt);

        Ok(PageFaultResolution::ResolvedCow {
            virt: page_virt,
            new_phys: old_phys,
        })
    }
}

/// Unit test verifying bitfields and constants for Copy-on-Write subsystem.
pub fn test_cow_bitfields() -> bool {
    assert_eq!(PTE_COW_BIT, 1 << 9);
    assert_eq!(PTE_AVL_MASK, (1 << 9) | (1 << 10) | (1 << 11));
    assert!((PTE_COW_BIT & PTE_AVL_MASK) == PTE_COW_BIT);
    true
}

/// Unit test verifying COW address space cloning and physical frame reference tracking.
pub fn test_cow_fork_and_reference_counter() -> bool {
    unsafe {
        static mut PARENT_PML4: PageTable = PageTable::new();
        static mut CHILD_PML4: PageTable = PageTable::new();

        let parent_pml4_ptr = core::ptr::addr_of_mut!(PARENT_PML4);
        let child_pml4_ptr = core::ptr::addr_of_mut!(CHILD_PML4);
        (*parent_pml4_ptr).zero();
        (*child_pml4_ptr).zero();

        let mut parent_aspace = AddressSpace::new(parent_pml4_ptr);
        let mut pmm = super::PMM.lock();

        let vma_start = VirtAddr::new(0x0000_7100_0000_0000);
        let vma_end = VirtAddr::new(0x0000_7100_0001_0000); // 64 KiB
        let vma = VmArea::new(
            vma_start,
            vma_end,
            VmFlags::VM_READ | VmFlags::VM_WRITE | VmFlags::VM_ANONYMOUS | VmFlags::VM_USER,
        );
        assert!(parent_aspace.add_vma(vma));

        // 1. Demand allocate a page in parent
        let fault_addr = VirtAddr::new(0x0000_7100_0000_0000);
        let err_code = PageFaultErrorCode::new(PageFaultErrorCode::WRITE | PageFaultErrorCode::USER);
        let demand_res = super::demand_paging::handle_page_fault(&mut parent_aspace, fault_addr, err_code, &mut *pmm);

        let parent_phys = match demand_res {
            Ok(PageFaultResolution::ResolvedDemandPage { phys, .. }) => phys,
            _ => panic!("Expected ResolvedDemandPage"),
        };

        // Write a test signature into parent's memory
        let slice = core::slice::from_raw_parts_mut(parent_phys.0 as *mut u8, PAGE_SIZE);
        slice[0] = 0xAA;
        slice[1] = 0xBB;
        slice[2] = 0xCC;

        FRAME_REF_COUNTER.lock().set_ref(parent_phys, 1);
        assert_eq!(FRAME_REF_COUNTER.lock().get_ref(parent_phys), 1);

        // 2. Fork parent into child
        let mut child_aspace = fork_address_space(&mut parent_aspace, child_pml4_ptr, &mut *pmm)
            .expect("fork_address_space failed");

        // Verify both parent and child now have COW=1, WRITABLE=0, refcount=2
        assert_eq!(FRAME_REF_COUNTER.lock().get_ref(parent_phys), 2);

        let parent_pte = parent_aspace.page_table_manager().get_pte(fault_addr).unwrap();
        assert!(parent_pte.is_cow());
        assert!(!parent_pte.is_writable());

        let child_pte = child_aspace.page_table_manager().get_pte(fault_addr).unwrap();
        assert!(child_pte.is_cow());
        assert!(!child_pte.is_writable());
        assert_eq!(child_pte.addr(), parent_phys);

        // 3. Child triggers write fault to shared COW page
        let cow_write_code = PageFaultErrorCode::new(
            PageFaultErrorCode::PRESENT | PageFaultErrorCode::WRITE | PageFaultErrorCode::USER,
        );
        let child_fault_res = super::demand_paging::handle_page_fault(
            &mut child_aspace,
            fault_addr,
            cow_write_code,
            &mut *pmm,
        );

        let child_new_phys = match child_fault_res {
            Ok(PageFaultResolution::ResolvedCow { new_phys, .. }) => new_phys,
            _ => panic!("Expected ResolvedCow for child write"),
        };

        // Child's new frame must be distinct from parent's frame
        assert_ne!(child_new_phys, parent_phys);
        assert_eq!(FRAME_REF_COUNTER.lock().get_ref(parent_phys), 1);
        assert_eq!(FRAME_REF_COUNTER.lock().get_ref(child_new_phys), 1);

        // Modify child's memory
        let child_slice = core::slice::from_raw_parts_mut(child_new_phys.0 as *mut u8, PAGE_SIZE);
        assert_eq!(child_slice[0], 0xAA); // Initial copied data
        child_slice[0] = 0xFF; // Modify in child

        // Verify parent memory remains untouched
        let parent_slice = core::slice::from_raw_parts(parent_phys.0 as *const u8, PAGE_SIZE);
        assert_eq!(parent_slice[0], 0xAA, "Parent memory must remain unchanged");

        // 4. Parent triggers write fault (sole owner now)
        let parent_fault_res = super::demand_paging::handle_page_fault(
            &mut parent_aspace,
            fault_addr,
            cow_write_code,
            &mut *pmm,
        );

        match parent_fault_res {
            Ok(PageFaultResolution::ResolvedCow { new_phys, .. }) => {
                // Must be unshared in-place
                assert_eq!(new_phys, parent_phys);
            }
            _ => panic!("Expected ResolvedCow in-place for parent write"),
        }

        let parent_pte_after = parent_aspace.page_table_manager().get_pte(fault_addr).unwrap();
        assert!(!parent_pte_after.is_cow());
        assert!(parent_pte_after.is_writable());

        true
    }
}
