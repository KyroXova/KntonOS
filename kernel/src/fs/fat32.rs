//! FAT32 Persistent File System Driver for KntonOS
//!
//! Implements Microsoft FAT32 specification volume geometry, BIOS Parameter Block (BPB)
//! parsing, cluster-to-sector address resolution, and volume statistics.

use crate::fs::vfs::FsError;

/// Standard FAT boot sector signature (0x55, 0xAA at offsets 510..511).
pub const FAT_BOOT_SIGNATURE: u16 = 0xAA55;

/// Sector size expected for standard FAT32 storage devices.
pub const FAT32_SECTOR_SIZE: usize = 512;

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
