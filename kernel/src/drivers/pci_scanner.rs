//! Full PCI Configuration Space Bus Scanner and Device Discovery.

use alloc::vec::Vec;
use super::pci_db::{pci_class_name, pci_progif_name, pci_subclass_name, pci_vendor_name};
use super::virtio::pci::{pci_read_u32, pci_write_u32};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BarType {
    Memory32 { prefetchable: bool },
    Memory64 { prefetchable: bool },
    IoPort,
    None,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PciBar {
    pub index: u8,
    pub raw: u32,
    pub address: u64,
    pub size: u64,
    pub bar_type: BarType,
}

#[derive(Debug, Clone)]
pub struct PciDevice {
    pub bus: u8,
    pub slot: u8,
    pub function: u8,
    pub vendor_id: u16,
    pub device_id: u16,
    pub command: u16,
    pub status: u16,
    pub revision_id: u8,
    pub prog_if: u8,
    pub subclass: u8,
    pub class_code: u8,
    pub header_type: u8,
    pub is_multi_function: bool,
    pub bars: [Option<PciBar>; 6],
    pub interrupt_line: u8,
    pub interrupt_pin: u8,
}

impl PciDevice {
    pub fn vendor_name(&self) -> &'static str {
        pci_vendor_name(self.vendor_id)
    }

    pub fn class_name(&self) -> &'static str {
        pci_class_name(self.class_code)
    }

    pub fn subclass_name(&self) -> &'static str {
        pci_subclass_name(self.class_code, self.subclass_code())
    }

    pub fn progif_name(&self) -> &'static str {
        pci_progif_name(self.class_code, self.subclass, self.prog_if)
    }

    #[inline(always)]
    pub fn subclass_code(&self) -> u8 {
        self.subclass
    }

    pub fn get_bar(&self, index: usize) -> Option<PciBar> {
        if index < 6 {
            self.bars[index]
        } else {
            None
        }
    }
}

pub struct PciScanner;

impl PciScanner {
    /// Read 16-bit word from PCI configuration space.
    #[inline(always)]
    pub unsafe fn read_config_u16(bus: u8, slot: u8, func: u8, offset: u8) -> u16 {
        let dword = pci_read_u32(bus, slot, func, offset);
        ((dword >> ((offset & 2) * 8)) & 0xFFFF) as u16
    }

    /// Read 8-bit byte from PCI configuration space.
    #[inline(always)]
    pub unsafe fn read_config_u8(bus: u8, slot: u8, func: u8, offset: u8) -> u8 {
        let dword = pci_read_u32(bus, slot, func, offset);
        ((dword >> ((offset & 3) * 8)) & 0xFF) as u8
    }

    /// Read 32-bit dword from PCI configuration space.
    #[inline(always)]
    pub unsafe fn read_config_u32(bus: u8, slot: u8, func: u8, offset: u8) -> u32 {
        pci_read_u32(bus, slot, func, offset)
    }

    /// Write 32-bit dword to PCI configuration space.
    #[inline(always)]
    pub unsafe fn write_config_u32(bus: u8, slot: u8, func: u8, offset: u8, val: u32) {
        pci_write_u32(bus, slot, func, offset, val);
    }

    /// Probes and decodes a specific function at (bus, slot, func).
    pub fn probe_device(bus: u8, slot: u8, func: u8) -> Option<PciDevice> {
        unsafe {
            let reg0 = Self::read_config_u32(bus, slot, func, 0x00);
            let vendor_id = (reg0 & 0xFFFF) as u16;
            let device_id = ((reg0 >> 16) & 0xFFFF) as u16;

            // 0xFFFF or 0x0000 indicates non-existent function
            if vendor_id == 0xFFFF || vendor_id == 0x0000 {
                return None;
            }

            let reg1 = Self::read_config_u32(bus, slot, func, 0x04);
            let command = (reg1 & 0xFFFF) as u16;
            let status = ((reg1 >> 16) & 0xFFFF) as u16;

            let reg2 = Self::read_config_u32(bus, slot, func, 0x08);
            let revision_id = (reg2 & 0xFF) as u8;
            let prog_if = ((reg2 >> 8) & 0xFF) as u8;
            let subclass = ((reg2 >> 16) & 0xFF) as u8;
            let class_code = ((reg2 >> 24) & 0xFF) as u8;

            let reg3 = Self::read_config_u32(bus, slot, func, 0x0C);
            let raw_header = ((reg3 >> 16) & 0xFF) as u8;
            let header_type = raw_header & 0x7F;
            let is_multi_function = (raw_header & 0x80) != 0;

            let mut bars = [None; 6];

            // Standard PCI devices (Header Type 0x00) have up to 6 BARs
            if header_type == 0x00 {
                let mut bar_idx = 0;
                while bar_idx < 6 {
                    let offset = 0x10 + (bar_idx as u8) * 4;
                    let raw_bar = Self::read_config_u32(bus, slot, func, offset);

                    if raw_bar == 0 || raw_bar == 0xFFFF_FFFF {
                        bar_idx += 1;
                        continue;
                    }

                    let is_io = (raw_bar & 0x01) != 0;
                    if is_io {
                        let address = (raw_bar & !0x03) as u64;
                        // Probe size
                        Self::write_config_u32(bus, slot, func, offset, 0xFFFF_FFFF);
                        let mask = Self::read_config_u32(bus, slot, func, offset);
                        Self::write_config_u32(bus, slot, func, offset, raw_bar);
                        let size = (!(mask & !0x03)).wrapping_add(1) as u64;

                        bars[bar_idx] = Some(PciBar {
                            index: bar_idx as u8,
                            raw: raw_bar,
                            address,
                            size,
                            bar_type: BarType::IoPort,
                        });
                        bar_idx += 1;
                    } else {
                        let mem_type = (raw_bar >> 1) & 0x03;
                        let prefetchable = (raw_bar & 0x08) != 0;

                        if mem_type == 0x02 && bar_idx < 5 {
                            // 64-bit Memory BAR (spans bar_idx and bar_idx + 1)
                            let next_offset = offset + 4;
                            let upper_raw = Self::read_config_u32(bus, slot, func, next_offset);
                            let address = ((upper_raw as u64) << 32) | ((raw_bar & !0x0F) as u64);

                            // Probe size
                            Self::write_config_u32(bus, slot, func, offset, 0xFFFF_FFFF);
                            let mask_lo = Self::read_config_u32(bus, slot, func, offset);
                            Self::write_config_u32(bus, slot, func, offset, raw_bar);

                            Self::write_config_u32(bus, slot, func, next_offset, 0xFFFF_FFFF);
                            let mask_hi = Self::read_config_u32(bus, slot, func, next_offset);
                            Self::write_config_u32(bus, slot, func, next_offset, upper_raw);

                            let full_mask = ((mask_hi as u64) << 32) | ((mask_lo & !0x0F) as u64);
                            let size = (!full_mask).wrapping_add(1);

                            bars[bar_idx] = Some(PciBar {
                                index: bar_idx as u8,
                                raw: raw_bar,
                                address,
                                size,
                                bar_type: BarType::Memory64 { prefetchable },
                            });
                            bar_idx += 2; // Consumed two BAR slots
                        } else {
                            // 32-bit Memory BAR
                            let address = (raw_bar & !0x0F) as u64;

                            Self::write_config_u32(bus, slot, func, offset, 0xFFFF_FFFF);
                            let mask = Self::read_config_u32(bus, slot, func, offset);
                            Self::write_config_u32(bus, slot, func, offset, raw_bar);
                            let size = (!(mask & !0x0F)).wrapping_add(1) as u64;

                            bars[bar_idx] = Some(PciBar {
                                index: bar_idx as u8,
                                raw: raw_bar,
                                address,
                                size,
                                bar_type: BarType::Memory32 { prefetchable },
                            });
                            bar_idx += 1;
                        }
                    }
                }
            }

            let intr_reg = Self::read_config_u32(bus, slot, func, 0x3C);
            let interrupt_line = (intr_reg & 0xFF) as u8;
            let interrupt_pin = ((intr_reg >> 8) & 0xFF) as u8;

            Some(PciDevice {
                bus,
                slot,
                function: func,
                vendor_id,
                device_id,
                command,
                status,
                revision_id,
                prog_if,
                subclass,
                class_code,
                header_type,
                is_multi_function,
                bars,
                interrupt_line,
                interrupt_pin,
            })
        }
    }

    /// Full PCI scan across standard bus ranges (buses 0..max_buses, 32 slots, up to 8 functions).
    pub fn scan(max_buses: u16) -> Vec<PciDevice> {
        let mut devices = Vec::new();

        for bus in 0..max_buses {
            for slot in 0..32 {
                // Probe function 0 first
                if let Some(dev0) = Self::probe_device(bus as u8, slot as u8, 0) {
                    let is_multi = dev0.is_multi_function;
                    devices.push(dev0);

                    // If multi-function, check functions 1..8
                    if is_multi {
                        for func in 1..8 {
                            if let Some(dev_fn) = Self::probe_device(bus as u8, slot as u8, func) {
                                devices.push(dev_fn);
                            }
                        }
                    }
                }
            }
        }

        devices
    }
}
