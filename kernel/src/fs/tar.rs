//! POSIX UStar Archive Parser for In-Memory Initramfs
//! Parses 512-byte block structured TAR archives loaded in memory without

use super::vfs::FsError;

pub const TAR_BLOCK_SIZE: usize = 512;

#[derive(Debug, Clone, Copy)]
pub struct TarEntry<'a> {
    pub name: &'a str,
    pub size: usize,
    pub is_dir: bool,
    pub data: &'a [u8],
}

pub struct TarArchive<'a> {
    raw: &'a [u8],
}

impl<'a> TarArchive<'a> {
    pub fn new(raw: &'a [u8]) -> Self {
        Self { raw }
    }

    /// Parse octal ascii string from header slice.
    fn parse_octal(slice: &[u8]) -> Result<usize, FsError> {
        let mut val = 0usize;
        let mut empty = true;
        for &b in slice {
            if b == 0 || b == b' ' {
                if empty {
                    continue;
                } else {
                    break;
                }
            }
            if b < b'0' || b > b'7' {
                return Err(FsError::CorruptArchive);
            }
            empty = false;
            val = val.checked_mul(8).ok_or(FsError::CorruptArchive)?
                + (b - b'0') as usize;
        }
        Ok(if empty { 0 } else { val })
    }

    /// Check whether a 512-byte block is all zeros (marks end of tar archive).
    fn is_zero_block(block: &[u8]) -> bool {
        block.iter().all(|&b| b == 0)
    }

    /// Iterate over entries in the tar archive.
    pub fn entries(&self) -> TarIterator<'a> {
        TarIterator {
            archive: self.raw,
            offset: 0,
        }
    }
}

pub struct TarIterator<'a> {
    archive: &'a [u8],
    offset: usize,
}

impl<'a> Iterator for TarIterator<'a> {
    type Item = Result<TarEntry<'a>, FsError>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.offset + TAR_BLOCK_SIZE > self.archive.len() {
            return None;
        }

        let header = &self.archive[self.offset..self.offset + TAR_BLOCK_SIZE];
        if TarArchive::is_zero_block(header) {
            return None;
        }

        // Parse null-terminated name
        let name_bytes = &header[0..100];
        let name_len = name_bytes.iter().position(|&b| b == 0).unwrap_or(100);
        let name_str = match core::str::from_utf8(&name_bytes[..name_len]) {
            Ok(s) => s.trim_start_matches("./"),
            Err(_) => return Some(Err(FsError::CorruptArchive)),
        };

        // Parse file size
        let size = match TarArchive::parse_octal(&header[124..136]) {
            Ok(s) => s,
            Err(e) => return Some(Err(e)),
        };

        // Parse type flag
        let type_flag = header[156];
        let is_dir = type_flag == b'5' || name_str.ends_with('/');

        let data_start = self.offset + TAR_BLOCK_SIZE;
        let data_end = data_start + size;
        if data_end > self.archive.len() {
            return Some(Err(FsError::CorruptArchive));
        }

        let data = &self.archive[data_start..data_end];

        // Advance offset rounded up to 512-byte block boundary
        let padded_size = (size + TAR_BLOCK_SIZE - 1) & !(TAR_BLOCK_SIZE - 1);
        self.offset = data_start + padded_size;

        Some(Ok(TarEntry {
            name: name_str.trim_end_matches('/'),
            size,
            is_dir,
            data,
        }))
    }
}
