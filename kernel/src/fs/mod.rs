//! KntonOS Virtual File System (VFS) Subsystem
//!
//! Exposes universal file and directory tree abstractions for KntonOS.

pub mod vfs;
pub mod inode;
pub mod tar;
pub mod path;
pub mod initramfs;
pub mod fat32;

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

use crate::memory::spinlock::Spinlock;
use crate::serial_write_str;

pub struct VfsManager {
    root_fs: Option<Arc<dyn FileSystem>>,
    mounts: alloc::collections::BTreeMap<alloc::string::String, Arc<dyn FileSystem>>,
    dynamic_files: alloc::collections::BTreeMap<alloc::string::String, Vec<u8>>,
}

impl VfsManager {
    pub const fn new() -> Self {
        Self {
            root_fs: None,
            mounts: alloc::collections::BTreeMap::new(),
            dynamic_files: alloc::collections::BTreeMap::new(),
        }
    }

    pub fn set_root(&mut self, fs: Arc<dyn FileSystem>) {
        self.root_fs = Some(fs);
    }

    pub fn mount(&mut self, mountpoint: &str, fs: Arc<dyn FileSystem>) {
        let name = mountpoint.trim_matches('/');
        self.mounts.insert(alloc::string::String::from(name), fs);
    }

    pub fn write_file(&mut self, path: &str, data: &[u8]) {
        self.dynamic_files.insert(alloc::string::String::from(path), data.to_vec());
    }

    pub fn append_file(&mut self, path: &str, data: &[u8]) {
        let entry = self.dynamic_files.entry(alloc::string::String::from(path)).or_insert_with(Vec::new);
        entry.extend_from_slice(data);
    }

    pub fn open(&self, path: &str) -> Result<Arc<dyn Inode>, FsError> {
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

    pub fn read_file(&self, path: &str) -> Result<Vec<u8>, FsError> {
        if let Some(data) = self.dynamic_files.get(path) {
            return Ok(data.clone());
        }
        let inode = self.open(path)?;
        let meta = inode.metadata();
        if meta.file_type != FileType::File {
            return Err(FsError::IsADirectory);
        }

        let mut buf = alloc::vec![0u8; meta.size as usize];
        let bytes_read = inode.read_at(0, &mut buf)?;
        buf.truncate(bytes_read);
        Ok(buf)
    }
}

pub static VFS: Spinlock<VfsManager> = Spinlock::new(VfsManager::new());

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
