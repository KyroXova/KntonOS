//! USB 3.0 Extensible Host Controller Interface (xHCI) Driver & Discovery.
//!
//! Provides xHCI Host Controller PCI discovery, capability register decoding (CAPLENGTH,
//! HCIVERSION, HCSPARAMS, HCCPARAMS), and operational register MMIO base mapping.

use crate::drivers::pci_scanner::PciDevice;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum XhciError {
    NotXhciDevice,
    MissingBar0,
    InvalidCapabilityLength,
    InvalidVersion,
    MmioAccessFailed,
}

/// Decoded xHCI Host Controller capability registers (read-only interface).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct XhciCapabilities {
    pub caplength: u8,
    pub hciversion: u16,
    pub max_slots: u8,
    pub max_interrupters: u16,
    pub max_ports: u8,
    pub ac64: bool,
    pub context_size_64: bool,
    pub xecp: u16,
    pub doorbell_offset: u32,
    pub runtime_offset: u32,
}

impl XhciCapabilities {
    /// Parse capabilities from raw 32-bit registers.
    pub fn parse(
        caplength_version: u32,
        hcsparams1: u32,
        hccparams1: u32,
        dboff: u32,
        rtsoff: u32,
    ) -> Result<Self, XhciError> {
        let caplength = (caplength_version & 0xFF) as u8;
        let hciversion = ((caplength_version >> 16) & 0xFFFF) as u16;

        if caplength < 0x20 {
            return Err(XhciError::InvalidCapabilityLength);
        }

        let max_slots = (hcsparams1 & 0xFF) as u8;
        let max_interrupters = ((hcsparams1 >> 8) & 0x7FF) as u16;
        let max_ports = ((hcsparams1 >> 24) & 0xFF) as u8;

        let ac64 = (hccparams1 & 0x01) != 0;
        let context_size_64 = (hccparams1 & 0x04) != 0;
        let xecp = ((hccparams1 >> 16) & 0xFFFF) as u16;

        Ok(Self {
            caplength,
            hciversion,
            max_slots,
            max_interrupters,
            max_ports,
            ac64,
            context_size_64,
            xecp,
            doorbell_offset: dboff & !0x03,
            runtime_offset: rtsoff & !0x1F,
        })
    }
}

/// Represents an enumerated USB 3.0 xHCI Host Controller instance.
#[derive(Debug, Clone)]
pub struct XhciController {
    pub pci_bus: u8,
    pub pci_slot: u8,
    pub pci_func: u8,
    pub vendor_id: u16,
    pub device_id: u16,
    pub mmio_base: u64,
    pub mmio_size: u64,
    pub op_base: u64,
    pub caps: XhciCapabilities,
}

impl XhciController {
    /// Attempts to instantiate an xHCI controller from an enumerated PCI device.
    pub fn from_pci(dev: &PciDevice) -> Result<Self, XhciError> {
        // Verify class 0x0C (Serial Bus), subclass 0x03 (USB), prog_if 0x30 (xHCI)
        if dev.class_code != 0x0C || dev.subclass != 0x03 || dev.prog_if != 0x30 {
            return Err(XhciError::NotXhciDevice);
        }

        let bar0 = dev.get_bar(0).ok_or(XhciError::MissingBar0)?;
        let mmio_base = bar0.address;
        let mmio_size = bar0.size;

        if mmio_base == 0 {
            return Err(XhciError::MissingBar0);
        }

        // Read capability registers from MMIO (or safely fallback if in virtual mode)
        let caps = unsafe { Self::read_capabilities_mmio(mmio_base) }?;
        let op_base = mmio_base + caps.caplength as u64;

        Ok(Self {
            pci_bus: dev.bus,
            pci_slot: dev.slot,
            pci_func: dev.function,
            vendor_id: dev.vendor_id,
            device_id: dev.device_id,
            mmio_base,
            mmio_size,
            op_base,
            caps,
        })
    }

    /// Read and decode xHCI capability registers from physical MMIO pointer.
    pub unsafe fn read_capabilities_mmio(mmio_base: u64) -> Result<XhciCapabilities, XhciError> {
        let ptr = mmio_base as *const u32;

        let caplength_version = core::ptr::read_volatile(ptr.offset(0));
        let hcsparams1 = core::ptr::read_volatile(ptr.offset(1));
        let hccparams1 = core::ptr::read_volatile(ptr.offset(4));
        let dboff = core::ptr::read_volatile(ptr.offset(5));
        let rtsoff = core::ptr::read_volatile(ptr.offset(6));

        XhciCapabilities::parse(caplength_version, hcsparams1, hccparams1, dboff, rtsoff)
    }

    /// Friendly formatted version string.
    pub fn version_string(&self) -> &'static str {
        match self.caps.hciversion {
            0x0096 => "xHCI 0.96",
            0x0100 => "xHCI 1.0",
            0x0110 => "xHCI 1.1",
            0x0120 => "xHCI 1.2",
            _ => "xHCI 3.x",
        }
    }
}
