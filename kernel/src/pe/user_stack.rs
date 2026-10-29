//! Userland Stack Allocator with Guard Page Protection for Win32/x86-64 user processes.

use alloc::vec::Vec;
use crate::memory::pmm::{PhysAddr, PAGE_SIZE};
use crate::memory::vmm::{PageTableFlags, PageTableManager, VirtAddr, VmmError};
use crate::memory::PMM;

/// Default user stack base address in canonical 48-bit user virtual address space.
pub const DEFAULT_USER_STACK_BASE: usize = 0x0000_7FFF_FFFF_0000;
/// 64 KiB user stack size (16 pages).
pub const USER_STACK_SIZE: usize = 64 * 1024;
pub const USER_STACK_PAGES: usize = USER_STACK_SIZE / PAGE_SIZE;

#[derive(Debug)]
pub enum UserStackError {
    PhysicalAllocationFailed,
    PageTableMappingFailed(VmmError),
}

#[derive(Debug, Clone)]
pub struct UserStack {
    pub stack_bottom: VirtAddr,
    pub stack_top: VirtAddr,
    pub initial_rsp: u64,
    pub guard_page: VirtAddr,
    pub allocated_frames: Vec<PhysAddr>,
}

pub struct UserStackAllocator;

impl UserStackAllocator {
    /// Allocates and maps a 64 KiB userland stack with a preceding unmapped guard page.
    pub unsafe fn allocate(
        ptm: &mut PageTableManager,
        stack_base: usize,
    ) -> Result<UserStack, UserStackError> {
        let mut pmm_guard = PMM.lock();
        let mut allocated_frames = Vec::with_capacity(USER_STACK_PAGES);

        // Guard page is placed immediately below the stack bottom (unmapped)
        let guard_page = VirtAddr::new(stack_base - PAGE_SIZE);
        let stack_bottom = VirtAddr::new(stack_base);
        let stack_top = VirtAddr::new(stack_base + USER_STACK_SIZE);

        // Standard user stack flags: present, writable, user-accessible, non-executable
        let stack_flags = PageTableFlags::PRESENT
            | PageTableFlags::WRITABLE
            | PageTableFlags::USER_ACCESSIBLE
            | PageTableFlags::NO_EXECUTE;

        for page_idx in 0..USER_STACK_PAGES {
            let vaddr = VirtAddr::new(stack_base + (page_idx * PAGE_SIZE));
            let frame = pmm_guard
                .allocate_frame()
                .ok_or(UserStackError::PhysicalAllocationFailed)?;

            ptm.map_page(vaddr, frame.start_address(), stack_flags, &mut *pmm_guard)
                .map_err(UserStackError::PageTableMappingFailed)?;

            allocated_frames.push(frame.start_address());

            // Zero-fill stack frame memory via kernel physical identity mapping
            core::ptr::write_bytes(frame.start_address().0 as *mut u8, 0, PAGE_SIZE);
        }

        // Align initial RSP to 16 bytes, reserving 32 bytes of Win64 shadow store
        let initial_rsp = (stack_top.as_usize() - 32) as u64 & !0x0F;

        Ok(UserStack {
            stack_bottom,
            stack_top,
            initial_rsp,
            guard_page,
            allocated_frames,
        })
    }
}
