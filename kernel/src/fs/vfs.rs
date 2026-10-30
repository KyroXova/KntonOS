//! KntonOS Virtual File System (VFS) Core Abstractions
//!
//! Provides fundamental traits, types, error representations, and file operations
//! for filesystem-agnostic path resolution and I/O.

use alloc::sync::Arc;
use super::inode::Inode;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FsError {
    NotFound,
    PermissionDenied,
    AlreadyExists,
    NotADirectory,
    IsADirectory,
    InvalidArgument,
    IoError,
    CorruptArchive,
    NoSpace,
    NotSupported,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileType {
    File,
    Directory,
    Symlink,
    Device,
    Socket,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FilePerms {
    pub readable: bool,
    pub writable: bool,
    pub executable: bool,
}

impl FilePerms {
    pub const fn readonly() -> Self {
        Self {
            readable: true,
            writable: false,
            executable: false,
        }
    }

    pub const fn rwx() -> Self {
        Self {
            readable: true,
            writable: true,
            executable: true,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SeekMode {
    Start(u64),
    Current(i64),
    End(i64),
}

#[derive(Debug, Clone)]
pub struct VfsMetadata {
    pub file_type: FileType,
    pub size: u64,
    pub permissions: FilePerms,
    pub inode_id: u64,
}

/// Filesystem interface representing a mounted volume or filesystem instance.
pub trait FileSystem: Send + Sync {
    /// Friendly filesystem type name (e.g., "initramfs", "fat32", "devfs").
    fn name(&self) -> &'static str;

    /// Retrieve the root inode of this mounted filesystem instance.
    fn root(&self) -> Result<Arc<dyn Inode>, FsError>;
}
