//! KntonOS Virtual File System (VFS) Subsystem
//!
//! Exposes universal file and directory tree abstractions for KntonOS.

pub mod vfs;
pub mod inode;
pub mod tar;
pub mod path;
pub mod initramfs;
pub mod fat32;
pub mod acl;
pub mod cache;

use alloc::sync::Arc;
use alloc::vec::Vec;

pub use vfs::{FileType, FilePerms, FsError, SeekMode, VfsMetadata, FileSystem};
pub use inode::{DirEntry, Inode};
pub use tar::{TarArchive, TarEntry, TAR_BLOCK_SIZE};
pub use path::Path;
pub use initramfs::Initramfs;
pub use fat32::{
    BiosParameterBlock32, BlockDevice, Fat32DirEntry, Fat32FileSystem, Fat32Inode,
    MemoryBlockDevice, VirtioBlockDeviceAdapter, test_fat32_subsystem,
};
pub use acl::{
    AccessControlEntry, AccessControlList, AceType, FileAccessMask, FilePermissions,
    ModePermissions, SecurityIdentifier,
};
pub use cache::{
    ByteRangeLock, InodeCacheEntry, InodeCacheStats, InodeLockManager, InodeLockType,
    InodeLookupCache, INODE_CACHE, INODE_CACHE_CAPACITY, INODE_LOCK_MGR,
};

use crate::memory::spinlock::Spinlock;
use crate::serial_write_str;

/// Security context attached to a VFS file or directory path.
#[derive(Clone, Debug)]
pub struct VfsSecurityContext {
    pub owner: SecurityIdentifier,
    pub permissions: FilePermissions,
    pub acl: Option<AccessControlList>,
}

/// Inode representation of an in-memory dynamic VFS file.
pub struct DynamicFileInode {
    pub name: alloc::string::String,
    pub size: u64,
    pub permissions: FilePerms,
    pub inode_id: u64,
    pub data: Vec<u8>,
}

impl Inode for DynamicFileInode {
    fn metadata(&self) -> VfsMetadata {
        VfsMetadata {
            file_type: FileType::File,
            size: self.size,
            permissions: self.permissions,
            inode_id: self.inode_id,
        }
    }

    fn read_at(&self, offset: u64, buf: &mut [u8]) -> Result<usize, FsError> {
        if offset >= self.size {
            return Ok(0);
        }
        let start = offset as usize;
        if start >= self.data.len() {
            return Ok(0);
        }
        let to_read = core::cmp::min(buf.len(), self.data.len() - start);
        buf[..to_read].copy_from_slice(&self.data[start..start + to_read]);
        Ok(to_read)
    }

    fn write_at(&self, _offset: u64, _buf: &[u8]) -> Result<usize, FsError> {
        Err(FsError::NotSupported)
    }

    fn lookup(&self, _name: &str) -> Result<Arc<dyn Inode>, FsError> {
        Err(FsError::NotADirectory)
    }

    fn readdir(&self) -> Result<Vec<DirEntry>, FsError> {
        Err(FsError::NotADirectory)
    }
}


/// Inode representation of an in-memory Unix domain socket node in VFS.
pub struct SocketInode {
    pub name: alloc::string::String,
    pub permissions: FilePerms,
    pub inode_id: u64,
    pub socket_handle: usize,
}

impl Inode for SocketInode {
    fn metadata(&self) -> VfsMetadata {
        VfsMetadata {
            file_type: FileType::Socket,
            size: 0,
            permissions: self.permissions,
            inode_id: self.inode_id,
        }
    }

    fn read_at(&self, _offset: u64, _buf: &mut [u8]) -> Result<usize, FsError> {
        Err(FsError::NotSupported)
    }

    fn write_at(&self, _offset: u64, _buf: &[u8]) -> Result<usize, FsError> {
        Err(FsError::NotSupported)
    }

    fn lookup(&self, _name: &str) -> Result<Arc<dyn Inode>, FsError> {
        Err(FsError::NotADirectory)
    }

    fn readdir(&self) -> Result<Vec<DirEntry>, FsError> {
        Err(FsError::NotADirectory)
    }
}
pub struct VfsManager {
    root_fs: Option<Arc<dyn FileSystem>>,
    mounts: alloc::collections::BTreeMap<alloc::string::String, Arc<dyn FileSystem>>,
    dynamic_files: alloc::collections::BTreeMap<alloc::string::String, Vec<u8>>,
    security: alloc::collections::BTreeMap<alloc::string::String, VfsSecurityContext>,
    socket_nodes: alloc::collections::BTreeMap<alloc::string::String, (usize, FilePerms, u64)>,
}

impl VfsManager {
    pub const fn new() -> Self {
        Self {
            root_fs: None,
            mounts: alloc::collections::BTreeMap::new(),
            dynamic_files: alloc::collections::BTreeMap::new(),
            security: alloc::collections::BTreeMap::new(),
            socket_nodes: alloc::collections::BTreeMap::new(),
        }
    }

    pub fn set_root(&mut self, fs: Arc<dyn FileSystem>) {
        self.root_fs = Some(fs);
    }

    pub fn mount(&mut self, mountpoint: &str, fs: Arc<dyn FileSystem>) {
        let name = mountpoint.trim_matches('/');
        self.mounts.insert(alloc::string::String::from(name), fs);
    }

    /// Internal path resolver that traverses mounts and roots to find an Inode.
    fn resolve_path(&self, path: &str) -> Result<Arc<dyn Inode>, FsError> {
        if let Some(&(handle, perms, inode_id)) = self.socket_nodes.get(path) {
            return Ok(Arc::new(SocketInode {
                name: alloc::string::String::from(path),
                permissions: perms,
                inode_id,
                socket_handle: handle,
            }));
        }
        if let Some(data) = self.dynamic_files.get(path) {
            let inode_id = 0x8000_0000_0000_0000u64 | (path.len() as u64);
            let perms = if let Some(sec) = self.security.get(path) {
                sec.permissions.to_file_perms()
            } else {
                FilePerms::rwx()
            };
            return Ok(Arc::new(DynamicFileInode {
                name: alloc::string::String::from(path),
                size: data.len() as u64,
                permissions: perms,
                inode_id,
                data: data.clone(),
            }));
        }

        let segments = Path::parse_segments(path)?;

        if let Some(&first_seg) = segments.first() {
            if let Some(fs) = self.mounts.get(first_seg) {
                let mut curr = fs.root()?;
                for &seg in &segments[1..] {
                    let next = curr.lookup(seg)?;
                    curr = next;
                }
                return Ok(curr);
            }
        }

        let root = self.root_fs.as_ref().ok_or(FsError::NotFound)?.root()?;
        let mut curr = root;
        for seg in segments {
            let next = curr.lookup(seg)?;
            curr = next;
        }

        Ok(curr)
    }

    /// Opens an Inode with ACL and POSIX permission validation.
    pub fn open_with_perm(
        &self,
        path: &str,
        subject: &SecurityIdentifier,
        mask: FileAccessMask,
    ) -> Result<Arc<dyn Inode>, FsError> {
        // 1. Check Inode Cache first
        if let Some(cached) = INODE_CACHE.lock().get_by_path(path) {
            let allowed = if let Some(acl) = &cached.acl {
                acl.check_access(subject, mask)
            } else {
                cached.permissions.check_permission(&cached.owner, subject, mask)
            };

            if !allowed {
                return Err(FsError::PermissionDenied);
            }
            return Ok(cached.inode);
        }

        // 2. Cache miss: Resolve Inode through filesystem
        let inode = self.resolve_path(path)?;
        let meta = inode.metadata();

        let (owner, perms, acl) = if let Some(sec) = self.security.get(path) {
            (sec.owner.clone(), sec.permissions, sec.acl.clone())
        } else {
            (SecurityIdentifier::root(), FilePermissions::from(meta.permissions), None)
        };

        // Cache the resolved Inode
        let mut entry = InodeCacheEntry::new(meta.inode_id, path, inode.clone(), perms, owner.clone());
        if let Some(a) = acl.clone() {
            entry = entry.with_acl(a);
        }
        INODE_CACHE.lock().insert(entry);

        // Verify permissions
        let allowed = if let Some(a) = &acl {
            a.check_access(subject, mask)
        } else {
            perms.check_permission(&owner, subject, mask)
        };

        if !allowed {
            return Err(FsError::PermissionDenied);
        }

        Ok(inode)
    }

    /// Opens an Inode using root/kernel privileges.
    pub fn open(&self, path: &str) -> Result<Arc<dyn Inode>, FsError> {
        self.open_with_perm(path, &SecurityIdentifier::root(), FileAccessMask::read())
    }

    /// Reads file data with subject access permission verification.
    pub fn read_file_with_perm(
        &self,
        path: &str,
        subject: &SecurityIdentifier,
    ) -> Result<Vec<u8>, FsError> {
        let inode = self.open_with_perm(path, subject, FileAccessMask::read())?;
        let meta = inode.metadata();
        if meta.file_type != FileType::File {
            return Err(FsError::IsADirectory);
        }

        if let Some(data) = self.dynamic_files.get(path) {
            return Ok(data.clone());
        }

        let mut buf = alloc::vec![0u8; meta.size as usize];
        let bytes_read = inode.read_at(0, &mut buf)?;
        buf.truncate(bytes_read);
        Ok(buf)
    }

    /// Reads file data using root privileges.
    pub fn read_file(&self, path: &str) -> Result<Vec<u8>, FsError> {
        self.read_file_with_perm(path, &SecurityIdentifier::root())
    }

    /// Writes data to a file validating write permissions and lock contention.
    pub fn write_file_with_perm(
        &mut self,
        path: &str,
        data: &[u8],
        subject: &SecurityIdentifier,
    ) -> Result<(), FsError> {
        // Validate write permissions
        if let Some(sec) = self.security.get(path) {
            let allowed = if let Some(acl) = &sec.acl {
                acl.check_access(subject, FileAccessMask::write())
            } else {
                sec.permissions.check_permission(&sec.owner, subject, FileAccessMask::write())
            };
            if !allowed {
                return Err(FsError::PermissionDenied);
            }
        }

        // Validate lock contention
        if let Some(entry) = INODE_CACHE.lock().get_by_path(path) {
            if INODE_LOCK_MGR.lock().has_conflict(
                entry.inode_id,
                InodeLockType::Exclusive,
                0,
                data.len() as u64,
                subject.uid as u64,
            ) {
                return Err(FsError::PermissionDenied);
            }
        }

        self.dynamic_files.insert(alloc::string::String::from(path), data.to_vec());

        {
            let mut cache = INODE_CACHE.lock();
            if let Some(mut entry) = cache.get_by_path(path) {
                entry.mark_dirty();
                cache.insert(entry);
            }
        }

        Ok(())
    }

    pub fn write_file(&mut self, path: &str, data: &[u8]) {
        let _ = self.write_file_with_perm(path, data, &SecurityIdentifier::root());
    }

    pub fn append_file(&mut self, path: &str, data: &[u8]) {
        let entry = self.dynamic_files.entry(alloc::string::String::from(path)).or_insert_with(Vec::new);
        entry.extend_from_slice(data);
    }

    /// Creates a new file with explicit owner, POSIX permissions, and optional ACL.
    pub fn create_file_with_perm(
        &mut self,
        path: &str,
        data: &[u8],
        owner: SecurityIdentifier,
        permissions: FilePermissions,
        acl: Option<AccessControlList>,
        subject: &SecurityIdentifier,
    ) -> Result<(), FsError> {
        // Validate parent directory permissions
        if let Some((parent, _)) = path.rsplit_once('/') {
            let parent_path = if parent.is_empty() { "/" } else { parent };
            if let Some(sec) = self.security.get(parent_path) {
                let mask = FileAccessMask::from_bits(FileAccessMask::WRITE | FileAccessMask::EXECUTE);
                let allowed = if let Some(p_acl) = &sec.acl {
                    p_acl.check_access(subject, mask)
                } else {
                    sec.permissions.check_permission(&sec.owner, subject, mask)
                };
                if !allowed {
                    return Err(FsError::PermissionDenied);
                }
            }
        }

        self.security.insert(
            alloc::string::String::from(path),
            VfsSecurityContext {
                owner: owner.clone(),
                permissions,
                acl: acl.clone(),
            },
        );

        self.dynamic_files.insert(alloc::string::String::from(path), data.to_vec());

        let inode_id = 0x8000_0000_0000_0000u64 | (self.dynamic_files.len() as u64);
        let mut entry = InodeCacheEntry::new(
            inode_id,
            path,
            Arc::new(DynamicFileInode {
                name: alloc::string::String::from(path),
                size: data.len() as u64,
                permissions: permissions.to_file_perms(),
                inode_id,
                data: data.to_vec(),
            }),
            permissions,
            owner,
        );
        if let Some(a) = acl {
            entry = entry.with_acl(a);
        }
        INODE_CACHE.lock().insert(entry);

        Ok(())
    }

    /// Sets or updates the Access Control List on an existing file.
    pub fn set_acl(
        &mut self,
        path: &str,
        acl: AccessControlList,
        subject: &SecurityIdentifier,
    ) -> Result<(), FsError> {
        let mut sec = if let Some(s) = self.security.get(path) {
            s.clone()
        } else {
            VfsSecurityContext {
                owner: SecurityIdentifier::root(),
                permissions: FilePermissions::default_file(),
                acl: None,
            }
        };

        if !subject.is_root() && subject.uid != sec.owner.uid {
            return Err(FsError::PermissionDenied);
        }

        sec.acl = Some(acl.clone());
        self.security.insert(alloc::string::String::from(path), sec);

        {
            let mut cache = INODE_CACHE.lock();
            if let Some(mut cached) = cache.get_by_path(path) {
                cached.acl = Some(acl);
                cache.insert(cached);
            }
        }

        Ok(())
    }

    /// Sets or updates POSIX file mode permissions on an existing file.
    
    pub fn bind_socket(&mut self, path: &str, socket_handle: usize, perms: FilePerms) -> Result<u64, FsError> {
        if self.dynamic_files.contains_key(path) || self.socket_nodes.contains_key(path) {
            return Err(FsError::AlreadyExists);
        }
        let inode_id = 0xC000_0000_0000_0000u64 | (self.socket_nodes.len() as u64 + 1);
        self.socket_nodes.insert(alloc::string::String::from(path), (socket_handle, perms, inode_id));
        let inode = Arc::new(SocketInode {
            name: alloc::string::String::from(path),
            permissions: perms,
            inode_id,
            socket_handle,
        });
        let entry = InodeCacheEntry::new(
            inode_id,
            path,
            inode,
            FilePermissions::from_octal(0o777),
            SecurityIdentifier::root(),
        );
        INODE_CACHE.lock().insert(entry);
        Ok(inode_id)
    }

    pub fn get_socket_node(&self, path: &str) -> Option<usize> {
        self.socket_nodes.get(path).map(|(handle, _, _)| *handle)
    }

    pub fn unlink(&mut self, path: &str) -> Result<(), FsError> {
        let removed_socket = self.socket_nodes.remove(path).is_some();
        let removed_file = self.dynamic_files.remove(path).is_some();
        if removed_socket || removed_file {
            self.security.remove(path);
            INODE_CACHE.lock().invalidate_path(path);
            Ok(())
        } else {
            Err(FsError::NotFound)
        }
    }
    pub fn set_permissions(
        &mut self,
        path: &str,
        permissions: FilePermissions,
        subject: &SecurityIdentifier,
    ) -> Result<(), FsError> {
        let mut sec = if let Some(s) = self.security.get(path) {
            s.clone()
        } else {
            VfsSecurityContext {
                owner: SecurityIdentifier::root(),
                permissions: FilePermissions::default_file(),
                acl: None,
            }
        };

        if !subject.is_root() && subject.uid != sec.owner.uid {
            return Err(FsError::PermissionDenied);
        }

        sec.permissions = permissions;
        self.security.insert(alloc::string::String::from(path), sec);

        {
            let mut cache = INODE_CACHE.lock();
            if let Some(mut cached) = cache.get_by_path(path) {
                cached.permissions = permissions;
                cache.insert(cached);
            }
        }

        Ok(())
    }
}

pub static VFS: Spinlock<VfsManager> = Spinlock::new(VfsManager::new());

pub fn unlink(path: &str) -> Result<(), FsError> {
    VFS.lock().unlink(path)
}

pub fn init() {
    serial_write_str("[KNT-FS] Initializing Virtual File System (VFS)...\r\n");
}

/// Helper function to create an in-memory test UStar TAR archive with two test files.
fn build_demo_tar() -> Vec<u8> {
    let mut tar = Vec::new();

    // Helper closure to write a 512-byte tar header
    let append_entry = |tar: &mut Vec<u8>, name: &str, content: &[u8], is_dir: bool| {
        let mut header = [0u8; TAR_BLOCK_SIZE];
        // 0..100: name
        let name_bytes = name.as_bytes();
        let name_len = core::cmp::min(name_bytes.len(), 100);
        header[..name_len].copy_from_slice(&name_bytes[..name_len]);

        // 100..108: mode (0000755\0 for dir, 0000644\0 for file)
        let mode = if is_dir { b"0000755\0" } else { b"0000644\0" };
        header[100..108].copy_from_slice(mode);

        // 108..116: uid (0000000\0)
        header[108..116].copy_from_slice(b"0000000\0");
        // 116..124: gid (0000000\0)
        header[116..124].copy_from_slice(b"0000000\0");

        // 124..136: size (11 octal digits + null/space)
        let size = if is_dir { 0 } else { content.len() };
        let mut sz_buf = [b'0'; 11];
        let mut temp_sz = size;
        for i in (0..11).rev() {
            sz_buf[i] = b'0' + (temp_sz % 8) as u8;
            temp_sz /= 8;
        }
        header[124..135].copy_from_slice(&sz_buf);
        header[135] = 0;

        // 136..148: mtime
        header[136..148].copy_from_slice(b"00000000000\0");

        // 148..156: checksum placeholder (spaces during sum calculation)
        header[148..156].copy_from_slice(b"        ");

        // 156: typeflag ('5' dir, '0' file)
        header[156] = if is_dir { b'5' } else { b'0' };

        // 257..263: magic ("ustar\0")
        header[257..263].copy_from_slice(b"ustar\0");
        // 263..265: version ("00")
        header[263..265].copy_from_slice(b"00");

        // Calculate simple byte checksum
        let sum: u32 = header.iter().map(|&b| b as u32).sum();
        let mut chk_buf = [b'0'; 6];
        let mut temp_sum = sum;
        for i in (0..6).rev() {
            chk_buf[i] = b'0' + (temp_sum % 8) as u8;
            temp_sum /= 8;
        }
        header[148..154].copy_from_slice(&chk_buf);
        header[154] = 0;
        header[155] = b' ';

        tar.extend_from_slice(&header);

        // File data
        if !is_dir && !content.is_empty() {
            tar.extend_from_slice(content);
            let padding = (TAR_BLOCK_SIZE - (content.len() % TAR_BLOCK_SIZE)) % TAR_BLOCK_SIZE;
            if padding > 0 {
                tar.resize(tar.len() + padding, 0);
            }
        }
    };

    // 1. Directory "etc/"
    append_entry(&mut tar, "etc/", &[], true);

    // 2. File "etc/os-release"
    let os_release_data = b"NAME=KntonOS\nVERSION=0.1.0\nID=knton\n";
    append_entry(&mut tar, "etc/os-release", os_release_data, false);

    // 3. Directory "bin/"
    append_entry(&mut tar, "bin/", &[], true);

    // 4. File "bin/init.exe"
    let exe_data = crate::pe::build_demo_pe();
    append_entry(&mut tar, "bin/init.exe", &exe_data, false);

    // Two 512-byte zero blocks terminating the archive
    tar.resize(tar.len() + TAR_BLOCK_SIZE * 2, 0);

    tar
}

/// Verification self-test for VFS and memory-mapped Initramfs.
pub fn test_vfs_subsystem() -> bool {
    serial_write_str("[KNT-FS-TEST] Testing VFS mount, lookup, and reading...\r\n");

    let demo_tar = build_demo_tar();
    let initramfs = match Initramfs::from_tar(&demo_tar) {
        Ok(fs) => Arc::new(fs),
        Err(_) => {
            serial_write_str("[KNT-PANIC] Failed to parse demo initramfs archive!\r\n");
            return false;
        }
    };

    // Mount at root
    {
        let mut vfs = VFS.lock();
        vfs.set_root(initramfs);
    }
    serial_write_str("  * Initramfs successfully mounted at root (/)\r\n");

    let vfs = VFS.lock();

    // 1. Lookup and read "/etc/os-release"
    match vfs.read_file("/etc/os-release") {
        Ok(data) => {
            if !data.starts_with(b"NAME=KntonOS") {
                serial_write_str("[KNT-PANIC] /etc/os-release content mismatch!\r\n");
                return false;
            }
        }
        Err(_) => {
            serial_write_str("[KNT-PANIC] Failed to open /etc/os-release!\r\n");
            return false;
        }
    }

    // 2. Lookup and read "/bin/init.exe"
    match vfs.read_file("/bin/init.exe") {
        Ok(data) => {
            if !data.starts_with(b"MZ") {
                serial_write_str("[KNT-PANIC] /bin/init.exe MZ header missing!\r\n");
                return false;
            }
        }
        Err(_) => {
            serial_write_str("[KNT-PANIC] Failed to open /bin/init.exe!\r\n");
            return false;
        }
    }

    // 3. Verify non-existent file returns NotFound
    match vfs.read_file("/var/log/nonexistent.txt") {
        Err(FsError::NotFound) => {}
        _ => {
            serial_write_str("[KNT-PANIC] Non-existent path did not yield NotFound!\r\n");
            return false;
        }
    }

    serial_write_str("[KNT-FS-PASS] Initramfs directory tree and file reads verified\r\n");
    true
}


/// Verification test suite for VFS Unified Inode Cache, Lock Manager, & ACL Access Controls.
pub fn test_vfs_acl_and_cache() -> bool {
    serial_write_str("[KNT-FS-ACL-TEST] Running VFS Inode cache, lock contention, and ACL verification suite...\r\n");

    // 1. Verify POSIX Mode Permissions and SID evaluations
    let sid_root = SecurityIdentifier::root();
    let sid_owner = SecurityIdentifier::user(1001, 100);
    let sid_group_member = SecurityIdentifier::user(1002, 100);
    let sid_stranger = SecurityIdentifier::user(1003, 200);

    let perms_640 = FilePermissions::from_octal(0o640);
    if perms_640.to_octal() != 0o640 {
        serial_write_str("[KNT-PANIC] FilePermissions octal conversion mismatch!\r\n");
        return false;
    }

    // Owner can read & write 0o640
    if !perms_640.check_permission(&sid_owner, &sid_owner, FileAccessMask::read_write()) {
        serial_write_str("[KNT-PANIC] Owner failed read_write on 0o640!\r\n");
        return false;
    }
    // Group member can read, but cannot write 0o640
    if !perms_640.check_permission(&sid_owner, &sid_group_member, FileAccessMask::read()) {
        serial_write_str("[KNT-PANIC] Group member denied read on 0o640!\r\n");
        return false;
    }
    if perms_640.check_permission(&sid_owner, &sid_group_member, FileAccessMask::write()) {
        serial_write_str("[KNT-PANIC] Group member granted write on 0o640!\r\n");
        return false;
    }
    // Stranger cannot read or write 0o640
    if perms_640.check_permission(&sid_owner, &sid_stranger, FileAccessMask::read()) {
        serial_write_str("[KNT-PANIC] Stranger granted read on 0o640!\r\n");
        return false;
    }
    // Root has full access
    if !perms_640.check_permission(&sid_owner, &sid_root, FileAccessMask::all()) {
        serial_write_str("[KNT-PANIC] Root denied access on 0o640!\r\n");
        return false;
    }
    serial_write_str("  * POSIX mode permissions and SID evaluations verified\r\n");

    // 2. Win32 / NT Discretionary Access Control List (DACL) evaluation
    let mut acl = AccessControlList::new();
    let sid_denied = SecurityIdentifier::user(2001, 300);
    let sid_allowed = SecurityIdentifier::user(2002, 300);

    // Explicit Deny takes precedence over Allow
    acl.deny(sid_denied.clone(), FileAccessMask::read_write());
    acl.allow(sid_allowed.clone(), FileAccessMask::read());
    acl.allow(SecurityIdentifier::everyone(), FileAccessMask::read());

    // Denied user must be rejected for read despite Everyone having read
    if acl.check_access(&sid_denied, FileAccessMask::read()) {
        serial_write_str("[KNT-PANIC] Explicit Deny ACE failed to block read access!\r\n");
        return false;
    }
    // Allowed user must be granted read
    if !acl.check_access(&sid_allowed, FileAccessMask::read()) {
        serial_write_str("[KNT-PANIC] Explicit Allow ACE failed to grant read access!\r\n");
        return false;
    }
    // Allowed user must NOT be granted write (not permitted)
    if acl.check_access(&sid_allowed, FileAccessMask::write()) {
        serial_write_str("[KNT-PANIC] Allowed user granted unrequested write permission!\r\n");
        return false;
    }
    // Stranger gets read from Everyone, but no write
    if !acl.check_access(&sid_stranger, FileAccessMask::read()) {
        serial_write_str("[KNT-PANIC] Everyone ACE failed to grant read access!\r\n");
        return false;
    }
    if acl.check_access(&sid_stranger, FileAccessMask::write()) {
        serial_write_str("[KNT-PANIC] Stranger granted unpermitted write access!\r\n");
        return false;
    }
    serial_write_str("  * Win32/NT Discretionary ACL (DACL) allow/deny evaluation verified\r\n");

    // 3. InodeLockManager whole-file and byte-range contention
    let mut lock_mgr = InodeLockManager::new();
    let inode_test = 501u64;

    // Process 1 takes Shared lock on file
    if !lock_mgr.try_lock_file(inode_test, InodeLockType::Shared, 1) {
        serial_write_str("[KNT-PANIC] Process 1 failed to acquire Shared file lock!\r\n");
        return false;
    }
    // Process 2 takes Shared lock on file -> Should succeed (shared locks coexist)
    if !lock_mgr.try_lock_file(inode_test, InodeLockType::Shared, 2) {
        serial_write_str("[KNT-PANIC] Process 2 failed to acquire concurrent Shared file lock!\r\n");
        return false;
    }
    // Process 3 attempts Exclusive lock on file -> Must fail
    if lock_mgr.try_lock_file(inode_test, InodeLockType::Exclusive, 3) {
        serial_write_str("[KNT-PANIC] Process 3 acquired Exclusive file lock while Shared active!\r\n");
        return false;
    }
    // Unlock process 1 and 2
    if !lock_mgr.unlock_file(inode_test, 1) || !lock_mgr.unlock_file(inode_test, 2) {
        serial_write_str("[KNT-PANIC] Failed to release Shared file locks!\r\n");
        return false;
    }
    // Now Process 3 acquires Exclusive lock -> Should succeed
    if !lock_mgr.try_lock_file(inode_test, InodeLockType::Exclusive, 3) {
        serial_write_str("[KNT-PANIC] Process 3 failed to acquire Exclusive lock on free file!\r\n");
        return false;
    }
    // Process 1 attempts Shared lock -> Must fail
    if lock_mgr.try_lock_file(inode_test, InodeLockType::Shared, 1) {
        serial_write_str("[KNT-PANIC] Process 1 acquired Shared lock during active Exclusive lock!\r\n");
        return false;
    }
    lock_mgr.unlock_file(inode_test, 3);

    // Test byte-range lock non-overlapping vs overlapping
    if !lock_mgr.try_lock_range(inode_test, InodeLockType::Exclusive, 0, 100, 1) {
        serial_write_str("[KNT-PANIC] Process 1 failed byte-range [0..100) lock!\r\n");
        return false;
    }
    // Non-overlapping range [100..200) by Process 2 should succeed
    if !lock_mgr.try_lock_range(inode_test, InodeLockType::Exclusive, 100, 100, 2) {
        serial_write_str("[KNT-PANIC] Process 2 failed non-overlapping range [100..200) lock!\r\n");
        return false;
    }
    // Overlapping range [50..150) by Process 3 must fail
    if lock_mgr.try_lock_range(inode_test, InodeLockType::Exclusive, 50, 100, 3) {
        serial_write_str("[KNT-PANIC] Process 3 acquired overlapping range [50..150) lock!\r\n");
        return false;
    }
    lock_mgr.release_all_owner_locks(1);
    lock_mgr.release_all_owner_locks(2);
    if lock_mgr.is_locked(inode_test) {
        serial_write_str("[KNT-PANIC] Inode lock manager still reports locks after release_all!\r\n");
        return false;
    }
    serial_write_str("  * Thread-safe Inode read-write lock manager and byte-range contention verified\r\n");

    // 4. InodeLookupCache LRU aging, eviction, and hit-rate tracking
    let mut cache = InodeLookupCache::with_capacity(3);
    let dummy_inode = alloc::sync::Arc::new(DynamicFileInode {
        name: alloc::string::String::from("/test.txt"),
        size: 0,
        permissions: FilePerms::rwx(),
        inode_id: 101,
        data: alloc::vec::Vec::new(),
    });

    let e1 = InodeCacheEntry::new(101, "/file1.txt", dummy_inode.clone(), FilePermissions::default_file(), sid_owner.clone());
    let e2 = InodeCacheEntry::new(102, "/file2.txt", dummy_inode.clone(), FilePermissions::default_file(), sid_owner.clone());
    let e3 = InodeCacheEntry::new(103, "/file3.txt", dummy_inode.clone(), FilePermissions::default_file(), sid_owner.clone());
    let e4 = InodeCacheEntry::new(104, "/file4.txt", dummy_inode.clone(), FilePermissions::default_file(), sid_owner.clone());

    cache.insert(e1);
    cache.insert(e2);
    cache.insert(e3);
    if cache.len() != 3 {
        serial_write_str("[KNT-PANIC] InodeCache length is not 3!\r\n");
        return false;
    }

    // Touch /file1.txt to make /file2.txt the LRU entry
    let hit_e1 = cache.get_by_path("/file1.txt");
    if hit_e1.is_none() {
        serial_write_str("[KNT-PANIC] InodeCache get_by_path failed for /file1.txt!\r\n");
        return false;
    }

    // Insert e4 -> must evict /file2.txt
    let evicted = cache.insert(e4);
    if evicted.is_none() || evicted.as_ref().unwrap().path != "/file2.txt" {
        serial_write_str("[KNT-PANIC] InodeCache failed to evict LRU entry /file2.txt!\r\n");
        return false;
    }
    // Verify /file2.txt is a cache miss now
    if cache.get_by_path("/file2.txt").is_some() {
        serial_write_str("[KNT-PANIC] Evicted entry /file2.txt still present in cache!\r\n");
        return false;
    }
    let stats = cache.stats();
    if stats.hits != 1 || stats.misses != 1 || stats.evictions != 1 {
        serial_write_str("[KNT-PANIC] InodeCache stats accounting mismatch!\r\n");
        return false;
    }
    serial_write_str("  * 128-entry Inode lookup cache with LRU aging and hit-rate tracking verified\r\n");

    // 5. Integrated VFS Path Execution (open_with_perm, read_file_with_perm, write_file_with_perm, create_file_with_perm)
    {
        let mut vfs = VFS.lock();
        let secure_data = b"CONFIDENTIAL_KEY_987654";
        let res_create = vfs.create_file_with_perm(
            "/etc/shadow.txt",
            secure_data,
            sid_owner.clone(),
            FilePermissions::from_octal(0o600), // Owner rw only
            None,
            &sid_owner,
        );
        if res_create.is_err() {
            serial_write_str("[KNT-PANIC] create_file_with_perm failed for /etc/shadow.txt!\r\n");
            return false;
        }

        // Stranger attempts read -> must be PermissionDenied
        match vfs.read_file_with_perm("/etc/shadow.txt", &sid_stranger) {
            Err(FsError::PermissionDenied) => {}
            _ => {
                serial_write_str("[KNT-PANIC] Unauthorized user was not denied read on /etc/shadow.txt!\r\n");
                return false;
            }
        }

        // Owner reads -> succeeds
        match vfs.read_file_with_perm("/etc/shadow.txt", &sid_owner) {
            Ok(data) => {
                if data != secure_data {
                    serial_write_str("[KNT-PANIC] /etc/shadow.txt content mismatch on authorized read!\r\n");
                    return false;
                }
            }
            Err(_) => {
                serial_write_str("[KNT-PANIC] Authorized owner read failed on /etc/shadow.txt!\r\n");
                return false;
            }
        }

        // Add explicit ACL granting user_auditor read access
        let sid_auditor = SecurityIdentifier::user(3001, 500);
        let mut shadow_acl = AccessControlList::new();
        shadow_acl.allow(sid_auditor.clone(), FileAccessMask::read());
        let res_acl = vfs.set_acl("/etc/shadow.txt", shadow_acl, &sid_owner);
        if res_acl.is_err() {
            serial_write_str("[KNT-PANIC] set_acl failed on /etc/shadow.txt!\r\n");
            return false;
        }

        // Auditor reads -> succeeds via ACL
        match vfs.read_file_with_perm("/etc/shadow.txt", &sid_auditor) {
            Ok(data) => {
                if data != secure_data {
                    serial_write_str("[KNT-PANIC] /etc/shadow.txt content mismatch on auditor read!\r\n");
                    return false;
                }
            }
            Err(_) => {
                serial_write_str("[KNT-PANIC] Auditor ACL read failed on /etc/shadow.txt!\r\n");
                return false;
            }
        }

        // Auditor attempts write -> denied by ACL
        match vfs.write_file_with_perm("/etc/shadow.txt", b"TAMPERED_DATA", &sid_auditor) {
            Err(FsError::PermissionDenied) => {}
            _ => {
                serial_write_str("[KNT-PANIC] Unauthorized write succeeded on ACL protected file!\r\n");
                return false;
            }
        }

        // Test lock contention on write: Process acquires exclusive lock
        let shadow_inode_id = INODE_CACHE.lock().get_by_path("/etc/shadow.txt").map(|e| e.inode_id).unwrap_or(0);
        INODE_LOCK_MGR.lock().try_lock_file(shadow_inode_id, InodeLockType::Exclusive, 999);

        // Owner attempts write while locked by owner 999 -> must be blocked
        match vfs.write_file_with_perm("/etc/shadow.txt", b"UPDATE", &sid_owner) {
            Err(FsError::PermissionDenied) => {}
            _ => {
                serial_write_str("[KNT-PANIC] Write succeeded during active lock contention!\r\n");
                return false;
            }
        }
        INODE_LOCK_MGR.lock().unlock_file(shadow_inode_id, 999);
    }

    serial_write_str("[KNT-FS-ACL-PASS] VFS unified Inode cache, lock manager, and ACL access controls verified\r\n");
    true
}
