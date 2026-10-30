//! FAT32 Persistent File System Driver for KntonOS
//! Implements Microsoft FAT32 specification volume geometry, BIOS Parameter Block (BPB)

use alloc::string::String;
use alloc::sync::Arc;
use alloc::vec::Vec;
use crate::fs::inode::{DirEntry, Inode};
use crate::fs::vfs::{FilePerms, FileType, FileSystem, FsError, VfsMetadata};

/// Standard FAT boot sector signature (0x55, 0xAA at offsets 510..511).
pub const FAT_BOOT_SIGNATURE: u16 = 0xAA55;

/// Sector size expected for standard FAT32 storage devices.
pub const FAT32_SECTOR_SIZE: usize = 512;

/// Special cluster marker: Free/unallocated cluster.
pub const FAT32_CLUSTER_FREE: u32 = 0x0000_0000;

/// Special cluster marker: Reserved cluster.
pub const FAT32_CLUSTER_RESERVED: u32 = 0x0000_0001;

/// Special cluster marker: Defective/bad sector cluster.
pub const FAT32_CLUSTER_BAD: u32 = 0x0FFF_FFF7;

/// Minimum value indicating End-Of-Cluster-Chain (EOC).
pub const FAT32_CLUSTER_EOC_MIN: u32 = 0x0FFF_FFF8;

/// Standard End-Of-Cluster-Chain marker written to newly terminated chains.
pub const FAT32_CLUSTER_EOC: u32 = 0x0FFF_FFFF;

/// Bitmask to clear the reserved top 4 bits in FAT32 table entries.
pub const FAT32_ENTRY_MASK: u32 = 0x0FFF_FFFF;

/// Directory attribute: Read-only file.
pub const ATTR_READ_ONLY: u8 = 0x01;

/// Directory attribute: Hidden file.
pub const ATTR_HIDDEN: u8 = 0x02;

/// Directory attribute: Operating system file.
pub const ATTR_SYSTEM: u8 = 0x04;

/// Directory attribute: Volume label marker.
pub const ATTR_VOLUME_ID: u8 = 0x08;

/// Directory attribute: Subdirectory.
pub const ATTR_DIRECTORY: u8 = 0x10;

/// Directory attribute: Archive / modified.
pub const ATTR_ARCHIVE: u8 = 0x20;

/// Directory attribute combination for Long File Name (LFN) entries.
pub const ATTR_LONG_NAME: u8 = 0x0F;

/// Abstract interface for a block storage device capable of 512-byte sector I/O.
pub trait BlockDevice: Send + Sync {
    /// Read a single 512-byte sector at `sector` LBA into `buf`.
    fn read_sector(&self, sector: u64, buf: &mut [u8]) -> Result<(), FsError>;

    /// Write a single 512-byte sector at `sector` LBA from `buf`.
    fn write_sector(&self, sector: u64, buf: &[u8]) -> Result<(), FsError>;
}

/// In-memory RAM block device for testing and synthetic volumes.
pub struct MemoryBlockDevice {
    data: crate::memory::Spinlock<Vec<u8>>,
}

impl MemoryBlockDevice {
    pub fn new(data: Vec<u8>) -> Self {
        Self {
            data: crate::memory::Spinlock::new(data),
        }
    }

    pub fn snapshot(&self) -> Vec<u8> {
        self.data.lock().clone()
    }
}

impl BlockDevice for MemoryBlockDevice {
    fn read_sector(&self, sector: u64, buf: &mut [u8]) -> Result<(), FsError> {
        if buf.len() < FAT32_SECTOR_SIZE {
            return Err(FsError::InvalidArgument);
        }
        let offset = (sector as usize) * FAT32_SECTOR_SIZE;
        let lock = self.data.lock();
        if offset + FAT32_SECTOR_SIZE > lock.len() {
            return Err(FsError::IoError);
        }
        buf[..FAT32_SECTOR_SIZE].copy_from_slice(&lock[offset..offset + FAT32_SECTOR_SIZE]);
        Ok(())
    }

    fn write_sector(&self, sector: u64, buf: &[u8]) -> Result<(), FsError> {
        if buf.len() < FAT32_SECTOR_SIZE {
            return Err(FsError::InvalidArgument);
        }
        let offset = (sector as usize) * FAT32_SECTOR_SIZE;
        let mut lock = self.data.lock();
        if offset + FAT32_SECTOR_SIZE > lock.len() {
            return Err(FsError::NoSpace);
        }
        lock[offset..offset + FAT32_SECTOR_SIZE].copy_from_slice(&buf[..FAT32_SECTOR_SIZE]);
        Ok(())
    }
}

/// Hardware adapter bridging VirtIO 512-byte block controller to `BlockDevice`.
pub struct VirtioBlockDeviceAdapter;

impl BlockDevice for VirtioBlockDeviceAdapter {
    fn read_sector(&self, sector: u64, buf: &mut [u8]) -> Result<(), FsError> {
        let mut lock = crate::drivers::virtio::BLOCK_DEVICE.lock();
        let blk = lock.as_mut().ok_or(FsError::IoError)?;
        blk.read_sectors(sector, buf).map_err(|_| FsError::IoError)
    }

    fn write_sector(&self, sector: u64, buf: &[u8]) -> Result<(), FsError> {
        let mut lock = crate::drivers::virtio::BLOCK_DEVICE.lock();
        let blk = lock.as_mut().ok_or(FsError::IoError)?;
        blk.write_sectors(sector, buf).map_err(|_| FsError::IoError)
    }
}

/// Parsed FAT32 BIOS Parameter Block (BPB) and extended boot record.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BiosParameterBlock32 {
    pub bytes_per_sector: u16,
    pub sectors_per_cluster: u8,
    pub reserved_sector_count: u16,
    pub fat_count: u8,
    pub root_entry_count: u16,
    pub total_sectors_16: u16,
    pub media_type: u8,
    pub sectors_per_fat_16: u16,
    pub sectors_per_track: u16,
    pub head_count: u16,
    pub hidden_sectors: u32,
    pub total_sectors_32: u32,
    pub sectors_per_fat_32: u32,
    pub extended_flags: u16,
    pub fs_version: u16,
    pub root_cluster: u32,
    pub fs_info_sector: u16,
    pub backup_boot_sector: u16,
    pub drive_number: u8,
    pub boot_signature: u8,
    pub volume_id: u32,
    pub volume_label: [u8; 11],
    pub fs_type: [u8; 8],
}

impl BiosParameterBlock32 {
    /// Parse a 512-byte FAT32 boot sector.
    pub fn parse(boot_sector: &[u8]) -> Result<Self, FsError> {
        if boot_sector.len() < FAT32_SECTOR_SIZE {
            return Err(FsError::InvalidArgument);
        }

        // Verify standard MBR/VBR signature 0x55, 0xAA at 510..512
        let sig = u16::from_le_bytes([boot_sector[510], boot_sector[511]]);
        if sig != FAT_BOOT_SIGNATURE {
            return Err(FsError::CorruptArchive);
        }

        let bytes_per_sector = u16::from_le_bytes([boot_sector[11], boot_sector[12]]);
        let sectors_per_cluster = boot_sector[13];
        let reserved_sector_count = u16::from_le_bytes([boot_sector[14], boot_sector[15]]);
        let fat_count = boot_sector[16];
        let root_entry_count = u16::from_le_bytes([boot_sector[17], boot_sector[18]]);
        let total_sectors_16 = u16::from_le_bytes([boot_sector[19], boot_sector[20]]);
        let media_type = boot_sector[21];
        let sectors_per_fat_16 = u16::from_le_bytes([boot_sector[22], boot_sector[23]]);
        let sectors_per_track = u16::from_le_bytes([boot_sector[24], boot_sector[25]]);
        let head_count = u16::from_le_bytes([boot_sector[26], boot_sector[27]]);
        let hidden_sectors = u32::from_le_bytes([
            boot_sector[28],
            boot_sector[29],
            boot_sector[30],
            boot_sector[31],
        ]);
        let total_sectors_32 = u32::from_le_bytes([
            boot_sector[32],
            boot_sector[33],
            boot_sector[34],
            boot_sector[35],
        ]);

        // FAT32 Extended BPB
        let sectors_per_fat_32 = u32::from_le_bytes([
            boot_sector[36],
            boot_sector[37],
            boot_sector[38],
            boot_sector[39],
        ]);
        let extended_flags = u16::from_le_bytes([boot_sector[40], boot_sector[41]]);
        let fs_version = u16::from_le_bytes([boot_sector[42], boot_sector[43]]);
        let root_cluster = u32::from_le_bytes([
            boot_sector[44],
            boot_sector[45],
            boot_sector[46],
            boot_sector[47],
        ]);
        let fs_info_sector = u16::from_le_bytes([boot_sector[48], boot_sector[49]]);
        let backup_boot_sector = u16::from_le_bytes([boot_sector[50], boot_sector[51]]);

        let drive_number = boot_sector[64];
        let boot_signature = boot_sector[66];
        let volume_id = u32::from_le_bytes([
            boot_sector[67],
            boot_sector[68],
            boot_sector[69],
            boot_sector[70],
        ]);

        let mut volume_label = [0u8; 11];
        volume_label.copy_from_slice(&boot_sector[71..82]);

        let mut fs_type = [0u8; 8];
        fs_type.copy_from_slice(&boot_sector[82..90]);

        // Structural validation
        if bytes_per_sector != 512 && bytes_per_sector != 1024 && bytes_per_sector != 2048 && bytes_per_sector != 4096 {
            return Err(FsError::CorruptArchive);
        }
        if sectors_per_cluster == 0 || (sectors_per_cluster & (sectors_per_cluster - 1)) != 0 {
            return Err(FsError::CorruptArchive);
        }
        if reserved_sector_count == 0 || fat_count == 0 {
            return Err(FsError::CorruptArchive);
        }
        if sectors_per_fat_32 == 0 {
            return Err(FsError::CorruptArchive);
        }
        if root_cluster < 2 {
            return Err(FsError::CorruptArchive);
        }

        Ok(Self {
            bytes_per_sector,
            sectors_per_cluster,
            reserved_sector_count,
            fat_count,
            root_entry_count,
            total_sectors_16,
            media_type,
            sectors_per_fat_16,
            sectors_per_track,
            head_count,
            hidden_sectors,
            total_sectors_32,
            sectors_per_fat_32,
            extended_flags,
            fs_version,
            root_cluster,
            fs_info_sector,
            backup_boot_sector,
            drive_number,
            boot_signature,
            volume_id,
            volume_label,
            fs_type,
        })
    }

    /// Total sectors on the volume.
    pub fn total_sectors(&self) -> u64 {
        if self.total_sectors_16 != 0 {
            self.total_sectors_16 as u64
        } else {
            self.total_sectors_32 as u64
        }
    }

    /// First sector of the first File Allocation Table (FAT).
    pub fn first_fat_sector(&self) -> u64 {
        self.reserved_sector_count as u64
    }

    /// First sector of the data region (cluster 2).
    pub fn first_data_sector(&self) -> u64 {
        self.reserved_sector_count as u64
            + (self.fat_count as u64 * self.sectors_per_fat_32 as u64)
    }

    /// Number of bytes in a cluster.
    pub fn bytes_per_cluster(&self) -> u64 {
        self.bytes_per_sector as u64 * self.sectors_per_cluster as u64
    }

    /// Converts a cluster number into its starting absolute LBA sector number.
    /// Valid clusters start at cluster 2.
    pub fn cluster_to_sector(&self, cluster: u32) -> Result<u64, FsError> {
        if cluster < 2 {
            return Err(FsError::InvalidArgument);
        }
        let cluster_offset = (cluster - 2) as u64;
        let sector_offset = cluster_offset
            .checked_mul(self.sectors_per_cluster as u64)
            .ok_or(FsError::InvalidArgument)?;
        Ok(self.first_data_sector() + sector_offset)
    }

    /// Calculates total data clusters available on the volume.
    pub fn total_clusters(&self) -> u64 {
        let data_sectors = self.total_sectors().saturating_sub(self.first_data_sector());
        if self.sectors_per_cluster == 0 {
            0
        } else {
            data_sectors / self.sectors_per_cluster as u64
        }
    }
}

/// Returns true if the cluster value represents the End-Of-Chain marker.
pub fn is_end_of_chain(entry: u32) -> bool {
    (entry & FAT32_ENTRY_MASK) >= FAT32_CLUSTER_EOC_MIN
}

/// Returns true if the cluster value represents a bad/defective cluster.
pub fn is_bad_cluster(entry: u32) -> bool {
    (entry & FAT32_ENTRY_MASK) == FAT32_CLUSTER_BAD
}

/// Returns true if the cluster value represents an unallocated/free cluster.
pub fn is_free_cluster(entry: u32) -> bool {
    (entry & FAT32_ENTRY_MASK) == FAT32_CLUSTER_FREE
}

/// Read a 32-bit entry from the primary File Allocation Table (FAT1).
pub fn read_fat_entry<D: BlockDevice + ?Sized>(
    bpb: &BiosParameterBlock32,
    device: &D,
    cluster: u32,
) -> Result<u32, FsError> {
    if cluster < 2 {
        return Err(FsError::InvalidArgument);
    }
    let fat_offset = (cluster as u64) * 4;
    let sector = bpb.first_fat_sector() + (fat_offset / (bpb.bytes_per_sector as u64));
    let offset = (fat_offset % (bpb.bytes_per_sector as u64)) as usize;

    let mut buf = [0u8; FAT32_SECTOR_SIZE];
    device.read_sector(sector, &mut buf)?;
    let raw = u32::from_le_bytes([
        buf[offset],
        buf[offset + 1],
        buf[offset + 2],
        buf[offset + 3],
    ]);
    Ok(raw & FAT32_ENTRY_MASK)
}

/// Write a 32-bit entry to all active File Allocation Tables (FAT1 and FAT2),
/// preserving the high 4 reserved bits.
pub fn write_fat_entry<D: BlockDevice + ?Sized>(
    bpb: &BiosParameterBlock32,
    device: &D,
    cluster: u32,
    value: u32,
) -> Result<(), FsError> {
    if cluster < 2 {
        return Err(FsError::InvalidArgument);
    }
    let fat_offset = (cluster as u64) * 4;
    let offset_within_fat = fat_offset / (bpb.bytes_per_sector as u64);
    let byte_offset = (fat_offset % (bpb.bytes_per_sector as u64)) as usize;

    let mut buf = [0u8; FAT32_SECTOR_SIZE];
    for fat_idx in 0..(bpb.fat_count as u64) {
        let sector = bpb.first_fat_sector() + fat_idx * (bpb.sectors_per_fat_32 as u64) + offset_within_fat;
        device.read_sector(sector, &mut buf)?;
        let existing = u32::from_le_bytes([
            buf[byte_offset],
            buf[byte_offset + 1],
            buf[byte_offset + 2],
            buf[byte_offset + 3],
        ]);
        let updated = (existing & 0xF000_0000) | (value & FAT32_ENTRY_MASK);
        buf[byte_offset..byte_offset + 4].copy_from_slice(&updated.to_le_bytes());
        device.write_sector(sector, &buf)?;
    }
    Ok(())
}

/// Traverse a FAT32 cluster chain starting at `start_cluster` and return the ordered cluster list.
pub fn traverse_cluster_chain<D: BlockDevice + ?Sized>(
    bpb: &BiosParameterBlock32,
    device: &D,
    start_cluster: u32,
) -> Result<Vec<u32>, FsError> {
    let mut chain = Vec::new();
    let mut curr = start_cluster & FAT32_ENTRY_MASK;
    let max_clusters = bpb.total_clusters().max(1);

    while curr >= 2 && !is_end_of_chain(curr) && !is_bad_cluster(curr) {
        if chain.len() as u64 >= max_clusters || chain.contains(&curr) {
            return Err(FsError::CorruptArchive);
        }
        chain.push(curr);
        let next = read_fat_entry(bpb, device, curr)?;
        if is_end_of_chain(next) {
            break;
        }
        if is_bad_cluster(next) {
            return Err(FsError::IoError);
        }
        curr = next;
    }

    Ok(chain)
}

/// Read all sectors comprising `cluster` into `buf`.
pub fn read_cluster<D: BlockDevice + ?Sized>(
    bpb: &BiosParameterBlock32,
    device: &D,
    cluster: u32,
    buf: &mut [u8],
) -> Result<usize, FsError> {
    let cluster_bytes = bpb.bytes_per_cluster() as usize;
    if buf.len() < cluster_bytes {
        return Err(FsError::InvalidArgument);
    }
    let start_sec = bpb.cluster_to_sector(cluster)?;
    let sec_bytes = bpb.bytes_per_sector as usize;

    for s in 0..(bpb.sectors_per_cluster as u64) {
        let sec_offset = (s as usize) * sec_bytes;
        device.read_sector(
            start_sec + s,
            &mut buf[sec_offset..sec_offset + sec_bytes],
        )?;
    }
    Ok(cluster_bytes)
}

/// Read the entirety of a cluster chain into a single contiguous `Vec<u8>` buffer.
pub fn read_cluster_chain_data<D: BlockDevice + ?Sized>(
    bpb: &BiosParameterBlock32,
    device: &D,
    start_cluster: u32,
) -> Result<Vec<u8>, FsError> {
    let chain = traverse_cluster_chain(bpb, device, start_cluster)?;
    let cluster_size = bpb.bytes_per_cluster() as usize;
    let mut data = Vec::with_capacity(chain.len() * cluster_size);

    let mut temp = alloc::vec![0u8; cluster_size];
    for &cluster in &chain {
        read_cluster(bpb, device, cluster, &mut temp)?;
        data.extend_from_slice(&temp);
    }
    Ok(data)
}

/// Scans the File Allocation Table for the first free cluster, allocates it by marking
/// it as End-of-Chain (`FAT32_CLUSTER_EOC`), zeroes out the physical sectors, and returns the cluster id.
pub fn allocate_cluster<D: BlockDevice + ?Sized>(
    bpb: &BiosParameterBlock32,
    device: &D,
) -> Result<u32, FsError> {
    let total_clusters = bpb.total_clusters();
    let max_cluster = 2 + total_clusters as u32;

    for cluster in 2..max_cluster {
        let entry = read_fat_entry(bpb, device, cluster)?;
        if is_free_cluster(entry) {
            write_fat_entry(bpb, device, cluster, FAT32_CLUSTER_EOC)?;

            let start_sec = bpb.cluster_to_sector(cluster)?;
            let zero_sec = [0u8; FAT32_SECTOR_SIZE];
            for s in 0..(bpb.sectors_per_cluster as u64) {
                device.write_sector(start_sec + s, &zero_sec)?;
            }
            return Ok(cluster);
        }
    }
    Err(FsError::NoSpace)
}

/// Appends a newly allocated cluster to the end of an existing cluster chain.
pub fn append_cluster<D: BlockDevice + ?Sized>(
    bpb: &BiosParameterBlock32,
    device: &D,
    last_cluster: u32,
) -> Result<u32, FsError> {
    let new_cluster = allocate_cluster(bpb, device)?;
    write_fat_entry(bpb, device, last_cluster, new_cluster)?;
    Ok(new_cluster)
}

/// Frees all clusters belonging to a cluster chain starting at `start_cluster`.
pub fn free_cluster_chain<D: BlockDevice + ?Sized>(
    bpb: &BiosParameterBlock32,
    device: &D,
    start_cluster: u32,
) -> Result<(), FsError> {
    let chain = traverse_cluster_chain(bpb, device, start_cluster)?;
    for &cluster in &chain {
        write_fat_entry(bpb, device, cluster, FAT32_CLUSTER_FREE)?;
    }
    Ok(())
}

/// Decodes an 11-byte short 8.3 filename into a clean UTF-8 string.
pub fn format_8_3_name(raw: &[u8; 11]) -> String {
    if raw == b".          " {
        return String::from(".");
    }
    if raw == b"..         " {
        return String::from("..");
    }

    let name_part = &raw[0..8];
    let ext_part = &raw[8..11];

    let name_len = name_part.iter().rposition(|&b| b != b' ').map(|p| p + 1).unwrap_or(0);
    let ext_len = ext_part.iter().rposition(|&b| b != b' ').map(|p| p + 1).unwrap_or(0);

    let mut result = String::new();
    for &b in &name_part[..name_len] {
        result.push((b as char).to_ascii_lowercase());
    }

    if ext_len > 0 {
        result.push('.');
        for &b in &ext_part[..ext_len] {
            result.push((b as char).to_ascii_lowercase());
        }
    }

    result
}

/// Encodes a standard filename into an 11-byte short 8.3 representation.
pub fn encode_8_3_name(name: &str) -> [u8; 11] {
    let mut out = [b' '; 11];
    if name == "." {
        out[0] = b'.';
        return out;
    }
    if name == ".." {
        out[0] = b'.';
        out[1] = b'.';
        return out;
    }

    let parts: Vec<&str> = name.split('.').collect();
    let base = parts[0];
    let ext = if parts.len() > 1 { parts[1] } else { "" };

    let base_bytes = base.as_bytes();
    let base_len = core::cmp::min(base_bytes.len(), 8);
    for i in 0..base_len {
        out[i] = base_bytes[i].to_ascii_uppercase();
    }

    let ext_bytes = ext.as_bytes();
    let ext_len = core::cmp::min(ext_bytes.len(), 3);
    for i in 0..ext_len {
        out[8 + i] = ext_bytes[i].to_ascii_uppercase();
    }

    out
}

/// 32-byte FAT directory entry.
#[derive(Debug, Clone)]
pub struct Fat32DirEntry {
    pub name: String,
    pub raw_name: [u8; 11],
    pub attributes: u8,
    pub first_cluster: u32,
    pub file_size: u32,
}

impl Fat32DirEntry {
    /// Parse a 32-byte directory entry record.
    pub fn parse(raw: &[u8]) -> Result<Option<Self>, FsError> {
        if raw.len() < 32 {
            return Err(FsError::InvalidArgument);
        }
        // 0x00 indicates end of directory entries
        if raw[0] == 0x00 {
            return Ok(None);
        }
        // 0xE5 indicates free/deleted entry
        if raw[0] == 0xE5 {
            return Ok(None);
        }
        let attributes = raw[11];
        // Skip Long File Name (LFN) entries and Volume ID entries
        if attributes == ATTR_LONG_NAME || (attributes & ATTR_VOLUME_ID != 0) {
            return Ok(None);
        }

        let mut raw_name = [0u8; 11];
        raw_name.copy_from_slice(&raw[0..11]);
        let name = format_8_3_name(&raw_name);

        let first_cluster_hi = u16::from_le_bytes([raw[20], raw[21]]);
        let first_cluster_lo = u16::from_le_bytes([raw[26], raw[27]]);
        let first_cluster = ((first_cluster_hi as u32) << 16) | (first_cluster_lo as u32);
        let file_size = u32::from_le_bytes([raw[28], raw[29], raw[30], raw[31]]);

        Ok(Some(Self {
            name,
            raw_name,
            attributes,
            first_cluster,
            file_size,
        }))
    }

    /// Serializes this directory entry into a 32-byte binary record.
    pub fn serialize(&self) -> [u8; 32] {
        let mut out = [0u8; 32];
        out[0..11].copy_from_slice(&self.raw_name);
        out[11] = self.attributes;
        let hi = (self.first_cluster >> 16) as u16;
        let lo = (self.first_cluster & 0xFFFF) as u16;
        out[20..22].copy_from_slice(&hi.to_le_bytes());
        out[26..28].copy_from_slice(&lo.to_le_bytes());
        out[28..32].copy_from_slice(&self.file_size.to_le_bytes());
        out
    }

    pub fn is_directory(&self) -> bool {
        self.attributes & ATTR_DIRECTORY != 0
    }

    pub fn is_readonly(&self) -> bool {
        self.attributes & ATTR_READ_ONLY != 0
    }
}

/// Concrete Inode representation for FAT32 files and directories.
pub struct Fat32Inode {
    pub bpb: BiosParameterBlock32,
    pub device: Arc<dyn BlockDevice>,
    pub name: String,
    pub inode_id: u64,
    pub file_type: FileType,
    pub file_size: u64,
    pub first_cluster: u32,
    pub is_readonly: bool,
}

impl Fat32Inode {
    pub fn name(&self) -> &str {
        &self.name
    }
    fn read_dir_entries(&self) -> Result<Vec<Fat32DirEntry>, FsError> {
        let raw_data = read_cluster_chain_data(&self.bpb, self.device.as_ref(), self.first_cluster)?;
        let mut entries = Vec::new();

        for chunk in raw_data.chunks_exact(32) {
            if chunk[0] == 0x00 {
                break;
            }
            if let Some(entry) = Fat32DirEntry::parse(chunk)? {
                entries.push(entry);
            }
        }

        Ok(entries)
    }
}

impl Inode for Fat32Inode {
    fn metadata(&self) -> VfsMetadata {
        VfsMetadata {
            file_type: self.file_type,
            size: self.file_size,
            permissions: if self.is_readonly {
                FilePerms::readonly()
            } else {
                FilePerms::rwx()
            },
            inode_id: self.inode_id,
        }
    }

    fn read_at(&self, offset: u64, buf: &mut [u8]) -> Result<usize, FsError> {
        if self.file_type == FileType::Directory {
            return Err(FsError::IsADirectory);
        }
        if offset >= self.file_size || self.file_size == 0 || self.first_cluster < 2 {
            return Ok(0);
        }
        let data = read_cluster_chain_data(&self.bpb, self.device.as_ref(), self.first_cluster)?;
        let offset = offset as usize;
        if offset >= data.len() {
            return Ok(0);
        }
        let available = (self.file_size as usize).saturating_sub(offset);
        let to_read = core::cmp::min(available, buf.len());
        let actual_read = core::cmp::min(to_read, data.len().saturating_sub(offset));
        buf[..actual_read].copy_from_slice(&data[offset..offset + actual_read]);
        Ok(actual_read)
    }

    fn write_at(&self, offset: u64, buf: &[u8]) -> Result<usize, FsError> {
        if self.file_type == FileType::Directory {
            return Err(FsError::IsADirectory);
        }
        if self.is_readonly {
            return Err(FsError::PermissionDenied);
        }
        if self.first_cluster < 2 || buf.is_empty() {
            return Ok(0);
        }

        let chain = traverse_cluster_chain(&self.bpb, self.device.as_ref(), self.first_cluster)?;
        let cluster_size = self.bpb.bytes_per_cluster() as usize;
        let sec_size = self.bpb.bytes_per_sector as usize;

        let mut bytes_written = 0;
        for (i, &cluster) in chain.iter().enumerate() {
            let cluster_start = i * cluster_size;
            let cluster_end = cluster_start + cluster_size;

            if (offset as usize) < cluster_end && (offset as usize) + buf.len() > cluster_start {
                let start_sec = self.bpb.cluster_to_sector(cluster)?;
                for s in 0..(self.bpb.sectors_per_cluster as u64) {
                    let sec_start = cluster_start + (s as usize) * sec_size;
                    let sec_end = sec_start + sec_size;

                    let write_start = (offset as usize).max(sec_start);
                    let write_end = ((offset as usize) + buf.len()).min(sec_end);

                    if write_start < write_end {
                        let mut sec_buf = [0u8; FAT32_SECTOR_SIZE];
                        self.device.read_sector(start_sec + s, &mut sec_buf)?;

                        let buf_offset = write_start - (offset as usize);
                        let sec_offset = write_start - sec_start;
                        let chunk_len = write_end - write_start;

                        sec_buf[sec_offset..sec_offset + chunk_len]
                            .copy_from_slice(&buf[buf_offset..buf_offset + chunk_len]);
                        self.device.write_sector(start_sec + s, &sec_buf)?;
                        bytes_written += chunk_len;
                    }
                }
            }
        }
        Ok(bytes_written)
    }

    fn lookup(&self, name: &str) -> Result<Arc<dyn Inode>, FsError> {
        if self.file_type != FileType::Directory {
            return Err(FsError::NotADirectory);
        }

        let entries = self.read_dir_entries()?;
        for entry in entries {
            if entry.name.eq_ignore_ascii_case(name) {
                let file_type = if entry.is_directory() {
                    FileType::Directory
                } else {
                    FileType::File
                };
                let is_readonly = entry.is_readonly();
                let inode_id = if entry.first_cluster != 0 {
                    entry.first_cluster as u64
                } else {
                    0x10000
                };
                return Ok(Arc::new(Fat32Inode {
                    bpb: self.bpb,
                    device: Arc::clone(&self.device),
                    name: entry.name,
                    inode_id,
                    file_type,
                    file_size: entry.file_size as u64,
                    first_cluster: entry.first_cluster,
                    is_readonly,
                }));
            }
        }

        Err(FsError::NotFound)
    }

    fn readdir(&self) -> Result<Vec<DirEntry>, FsError> {
        if self.file_type != FileType::Directory {
            return Err(FsError::NotADirectory);
        }

        let entries = self.read_dir_entries()?;
        let mut list = Vec::new();
        for entry in entries {
            if entry.name == "." || entry.name == ".." {
                continue;
            }
            let file_type = if entry.is_directory() {
                FileType::Directory
            } else {
                FileType::File
            };
            list.push(DirEntry {
                name: entry.name,
                inode_id: if entry.first_cluster != 0 {
                    entry.first_cluster as u64
                } else {
                    0x10000
                },
                file_type,
            });
        }

        Ok(list)
    }
}

/// FAT32 Filesystem instance implementing the VFS `FileSystem` trait.
pub struct Fat32FileSystem {
    bpb: BiosParameterBlock32,
    device: Arc<dyn BlockDevice>,
}

impl Fat32FileSystem {
    /// Mount a FAT32 filesystem from a block storage device.
    pub fn new(device: Arc<dyn BlockDevice>) -> Result<Self, FsError> {
        let mut boot_sector = [0u8; FAT32_SECTOR_SIZE];
        device.read_sector(0, &mut boot_sector)?;
        let bpb = BiosParameterBlock32::parse(&boot_sector)?;
        Ok(Self { bpb, device })
    }

    pub fn bpb(&self) -> &BiosParameterBlock32 {
        &self.bpb
    }

    pub fn device(&self) -> &Arc<dyn BlockDevice> {
        &self.device
    }
}

impl FileSystem for Fat32FileSystem {
    fn name(&self) -> &'static str {
        "fat32"
    }

    fn root(&self) -> Result<Arc<dyn Inode>, FsError> {
        Ok(Arc::new(Fat32Inode {
            bpb: self.bpb,
            device: Arc::clone(&self.device),
            name: String::from("/"),
            inode_id: self.bpb.root_cluster as u64,
            file_type: FileType::Directory,
            file_size: 0,
            first_cluster: self.bpb.root_cluster,
            is_readonly: false,
        }))
    }
}

/// Constructs an in-memory synthetic FAT32 filesystem image formatted with
/// root directory, cluster chains, and test files (`/readme.txt` and `/docs/kernel.log`).
pub fn build_synthetic_fat32_image() -> Vec<u8> {
    let total_sectors = 128;
    let mut image = alloc::vec![0u8; total_sectors * FAT32_SECTOR_SIZE];

    // --- Sector 0: Boot Sector / BPB ---
    let mut boot = [0u8; FAT32_SECTOR_SIZE];
    boot[0] = 0xEB;
    boot[1] = 0x58;
    boot[2] = 0x90;
    boot[3..11].copy_from_slice(b"MSWIN4.1");
    boot[11..13].copy_from_slice(&512u16.to_le_bytes());
    boot[13] = 1; // 1 sector per cluster (512 bytes)
    boot[14..16].copy_from_slice(&32u16.to_le_bytes()); // 32 reserved sectors
    boot[16] = 2; // 2 FAT tables
    boot[21] = 0xF8; // fixed disk
    boot[32..36].copy_from_slice(&(total_sectors as u32).to_le_bytes());
    boot[36..40].copy_from_slice(&32u32.to_le_bytes()); // 32 sectors per FAT
    boot[44..48].copy_from_slice(&2u32.to_le_bytes()); // Root dir cluster 2
    boot[48..50].copy_from_slice(&1u16.to_le_bytes());
    boot[50..52].copy_from_slice(&6u16.to_le_bytes());
    boot[64] = 0x80;
    boot[66] = 0x29;
    boot[67..71].copy_from_slice(&0x12345678u32.to_le_bytes());
    boot[71..82].copy_from_slice(b"KNTON_FAT32");
    boot[82..90].copy_from_slice(b"FAT32   ");
    boot[510] = 0x55;
    boot[511] = 0xAA;

    image[0..512].copy_from_slice(&boot);

    // --- FAT1 & FAT2 Tables (Sectors 32..63 and 64..95) ---
    let write_entry = |img: &mut [u8], cluster: usize, val: u32| {
        let fat1_offset = 32 * FAT32_SECTOR_SIZE + cluster * 4;
        let fat2_offset = 64 * FAT32_SECTOR_SIZE + cluster * 4;
        img[fat1_offset..fat1_offset + 4].copy_from_slice(&val.to_le_bytes());
        img[fat2_offset..fat2_offset + 4].copy_from_slice(&val.to_le_bytes());
    };

    write_entry(&mut image, 0, 0x0FFF_FFF8); // Media indicator
    write_entry(&mut image, 1, 0x0FFF_FFFF); // EOC marker
    write_entry(&mut image, 2, 0x0FFF_FFFF); // Root Directory (cluster 2 EOC)
    write_entry(&mut image, 3, 4);           // Cluster 3 -> Cluster 4 (README.TXT)
    write_entry(&mut image, 4, 0x0FFF_FFFF); // Cluster 4 (README.TXT EOC)
    write_entry(&mut image, 5, 0x0FFF_FFFF); // Cluster 5 (DOCS dir EOC)
    write_entry(&mut image, 6, 0x0FFF_FFFF); // Cluster 6 (KERNEL.LOG EOC)

    // Data region starts at sector 96 (cluster 2)
    let root_sec_offset = 96 * FAT32_SECTOR_SIZE;

    // Entry 1: README.TXT (cluster 3, 545 bytes)
    let readme_entry = Fat32DirEntry {
        name: String::from("readme.txt"),
        raw_name: encode_8_3_name("README.TXT"),
        attributes: ATTR_ARCHIVE,
        first_cluster: 3,
        file_size: 544,
    };
    image[root_sec_offset..root_sec_offset + 32].copy_from_slice(&readme_entry.serialize());

    // Entry 2: DOCS (cluster 5, directory)
    let docs_entry = Fat32DirEntry {
        name: String::from("docs"),
        raw_name: encode_8_3_name("DOCS"),
        attributes: ATTR_DIRECTORY,
        first_cluster: 5,
        file_size: 0,
    };
    image[root_sec_offset + 32..root_sec_offset + 64].copy_from_slice(&docs_entry.serialize());

    // --- Cluster 3 (Sector 97): README.TXT first 512 bytes ---
    let c3_offset = 97 * FAT32_SECTOR_SIZE;
    let header_msg = b"KntonOS FAT32 Persistent File System Driver.\nSpecification: Microsoft FAT32 Extended BPB Architecture.\nCluster Chaining & 512-Byte VirtIO Block Integration Active.\n";
    image[c3_offset..c3_offset + header_msg.len()].copy_from_slice(header_msg);
    for i in header_msg.len()..512 {
        image[c3_offset + i] = b'=';
    }

    // --- Cluster 4 (Sector 98): README.TXT remaining 33 bytes ---
    let c4_offset = 98 * FAT32_SECTOR_SIZE;
    let tail_msg = b" Successfully mounted on /disk.\n";
    image[c4_offset..c4_offset + tail_msg.len()].copy_from_slice(tail_msg);

    // --- Cluster 5 (Sector 99): DOCS Subdirectory Entries ---
    let c5_offset = 99 * FAT32_SECTOR_SIZE;
    let dot_entry = Fat32DirEntry {
        name: String::from("."),
        raw_name: encode_8_3_name("."),
        attributes: ATTR_DIRECTORY,
        first_cluster: 5,
        file_size: 0,
    };
    image[c5_offset..c5_offset + 32].copy_from_slice(&dot_entry.serialize());

    let dotdot_entry = Fat32DirEntry {
        name: String::from(".."),
        raw_name: encode_8_3_name(".."),
        attributes: ATTR_DIRECTORY,
        first_cluster: 0,
        file_size: 0,
    };
    image[c5_offset + 32..c5_offset + 64].copy_from_slice(&dotdot_entry.serialize());

    let log_msg = b"KntonOS kernel boot log ok\n";
    let log_entry = Fat32DirEntry {
        name: String::from("kernel.log"),
        raw_name: encode_8_3_name("KERNEL.LOG"),
        attributes: ATTR_ARCHIVE,
        first_cluster: 6,
        file_size: log_msg.len() as u32,
    };
    image[c5_offset + 64..c5_offset + 96].copy_from_slice(&log_entry.serialize());

    // --- Cluster 6 (Sector 100): KERNEL.LOG data ---
    let c6_offset = 100 * FAT32_SECTOR_SIZE;
    image[c6_offset..c6_offset + log_msg.len()].copy_from_slice(log_msg);

    image
}

/// Comprehensive self-test suite for FAT32 BPB parser, cluster chain traversal,
/// directory hierarchy parsing, and VirtIO block mount integration.
pub fn test_fat32_subsystem() -> bool {
    crate::serial_write_str("[KNT-FS-TEST] Testing FAT32 filesystem driver & VirtIO block mount...\r\n");

    let synth_img = build_synthetic_fat32_image();

    // 1. In-memory block device verification
    let mem_dev = Arc::new(MemoryBlockDevice::new(synth_img.clone()));
    let fs = match Fat32FileSystem::new(mem_dev.clone()) {
        Ok(f) => Arc::new(f),
        Err(_) => {
            crate::serial_write_str("[KNT-PANIC] Failed to parse synthetic FAT32 BPB!\r\n");
            return false;
        }
    };

    let bpb = fs.bpb();
    if bpb.bytes_per_sector != 512 || bpb.sectors_per_cluster != 1 || bpb.first_data_sector() != 96 {
        crate::serial_write_str("[KNT-PANIC] FAT32 BPB geometry verification failed!\r\n");
        return false;
    }

    // 2. Verify cluster chain traversal: cluster 3 -> [3, 4]
    match traverse_cluster_chain(bpb, mem_dev.as_ref(), 3) {
        Ok(chain) => {
            if chain != alloc::vec![3, 4] {
                crate::serial_write_str("[KNT-PANIC] FAT32 cluster chain traversal mismatch!\r\n");
                return false;
            }
        }
        Err(_) => {
            crate::serial_write_str("[KNT-PANIC] FAT32 cluster chain traversal failed!\r\n");
            return false;
        }
    }

    // 3. Mount synthetic FAT32 image directly to VirtIO block driver storage
    {
        let mut blk_lock = crate::drivers::virtio::BLOCK_DEVICE.lock();
        if let Some(blk) = blk_lock.as_mut() {
            for s in 0..128 {
                let offset = s * FAT32_SECTOR_SIZE;
                if offset + FAT32_SECTOR_SIZE <= synth_img.len() {
                    let _ = blk.write_sectors(s as u64, &synth_img[offset..offset + FAT32_SECTOR_SIZE]);
                }
            }
        }
    }

    // 4. Mount VirtIO block device onto VFS at "/disk"
    let virtio_dev = Arc::new(VirtioBlockDeviceAdapter);
    let virtio_fs = match Fat32FileSystem::new(virtio_dev) {
        Ok(vfs_fs) => Arc::new(vfs_fs),
        Err(_) => {
            crate::serial_write_str("[KNT-PANIC] Failed to mount FAT32 from VirtIO block device!\r\n");
            return false;
        }
    };

    crate::fs::VFS.lock().mount("/disk", virtio_fs);
    crate::serial_write_str("  * VirtIO Block FAT32 volume mounted at /disk\r\n");

    let vfs = crate::fs::VFS.lock();

    // 5. Verify reading "/disk/readme.txt"
    match vfs.read_file("/disk/readme.txt") {
        Ok(data) => {
            if !data.starts_with(b"KntonOS FAT32 Persistent File System Driver.") {
                crate::serial_write_str("[KNT-PANIC] /disk/readme.txt header content mismatch!\r\n");
                return false;
            }
            if !data.ends_with(b"Successfully mounted on /disk.\n") {
                crate::serial_write_str("[KNT-PANIC] /disk/readme.txt multi-cluster tail mismatch!\r\n");
                return false;
            }
            if data.len() != 544 {
                crate::serial_write_str("[KNT-PANIC] /disk/readme.txt size mismatch!\r\n");
                return false;
            }
        }
        Err(_) => {
            crate::serial_write_str("[KNT-PANIC] Failed to open /disk/readme.txt through VFS!\r\n");
            return false;
        }
    }

    // 6. Verify reading nested file "/disk/docs/kernel.log"
    match vfs.read_file("/disk/docs/kernel.log") {
        Ok(data) => {
            if data != b"KntonOS kernel boot log ok\n" {
                crate::serial_write_str("[KNT-PANIC] /disk/docs/kernel.log content mismatch!\r\n");
                return false;
            }
        }
        Err(_) => {
            crate::serial_write_str("[KNT-PANIC] Failed to open /disk/docs/kernel.log through VFS!\r\n");
            return false;
        }
    }

    // 7. Verify directory listing on root of "/disk"
    match vfs.open("/disk") {
        Ok(disk_root) => match disk_root.readdir() {
            Ok(entries) => {
                let has_readme = entries.iter().any(|e| e.name == "readme.txt");
                let has_docs = entries.iter().any(|e| e.name == "docs");
                if !has_readme || !has_docs {
                    crate::serial_write_str("[KNT-PANIC] /disk directory listing missing expected entries!\r\n");
                    return false;
                }
            }
            Err(_) => {
                crate::serial_write_str("[KNT-PANIC] Failed to readdir on /disk!\r\n");
                return false;
            }
        },
        Err(_) => {
            crate::serial_write_str("[KNT-PANIC] Failed to open /disk root inode!\r\n");
            return false;
        }
    }

    // 8. Verify write_at transaction on "/disk/docs/kernel.log"
    match vfs.open("/disk/docs/kernel.log") {
        Ok(log_inode) => {
            let update_text = b"UPDATE: VirtIO OK\n";
            match log_inode.write_at(0, update_text) {
                Ok(written) => {
                    if written != update_text.len() {
                        crate::serial_write_str("[KNT-PANIC] Partial write on /disk/docs/kernel.log!\r\n");
                        return false;
                    }
                }
                Err(_) => {
                    crate::serial_write_str("[KNT-PANIC] Failed to write to /disk/docs/kernel.log!\r\n");
                    return false;
                }
            }
        }
        Err(_) => {
            crate::serial_write_str("[KNT-PANIC] Failed to open /disk/docs/kernel.log for writing!\r\n");
            return false;
        }
    }

    // 9. Re-read and confirm write persistence
    match vfs.read_file("/disk/docs/kernel.log") {
        Ok(data) => {
            if !data.starts_with(b"UPDATE: VirtIO OK\n") {
                crate::serial_write_str("[KNT-PANIC] Persistence verification failed on /disk/docs/kernel.log!\r\n");
                return false;
            }
        }
        Err(_) => {
            crate::serial_write_str("[KNT-PANIC] Failed to re-read /disk/docs/kernel.log after write!\r\n");
            return false;
        }
    }

    // 10. Print required test sentinel
    crate::serial_write_str("[KNT-FS-FAT32-PASS] FAT32 cluster traversal, directory parsing, and VirtIO /disk mount verified\r\n");
    true
}
