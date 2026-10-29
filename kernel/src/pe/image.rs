//! Memory-mapped PE32+ image loader and virtual address space section mapper.

use alloc::vec::Vec;
use super::loader::{ParsedPe64, PeError};
use crate::memory::pmm::{PhysAddr, PAGE_SIZE};
use crate::memory::vmm::{PageTableFlags, PageTableManager, VirtAddr, VmmError};
use crate::memory::PMM;

pub const IMAGE_SCN_MEM_EXECUTE: u32 = 0x2000_0000;
pub const IMAGE_SCN_MEM_READ: u32 = 0x4000_0000;
pub const IMAGE_SCN_MEM_WRITE: u32 = 0x8000_0000;

#[derive(Debug)]
pub enum ImageLoadError {
    PeParsingFailed(PeError),
    PageTableMappingFailed(VmmError),
    PhysicalAllocationFailed,
    AddressOverflow,
}

#[derive(Debug, Clone)]
pub struct LoadedImage {
    pub entry_point: u64,
    pub image_base: u64,
    pub image_size: usize,
    pub section_count: usize,
    pub allocated_frames: Vec<PhysAddr>,
}

pub struct PeImageLoader;

impl PeImageLoader {
    /// Loads and maps sections of a parsed PE64 binary into the specified PageTableManager.
    pub unsafe fn load(
        pe: &ParsedPe64,
        ptm: &mut PageTableManager,
    ) -> Result<LoadedImage, ImageLoadError> {
        let mut pmm_guard = PMM.lock();
        let mut allocated_frames = Vec::new();

        let image_base = pe.image_base;
        let entry_point = image_base.checked_add(pe.entry_point_rva as u64)
            .ok_or(ImageLoadError::AddressOverflow)?;

        for i in 0..pe.number_of_sections {
            let sec = match pe.get_section(i) {
                Some(s) => s,
                None => continue,
            };

            let sec_vaddr = image_base.checked_add(sec.virtual_address as u64)
                .ok_or(ImageLoadError::AddressOverflow)?;
            let sec_size = core::cmp::max(sec.virtual_size, sec.size_of_raw_data) as usize;

            if sec_size == 0 {
                continue;
            }

            // Determine page attributes from PE section flags
            let mut page_flags = PageTableFlags::PRESENT | PageTableFlags::USER_ACCESSIBLE;
            if (sec.characteristics & IMAGE_SCN_MEM_WRITE) != 0 {
                page_flags |= PageTableFlags::WRITABLE;
            }
            if (sec.characteristics & IMAGE_SCN_MEM_EXECUTE) == 0 {
                page_flags |= PageTableFlags::NO_EXECUTE;
            }

            // Map each 4 KiB page for this section
            let page_count = (sec_size + PAGE_SIZE - 1) / PAGE_SIZE;
            let raw_offset = sec.pointer_to_raw_data as usize;
            let raw_size = sec.size_of_raw_data as usize;

            for page_idx in 0..page_count {
                let vaddr = VirtAddr::new((sec_vaddr as usize) + (page_idx * PAGE_SIZE));
                let frame = pmm_guard.allocate_frame()
                    .ok_or(ImageLoadError::PhysicalAllocationFailed)?;

                ptm.map_page(vaddr, frame.start_address(), page_flags, &mut *pmm_guard)
                    .map_err(ImageLoadError::PageTableMappingFailed)?;

                allocated_frames.push(frame.start_address());

                // Zero-fill physical frame memory directly via kernel identity mapping
                let dest_ptr = frame.start_address().0 as *mut u8;
                core::ptr::write_bytes(dest_ptr, 0, PAGE_SIZE);

                // Copy raw binary section data into page
                let page_file_offset = page_idx * PAGE_SIZE;
                if page_file_offset < raw_size && (raw_offset + page_file_offset) < pe.raw_data.len() {
                    let src_start = raw_offset + page_file_offset;
                    let remaining_raw = raw_size - page_file_offset;
                    let copy_len = core::cmp::min(PAGE_SIZE, remaining_raw);
                    let actual_copy = core::cmp::min(copy_len, pe.raw_data.len().saturating_sub(src_start));

                    core::ptr::copy_nonoverlapping(
                        pe.raw_data.as_ptr().add(src_start),
                        dest_ptr,
                        actual_copy,
                    );
                }
            }
        }

        Ok(LoadedImage {
            entry_point,
            image_base,
            image_size: pe.size_of_image as usize,
            section_count: pe.number_of_sections,
            allocated_frames,
        })
    }
}
