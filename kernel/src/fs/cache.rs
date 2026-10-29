//! VFS Unified Inode Cache and Metadata Tracking.
//!
//! Provides fast lookup cache representations for resolved inodes, security descriptors,
//! access metrics, and dirty state tracking.

use alloc::string::String;
use alloc::sync::Arc;
use super::acl::{AccessControlList, FilePermissions, SecurityIdentifier};
use super::inode::Inode;

/// Cache entry representing a cached VFS Inode with metadata, ACLs, and hit counters.
#[derive(Clone)]
pub struct InodeCacheEntry {
    pub inode_id: u64,
    pub path: String,
    pub inode: Arc<dyn Inode>,
    pub permissions: FilePermissions,
    pub owner: SecurityIdentifier,
    pub acl: Option<AccessControlList>,
    pub access_count: u64,
    pub last_accessed: u64,
    pub is_dirty: bool,
}

impl InodeCacheEntry {
    pub fn new(
        inode_id: u64,
        path: &str,
        inode: Arc<dyn Inode>,
        permissions: FilePermissions,
        owner: SecurityIdentifier,
    ) -> Self {
        Self {
            inode_id,
            path: String::from(path),
            inode,
            permissions,
            owner,
            acl: None,
            access_count: 1,
            last_accessed: 0,
            is_dirty: false,
        }
    }

    pub fn with_acl(mut self, acl: AccessControlList) -> Self {
        self.acl = Some(acl);
        self
    }

    pub fn record_access(&mut self, timestamp: u64) {
        self.access_count = self.access_count.saturating_add(1);
        self.last_accessed = timestamp;
    }

    pub fn mark_dirty(&mut self) {
        self.is_dirty = true;
    }

    pub fn clear_dirty(&mut self) {
        self.is_dirty = false;
    }
}
