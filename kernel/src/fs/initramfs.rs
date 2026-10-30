//! Memory-Mapped Initramfs Root Filesystem Driver
//! Exposes a read-only hierarchical filesystem backed by an in-memory UStar archive.

use alloc::collections::BTreeMap;
use alloc::string::{String, ToString};
use alloc::sync::Arc;
use alloc::vec::Vec;

use crate::memory::spinlock::Spinlock;
use super::inode::{DirEntry, Inode};
use super::path::Path;
use super::tar::TarArchive;
use super::vfs::{FilePerms, FileType, FileSystem, FsError, VfsMetadata};

/// In-memory file node backed by binary data.
pub struct InitramfsFileNode {
    inode_id: u64,
    data: Vec<u8>,
}

impl InitramfsFileNode {
    pub fn new(inode_id: u64, data: &[u8]) -> Self {
        Self {
            inode_id,
            data: data.to_vec(),
        }
    }
}

impl Inode for InitramfsFileNode {
    fn metadata(&self) -> VfsMetadata {
        VfsMetadata {
            file_type: FileType::File,
            size: self.data.len() as u64,
            permissions: FilePerms::readonly(),
            inode_id: self.inode_id,
        }
    }

    fn read_at(&self, offset: u64, buf: &mut [u8]) -> Result<usize, FsError> {
        let offset = offset as usize;
        if offset >= self.data.len() {
            return Ok(0);
        }
        let available = self.data.len() - offset;
        let to_read = core::cmp::min(available, buf.len());
        buf[..to_read].copy_from_slice(&self.data[offset..offset + to_read]);
        Ok(to_read)
    }

    fn write_at(&self, _offset: u64, _buf: &[u8]) -> Result<usize, FsError> {
        Err(FsError::PermissionDenied)
    }

    fn lookup(&self, _name: &str) -> Result<Arc<dyn Inode>, FsError> {
        Err(FsError::NotADirectory)
    }

    fn readdir(&self) -> Result<Vec<DirEntry>, FsError> {
        Err(FsError::NotADirectory)
    }
}

/// In-memory directory node containing child entries.
pub struct InitramfsDirNode {
    inode_id: u64,
    children: Spinlock<BTreeMap<String, Arc<dyn Inode>>>,
}

impl InitramfsDirNode {
    pub fn new(inode_id: u64) -> Self {
        Self {
            inode_id,
            children: Spinlock::new(BTreeMap::new()),
        }
    }

    pub fn insert_child(&self, name: String, node: Arc<dyn Inode>) {
        self.children.lock().insert(name, node);
    }
}

impl Inode for InitramfsDirNode {
    fn metadata(&self) -> VfsMetadata {
        VfsMetadata {
            file_type: FileType::Directory,
            size: 0,
            permissions: FilePerms::readonly(),
            inode_id: self.inode_id,
        }
    }

    fn read_at(&self, _offset: u64, _buf: &mut [u8]) -> Result<usize, FsError> {
        Err(FsError::IsADirectory)
    }

    fn write_at(&self, _offset: u64, _buf: &[u8]) -> Result<usize, FsError> {
        Err(FsError::PermissionDenied)
    }

    fn lookup(&self, name: &str) -> Result<Arc<dyn Inode>, FsError> {
        let lock = self.children.lock();
        match lock.get(name) {
            Some(node) => Ok(Arc::clone(node)),
            None => Err(FsError::NotFound),
        }
    }

    fn readdir(&self) -> Result<Vec<DirEntry>, FsError> {
        let lock = self.children.lock();
        let mut list = Vec::with_capacity(lock.len());
        for (name, node) in lock.iter() {
            let meta = node.metadata();
            list.push(DirEntry {
                name: name.clone(),
                inode_id: meta.inode_id,
                file_type: meta.file_type,
            });
        }
        Ok(list)
    }
}

pub struct Initramfs {
    root: Arc<InitramfsDirNode>,
}

impl Initramfs {
    /// Populate a hierarchical directory tree from raw UStar memory buffer.
    pub fn from_tar(raw: &[u8]) -> Result<Self, FsError> {
        let root = Arc::new(InitramfsDirNode::new(1));
        let mut dir_map: BTreeMap<String, Arc<InitramfsDirNode>> = BTreeMap::new();
        dir_map.insert(String::new(), Arc::clone(&root));

        let archive = TarArchive::new(raw);
        let mut next_inode = 2u64;

        for entry_res in archive.entries() {
            let entry = entry_res?;
            let segments = Path::parse_segments(entry.name)?;
            if segments.is_empty() {
                continue;
            }

            // Ensure intermediate directory ancestors exist in dir_map
            let mut prefix = String::new();
            for i in 0..(segments.len() - 1) {
                let seg = segments[i];
                let parent_key = prefix.clone();
                if !prefix.is_empty() {
                    prefix.push('/');
                }
                prefix.push_str(seg);

                if !dir_map.contains_key(&prefix) {
                    let new_dir = Arc::new(InitramfsDirNode::new(next_inode));
                    next_inode += 1;
                    if let Some(parent) = dir_map.get(&parent_key) {
                        parent.insert_child(seg.to_string(), Arc::clone(&new_dir) as Arc<dyn Inode>);
                    }
                    dir_map.insert(prefix.clone(), new_dir);
                }
            }

            let leaf = segments[segments.len() - 1];
            let parent_key = prefix;

            if entry.is_dir {
                let mut dir_key = parent_key.clone();
                if !dir_key.is_empty() {
                    dir_key.push('/');
                }
                dir_key.push_str(leaf);

                if !dir_map.contains_key(&dir_key) {
                    let new_dir = Arc::new(InitramfsDirNode::new(next_inode));
                    next_inode += 1;
                    if let Some(parent) = dir_map.get(&parent_key) {
                        parent.insert_child(leaf.to_string(), Arc::clone(&new_dir) as Arc<dyn Inode>);
                    }
                    dir_map.insert(dir_key, new_dir);
                }
            } else {
                let file_node = Arc::new(InitramfsFileNode::new(next_inode, entry.data));
                next_inode += 1;
                if let Some(parent) = dir_map.get(&parent_key) {
                    parent.insert_child(leaf.to_string(), file_node as Arc<dyn Inode>);
                }
            }
        }

        Ok(Self { root })
    }
}

impl FileSystem for Initramfs {
    fn name(&self) -> &'static str {
        "initramfs"
    }

    fn root(&self) -> Result<Arc<dyn Inode>, FsError> {
        Ok(Arc::clone(&self.root) as Arc<dyn Inode>)
    }
}
