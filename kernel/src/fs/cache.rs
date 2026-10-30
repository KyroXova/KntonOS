//! VFS Unified Inode Cache, LRU Eviction, and Read-Write Lock Manager.
//! Provides a 128-entry fast-lookup inode cache with LRU aging, hit/miss metrics,

use alloc::collections::BTreeMap;
use alloc::string::String;
use alloc::sync::Arc;
use alloc::vec::Vec;
use crate::memory::spinlock::Spinlock;
use super::acl::{AccessControlList, FilePermissions, SecurityIdentifier};
use super::inode::Inode;

/// Maximum number of active cached inodes before LRU eviction triggers.
pub const INODE_CACHE_CAPACITY: usize = 128;

/// Lock mode for file-level and byte-range locks.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InodeLockType {
    /// Shared read lock: multiple readers permitted simultaneously.
    Shared,
    /// Exclusive write lock: single writer exclusive access.
    Exclusive,
}

/// Represents an active byte-range or whole-file lock on an Inode.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ByteRangeLock {
    pub lock_type: InodeLockType,
    pub owner_id: u64,
    pub offset: u64,
    pub length: u64,
}

impl ByteRangeLock {
    /// Checks if this lock's byte interval overlaps with `[offset, offset + length)`.
    pub fn overlaps(&self, offset: u64, length: u64) -> bool {
        if self.length == 0 || length == 0 {
            return false;
        }
        let self_end = self.offset.saturating_add(self.length);
        let req_end = offset.saturating_add(length);
        self.offset < req_end && offset < self_end
    }

    /// Determines if a requested lock conflicts with this active lock.
    pub fn conflicts_with(
        &self,
        req_type: InodeLockType,
        offset: u64,
        length: u64,
        owner_id: u64,
    ) -> bool {
        if !self.overlaps(offset, length) {
            return false;
        }
        // Same owner does not conflict with self
        if self.owner_id == owner_id {
            return false;
        }
        // Shared-shared is non-conflicting; any exclusive is conflicting
        match (self.lock_type, req_type) {
            (InodeLockType::Shared, InodeLockType::Shared) => false,
            _ => true,
        }
    }
}

/// Inode Lock Manager tracking file-level and byte-range locks across the VFS.
#[derive(Default)]
pub struct InodeLockManager {
    locks: BTreeMap<u64, Vec<ByteRangeLock>>,
}

impl InodeLockManager {
    /// Creates a new lock manager with empty lock tables.
    pub const fn new() -> Self {
        Self {
            locks: BTreeMap::new(),
        }
    }

    /// Attempts to acquire a whole-file lock for `inode_id`.
    pub fn try_lock_file(&mut self, inode_id: u64, lock_type: InodeLockType, owner_id: u64) -> bool {
        self.try_lock_range(inode_id, lock_type, 0, u64::MAX, owner_id)
    }

    /// Releases a whole-file lock on `inode_id` held by `owner_id`.
    pub fn unlock_file(&mut self, inode_id: u64, owner_id: u64) -> bool {
        self.unlock_range(inode_id, 0, u64::MAX, owner_id)
    }

    /// Attempts to acquire a byte-range lock on `[offset, offset + length)`.
    pub fn try_lock_range(
        &mut self,
        inode_id: u64,
        lock_type: InodeLockType,
        offset: u64,
        length: u64,
        owner_id: u64,
    ) -> bool {
        if self.has_conflict(inode_id, lock_type, offset, length, owner_id) {
            return false;
        }

        let file_locks = self.locks.entry(inode_id).or_insert_with(Vec::new);
        file_locks.push(ByteRangeLock {
            lock_type,
            owner_id,
            offset,
            length,
        });
        true
    }

    /// Releases a byte-range lock on `[offset, offset + length)` for `owner_id`.
    pub fn unlock_range(
        &mut self,
        inode_id: u64,
        offset: u64,
        length: u64,
        owner_id: u64,
    ) -> bool {
        if let Some(file_locks) = self.locks.get_mut(&inode_id) {
            let initial_len = file_locks.len();
            file_locks.retain(|l| {
                !(l.owner_id == owner_id && l.offset == offset && l.length == length)
            });
            let removed = file_locks.len() < initial_len;
            if file_locks.is_empty() {
                self.locks.remove(&inode_id);
            }
            removed
        } else {
            false
        }
    }

    /// Checks if a proposed lock conflicts with any existing active locks.
    pub fn has_conflict(
        &self,
        inode_id: u64,
        req_type: InodeLockType,
        offset: u64,
        length: u64,
        owner_id: u64,
    ) -> bool {
        if let Some(file_locks) = self.locks.get(&inode_id) {
            for lock in file_locks {
                if lock.conflicts_with(req_type, offset, length, owner_id) {
                    return true;
                }
            }
        }
        false
    }

    /// Returns true if any active lock is held on `inode_id`.
    pub fn is_locked(&self, inode_id: u64) -> bool {
        self.locks.get(&inode_id).map(|l| !l.is_empty()).unwrap_or(false)
    }

    /// Releases all locks across all inodes associated with `owner_id`.
    pub fn release_all_owner_locks(&mut self, owner_id: u64) {
        let mut empty_inodes = Vec::new();
        for (&inode_id, file_locks) in self.locks.iter_mut() {
            file_locks.retain(|l| l.owner_id != owner_id);
            if file_locks.is_empty() {
                empty_inodes.push(inode_id);
            }
        }
        for id in empty_inodes {
            self.locks.remove(&id);
        }
    }
}

/// Statistics metrics tracking Inode cache efficiency.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct InodeCacheStats {
    pub hits: u64,
    pub misses: u64,
    pub evictions: u64,
}

impl InodeCacheStats {
    /// Computes cache hit rate as an integer percentage (0..=100).
    pub fn hit_rate_pct(&self) -> u32 {
        let total = self.hits + self.misses;
        if total == 0 {
            0
        } else {
            ((self.hits * 100) / total) as u32
        }
    }
}

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

/// 128-entry Fast Lookup Inode Cache with LRU aging and eviction.
pub struct InodeLookupCache {
    entries: BTreeMap<String, InodeCacheEntry>,
    id_to_path: BTreeMap<u64, String>,
    capacity: usize,
    clock: u64,
    stats: InodeCacheStats,
}

impl InodeLookupCache {
    /// Creates a new Inode cache with default capacity (128 entries).
    pub const fn new() -> Self {
        Self {
            entries: BTreeMap::new(),
            id_to_path: BTreeMap::new(),
            capacity: INODE_CACHE_CAPACITY,
            clock: 0,
            stats: InodeCacheStats {
                hits: 0,
                misses: 0,
                evictions: 0,
            },
        }
    }

    /// Creates an Inode cache with custom capacity limit.
    pub fn with_capacity(capacity: usize) -> Self {
        Self {
            entries: BTreeMap::new(),
            id_to_path: BTreeMap::new(),
            capacity: capacity.max(1),
            clock: 0,
            stats: InodeCacheStats::default(),
        }
    }

    /// Retrieves an inode cache entry by canonical path.
    pub fn get_by_path(&mut self, path: &str) -> Option<InodeCacheEntry> {
        self.clock = self.clock.wrapping_add(1);
        let clock = self.clock;
        if let Some(entry) = self.entries.get_mut(path) {
            self.stats.hits += 1;
            entry.record_access(clock);
            Some(entry.clone())
        } else {
            self.stats.misses += 1;
            None
        }
    }

    /// Retrieves an inode cache entry by inode ID.
    pub fn get_by_id(&mut self, inode_id: u64) -> Option<InodeCacheEntry> {
        if let Some(path) = self.id_to_path.get(&inode_id).cloned() {
            self.get_by_path(&path)
        } else {
            self.stats.misses += 1;
            None
        }
    }

    /// Inserts or updates an inode in the cache, performing LRU eviction if full.
    pub fn insert(&mut self, mut entry: InodeCacheEntry) -> Option<InodeCacheEntry> {
        self.clock = self.clock.wrapping_add(1);
        entry.last_accessed = self.clock;

        if let Some(existing) = self.entries.get_mut(&entry.path) {
            self.id_to_path.remove(&existing.inode_id);
            self.id_to_path.insert(entry.inode_id, entry.path.clone());
            let old = core::mem::replace(existing, entry);
            return Some(old);
        }

        let mut evicted = None;
        if self.entries.len() >= self.capacity {
            // Find LRU entry
            let mut lru_path = None;
            let mut min_accessed = u64::MAX;

            for (p, e) in &self.entries {
                if e.last_accessed < min_accessed {
                    min_accessed = e.last_accessed;
                    lru_path = Some(p.clone());
                }
            }

            if let Some(p) = lru_path {
                if let Some(ev) = self.entries.remove(&p) {
                    self.id_to_path.remove(&ev.inode_id);
                    self.stats.evictions += 1;
                    evicted = Some(ev);
                }
            }
        }

        self.id_to_path.insert(entry.inode_id, entry.path.clone());
        self.entries.insert(entry.path.clone(), entry);
        evicted
    }

    /// Removes an entry from the cache by path.
    pub fn invalidate_path(&mut self, path: &str) -> Option<InodeCacheEntry> {
        if let Some(entry) = self.entries.remove(path) {
            self.id_to_path.remove(&entry.inode_id);
            Some(entry)
        } else {
            None
        }
    }

    /// Removes an entry from the cache by inode ID.
    pub fn invalidate_id(&mut self, inode_id: u64) -> Option<InodeCacheEntry> {
        if let Some(path) = self.id_to_path.remove(&inode_id) {
            self.entries.remove(&path)
        } else {
            None
        }
    }

    /// Returns the number of currently cached inode entries.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Returns true if the cache contains no entries.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Resets all cache entries and counters.
    pub fn clear(&mut self) {
        self.entries.clear();
        self.id_to_path.clear();
        self.clock = 0;
    }

    /// Returns a copy of the cache statistics (hits, misses, evictions).
    pub fn stats(&self) -> InodeCacheStats {
        self.stats
    }
}

/// Global Inode lookup cache singleton.
pub static INODE_CACHE: Spinlock<InodeLookupCache> = Spinlock::new(InodeLookupCache::new());

/// Global Inode lock manager singleton.
pub static INODE_LOCK_MGR: Spinlock<InodeLockManager> = Spinlock::new(InodeLockManager::new());
