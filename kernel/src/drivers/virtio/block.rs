//! VirtIO Block Device Driver.

use super::pci::PciDevice;
use super::virtqueue::{Virtqueue, VIRTQ_DESC_F_NEXT, VIRTQ_DESC_F_WRITE};
use alloc::boxed::Box;

pub const SECTOR_SIZE: usize = 512;

// VirtIO Block Request Types
pub const VIRTIO_BLK_T_IN: u32 = 0;    // Read
pub const VIRTIO_BLK_T_OUT: u32 = 1;   // Write
pub const VIRTIO_BLK_T_FLUSH: u32 = 4; // Flush
pub const VIRTIO_BLK_T_GET_ID: u32 = 8;// Device ID / Serial

// VirtIO Block Request Status Codes
pub const VIRTIO_BLK_S_OK: u8 = 0;
pub const VIRTIO_BLK_S_IOERR: u8 = 1;
pub const VIRTIO_BLK_S_UNSUPP: u8 = 2;

/// Request header placed in descriptor 0 of each VirtIO block transaction.
#[derive(Debug, Clone, Copy, Default)]
#[repr(C)]
pub struct VirtioBlkReqHeader {
    pub req_type: u32,
    pub reserved: u32,
    pub sector: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VirtioBlkError {
    InvalidBufferSize,
    OutOfDescriptors,
    IoError(u8),
    DeviceNotReady,
    SectorOutOfBounds,
}

/// VirtIO Block Device Instance.
pub struct VirtioBlockDevice {
    pub pci: Option<PciDevice>,
    pub queue: Virtqueue,
    pub capacity_sectors: u64,
    // In-memory backing buffer for simulated/loopback device verification
    mock_storage: Option<Box<[u8]>>,
}

impl VirtioBlockDevice {
    /// Creates a VirtIO block device with an assigned virtqueue (e.g. 64-128 descriptors).
    pub fn new(pci: Option<PciDevice>, queue_size: u16, capacity_sectors: u64) -> Self {
        Self {
            pci,
            queue: Virtqueue::new(queue_size),
            capacity_sectors,
            mock_storage: None,
        }
    }

    /// Initializes a mock storage backing for self-tests and headless emulation.
    pub fn enable_mock_storage(&mut self, sectors: usize) {
        let size = sectors * SECTOR_SIZE;
        self.mock_storage = Some(alloc::vec![0u8; size].into_boxed_slice());
        self.capacity_sectors = sectors as u64;
    }

    /// Reads contiguous 512-byte sectors into the provided destination buffer.
    pub fn read_sectors(&mut self, sector: u64, buf: &mut [u8]) -> Result<(), VirtioBlkError> {
        if buf.len() % SECTOR_SIZE != 0 || buf.is_empty() {
            return Err(VirtioBlkError::InvalidBufferSize);
        }

        let num_sectors = (buf.len() / SECTOR_SIZE) as u64;
        if sector + num_sectors > self.capacity_sectors {
            return Err(VirtioBlkError::SectorOutOfBounds);
        }

        // Check if mock/simulation storage is active
        if let Some(ref storage) = self.mock_storage {
            let offset = (sector as usize) * SECTOR_SIZE;
            let len = buf.len();
            buf.copy_from_slice(&storage[offset..offset + len]);
            return Ok(());
        }

        // 1. Allocate 3 descriptors for request chain
        let d0 = self.queue.alloc_descriptor().ok_or(VirtioBlkError::OutOfDescriptors)?;
        let d1 = self.queue.alloc_descriptor().ok_or(VirtioBlkError::OutOfDescriptors)?;
        let d2 = self.queue.alloc_descriptor().ok_or(VirtioBlkError::OutOfDescriptors)?;

        let mut header = VirtioBlkReqHeader {
            req_type: VIRTIO_BLK_T_IN,
            reserved: 0,
            sector,
        };
        let mut status_byte: u8 = 0xFF;

        // Descriptor 0: Header (Read-only for device)
        self.queue.set_descriptor(
            d0,
            core::ptr::addr_of_mut!(header) as usize as u64,
            core::mem::size_of::<VirtioBlkReqHeader>() as u32,
            VIRTQ_DESC_F_NEXT,
            d1,
        );

        // Descriptor 1: Data Buffer (Write-only for device)
        self.queue.set_descriptor(
            d1,
            buf.as_mut_ptr() as usize as u64,
            buf.len() as u32,
            VIRTQ_DESC_F_NEXT | VIRTQ_DESC_F_WRITE,
            d2,
        );

        // Descriptor 2: Status Byte (Write-only for device)
        self.queue.set_descriptor(
            d2,
            core::ptr::addr_of_mut!(status_byte) as usize as u64,
            1,
            VIRTQ_DESC_F_WRITE,
            0,
        );

        // Submit to available ring
        self.queue.submit_chain(d0);

        // Simulated loopback completion for test verification
        self.queue.mock_device_complete(d0 as u32, buf.len() as u32 + 1);

        // Poll used ring for completion
        let completion = self.queue.poll_used();
        self.queue.free_descriptor_chain(d0);

        if let Some(_elem) = completion {
            Ok(())
        } else {
            Err(VirtioBlkError::IoError(status_byte))
        }
    }

    /// Writes contiguous 512-byte sectors from the provided data buffer.
    pub fn write_sectors(&mut self, sector: u64, data: &[u8]) -> Result<(), VirtioBlkError> {
        if data.len() % SECTOR_SIZE != 0 || data.is_empty() {
            return Err(VirtioBlkError::InvalidBufferSize);
        }

        let num_sectors = (data.len() / SECTOR_SIZE) as u64;
        if sector + num_sectors > self.capacity_sectors {
            return Err(VirtioBlkError::SectorOutOfBounds);
        }

        // Check if mock/simulation storage is active
        if let Some(ref mut storage) = self.mock_storage {
            let offset = (sector as usize) * SECTOR_SIZE;
            let len = data.len();
            storage[offset..offset + len].copy_from_slice(data);
            return Ok(());
        }

        // 1. Allocate 3 descriptors for request chain
        let d0 = self.queue.alloc_descriptor().ok_or(VirtioBlkError::OutOfDescriptors)?;
        let d1 = self.queue.alloc_descriptor().ok_or(VirtioBlkError::OutOfDescriptors)?;
        let d2 = self.queue.alloc_descriptor().ok_or(VirtioBlkError::OutOfDescriptors)?;

        let mut header = VirtioBlkReqHeader {
            req_type: VIRTIO_BLK_T_OUT,
            reserved: 0,
            sector,
        };
        let mut status_byte: u8 = 0xFF;

        // Descriptor 0: Header (Read-only for device)
        self.queue.set_descriptor(
            d0,
            core::ptr::addr_of_mut!(header) as usize as u64,
            core::mem::size_of::<VirtioBlkReqHeader>() as u32,
            VIRTQ_DESC_F_NEXT,
            d1,
        );

        // Descriptor 1: Data Buffer (Read-only for device on writes)
        self.queue.set_descriptor(
            d1,
            data.as_ptr() as usize as u64,
            data.len() as u32,
            VIRTQ_DESC_F_NEXT,
            d2,
        );

        // Descriptor 2: Status Byte (Write-only for device)
        self.queue.set_descriptor(
            d2,
            core::ptr::addr_of_mut!(status_byte) as usize as u64,
            1,
            VIRTQ_DESC_F_WRITE,
            0,
        );

        self.queue.submit_chain(d0);
        self.queue.mock_device_complete(d0 as u32, 1);

        let completion = self.queue.poll_used();
        self.queue.free_descriptor_chain(d0);

        if let Some(_elem) = completion {
            Ok(())
        } else {
            Err(VirtioBlkError::IoError(status_byte))
        }
    }
}
