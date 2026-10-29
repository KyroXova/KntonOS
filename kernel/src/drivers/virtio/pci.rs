//! VirtIO Modern PCI Capability Scanner and Register Interface.
//!
//! Implements PCI standard configuration space enumeration (via I/O ports 0xCF8 / 0xCFC)
//! and parses VirtIO 1.0+ vendor-specific capabilities (type 0x09) to locate Common,
//! Notify, ISR, and Device Configuration structures.

use alloc::vec::Vec;

pub const PCI_CONFIG_ADDRESS: u16 = 0xCF8;
pub const PCI_CONFIG_DATA: u16 = 0xCFC;

pub const VIRTIO_VENDOR_ID: u16 = 0x1AF4;

// VirtIO Subsystem Device IDs (Transitional & Modern)
pub const VIRTIO_DEV_NET_TRANSITIONAL: u16 = 0x1000;
pub const VIRTIO_DEV_BLOCK_TRANSITIONAL: u16 = 0x1001;
pub const VIRTIO_DEV_NET_MODERN: u16 = 0x1041;
pub const VIRTIO_DEV_BLOCK_MODERN: u16 = 0x1042;
pub const VIRTIO_DEV_CONSOLE_MODERN: u16 = 0x1043;
pub const VIRTIO_DEV_ENTROPY_MODERN: u16 = 0x1044;
pub const VIRTIO_DEV_GPU_MODERN: u16 = 0x1050;

// VirtIO PCI Capability Types
pub const VIRTIO_PCI_CAP_COMMON_CFG: u8 = 1;
pub const VIRTIO_PCI_CAP_NOTIFY_CFG: u8 = 2;
pub const VIRTIO_PCI_CAP_ISR_CFG: u8 = 3;
pub const VIRTIO_PCI_CAP_DEVICE_CFG: u8 = 4;
pub const VIRTIO_PCI_CAP_PCI_CFG: u8 = 5;

// VirtIO Device Status Bits
pub const VIRTIO_STATUS_ACKNOWLEDGE: u8 = 1;
pub const VIRTIO_STATUS_DRIVER: u8 = 2;
pub const VIRTIO_STATUS_DRIVER_OK: u8 = 4;
pub const VIRTIO_STATUS_FEATURES_OK: u8 = 8;
pub const VIRTIO_STATUS_DEVICE_NEEDS_RESET: u8 = 64;
pub const VIRTIO_STATUS_FAILED: u8 = 128;

#[inline(always)]
pub unsafe fn pci_read_u32(bus: u8, slot: u8, func: u8, offset: u8) -> u32 {
    let address = ((bus as u32) << 16)
        | ((slot as u32) << 11)
        | ((func as u32) << 8)
        | ((offset as u32) & 0xFC)
        | 0x8000_0000;

    crate::outb(PCI_CONFIG_ADDRESS + 0, (address & 0xFF) as u8);
    crate::outb(PCI_CONFIG_ADDRESS + 1, ((address >> 8) & 0xFF) as u8);
    crate::outb(PCI_CONFIG_ADDRESS + 2, ((address >> 16) & 0xFF) as u8);
    crate::outb(PCI_CONFIG_ADDRESS + 3, ((address >> 24) & 0xFF) as u8);

    let b0 = crate::inb(PCI_CONFIG_DATA + 0) as u32;
    let b1 = crate::inb(PCI_CONFIG_DATA + 1) as u32;
    let b2 = crate::inb(PCI_CONFIG_DATA + 2) as u32;
    let b3 = crate::inb(PCI_CONFIG_DATA + 3) as u32;

    b0 | (b1 << 8) | (b2 << 16) | (b3 << 24)
}

#[inline(always)]
pub unsafe fn pci_write_u32(bus: u8, slot: u8, func: u8, offset: u8, val: u32) {
    let address = ((bus as u32) << 16)
        | ((slot as u32) << 11)
        | ((func as u32) << 8)
        | ((offset as u32) & 0xFC)
        | 0x8000_0000;

    crate::outb(PCI_CONFIG_ADDRESS + 0, (address & 0xFF) as u8);
    crate::outb(PCI_CONFIG_ADDRESS + 1, ((address >> 8) & 0xFF) as u8);
    crate::outb(PCI_CONFIG_ADDRESS + 2, ((address >> 16) & 0xFF) as u8);
    crate::outb(PCI_CONFIG_ADDRESS + 3, ((address >> 24) & 0xFF) as u8);

    crate::outb(PCI_CONFIG_DATA + 0, (val & 0xFF) as u8);
    crate::outb(PCI_CONFIG_DATA + 1, ((val >> 8) & 0xFF) as u8);
    crate::outb(PCI_CONFIG_DATA + 2, ((val >> 16) & 0xFF) as u8);
    crate::outb(PCI_CONFIG_DATA + 3, ((val >> 24) & 0xFF) as u8);
}

#[inline(always)]
pub unsafe fn pci_read_u16(bus: u8, slot: u8, func: u8, offset: u8) -> u16 {
    let dword = pci_read_u32(bus, slot, func, offset);
    ((dword >> ((offset & 2) * 8)) & 0xFFFF) as u16
}

#[inline(always)]
pub unsafe fn pci_read_u8(bus: u8, slot: u8, func: u8, offset: u8) -> u8 {
    let dword = pci_read_u32(bus, slot, func, offset);
    ((dword >> ((offset & 3) * 8)) & 0xFF) as u8
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PciAddress {
    pub bus: u8,
    pub slot: u8,
    pub func: u8,
}

#[derive(Debug, Clone)]
pub struct PciDevice {
    pub addr: PciAddress,
    pub vendor_id: u16,
    pub device_id: u16,
    pub class_code: u8,
    pub subclass: u8,
    pub header_type: u8,
}

#[derive(Debug, Clone, Copy)]
pub struct VirtioCapability {
    pub cfg_type: u8,
    pub bar: u8,
    pub offset: u32,
    pub length: u32,
}

impl PciDevice {
    /// Reads Base Address Register (BAR) physical address and size.
    pub unsafe fn read_bar(&self, bar_index: u8) -> u32 {
        if bar_index >= 6 {
            return 0;
        }
        let offset = 0x10 + (bar_index * 4);
        pci_read_u32(self.addr.bus, self.addr.slot, self.addr.func, offset)
    }

    /// Scans PCI capability linked-list to find VirtIO 1.0 vendor capabilities.
    pub unsafe fn find_virtio_capabilities(&self) -> Vec<VirtioCapability> {
        let mut caps = Vec::new();
        let status = pci_read_u16(self.addr.bus, self.addr.slot, self.addr.func, 0x06);

        // Check if Capabilities List bit (bit 4) is set
        if (status & (1 << 4)) == 0 {
            return caps;
        }

        let mut cap_ptr = pci_read_u8(self.addr.bus, self.addr.slot, self.addr.func, 0x34) & 0xFC;

        while cap_ptr != 0 {
            let cap_vndr = pci_read_u8(self.addr.bus, self.addr.slot, self.addr.func, cap_ptr);
            let cap_next = pci_read_u8(self.addr.bus, self.addr.slot, self.addr.func, cap_ptr + 1) & 0xFC;
            let cap_len = pci_read_u8(self.addr.bus, self.addr.slot, self.addr.func, cap_ptr + 2);

            // Vendor-specific capability is 0x09 for VirtIO
            if cap_vndr == 0x09 && cap_len >= 16 {
                let cfg_type = pci_read_u8(self.addr.bus, self.addr.slot, self.addr.func, cap_ptr + 3);
                let bar = pci_read_u8(self.addr.bus, self.addr.slot, self.addr.func, cap_ptr + 4);
                let offset = pci_read_u32(self.addr.bus, self.addr.slot, self.addr.func, cap_ptr + 8);
                let length = pci_read_u32(self.addr.bus, self.addr.slot, self.addr.func, cap_ptr + 12);

                caps.push(VirtioCapability {
                    cfg_type,
                    bar,
                    offset,
                    length,
                });
            }

            cap_ptr = cap_next;
        }

        caps
    }
}

/// Enumerates the entire PCI bus hierarchy to identify connected VirtIO devices.
pub fn scan_pci() -> Vec<PciDevice> {
    let mut devices = Vec::new();

    for bus in 0..=3 {
        for slot in 0..32 {
            unsafe {
                let vendor_id = pci_read_u16(bus, slot, 0, 0x00);
                if vendor_id == 0xFFFF || vendor_id == 0 {
                    continue;
                }

                let device_id = pci_read_u16(bus, slot, 0, 0x02);
                let class_subclass = pci_read_u16(bus, slot, 0, 0x0A);
                let class_code = ((class_subclass >> 8) & 0xFF) as u8;
                let subclass = (class_subclass & 0xFF) as u8;
                let header_type = pci_read_u8(bus, slot, 0, 0x0E);

                devices.push(PciDevice {
                    addr: PciAddress { bus, slot, func: 0 },
                    vendor_id,
                    device_id,
                    class_code,
                    subclass,
                    header_type,
                });
            }
        }
    }

    devices
}
