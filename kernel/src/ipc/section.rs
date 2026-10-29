//! NT Shared Memory Sections (Section Objects) for Zero-Copy IPC.
//!
//! Provides creation and mapping of contiguous physical frame backed
//! memory sections shareable across virtual address spaces.

use alloc::string::String;
use alloc::vec::Vec;
use core::sync::atomic::{AtomicU64, Ordering};
use crate::memory::pmm::{PhysAddr, PAGE_SIZE};
use crate::memory::spinlock::Spinlock;
use crate::memory::vmm::{PageTableFlags, PageTableManager, VirtAddr};
use crate::memory::PMM;

static NEXT_SECTION_HANDLE: AtomicU64 = AtomicU64::new(0x1000);

pub struct SharedSection {
    pub handle: u64,
    pub name: String,
    pub size: usize,
    pub frames: Vec<PhysAddr>,
}

pub struct SectionManager {
    pub sections: Vec<SharedSection>,
}

impl SectionManager {
    pub const fn new() -> Self {
        Self {
            sections: Vec::new(),
        }
    }

    /// Creates a newly allocated shared memory section of specified byte length.
    pub fn create_section(&mut self, name: &str, size: usize) -> Result<u64, u32> {
        if size == 0 {
            return Err(crate::syscall::STATUS_INVALID_PARAMETER);
        }

        let page_count = (size + PAGE_SIZE - 1) / PAGE_SIZE;
        let mut frames = Vec::with_capacity(page_count);

        {
            let mut pmm = PMM.lock();
            for _ in 0..page_count {
                match pmm.allocate_frame() {
                    Some(frame) => frames.push(frame.start_address()),
                    None => {
                        for f in frames {
                            pmm.deallocate_frame(
                                crate::memory::pmm::PhysFrame::from_number(f.0 / PAGE_SIZE),
                            );
                        }
                        return Err(0xC000_0017); // STATUS_NO_MEMORY
                    }
                }
            }
        }

        // Zero physical frames
        for frame in &frames {
            unsafe {
                core::ptr::write_bytes(frame.0 as *mut u8, 0, PAGE_SIZE);
            }
        }

        let handle = NEXT_SECTION_HANDLE.fetch_add(1, Ordering::Relaxed);
        let section = SharedSection {
            handle,
            name: String::from(name),
            size,
            frames,
        };

        self.sections.push(section);
        Ok(handle)
    }

    /// Maps the physical pages backing `handle` into the active page table at `preferred_vaddr`.
    pub fn map_view(&self, handle: u64, preferred_vaddr: usize) -> Result<usize, u32> {
        let section = match self.sections.iter().find(|s| s.handle == handle) {
            Some(s) => s,
            None => return Err(crate::syscall::STATUS_INVALID_HANDLE),
        };

        let base_vaddr = if preferred_vaddr != 0 {
            preferred_vaddr
        } else {
            0x0000_7FFF_8000_0000
        };

        let cr3: u64;
        unsafe {
            // Temporarily clear CR0.WP (bit 16) to allow writing intermediate paging structures in UEFI
            let cr0: u64;
            core::arch::asm!("mov {}, cr0", out(reg) cr0, options(nomem, nostack, preserves_flags));
            core::arch::asm!("mov cr0, {}", in(reg) (cr0 & !(1 << 16)), options(nomem, nostack, preserves_flags));

            core::arch::asm!("mov {}, cr3", out(reg) cr3, options(nomem, nostack, preserves_flags));
            let root_pml4 = (cr3 & !0xFFF) as *mut crate::memory::vmm::PageTable;
            let mut ptm = PageTableManager::new(root_pml4);
            let mut pmm = PMM.lock();

            let flags = PageTableFlags::PRESENT
                | PageTableFlags::WRITABLE
                | PageTableFlags::USER_ACCESSIBLE;

            for (i, frame) in section.frames.iter().enumerate() {
                let vaddr = VirtAddr::new(base_vaddr + i * PAGE_SIZE);
                if ptm.translate(vaddr).is_none() {
                    let _ = ptm.map_page(vaddr, *frame, flags, &mut *pmm);
                }
            }

            core::arch::asm!("mov cr0, {}", in(reg) cr0, options(nomem, nostack, preserves_flags));
        }

        Ok(base_vaddr)
    }

    /// Returns the backing physical frames for a section handle.
    pub fn get_frames(&self, handle: u64) -> Option<Vec<PhysAddr>> {
        self.sections.iter().find(|s| s.handle == handle).map(|s| s.frames.clone())
    }
}

pub static SECTION_MANAGER: Spinlock<SectionManager> = Spinlock::new(SectionManager::new());
