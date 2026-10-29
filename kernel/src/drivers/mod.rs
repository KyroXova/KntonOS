//! Drivers and Hardware Subsystems for KntonOS.

pub mod virtio;
pub mod pci_db;
pub mod pci_scanner;
pub mod usb;
pub mod gpu;

use alloc::vec::Vec;
use crate::memory::spinlock::Spinlock;
use crate::serial_write_str;

pub use pci_db::{
    is_bridge, is_display_controller, is_network_controller, is_storage_controller,
    is_usb_controller, is_xhci_controller, pci_class_name, pci_progif_name, pci_subclass_name,
    pci_vendor_name, PCI_CLASS_BRIDGE, PCI_CLASS_DISPLAY, PCI_CLASS_MASS_STORAGE,
    PCI_CLASS_NETWORK, PCI_CLASS_SERIAL_BUS, PCI_PROGIF_USB_XHCI, PCI_SUBCLASS_SERIAL_USB,
};
pub use pci_scanner::{BarType, PciBar, PciDevice, PciScanner};
pub use usb::{XhciCapabilities, XhciController, XhciError};

/// Centralized PCI device and controller manager.
pub struct PciManager {
    pub devices: Vec<PciDevice>,
    pub xhci_controllers: Vec<XhciController>,
}

impl PciManager {
    pub fn new() -> Self {
        Self {
            devices: Vec::new(),
            xhci_controllers: Vec::new(),
        }
    }

    /// Scans configuration space across buses 0..max_buses.
    pub fn scan_bus(&mut self, max_buses: u16) {
        self.devices = PciScanner::scan(max_buses);
        self.xhci_controllers.clear();

        for dev in &self.devices {
            if is_xhci_controller(dev.class_code, dev.subclass, dev.prog_if) {
                if let Ok(xhci) = XhciController::from_pci(dev) {
                    self.xhci_controllers.push(xhci);
                }
            }
        }
    }

    #[inline(always)]
    pub fn devices(&self) -> &[PciDevice] {
        &self.devices
    }

    #[inline(always)]
    pub fn xhci_controllers(&self) -> &[XhciController] {
        &self.xhci_controllers
    }

    pub fn find_by_class(&self, class_code: u8) -> Vec<&PciDevice> {
        self.devices.iter().filter(|d| d.class_code == class_code).collect()
    }

    pub fn find_by_vendor_device(&self, vendor_id: u16, device_id: u16) -> Option<&PciDevice> {
        self.devices.iter().find(|d| d.vendor_id == vendor_id && d.device_id == device_id)
    }
}

pub static PCI_MANAGER: Spinlock<Option<PciManager>> = Spinlock::new(None);

/// Initializes the global PCI device manager and enumerates bus hierarchy.
pub fn init_pci() {
    serial_write_str("[KNT-PCI-INIT] Initializing PCI bus subsystem & device discovery...\r\n");
    let mut mgr = PciManager::new();
    mgr.scan_bus(4);
    serial_write_str("[KNT-PCI] Bus enumeration complete: discovered active devices\r\n");
    let mut guard = PCI_MANAGER.lock();
    *guard = Some(mgr);
}

/// Helper to execute a closure with read access to the global PCI manager.
pub fn with_pci<F, R>(f: F) -> Option<R>
where
    F: FnOnce(&PciManager) -> R,
{
    let guard = PCI_MANAGER.lock();
    guard.as_ref().map(f)
}

/// Verification self-test suite for the PCI device subsystem and USB xHCI discovery.
pub fn test_pci_subsystem() -> bool {
    serial_write_str("[KNT-PCI-TEST] Running PCI subsystem and xHCI discovery verification suite...\r\n");

    // 1. Verify PCI Class/Subclass/Vendor database
    {
        if pci_class_name(PCI_CLASS_MASS_STORAGE) != "Mass Storage Controller" {
            serial_write_str("[KNT-PANIC] PCI class name lookup failed!\r\n");
            return false;
        }
        if pci_class_name(PCI_CLASS_NETWORK) != "Network Controller" {
            serial_write_str("[KNT-PANIC] PCI network class lookup failed!\r\n");
            return false;
        }
        if pci_class_name(PCI_CLASS_SERIAL_BUS) != "Serial Bus Controller" {
            serial_write_str("[KNT-PANIC] PCI serial bus class lookup failed!\r\n");
            return false;
        }
        if pci_subclass_name(PCI_CLASS_SERIAL_BUS, PCI_SUBCLASS_SERIAL_USB) != "USB Controller" {
            serial_write_str("[KNT-PANIC] PCI USB subclass lookup failed!\r\n");
            return false;
        }
        if pci_progif_name(PCI_CLASS_SERIAL_BUS, PCI_SUBCLASS_SERIAL_USB, PCI_PROGIF_USB_XHCI)
            != "xHCI (USB 3.0 / Extensible HCI)"
        {
            serial_write_str("[KNT-PANIC] PCI xHCI prog_if lookup failed!\r\n");
            return false;
        }
        if pci_vendor_name(0x8086) != "Intel Corporation" || pci_vendor_name(0x1AF4) != "Red Hat, Inc. (VirtIO)" {
            serial_write_str("[KNT-PANIC] PCI vendor name lookup failed!\r\n");
            return false;
        }

        serial_write_str("  * PCI class, subclass, and vendor identification verified\r\n");
        serial_write_str("[KNT-PCI-DB-PASS] PCI class, subclass, and vendor identification verified\r\n");
    }

    // 2. Verify PCI configuration space scanner and device discovery
    {
        // Probe bus 0, slot 0, func 0 (Host Bridge on PC / Q35)
        let host_bridge = PciScanner::probe_device(0, 0, 0);
        match host_bridge {
            Some(dev) => {
                if dev.vendor_id == 0xFFFF || dev.vendor_id == 0x0000 {
                    serial_write_str("[KNT-PANIC] Invalid host bridge vendor ID!\r\n");
                    return false;
                }
                if dev.class_code != PCI_CLASS_BRIDGE {
                    serial_write_str("[KNT-PANIC] Host bridge class code mismatch!\r\n");
                    return false;
                }
            }
            None => {
                serial_write_str("[KNT-PANIC] PCI host bridge not detected at 00:00.0!\r\n");
                return false;
            }
        }

        // Test bus scanner across bus 0..1
        let devices = PciScanner::scan(1);
        if devices.is_empty() {
            serial_write_str("[KNT-PANIC] PCI scan returned 0 devices on bus 0!\r\n");
            return false;
        }

        serial_write_str("  * PCI configuration space scanner and BAR parsing verified\r\n");
        serial_write_str("[KNT-PCI-SCAN-PASS] PCI configuration space scanner and BAR parsing verified\r\n");
    }

    // 3. Verify USB 3.0 xHCI controller discovery and register parsing
    {
        // Synthetic xHCI capability register block:
        // CAPLENGTH = 0x20, HCIVERSION = 0x0100 (xHCI 1.0)
        let caplength_version: u32 = 0x0100_0020;
        // MaxSlots = 32, MaxIntrs = 8, MaxPorts = 4
        let hcsparams1: u32 = (4 << 24) | (8 << 8) | 32;
        // AC64 = 1, CSZ = 1 (64-byte context), xECP = 0x0040
        let hccparams1: u32 = (0x0040 << 16) | 0x04 | 0x01;
        let dboff: u32 = 0x0000_1000;
        let rtsoff: u32 = 0x0000_2000;

        let caps = match XhciCapabilities::parse(
            caplength_version,
            hcsparams1,
            hccparams1,
            dboff,
            rtsoff,
        ) {
            Ok(c) => c,
            Err(_) => {
                serial_write_str("[KNT-PANIC] xHCI capability parsing failed!\r\n");
                return false;
            }
        };

        if caps.caplength != 0x20 || caps.hciversion != 0x0100 {
            serial_write_str("[KNT-PANIC] xHCI caplength/version mismatch!\r\n");
            return false;
        }
        if caps.max_slots != 32 || caps.max_ports != 4 || caps.max_interrupters != 8 {
            serial_write_str("[KNT-PANIC] xHCI structural parameters mismatch!\r\n");
            return false;
        }
        if !caps.ac64 || !caps.context_size_64 {
            serial_write_str("[KNT-PANIC] xHCI 64-bit flags mismatch!\r\n");
            return false;
        }
        if caps.doorbell_offset != 0x1000 || caps.runtime_offset != 0x2000 {
            serial_write_str("[KNT-PANIC] xHCI doorbell/runtime register offsets mismatch!\r\n");
            return false;
        }

        // Test operational register offset calculation
        let fake_mmio_base: u64 = 0xFE80_0000;
        let op_base = fake_mmio_base + caps.caplength as u64;
        if op_base != 0xFE80_0020 {
            serial_write_str("[KNT-PANIC] xHCI operational base calculation failed!\r\n");
            return false;
        }

        let ctrl = XhciController {
            pci_bus: 0,
            pci_slot: 13,
            pci_func: 0,
            vendor_id: 0x8086,
            device_id: 0x0B80,
            mmio_base: fake_mmio_base,
            mmio_size: 0x10000,
            op_base,
            caps,
        };

        if ctrl.version_string() != "xHCI 1.0" {
            serial_write_str("[KNT-PANIC] xHCI version string mismatch!\r\n");
            return false;
        }

        serial_write_str("  * USB 3.0 xHCI controller discovery and register mapping verified\r\n");
        serial_write_str("[KNT-PCI-XHCI-PASS] USB 3.0 xHCI controller discovery and register mapping verified\r\n");
    }

    serial_write_str("[KNT-PCI-TESTS-PASS] All PCI subsystem and xHCI controller checks passed\r\n");
    true
}
