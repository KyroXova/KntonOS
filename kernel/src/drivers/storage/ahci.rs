//! AHCI SATA Specification 1.3.1 Memory Registers & Host Controller Structures.
//!
//! Provides the memory-mapped register definitions for the Advanced Host Controller
//! Interface (AHCI), including Generic Host Control (GHC), Port Control registers,
//! and SATA device signature classifications.

pub const PCI_CLASS_MASS_STORAGE: u8 = 0x01;
pub const PCI_SUBCLASS_SATA: u8 = 0x06;
pub const PCI_PROGIF_AHCI: u8 = 0x01;

// --- Generic Host Control (GHC) Register Offsets ---
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

// --- GHC Register Bitmasks ---
pub const GHC_HR: u32 = 1 << 0;            // HBA Reset (0: operational, 1: reset)
pub const GHC_IE: u32 = 1 << 1;            // Global Interrupt Enable
pub const GHC_MRSM: u32 = 1 << 2;          // MSI Revert to Single Message
pub const GHC_AE: u32 = 1 << 31;           // AHCI Enable (1: AHCI mode, 0: legacy IDE)

// --- Host Capabilities (CAP) Bitmasks ---
pub const CAP_S64A: u32 = 1 << 31;         // Supports 64-bit Addressing
pub const CAP_SNCQ: u32 = 1 << 30;         // Supports Native Command Queuing
pub const CAP_SSNTF: u32 = 1 << 29;        // Supports SNotification Register
pub const CAP_SMPS: u32 = 1 << 28;         // Supports Mechanical Presence Switch
pub const CAP_SSS: u32 = 1 << 27;          // Supports Staggered Spin-up
pub const CAP_ALPE: u32 = 1 << 26;         // Supports Aggressive Link Power Management
pub const CAP_SAM: u32 = 1 << 18;          // Supports AHCI Mode Only
pub const CAP_SPM: u32 = 1 << 17;          // Supports Port Multiplier
pub const CAP_FBSS: u32 = 1 << 16;         // Supports FIS-based Switching

// --- BIOS/OS Handoff (BOHC) Bitmasks ---
pub const BOHC_BOS: u32 = 1 << 0;          // BIOS Owned Semaphore
pub const BOHC_OOS: u32 = 1 << 1;          // OS Owned Semaphore
pub const BOHC_SOOE: u32 = 1 << 2;         // SMI on OS Ownership Change
pub const BOHC_OOC: u32 = 1 << 3;          // OS Ownership Change
pub const BOHC_BB: u32 = 1 << 4;           // BIOS Busy

// --- Port Register Offsets & Strides ---
pub const AHCI_PORT_BASE: usize = 0x100;
pub const AHCI_PORT_STRIDE: usize = 0x80;
pub const MAX_PORTS: usize = 32;

// --- Port Registers (Offset within Port structure) ---
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

// --- Port Command (CMD) Bitmasks ---
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

// --- Port Interrupt Status (IS) & Enable (IE) Bitmasks ---
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

// --- Task File Data (TFD) Status Bits ---
pub const ATA_DEV_BUSY: u8 = 0x80;         // BSY: Device Busy
pub const ATA_DEV_DRDY: u8 = 0x40;         // DRDY: Device Ready
pub const ATA_DEV_DRQ: u8 = 0x08;          // DRQ: Data Request
pub const ATA_DEV_ERR: u8 = 0x01;          // ERR: Error

// --- SATA Device Signatures (SIG register) ---
pub const SATA_SIG_ATA: u32 = 0x0000_0101;  // SATA Hard Disk
pub const SATA_SIG_ATAPI: u32 = 0xEB14_0101;// SATAPI Optical / Tape Device
pub const SATA_SIG_SEMB: u32 = 0xC33C_0101; // Enclosure Management Bridge
pub const SATA_SIG_PM: u32 = 0x9669_0101;   // Port Multiplier

// --- SATA Status (SSTS) Detection Constants ---
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
