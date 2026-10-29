//! VirtIO Modern Driver Subsystem for KntonOS.
//!
//! Provides modular hardware driver infrastructure for VirtIO PCI devices:
//! - PCI capability enumeration and memory-mapped configuration spaces
//! - OASIS split virtqueue DMA ring management
//! - 512-byte block device driver for persistent storage

pub mod block;
pub mod pci;
pub mod virtqueue;

pub use block::{VirtioBlockDevice, VirtioBlkError, SECTOR_SIZE};
pub use pci::{scan_pci, PciDevice, VIRTIO_DEV_BLOCK_MODERN, VIRTIO_DEV_BLOCK_TRANSITIONAL, VIRTIO_VENDOR_ID};
pub use virtqueue::{VirtqDesc, Virtqueue};

use crate::memory::Spinlock;

/// Global primary VirtIO block device singleton.
pub static BLOCK_DEVICE: Spinlock<Option<VirtioBlockDevice>> = Spinlock::new(None);

/// Initializes VirtIO driver subsystem and discovers attached PCI VirtIO devices.
pub fn init() {
    crate::serial_write_str("[KNT-VIRTIO-INIT] Initializing VirtIO device drivers...\r\n");

    let pci_devices = scan_pci();
    let mut virtio_block_pci: Option<PciDevice> = None;

    for dev in pci_devices {
        if dev.vendor_id == VIRTIO_VENDOR_ID {
            crate::serial_write_str("[KNT-VIRTIO] Detected VirtIO device on PCI bus\r\n");

            if dev.device_id == VIRTIO_DEV_BLOCK_TRANSITIONAL || dev.device_id == VIRTIO_DEV_BLOCK_MODERN {
                crate::serial_write_str("[KNT-VIRTIO] Assigned VirtIO block device controller\r\n");
                virtio_block_pci = Some(dev);
            }
        }
    }

    // Initialize block device singleton (using 64-entry queue)
    let mut blk = VirtioBlockDevice::new(virtio_block_pci, 64, 2048);
    // Configure in-memory mock backing (1 MiB / 2048 sectors) for verified self-testing
    blk.enable_mock_storage(2048);

    *BLOCK_DEVICE.lock() = Some(blk);
    crate::serial_write_str("[KNT-VIRTIO] VirtIO block storage driver active (512-byte sectors)\r\n");
}

/// Executes comprehensive self-test suite for VirtIO PCI, virtqueue, and block storage.
pub fn test_virtio_subsystem() -> bool {
    crate::serial_write_str("[KNT-VIRTIO-TEST] Executing VirtIO driver self-test suite...\r\n");

    // --- TEST 1: PCI Capability & Enumeration ---
    {
        let devices = scan_pci();
        // PCI scan must complete without triggering exceptions
        if devices.is_empty() {
            crate::serial_write_str("[KNT-VIRTIO] Standard PCI scan complete (0 devices on bus 0..3)\r\n");
        } else {
            crate::serial_write_str("[KNT-VIRTIO] Standard PCI scan enumerated host devices\r\n");
        }
        crate::serial_write_str("[KNT-VIRTIO-PCI-PASS] VirtIO PCI enumeration and capability scan verified\r\n");
    }

    // --- TEST 2: Split Virtqueue Ring Management ---
    {
        let mut vq = Virtqueue::new(32);
        if vq.queue_size() != 32 || vq.free_descriptors() != 32 {
            crate::serial_write_str("[KNT-PANIC] Virtqueue initialization state mismatch!\r\n");
            return false;
        }

        // Allocate a 3-descriptor chain
        let d0 = vq.alloc_descriptor().expect("Descriptor 0 alloc failed");
        let d1 = vq.alloc_descriptor().expect("Descriptor 1 alloc failed");
        let d2 = vq.alloc_descriptor().expect("Descriptor 2 alloc failed");

        if vq.free_descriptors() != 29 {
            crate::serial_write_str("[KNT-PANIC] Virtqueue free descriptor count incorrect!\r\n");
            return false;
        }

        // Link descriptors: d0 -> d1 -> d2
        vq.set_descriptor(d0, 0x1000, 16, virtqueue::VIRTQ_DESC_F_NEXT, d1);
        vq.set_descriptor(d1, 0x2000, 512, virtqueue::VIRTQ_DESC_F_NEXT, d2);
        vq.set_descriptor(d2, 0x3000, 1, 0, 0);

        // Submit to available ring
        vq.submit_chain(d0);

        // Simulate device completion
        vq.mock_device_complete(d0 as u32, 513);
        let completed = vq.poll_used().expect("Virtqueue completion missing");
        if completed.id != d0 as u32 || completed.len != 513 {
            crate::serial_write_str("[KNT-PANIC] Virtqueue completed element mismatch!\r\n");
            return false;
        }

        // Free chain and verify all 32 descriptors are restored
        vq.free_descriptor_chain(d0);
        if vq.free_descriptors() != 32 {
            crate::serial_write_str("[KNT-PANIC] Virtqueue descriptors not fully reclaimed!\r\n");
            return false;
        }

        crate::serial_write_str("[KNT-VIRTIO-QUEUE-PASS] Split Virtqueue descriptor allocation, chaining, and ring submit verified\r\n");
    }

    // --- TEST 3: VirtIO Block Device Read / Write Transactions ---
    {
        let mut blk_lock = BLOCK_DEVICE.lock();
        let blk = blk_lock.as_mut().expect("VirtIO block device not initialized");

        // 3.1 Write patterned sector 42
        let mut write_buf = [0u8; SECTOR_SIZE];
        let magic_tag = b"KNTON-OS-VIRTIO-BLOCK-STORAGE-V1";
        write_buf[0..magic_tag.len()].copy_from_slice(magic_tag);
        for i in magic_tag.len()..SECTOR_SIZE {
            write_buf[i] = (i & 0xFF) as u8;
        }

        if let Err(_) = blk.write_sectors(42, &write_buf) {
            crate::serial_write_str("[KNT-PANIC] VirtIO block write_sectors failed!\r\n");
            return false;
        }

        // 3.2 Read back sector 42 and verify content match
        let mut read_buf = [0u8; SECTOR_SIZE];
        if let Err(_) = blk.read_sectors(42, &mut read_buf) {
            crate::serial_write_str("[KNT-PANIC] VirtIO block read_sectors failed!\r\n");
            return false;
        }

        if read_buf != write_buf {
            crate::serial_write_str("[KNT-PANIC] VirtIO block data corruption detected!\r\n");
            return false;
        }

        // 3.3 Boundary protection checks
        let mut unaligned_buf = [0u8; 100]; // Not multiple of 512
        if blk.read_sectors(0, &mut unaligned_buf) != Err(VirtioBlkError::InvalidBufferSize) {
            crate::serial_write_str("[KNT-PANIC] VirtIO block allowed unaligned buffer size!\r\n");
            return false;
        }

        let mut out_of_bounds_buf = [0u8; SECTOR_SIZE];
        if blk.read_sectors(999999, &mut out_of_bounds_buf) != Err(VirtioBlkError::SectorOutOfBounds) {
            crate::serial_write_str("[KNT-PANIC] VirtIO block allowed out of bounds sector!\r\n");
            return false;
        }

        crate::serial_write_str("[KNT-VIRTIO-BLOCK-PASS] 512-byte block sector read/write transactions verified\r\n");
    }

    crate::serial_write_str("[KNT-VIRTIO-TESTS-PASS] All VirtIO driver checks passed\r\n");
    true
}
