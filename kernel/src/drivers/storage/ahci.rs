//! AHCI SATA Specification 1.3.1 Memory Registers & Host Controller Structures.

use crate::drivers::pci_scanner::PciDevice;

pub const PCI_CLASS_MASS_STORAGE: u8 = 0x01;
pub const PCI_SUBCLASS_SATA: u8 = 0x06;
pub const PCI_PROGIF_AHCI: u8 = 0x01;

// Generic Host Control (GHC) Register Offsets.
pub const AHCI_REG_CAP: usize = 0x00;      // Host Capabilities (32-bit)
pub const AHCI_REG_GHC: usize = 0x04;      // Global Host Control (32-bit)
pub const AHCI_REG_IS: usize = 0x08;       // Interrupt Status (32-bit)
pub const AHCI_REG_PI: usize = 0x0C;       // Ports Implemented (32-bit)
pub const AHCI_REG_VS: usize = 0x10;       // AHCI Version (32-bit)
pub const AHCI_REG_CCC_CTL: usize = 0x14;  // Command Completion Coalescing Control (32-bit)
pub const AHCI_REG_CCC_PORTS: usize = 0x18;// Command Completion Coalescing Ports (32-bit)
pub const AHCI_REG_EM_LOC: usize = 0x1C;   // Enclosure Management Location (32-bit)
pub const AHCI_REG_EM_CTL: usize = 0x20;   // Enclosure Management Control (32-bit)
pub const AHCI_REG_CAP2: usize = 0x24;     // Host Capabilities Extended (32-bit)
pub const AHCI_REG_BOHC: usize = 0x28;     // BIOS/OS Handoff Control and Status (32-bit)

// GHC Register Bitmasks.
pub const GHC_HR: u32 = 1 << 0;            // HBA Reset (0: operational, 1: reset)
pub const GHC_IE: u32 = 1 << 1;            // Global Interrupt Enable
pub const GHC_MRSM: u32 = 1 << 2;          // MSI Revert to Single Message
pub const GHC_AE: u32 = 1 << 31;           // AHCI Enable (1: AHCI mode, 0: legacy IDE)

// Host Capabilities (CAP) Bitmasks.
pub const CAP_S64A: u32 = 1 << 31;         // Supports 64-bit Addressing
pub const CAP_SNCQ: u32 = 1 << 30;         // Supports Native Command Queuing
pub const CAP_SSNTF: u32 = 1 << 29;        // Supports SNotification Register
pub const CAP_SMPS: u32 = 1 << 28;         // Supports Mechanical Presence Switch
pub const CAP_SSS: u32 = 1 << 27;          // Supports Staggered Spin-up
pub const CAP_ALPE: u32 = 1 << 26;         // Supports Aggressive Link Power Management
pub const CAP_SAM: u32 = 1 << 18;          // Supports AHCI Mode Only
pub const CAP_SPM: u32 = 1 << 17;          // Supports Port Multiplier
pub const CAP_FBSS: u32 = 1 << 16;         // Supports FIS-based Switching

// BIOS/OS Handoff (BOHC) Bitmasks.
pub const BOHC_BOS: u32 = 1 << 0;          // BIOS Owned Semaphore
pub const BOHC_OOS: u32 = 1 << 1;          // OS Owned Semaphore
pub const BOHC_SOOE: u32 = 1 << 2;         // SMI on OS Ownership Change
pub const BOHC_OOC: u32 = 1 << 3;          // OS Ownership Change
pub const BOHC_BB: u32 = 1 << 4;           // BIOS Busy

// Port Register Offsets & Strides.
pub const AHCI_PORT_BASE: usize = 0x100;
pub const AHCI_PORT_STRIDE: usize = 0x80;
pub const MAX_PORTS: usize = 32;

// Port Registers (Offset within Port structure).
pub const PORT_REG_CLB: usize = 0x00;      // Command List Base Address (32-bit)
pub const PORT_REG_CLBU: usize = 0x04;     // Command List Base Address Upper (32-bit)
pub const PORT_REG_FB: usize = 0x08;       // FIS Base Address (32-bit)
pub const PORT_REG_FBU: usize = 0x0C;      // FIS Base Address Upper (32-bit)
pub const PORT_REG_IS: usize = 0x10;       // Interrupt Status (32-bit)
pub const PORT_REG_IE: usize = 0x14;       // Interrupt Enable (32-bit)
pub const PORT_REG_CMD: usize = 0x18;      // Command and Status (32-bit)
pub const PORT_REG_TFD: usize = 0x20;      // Task File Data (32-bit)
pub const PORT_REG_SIG: usize = 0x24;      // Signature (32-bit)
pub const PORT_REG_SSTS: usize = 0x28;     // SATA Status / SStatus (32-bit)
pub const PORT_REG_SCTL: usize = 0x2C;     // SATA Control / SControl (32-bit)
pub const PORT_REG_SERR: usize = 0x30;     // SATA Error / SError (32-bit)
pub const PORT_REG_SACT: usize = 0x34;     // SATA Active / SActive (32-bit)
pub const PORT_REG_CI: usize = 0x38;       // Command Issue (32-bit)
pub const PORT_REG_SNTF: usize = 0x3C;     // SATA Notification (32-bit)
pub const PORT_REG_FBS: usize = 0x40;      // FIS-based Switching (32-bit)

// Port Command (CMD) Bitmasks.
pub const PORT_CMD_ST: u32 = 1 << 0;       // Start command processing
pub const PORT_CMD_SUD: u32 = 1 << 1;      // Spin-Up Device
pub const PORT_CMD_POD: u32 = 1 << 2;      // Power On Device
pub const PORT_CMD_CLO: u32 = 1 << 3;      // Command List Override
pub const PORT_CMD_FRE: u32 = 1 << 4;      // FIS Receive Enable
pub const PORT_CMD_MPSS: u32 = 1 << 13;    // Mechanical Presence Switch State
pub const PORT_CMD_FR: u32 = 1 << 14;      // FIS Receive Running
pub const PORT_CMD_CR: u32 = 1 << 15;      // Command List Running
pub const PORT_CMD_CPS: u32 = 1 << 16;     // Cold Presence State
pub const PORT_CMD_PMA: u32 = 1 << 17;     // Port Multiplier Attached
pub const PORT_CMD_HPCP: u32 = 1 << 18;    // Hot Plug Capable Port
pub const PORT_CMD_MPSP: u32 = 1 << 19;    // Mechanical Presence Switch Attached
pub const PORT_CMD_CPD: u32 = 1 << 20;     // Cold Presence Detection
pub const PORT_CMD_ESP: u32 = 1 << 21;     // External SATA Port
pub const PORT_CMD_FBSCP: u32 = 1 << 22;   // FIS-based Switching Capable Port
pub const PORT_CMD_APSTE: u32 = 1 << 23;   // Automatic Partial to Slumber Transitions
pub const PORT_CMD_ATAPI: u32 = 1 << 24;   // Device is ATAPI
pub const PORT_CMD_DLAE: u32 = 1 << 25;    // Drive LED on ATAPI Enable
pub const PORT_CMD_ALPE: u32 = 1 << 26;    // Aggressive Link Power Management
pub const PORT_CMD_ASP: u32 = 1 << 27;     // Aggressive Slumber / Partial
pub const PORT_CMD_ICC_ACTIVE: u32 = 1 << 28; // Interface Communication: Active

// Port Interrupt Status (IS) & Enable (IE) Bitmasks.
pub const PORT_IS_DHRS: u32 = 1 << 0;      // Device to Host Register FIS Interrupt
pub const PORT_IS_PSS: u32 = 1 << 1;       // PIO Setup FIS Interrupt
pub const PORT_IS_DSS: u32 = 1 << 2;       // DMA Setup FIS Interrupt
pub const PORT_IS_SDBS: u32 = 1 << 3;      // Set Device Bits Interrupt
pub const PORT_IS_UFS: u32 = 1 << 4;       // Unknown FIS Interrupt
pub const PORT_IS_DPS: u32 = 1 << 5;       // Descriptor Processed Interrupt
pub const PORT_IS_PCS: u32 = 1 << 6;       // Port Connect Change Status
pub const PORT_IS_DMPS: u32 = 1 << 7;      // Device Mechanical Presence Status
pub const PORT_IS_PRCS: u32 = 1 << 22;     // PhyRdy Change Status
pub const PORT_IS_IPMS: u32 = 1 << 23;     // Incorrect Port Multiplier Status
pub const PORT_IS_OFS: u32 = 1 << 24;      // Overflow Status
pub const PORT_IS_INFS: u32 = 1 << 26;     // Interface Non-fatal Error Status
pub const PORT_IS_IFS: u32 = 1 << 27;      // Interface Fatal Error Status
pub const PORT_IS_HBDS: u32 = 1 << 28;     // Host Bus Data Error Status
pub const PORT_IS_HBFS: u32 = 1 << 29;     // Host Bus Fatal Error Status
pub const PORT_IS_TFES: u32 = 1 << 30;     // Task File Error Status
pub const PORT_IS_CPDS: u32 = 1 << 31;     // Cold Port Detect Status

// Task File Data (TFD) Status Bits.
pub const ATA_DEV_BUSY: u8 = 0x80;         // BSY: Device Busy
pub const ATA_DEV_DRDY: u8 = 0x40;         // DRDY: Device Ready
pub const ATA_DEV_DRQ: u8 = 0x08;          // DRQ: Data Request
pub const ATA_DEV_ERR: u8 = 0x01;          // ERR: Error

// SATA Device Signatures (SIG register).
pub const SATA_SIG_ATA: u32 = 0x0000_0101;  // SATA Hard Disk
pub const SATA_SIG_ATAPI: u32 = 0xEB14_0101;// SATAPI Optical / Tape Device
pub const SATA_SIG_SEMB: u32 = 0xC33C_0101; // Enclosure Management Bridge
pub const SATA_SIG_PM: u32 = 0x9669_0101;   // Port Multiplier

// SATA Status (SSTS) Detection Constants.
pub const SSTS_DET_MASK: u32 = 0x0F;
pub const SSTS_DET_PRESENT_PHY: u32 = 0x03; // Device detected and Phy connection established
pub const SSTS_IPM_MASK: u32 = 0x0F00;
pub const SSTS_IPM_ACTIVE: u32 = 0x0100;    // Interface in active state

/// SATA Device Type Classification.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SataDeviceType {
    None,
    Sata,
    Satapi,
    Semb,
    PortMultiplier,
    Unknown(u32),
}

impl SataDeviceType {
    /// Classifies device type from 32-bit SATA signature register.
    pub fn from_signature(sig: u32) -> Self {
        match sig {
            0 => Self::None,
            SATA_SIG_ATA => Self::Sata,
            SATA_SIG_ATAPI => Self::Satapi,
            SATA_SIG_SEMB => Self::Semb,
            SATA_SIG_PM => Self::PortMultiplier,
            other => Self::Unknown(other),
        }
    }
}

/// Decoded HBA Host Capabilities.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HbaCapabilities {
    pub max_ports: u8,
    pub max_command_slots: u8,
    pub supports_64bit: bool,
    pub supports_ncq: bool,
    pub supports_ahci_only: bool,
    pub interface_speed: u8, // 1: 1.5 Gbps, 2: 3 Gbps, 3: 6 Gbps
}

impl HbaCapabilities {
    pub fn parse(cap: u32) -> Self {
        let max_ports = ((cap & 0x1F) as u8) + 1;
        let max_command_slots = (((cap >> 8) & 0x1F) as u8) + 1;
        let interface_speed = ((cap >> 20) & 0x0F) as u8;
        let supports_64bit = (cap & CAP_S64A) != 0;
        let supports_ncq = (cap & CAP_SNCQ) != 0;
        let supports_ahci_only = (cap & CAP_SAM) != 0;

        Self {
            max_ports,
            max_command_slots,
            supports_64bit,
            supports_ncq,
            supports_ahci_only,
            interface_speed,
        }
    }
}

/// Raw representation of an AHCI Port MMIO Register Window (128 bytes).
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct HbaPortRegisters {
    pub clb: u32,
    pub clbu: u32,
    pub fb: u32,
    pub fbu: u32,
    pub is: u32,
    pub ie: u32,
    pub cmd: u32,
    pub reserved0: u32,
    pub tfd: u32,
    pub sig: u32,
    pub ssts: u32,
    pub sctl: u32,
    pub serr: u32,
    pub sact: u32,
    pub ci: u32,
    pub sntf: u32,
    pub fbs: u32,
    pub reserved1: [u32; 11],
    pub vendor_specific: [u32; 4],
}

// AHCI Command Headers, Command Tables, PRDT Lists, and SATA FIS Structures.

pub const MAX_COMMAND_SLOTS: usize = 32;
pub const MAX_PRDT_ENTRIES: usize = 16;
pub const PRDT_MAX_BYTES: usize = 4 * 1024 * 1024; // 4 MiB max per PRDT descriptor
pub const PRDT_IOC: u32 = 1 << 31;                 // Interrupt on Completion flag

// SATA FIS Types (Serial ATA Revision 3.0 Section 10.3).
pub const FIS_TYPE_REG_H2D: u8 = 0x27;   // Register FIS - Host to Device
pub const FIS_TYPE_REG_D2H: u8 = 0x34;   // Register FIS - Device to Host
pub const FIS_TYPE_DMA_ACT: u8 = 0x39;   // DMA Activate FIS - Device to Host
pub const FIS_TYPE_DMA_SETUP: u8 = 0x41; // DMA Setup FIS - Bidirectional
pub const FIS_TYPE_DATA: u8 = 0x46;      // Data FIS - Bidirectional
pub const FIS_TYPE_BIST: u8 = 0x58;      // BIST Activate FIS - Bidirectional
pub const FIS_TYPE_PIO_SETUP: u8 = 0x5F; // PIO Setup FIS - Device to Host
pub const FIS_TYPE_DEV_BITS: u8 = 0xA1;  // Set Device Bits FIS - Device to Host

// ATA Command Opcode Constants.
pub const ATA_CMD_READ_DMA_EXT: u8 = 0x25;
pub const ATA_CMD_WRITE_DMA_EXT: u8 = 0x35;
pub const ATA_CMD_IDENTIFY: u8 = 0xEC;
pub const ATA_CMD_IDENTIFY_PACKET: u8 = 0xA1;

/// AHCI Command Header Structure (32 bytes per command slot, 1 KiB total for 32 slots).
#[repr(C, packed)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HbaCommandHeader {
    pub flags: u16,        // CFL (bits 4:0), A (bit 5), W (bit 6), P (bit 7), R (bit 8), B (bit 9), C (bit 10), PMP (bits 15:12)
    pub prdtl: u16,        // Physical Region Descriptor Table length (number of PRDT entries)
    pub prdbc: u32,        // Physical Region Descriptor Byte Count transferred
    pub ctba: u32,         // Command Table Descriptor Base Address lower 32-bit (128-byte aligned)
    pub ctbau: u32,        // Command Table Descriptor Base Address upper 32-bit
    pub reserved: [u32; 4],// Reserved (16 bytes)
}

impl HbaCommandHeader {
    pub const FLAG_WRITE: u16 = 1 << 6;
    pub const FLAG_ATAPI: u16 = 1 << 5;
    pub const FLAG_PREFETCH: u16 = 1 << 7;
    pub const FLAG_CLEAR_BSY: u16 = 1 << 10;

    /// Builds a standard ATA command header.
    pub fn new(ctba_phys: u64, prdt_count: u16, is_write: bool, fis_dwords: u8) -> Self {
        let mut flags = (fis_dwords as u16) & 0x1F;
        if is_write {
            flags |= Self::FLAG_WRITE;
        }
        flags |= Self::FLAG_PREFETCH;

        Self {
            flags,
            prdtl: prdt_count,
            prdbc: 0,
            ctba: (ctba_phys & 0xFFFF_FFFF) as u32,
            ctbau: ((ctba_phys >> 32) & 0xFFFF_FFFF) as u32,
            reserved: [0u32; 4],
        }
    }

    #[inline(always)]
    pub fn command_table_address(&self) -> u64 {
        (self.ctba as u64) | ((self.ctbau as u64) << 32)
    }

    #[inline(always)]
    pub fn is_write(&self) -> bool {
        (self.flags & Self::FLAG_WRITE) != 0
    }
}

/// Physical Region Descriptor Table (PRDT) Entry (16 bytes).
#[repr(C, packed)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HbaPrdtEntry {
    pub dba: u32,          // Data Base Address lower 32-bit
    pub dbau: u32,         // Data Base Address upper 32-bit
    pub reserved: u32,     // Reserved (0)
    pub dbc: u32,          // Byte count (bits 21:0, 0-based), bit 31 = Interrupt on Completion (IOC)
}

impl HbaPrdtEntry {
    pub const IOC_FLAG: u32 = 1 << 31;

    /// Creates a PRDT entry with given physical buffer address, byte count, and IOC flag.
    pub fn new(data_phys: u64, byte_count: u32, ioc: bool) -> Self {
        // DBC is 0-based (byte_count - 1), but bit 0 must be 1 for word alignment in AHCI
        let count_field = if byte_count > 0 { byte_count - 1 } else { 0 };
        let dbc = (count_field & 0x003F_FFFF) | if ioc { Self::IOC_FLAG } else { 0 };

        Self {
            dba: (data_phys & 0xFFFF_FFFF) as u32,
            dbau: ((data_phys >> 32) & 0xFFFF_FFFF) as u32,
            reserved: 0,
            dbc,
        }
    }

    #[inline(always)]
    pub fn data_address(&self) -> u64 {
        (self.dba as u64) | ((self.dbau as u64) << 32)
    }

    #[inline(always)]
    pub fn byte_count(&self) -> u32 {
        (self.dbc & 0x003F_FFFF) + 1
    }
}

/// Host to Device (H2D) Register FIS Structure (20 bytes).
#[repr(C, packed)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FisRegH2D {
    pub fis_type: u8,      // FIS_TYPE_REG_H2D (0x27)
    pub pmport_c: u8,      // Bit 7: 1 = Command, 0 = Control; Bits 3:0: Port Multiplier port
    pub command: u8,       // ATA Command Opcode
    pub feature_low: u8,   // Features register bits 7:0
    pub lba0: u8,          // LBA bits 7:0
    pub lba1: u8,          // LBA bits 15:8
    pub lba2: u8,          // LBA bits 23:16
    pub device: u8,        // Device register (Bit 6: 1 = LBA mode)
    pub lba3: u8,          // LBA bits 31:24
    pub lba4: u8,          // LBA bits 39:32
    pub lba5: u8,          // LBA bits 47:40
    pub feature_high: u8,  // Features register bits 15:8
    pub count_low: u8,     // Sector count bits 7:0
    pub count_high: u8,    // Sector count bits 15:8
    pub icc: u8,           // Isochronous command completion
    pub control: u8,       // Device control register
    pub reserved: [u8; 4], // Reserved
}

impl FisRegH2D {
    pub const C_BIT_COMMAND: u8 = 1 << 7;
    pub const DEV_LBA_MODE: u8 = 1 << 6;

    /// Constructs an ATA IDENTIFY DEVICE (0xEC) FIS.
    pub fn new_identify() -> Self {
        Self {
            fis_type: FIS_TYPE_REG_H2D,
            pmport_c: Self::C_BIT_COMMAND,
            command: ATA_CMD_IDENTIFY,
            feature_low: 0,
            lba0: 0,
            lba1: 0,
            lba2: 0,
            device: Self::DEV_LBA_MODE,
            lba3: 0,
            lba4: 0,
            lba5: 0,
            feature_high: 0,
            count_low: 0,
            count_high: 0,
            icc: 0,
            control: 0,
            reserved: [0u8; 4],
        }
    }

    /// Constructs an ATA READ DMA EXT (48-bit LBA) command FIS.
    pub fn new_read_dma_ext(lba: u64, sector_count: u16) -> Self {
        Self {
            fis_type: FIS_TYPE_REG_H2D,
            pmport_c: Self::C_BIT_COMMAND,
            command: ATA_CMD_READ_DMA_EXT,
            feature_low: 0,
            lba0: (lba & 0xFF) as u8,
            lba1: ((lba >> 8) & 0xFF) as u8,
            lba2: ((lba >> 16) & 0xFF) as u8,
            device: Self::DEV_LBA_MODE,
            lba3: ((lba >> 24) & 0xFF) as u8,
            lba4: ((lba >> 32) & 0xFF) as u8,
            lba5: ((lba >> 40) & 0xFF) as u8,
            feature_high: 0,
            count_low: (sector_count & 0xFF) as u8,
            count_high: ((sector_count >> 8) & 0xFF) as u8,
            icc: 0,
            control: 0,
            reserved: [0u8; 4],
        }
    }

    /// Constructs an ATA WRITE DMA EXT (48-bit LBA) command FIS.
    pub fn new_write_dma_ext(lba: u64, sector_count: u16) -> Self {
        Self {
            fis_type: FIS_TYPE_REG_H2D,
            pmport_c: Self::C_BIT_COMMAND,
            command: ATA_CMD_WRITE_DMA_EXT,
            feature_low: 0,
            lba0: (lba & 0xFF) as u8,
            lba1: ((lba >> 8) & 0xFF) as u8,
            lba2: ((lba >> 16) & 0xFF) as u8,
            device: Self::DEV_LBA_MODE,
            lba3: ((lba >> 24) & 0xFF) as u8,
            lba4: ((lba >> 32) & 0xFF) as u8,
            lba5: ((lba >> 40) & 0xFF) as u8,
            feature_high: 0,
            count_low: (sector_count & 0xFF) as u8,
            count_high: ((sector_count >> 8) & 0xFF) as u8,
            icc: 0,
            control: 0,
            reserved: [0u8; 4],
        }
    }
}

/// AHCI Command Table (128 bytes base + PRDT entries).
#[repr(C)]
#[derive(Debug, Clone)]
pub struct HbaCommandTable {
    pub cfis: [u8; 64],
    pub acmd: [u8; 16],
    pub reserved: [u8; 48],
    pub prdt_entries: [HbaPrdtEntry; MAX_PRDT_ENTRIES],
}

impl HbaCommandTable {
    pub fn new() -> Self {
        Self {
            cfis: [0u8; 64],
            acmd: [0u8; 16],
            reserved: [0u8; 48],
            prdt_entries: [HbaPrdtEntry::new(0, 0, false); MAX_PRDT_ENTRIES],
        }
    }

    pub fn set_cfis(&mut self, h2d: &FisRegH2D) {
        unsafe {
            let src = h2d as *const FisRegH2D as *const u8;
            core::ptr::copy_nonoverlapping(src, self.cfis.as_mut_ptr(), core::mem::size_of::<FisRegH2D>());
        }
    }
}

/// Received FIS buffer layout (256 bytes per port).
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct HbaReceivedFis {
    pub dsfis: [u8; 28],    // 0x00: DMA Setup FIS
    pub reserved0: [u8; 4], // 0x1C
    pub psfis: [u8; 20],    // 0x20: PIO Setup FIS
    pub reserved1: [u8; 12],// 0x34
    pub rfis: [u8; 20],     // 0x40: D2H Register FIS
    pub reserved2: [u8; 4], // 0x54
    pub sdbfis: [u8; 8],    // 0x58: Set Device Bits FIS
    pub ufis: [u8; 64],     // 0x60: Unknown FIS
    pub reserved3: [u8; 96],// 0xA0: Reserved
}

/// Self-test verifying AHCI structure sizing, offsets, and signature classification.
pub fn verify_ahci_structures() -> bool {
    // 1. Verify HbaPortRegisters size is exactly 128 bytes (0x80)
    if core::mem::size_of::<HbaPortRegisters>() != 128 {
        return false;
    }

    // 2. Verify signature translation
    if SataDeviceType::from_signature(SATA_SIG_ATA) != SataDeviceType::Sata {
        return false;
    }
    if SataDeviceType::from_signature(SATA_SIG_ATAPI) != SataDeviceType::Satapi {
        return false;
    }
    if SataDeviceType::from_signature(SATA_SIG_SEMB) != SataDeviceType::Semb {
        return false;
    }
    if SataDeviceType::from_signature(SATA_SIG_PM) != SataDeviceType::PortMultiplier {
        return false;
    }
    if SataDeviceType::from_signature(0) != SataDeviceType::None {
        return false;
    }

    // 3. Verify Capability Parsing
    let raw_cap: u32 = 0x8030_1F1F;
    let caps = HbaCapabilities::parse(raw_cap);
    if caps.max_ports != 32 || caps.max_command_slots != 32 || !caps.supports_64bit {
        return false;
    }
    if caps.interface_speed != 3 {
        return false;
    }

    // 4. Verify Port Register byte offsets
    if AHCI_PORT_BASE + 0 * AHCI_PORT_STRIDE != 0x100 {
        return false;
    }
    if AHCI_PORT_BASE + 1 * AHCI_PORT_STRIDE != 0x180 {
        return false;
    }

    true
}

/// Self-test verifying Command Headers, PRDT scatter-gather descriptors, and FIS structures.
pub fn verify_ahci_fis_and_commands() -> bool {
    // 1. Verify Command Header and PRDT sizing
    if core::mem::size_of::<HbaCommandHeader>() != 32 {
        return false;
    }
    if core::mem::size_of::<HbaPrdtEntry>() != 16 {
        return false;
    }
    if core::mem::size_of::<FisRegH2D>() != 20 {
        return false;
    }
    if core::mem::size_of::<HbaReceivedFis>() != 256 {
        return false;
    }

    // 2. Verify Command Header initialization
    let cmd_header = HbaCommandHeader::new(0x1_0000_1000, 1, false, 5);
    if cmd_header.command_table_address() != 0x1_0000_1000 || cmd_header.prdtl != 1 {
        return false;
    }
    if cmd_header.is_write() {
        return false;
    }

    let write_header = HbaCommandHeader::new(0x1_0000_2000, 2, true, 5);
    if !write_header.is_write() {
        return false;
    }

    // 3. Verify PRDT Entry descriptor
    let prdt = HbaPrdtEntry::new(0x2_0000_0000, 4096, true);
    if prdt.data_address() != 0x2_0000_0000 || prdt.byte_count() != 4096 {
        return false;
    }
    if (prdt.dbc & HbaPrdtEntry::IOC_FLAG) == 0 {
        return false;
    }

    // 4. Verify H2D FIS Construction
    let identify = FisRegH2D::new_identify();
    if identify.fis_type != FIS_TYPE_REG_H2D || identify.command != ATA_CMD_IDENTIFY {
        return false;
    }
    if (identify.pmport_c & FisRegH2D::C_BIT_COMMAND) == 0 {
        return false;
    }

    let read_dma = FisRegH2D::new_read_dma_ext(0x0123_4567_89AB, 64);
    if read_dma.command != ATA_CMD_READ_DMA_EXT {
        return false;
    }
    if read_dma.lba0 != 0xAB || read_dma.lba1 != 0x89 || read_dma.lba2 != 0x67 || read_dma.lba3 != 0x45 {
        return false;
    }
    if read_dma.count_low != 64 || read_dma.count_high != 0 {
        return false;
    }

    let write_dma = FisRegH2D::new_write_dma_ext(0x42, 1);
    if write_dma.command != ATA_CMD_WRITE_DMA_EXT || write_dma.lba0 != 0x42 {
        return false;
    }

    true
}


/// Error variants for AHCI SATA operations.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AhciError {
    NotAhciDevice,
    MissingBar5,
    ResetTimeout,
    PortTimeout,
    NoFreeSlot,
    DeviceError,
    BufferMisaligned,
    InvalidSectorCount,
    DeviceNotPresent,
}

/// Decoded ATA IDENTIFY DEVICE (0xEC) data buffer (512 bytes).
#[derive(Debug, Clone, Copy)]
pub struct AtaIdentifyData {
    pub serial_number: [u8; 20],
    pub firmware_revision: [u8; 8],
    pub model_number: [u8; 40],
    pub lba48_supported: bool,
    pub sector_count: u64,
}

impl AtaIdentifyData {
    pub fn parse(raw: &[u8; 512]) -> Self {
        let mut words = [0u16; 256];
        for i in 0..256 {
            words[i] = u16::from_le_bytes([raw[i * 2], raw[i * 2 + 1]]);
        }

        let mut serial_number = [b' '; 20];
        Self::copy_swapped_string(&words[10..20], &mut serial_number);

        let mut firmware_revision = [b' '; 8];
        Self::copy_swapped_string(&words[23..27], &mut firmware_revision);

        let mut model_number = [b' '; 40];
        Self::copy_swapped_string(&words[27..47], &mut model_number);

        let lba48_supported = (words[83] & (1 << 10)) != 0;

        let sector_count = if lba48_supported {
            (words[100] as u64)
                | ((words[101] as u64) << 16)
                | ((words[102] as u64) << 32)
                | ((words[103] as u64) << 48)
        } else {
            (words[60] as u64) | ((words[61] as u64) << 16)
        };

        Self {
            serial_number,
            firmware_revision,
            model_number,
            lba48_supported,
            sector_count,
        }
    }

    fn copy_swapped_string(words: &[u16], out: &mut [u8]) {
        for (i, &w) in words.iter().enumerate() {
            if i * 2 + 1 < out.len() {
                out[i * 2] = (w >> 8) as u8;
                out[i * 2 + 1] = (w & 0xFF) as u8;
            }
        }
    }

    #[inline(always)]
    pub fn capacity_bytes(&self) -> u64 {
        self.sector_count.saturating_mul(512)
    }
}

/// AHCI Port Driver abstraction.
#[derive(Debug, Clone)]
pub struct AhciPort {
    pub port_index: u8,
    pub device_type: SataDeviceType,
    pub port_mmio: usize,
    pub cmd_list_phys: u64,
    pub fis_phys: u64,
    pub cmd_table_phys: u64,
    pub identify: Option<AtaIdentifyData>,
    pub sector_count: u64,
    pub sector_size: u32,
}

impl AhciPort {
    pub fn new(port_index: u8, port_mmio: usize) -> Self {
        Self {
            port_index,
            device_type: SataDeviceType::None,
            port_mmio,
            cmd_list_phys: 0,
            fis_phys: 0,
            cmd_table_phys: 0,
            identify: None,
            sector_count: 0,
            sector_size: 512,
        }
    }

    pub fn set_dma_buffers(&mut self, cmd_list_phys: u64, fis_phys: u64, cmd_table_phys: u64) {
        self.cmd_list_phys = cmd_list_phys;
        self.fis_phys = fis_phys;
        self.cmd_table_phys = cmd_table_phys;
    }

    /// Finds first available command slot (from 0 to 31) where CI and SACT bits are clear.
    pub fn find_cmd_slot(&self, ci: u32, sact: u32) -> Option<u8> {
        let slots = ci | sact;
        for i in 0..MAX_COMMAND_SLOTS as u8 {
            if (slots & (1 << i)) == 0 {
                return Some(i);
            }
        }
        None
    }

    /// Prepares command header and table for ATA IDENTIFY DEVICE command.
    pub fn build_identify_command(
        &self,
        cmd_header: &mut HbaCommandHeader,
        cmd_table: &mut HbaCommandTable,
        buffer_phys: u64,
    ) {
        *cmd_header = HbaCommandHeader::new(self.cmd_table_phys, 1, false, 5);
        cmd_table.prdt_entries[0] = HbaPrdtEntry::new(buffer_phys, 512, true);
        let fis = FisRegH2D::new_identify();
        cmd_table.set_cfis(&fis);
    }

    /// Prepares command header and table for ATA READ DMA EXT (48-bit LBA) command.
    pub fn build_read_dma_command(
        &self,
        cmd_header: &mut HbaCommandHeader,
        cmd_table: &mut HbaCommandTable,
        buffer_phys: u64,
        lba: u64,
        sector_count: u16,
    ) {
        *cmd_header = HbaCommandHeader::new(self.cmd_table_phys, 1, false, 5);
        let total_bytes = (sector_count as u32) * self.sector_size;
        cmd_table.prdt_entries[0] = HbaPrdtEntry::new(buffer_phys, total_bytes, true);
        let fis = FisRegH2D::new_read_dma_ext(lba, sector_count);
        cmd_table.set_cfis(&fis);
    }

    /// Prepares command header and table for ATA WRITE DMA EXT (48-bit LBA) command.
    pub fn build_write_dma_command(
        &self,
        cmd_header: &mut HbaCommandHeader,
        cmd_table: &mut HbaCommandTable,
        buffer_phys: u64,
        lba: u64,
        sector_count: u16,
    ) {
        *cmd_header = HbaCommandHeader::new(self.cmd_table_phys, 1, true, 5);
        let total_bytes = (sector_count as u32) * self.sector_size;
        cmd_table.prdt_entries[0] = HbaPrdtEntry::new(buffer_phys, total_bytes, true);
        let fis = FisRegH2D::new_write_dma_ext(lba, sector_count);
        cmd_table.set_cfis(&fis);
    }
}

/// AHCI Host Bus Adapter Controller Instance.
pub struct AhciController {
    pub pci_bus: u8,
    pub pci_slot: u8,
    pub pci_func: u8,
    pub vendor_id: u16,
    pub device_id: u16,
    pub abar: u64,
    pub abar_size: u64,
    pub caps: HbaCapabilities,
    pub ports_implemented: u32,
    pub ports: [Option<AhciPort>; MAX_PORTS],
}

impl AhciController {
    pub fn new(abar: u64, abar_size: u64) -> Self {
        const NONE_PORT: Option<AhciPort> = None;
        Self {
            pci_bus: 0,
            pci_slot: 0,
            pci_func: 0,
            vendor_id: 0,
            device_id: 0,
            abar,
            abar_size,
            caps: HbaCapabilities {
                max_ports: 32,
                max_command_slots: 32,
                supports_64bit: true,
                supports_ncq: true,
                supports_ahci_only: true,
                interface_speed: 3,
            },
            ports_implemented: 0,
            ports: [NONE_PORT; MAX_PORTS],
        }
    }

    pub fn from_pci(dev: &PciDevice) -> Result<Self, AhciError> {
        if dev.class_code != PCI_CLASS_MASS_STORAGE || dev.subclass != PCI_SUBCLASS_SATA {
            return Err(AhciError::NotAhciDevice);
        }

        let bar5 = dev.get_bar(5).ok_or(AhciError::MissingBar5)?;
        if bar5.address == 0 {
            return Err(AhciError::MissingBar5);
        }

        let mut controller = Self::new(bar5.address, bar5.size);
        controller.pci_bus = dev.bus;
        controller.pci_slot = dev.slot;
        controller.pci_func = dev.function;
        controller.vendor_id = dev.vendor_id;
        controller.device_id = dev.device_id;

        Ok(controller)
    }

    pub fn port_offset(&self, index: usize) -> usize {
        AHCI_PORT_BASE + (index * AHCI_PORT_STRIDE)
    }

    pub fn primary_disk_port(&self) -> Option<u8> {
        for i in 0..MAX_PORTS {
            if let Some(port) = &self.ports[i] {
                if port.device_type == SataDeviceType::Sata {
                    return Some(i as u8);
                }
            }
        }
        None
    }
}

/// Block Storage Device abstraction for AHCI SATA Disks.
#[derive(Debug, Clone)]
pub struct AhciBlockDevice {
    pub port_index: u8,
    pub sector_count: u64,
    pub sector_size: u32,
    pub model: [u8; 40],
    pub serial: [u8; 20],
}

impl AhciBlockDevice {
    pub fn new(port_index: u8, identify: &AtaIdentifyData) -> Self {
        Self {
            port_index,
            sector_count: identify.sector_count,
            sector_size: 512,
            model: identify.model_number,
            serial: identify.serial_number,
        }
    }

    #[inline(always)]
    pub fn capacity_bytes(&self) -> u64 {
        self.sector_count.saturating_mul(self.sector_size as u64)
    }

    #[inline(always)]
    pub fn sector_count(&self) -> u64 {
        self.sector_count
    }
}

/// Self-test verifying ATA IDENTIFY parsing, command slot selection, and DMA command generation.
pub fn verify_ahci_port_engine() -> bool {
    // 1. Verify ATA IDENTIFY parsing
    let mut raw_identify = [0u8; 512];
    // Word 83 bit 10 set: 48-bit LBA supported
    raw_identify[83 * 2] = 0x00;
    raw_identify[83 * 2 + 1] = 0x04; // 1 << 10 = 0x0400 (high byte = 0x04)

    // Words 100..103: 2,097,152 sectors (1 GiB) -> 0x0020_0000
    // Word 100: 0x0000, Word 101: 0x0020
    raw_identify[100 * 2] = 0x00;
    raw_identify[100 * 2 + 1] = 0x00;
    raw_identify[101 * 2] = 0x20;
    raw_identify[101 * 2 + 1] = 0x00;

    let identify = AtaIdentifyData::parse(&raw_identify);
    if !identify.lba48_supported {
        return false;
    }
    if identify.sector_count != 2_097_152 {
        return false;
    }
    if identify.capacity_bytes() != 1_073_741_824 {
        return false;
    }

    // 2. Verify Port slot allocation
    let mut port = AhciPort::new(0, 0x100);
    port.set_dma_buffers(0x1_0000, 0x2_0000, 0x3_0000);

    // Slots 0, 1, 2 busy -> slot 3 free
    let slot = port.find_cmd_slot(0b0101, 0b0010);
    if slot != Some(3) {
        return false;
    }

    // 3. Verify IDENTIFY Command construction
    let mut header = HbaCommandHeader::new(0, 0, false, 0);
    let mut table = HbaCommandTable::new();
    port.build_identify_command(&mut header, &mut table, 0x5_0000);
    if header.command_table_address() != 0x3_0000 || header.prdtl != 1 {
        return false;
    }
    if table.prdt_entries[0].data_address() != 0x5_0000 || table.prdt_entries[0].byte_count() != 512 {
        return false;
    }
    if table.cfis[0] != FIS_TYPE_REG_H2D || table.cfis[2] != ATA_CMD_IDENTIFY {
        return false;
    }

    // 4. Verify READ DMA EXT command construction
    port.build_read_dma_command(&mut header, &mut table, 0x6_0000, 0x1000, 8);
    if header.is_write() {
        return false;
    }
    if table.prdt_entries[0].byte_count() != 4096 {
        return false;
    }
    if table.cfis[2] != ATA_CMD_READ_DMA_EXT || table.cfis[7] != FisRegH2D::DEV_LBA_MODE {
        return false;
    }

    // 5. Verify WRITE DMA EXT command construction
    port.build_write_dma_command(&mut header, &mut table, 0x7_0000, 0x2000, 2);
    if !header.is_write() {
        return false;
    }
    if table.prdt_entries[0].byte_count() != 1024 {
        return false;
    }
    if table.cfis[2] != ATA_CMD_WRITE_DMA_EXT {
        return false;
    }

    // 6. Verify Block Device abstraction
    let block_dev = AhciBlockDevice::new(0, &identify);
    if block_dev.capacity_bytes() != 1_073_741_824 || block_dev.sector_count() != 2_097_152 {
        return false;
    }

    true
}
