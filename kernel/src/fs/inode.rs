//! KntonOS Inode and Directory Entry Abstractions
//! Represents files, directories, and pseudofiles in the VFS hierarchy.

use alloc::string::String;
use alloc::sync::Arc;
use alloc::vec::Vec;
use super::vfs::{FileType, FsError, VfsMetadata};

#[derive(Debug, Clone)]
pub struct DirEntry {
    pub name: String,
    pub inode_id: u64,
    pub file_type: FileType,
}

/// Core interface for an individual filesystem node.
pub trait Inode: Send + Sync {
    /// Return metadata describing the inode.
    fn metadata(&self) -> VfsMetadata;

    /// Read up to `buf.len()` bytes starting at `offset`.
    fn read_at(&self, offset: u64, buf: &mut [u8]) -> Result<usize, FsError>;

    /// Write up to `buf.len()` bytes starting at `offset`.
    fn write_at(&self, offset: u64, buf: &[u8]) -> Result<usize, FsError>;

    /// For directory inodes: look up an immediate child entry by name.
    fn lookup(&self, name: &str) -> Result<Arc<dyn Inode>, FsError>;

    /// For directory inodes: enumerate all child directory entries.
    fn readdir(&self) -> Result<Vec<DirEntry>, FsError>;
}
