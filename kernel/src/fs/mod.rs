//! KntonOS Virtual File System (VFS) Subsystem
//!
//! Exposes universal file and directory tree abstractions for KntonOS.

pub mod vfs;
pub mod inode;
pub mod tar;

pub use vfs::{FileType, FilePerms, FsError, SeekMode, VfsMetadata, FileSystem};
pub use inode::{DirEntry, Inode};
pub use tar::{TarArchive, TarEntry};
