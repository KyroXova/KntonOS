//! Extended PCI Class/Subclass Database and Vendor Identification.
//!
//! Provides human-readable descriptions, classifications, and interface helpers for standard
//! PCI devices, including Mass Storage, Network, Display, Bridge, and Serial Bus (USB xHCI).

pub const PCI_CLASS_UNCLASSIFIED: u8 = 0x00;
pub const PCI_CLASS_MASS_STORAGE: u8 = 0x01;
pub const PCI_CLASS_NETWORK: u8 = 0x02;
pub const PCI_CLASS_DISPLAY: u8 = 0x03;
pub const PCI_CLASS_MULTIMEDIA: u8 = 0x04;
pub const PCI_CLASS_MEMORY: u8 = 0x05;
pub const PCI_CLASS_BRIDGE: u8 = 0x06;
pub const PCI_CLASS_COMMUNICATION: u8 = 0x07;
pub const PCI_CLASS_SYSTEM_PERIPHERAL: u8 = 0x08;
pub const PCI_CLASS_INPUT: u8 = 0x09;
pub const PCI_CLASS_SERIAL_BUS: u8 = 0x0C;
pub const PCI_CLASS_WIRELESS: u8 = 0x0D;
pub const PCI_CLASS_ACCELERATOR: u8 = 0x12;

// Serial Bus (0x0C) Subclasses
pub const PCI_SUBCLASS_SERIAL_FIREWIRE: u8 = 0x00;
pub const PCI_SUBCLASS_SERIAL_ACCESS: u8 = 0x01;
pub const PCI_SUBCLASS_SERIAL_SSA: u8 = 0x02;
pub const PCI_SUBCLASS_SERIAL_USB: u8 = 0x03;
pub const PCI_SUBCLASS_SERIAL_FIBRE: u8 = 0x04;
pub const PCI_SUBCLASS_SERIAL_SMBUS: u8 = 0x05;

// USB Programming Interfaces (ProgIF for Class 0x0C, Subclass 0x03)
pub const PCI_PROGIF_USB_UHCI: u8 = 0x00;
pub const PCI_PROGIF_USB_OHCI: u8 = 0x10;
pub const PCI_PROGIF_USB_EHCI: u8 = 0x20;
pub const PCI_PROGIF_USB_XHCI: u8 = 0x30;
pub const PCI_PROGIF_USB_OTHER: u8 = 0x80;
pub const PCI_PROGIF_USB_DEVICE: u8 = 0xFE;

// Mass Storage (0x01) Subclasses
pub const PCI_SUBCLASS_STORAGE_SCSI: u8 = 0x00;
pub const PCI_SUBCLASS_STORAGE_IDE: u8 = 0x01;
pub const PCI_SUBCLASS_STORAGE_FLOPPY: u8 = 0x02;
pub const PCI_SUBCLASS_STORAGE_RAID: u8 = 0x04;
pub const PCI_SUBCLASS_STORAGE_ATA: u8 = 0x05;
pub const PCI_SUBCLASS_STORAGE_SATA: u8 = 0x06;
pub const PCI_SUBCLASS_STORAGE_SAS: u8 = 0x07;
pub const PCI_SUBCLASS_STORAGE_NVME: u8 = 0x08;

// Network (0x02) Subclasses
pub const PCI_SUBCLASS_NET_ETHERNET: u8 = 0x00;
pub const PCI_SUBCLASS_NET_TOKEN_RING: u8 = 0x01;
pub const PCI_SUBCLASS_NET_FDDI: u8 = 0x02;
pub const PCI_SUBCLASS_NET_ATM: u8 = 0x03;
pub const PCI_SUBCLASS_NET_ISDN: u8 = 0x04;
pub const PCI_SUBCLASS_NET_INFINIBAND: u8 = 0x07;

// Display (0x03) Subclasses
pub const PCI_SUBCLASS_DISPLAY_VGA: u8 = 0x00;
pub const PCI_SUBCLASS_DISPLAY_XGA: u8 = 0x01;
pub const PCI_SUBCLASS_DISPLAY_3D: u8 = 0x02;

// Bridge (0x06) Subclasses
pub const PCI_SUBCLASS_BRIDGE_HOST: u8 = 0x00;
pub const PCI_SUBCLASS_BRIDGE_ISA: u8 = 0x01;
pub const PCI_SUBCLASS_BRIDGE_EISA: u8 = 0x02;
pub const PCI_SUBCLASS_BRIDGE_PCI_PCI: u8 = 0x04;
pub const PCI_SUBCLASS_BRIDGE_CARDBUS: u8 = 0x07;

/// Returns friendly string representation for standard PCI Class code.
pub fn pci_class_name(class_code: u8) -> &'static str {
    match class_code {
        0x00 => "Unclassified / Legacy Device",
        0x01 => "Mass Storage Controller",
        0x02 => "Network Controller",
        0x03 => "Display Controller",
        0x04 => "Multimedia Controller",
        0x05 => "Memory Controller",
        0x06 => "Bridge Device",
        0x07 => "Simple Communication Controller",
        0x08 => "Base System Peripheral",
        0x09 => "Input Device Controller",
        0x0A => "Docking Station",
        0x0B => "Processor",
        0x0C => "Serial Bus Controller",
        0x0D => "Wireless Controller",
        0x0E => "Intelligent Controller",
        0x0F => "Satellite Communication Controller",
        0x10 => "Encryption Controller",
        0x11 => "Signal Processing Controller",
        0x12 => "Processing Accelerator",
        0x13 => "Non-Essential Instrumentation",
        0xFF => "Unassigned / Vendor-Specific",
        _ => "Unknown Class",
    }
}

/// Returns friendly string representation for PCI Subclass code.
pub fn pci_subclass_name(class_code: u8, subclass_code: u8) -> &'static str {
    match (class_code, subclass_code) {
        // Mass Storage
        (0x01, 0x00) => "SCSI Bus Controller",
        (0x01, 0x01) => "IDE Interface",
        (0x01, 0x02) => "Floppy Disk Controller",
        (0x01, 0x04) => "RAID Bus Controller",
        (0x01, 0x05) => "ATA Controller",
        (0x01, 0x06) => "Serial ATA (SATA)",
        (0x01, 0x07) => "Serial Attached SCSI (SAS)",
        (0x01, 0x08) => "Non-Volatile Memory (NVMe)",
        (0x01, 0x80) => "Other Mass Storage",

        // Network
        (0x02, 0x00) => "Ethernet Controller",
        (0x02, 0x01) => "Token Ring Controller",
        (0x02, 0x02) => "FDDI Network Controller",
        (0x02, 0x03) => "ATM Network Controller",
        (0x02, 0x04) => "ISDN Controller",
        (0x02, 0x07) => "Infiniband Controller",
        (0x02, 0x80) => "Other Network Controller",

        // Display
        (0x03, 0x00) => "VGA Compatible Controller",
        (0x03, 0x01) => "XGA Controller",
        (0x03, 0x02) => "3D Controller",
        (0x03, 0x80) => "Other Display Controller",

        // Bridge
        (0x06, 0x00) => "Host Bridge",
        (0x06, 0x01) => "ISA Bridge",
        (0x06, 0x02) => "EISA Bridge",
        (0x06, 0x04) => "PCI-to-PCI Bridge",
        (0x06, 0x07) => "CardBus Bridge",
        (0x06, 0x80) => "Other Bridge Device",

        // Serial Bus
        (0x0C, 0x00) => "FireWire (IEEE 1394)",
        (0x0C, 0x01) => "ACCESS Bus",
        (0x0C, 0x02) => "SSA Controller",
        (0x0C, 0x03) => "USB Controller",
        (0x0C, 0x04) => "Fibre Channel",
        (0x0C, 0x05) => "SMBus Controller",
        (0x0C, 0x06) => "InfiniBand",
        (0x0C, 0x09) => "CANbus Controller",
        (0x0C, 0x80) => "Other Serial Bus",

        _ => "Generic Subclass",
    }
}

/// Returns specific programming interface description (e.g. UHCI, EHCI, xHCI for USB).
pub fn pci_progif_name(class_code: u8, subclass_code: u8, prog_if: u8) -> &'static str {
    match (class_code, subclass_code, prog_if) {
        (0x0C, 0x03, 0x00) => "UHCI (USB 1.1)",
        (0x0C, 0x03, 0x10) => "OHCI (USB 1.1)",
        (0x0C, 0x03, 0x20) => "EHCI (USB 2.0)",
        (0x0C, 0x03, 0x30) => "xHCI (USB 3.0 / Extensible HCI)",
        (0x0C, 0x03, 0x80) => "Unspecified USB Controller",
        (0x0C, 0x03, 0xFE) => "USB Device (non-host)",
        (0x01, 0x06, 0x01) => "AHCI 1.0 SATA Controller",
        (0x01, 0x08, 0x02) => "NVM Express (NVMe) I/O Controller",
        (0x03, 0x00, 0x00) => "VGA Compatible",
        (0x03, 0x00, 0x01) => "8514-Compatible",
        _ => "Standard Interface",
    }
}

/// Helper returning vendor name for common PCI vendor IDs.
pub fn pci_vendor_name(vendor_id: u16) -> &'static str {
    match vendor_id {
        0x8086 => "Intel Corporation",
        0x10DE => "NVIDIA Corporation",
        0x1002 => "Advanced Micro Devices, Inc. [AMD/ATI]",
        0x1AF4 => "Red Hat, Inc. (VirtIO)",
        0x1234 => "QEMU Standard Emulated Device",
        0x1B36 => "Red Hat, Inc. (QEMU PCI)",
        0x10EC => "Realtek Semiconductor Co., Ltd.",
        0x14E4 => "Broadcom Inc.",
        0x1022 => "Advanced Micro Devices, Inc. [AMD]",
        0x104C => "Texas Instruments",
        0x1033 => "NEC Corporation",
        0xFFFF => "Device Absent",
        _ => "Unknown Vendor",
    }
}

#[inline(always)]
pub fn is_usb_controller(class_code: u8, subclass_code: u8) -> bool {
    class_code == PCI_CLASS_SERIAL_BUS && subclass_code == PCI_SUBCLASS_SERIAL_USB
}

#[inline(always)]
pub fn is_xhci_controller(class_code: u8, subclass_code: u8, prog_if: u8) -> bool {
    is_usb_controller(class_code, subclass_code) && prog_if == PCI_PROGIF_USB_XHCI
}

#[inline(always)]
pub fn is_storage_controller(class_code: u8) -> bool {
    class_code == PCI_CLASS_MASS_STORAGE
}

#[inline(always)]
pub fn is_network_controller(class_code: u8) -> bool {
    class_code == PCI_CLASS_NETWORK
}

#[inline(always)]
pub fn is_display_controller(class_code: u8) -> bool {
    class_code == PCI_CLASS_DISPLAY
}

#[inline(always)]
pub fn is_bridge(class_code: u8) -> bool {
    class_code == PCI_CLASS_BRIDGE
}
