//! FAT32 Persistent File System Driver for KntonOS
//!
//! Implements Microsoft FAT32 specification volume geometry, BIOS Parameter Block (BPB)
//! parsing, cluster-to-sector address resolution, cluster chain traversal,
//! and free cluster allocation.

use alloc::vec::Vec;
use crate::fs::vfs::FsError;

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
    ///
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
            // Cycle detected or chain length exceeded volume capacity
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
