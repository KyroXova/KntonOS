//! PE32+ (64-bit) Executable binary parser and validation routines.

use super::header::*;

#[derive(Debug)]
pub enum PeError {
    BufferTooSmall,
    InvalidDosSignature,
    InvalidNtSignature,
    UnsupportedMachine,
    NotPe32Plus,
    InvalidSectionHeaders,
}

#[derive(Clone, Debug)]
pub struct ParsedPe64<'a> {
    pub entry_point_rva: u32,
    pub image_base: u64,
    pub size_of_image: u32,
    pub subsystem: u16,
    pub raw_data: &'a [u8],
    pub sections_offset: usize,
    pub number_of_sections: usize,
}

impl<'a> ParsedPe64<'a> {
    pub fn parse(bytes: &'a [u8]) -> Result<Self, PeError> {
        let dos_size = core::mem::size_of::<ImageDosHeader>();
        if bytes.len() < dos_size {
            return Err(PeError::BufferTooSmall);
        }

        // 1. Validate DOS Header
        let dos_header = unsafe { &*(bytes.as_ptr() as *const ImageDosHeader) };
        if dos_header.e_magic != IMAGE_DOS_SIGNATURE {
            return Err(PeError::InvalidDosSignature);
        }

        let nt_offset = dos_header.e_lfanew as usize;
        let file_header_size = core::mem::size_of::<ImageFileHeader>();
        let opt_header_size = core::mem::size_of::<ImageOptionalHeader64>();

        if bytes.len() < nt_offset + 4 + file_header_size + opt_header_size {
            return Err(PeError::BufferTooSmall);
        }

        // 2. Validate PE Signature ("PE\0\0")
        let nt_sig_ptr = unsafe { (bytes.as_ptr().add(nt_offset)) as *const u32 };
        if unsafe { *nt_sig_ptr } != IMAGE_NT_SIGNATURE {
            return Err(PeError::InvalidNtSignature);
        }

        // 3. Validate File Header
        let file_header_ptr = unsafe { (bytes.as_ptr().add(nt_offset + 4)) as *const ImageFileHeader };
        let file_header = unsafe { &*file_header_ptr };
        if file_header.machine != IMAGE_FILE_MACHINE_AMD64 {
            return Err(PeError::UnsupportedMachine);
        }

        // 4. Validate Optional Header (PE32+ 64-bit)
        let opt_header_ptr = unsafe {
            (bytes.as_ptr().add(nt_offset + 4 + file_header_size)) as *const ImageOptionalHeader64
        };
        let opt_header = unsafe { &*opt_header_ptr };
        if opt_header.magic != IMAGE_NT_OPTIONAL_HDR64_MAGIC {
            return Err(PeError::NotPe32Plus);
        }

        let sections_offset = nt_offset + 4 + file_header_size + file_header.size_of_optional_header as usize;
        let sec_size = core::mem::size_of::<ImageSectionHeader>();
        if bytes.len() < sections_offset + (file_header.number_of_sections as usize * sec_size) {
            return Err(PeError::InvalidSectionHeaders);
        }

        Ok(Self {
            entry_point_rva: opt_header.address_of_entry_point,
            image_base: opt_header.image_base,
            size_of_image: opt_header.size_of_image,
            subsystem: opt_header.subsystem,
            raw_data: bytes,
            sections_offset,
            number_of_sections: file_header.number_of_sections as usize,
        })
    }

    pub fn get_section(&self, index: usize) -> Option<&ImageSectionHeader> {
        if index >= self.number_of_sections {
            return None;
        }
        let sec_size = core::mem::size_of::<ImageSectionHeader>();
        let offset = self.sections_offset + (index * sec_size);
        let ptr = unsafe { self.raw_data.as_ptr().add(offset) as *const ImageSectionHeader };
        Some(unsafe { &*ptr })
    }
}
