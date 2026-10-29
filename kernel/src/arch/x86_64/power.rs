//! ACPI Power Management Subsystem (FADT/DSDT S3/S5, CPU Throttling, Reset & Shutdown).
//!
//! Provides parsing of Fixed ACPI Description Table (FADT), power management register blocks
//! (PM1a/PM1b Control, SMI command), reset register mechanisms, and sleep state transitions.

use super::acpi::{AcpiHeader, AcpiParser};
use crate::serial_write_str;

pub const FADT_SIGNATURE: &[u8; 4] = b"FACP";
pub const DSDT_SIGNATURE: &[u8; 4] = b"DSDT";

pub const GAS_ADDR_SPACE_SYS_MEM: u8 = 0;
pub const GAS_ADDR_SPACE_SYS_IO: u8 = 1;
pub const GAS_ADDR_SPACE_PCI_CFG: u8 = 2;

// PM1_CNT bitmask flags
pub const PM1_SCI_EN: u16 = 1 << 0;
pub const PM1_BM_RLD: u16 = 1 << 1;
pub const PM1_GBL_RLS: u16 = 1 << 2;
pub const PM1_SLP_TYP_MASK: u16 = 0x7 << 10;
pub const PM1_SLP_EN: u16 = 1 << 13;

// Common x86 PC reset registers
pub const PORT_KBD_RESET: u16 = 0x0064;
pub const PORT_CF9_RESET: u16 = 0x0CF9;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PowerState {
    S0Working = 0,
    S1CpuStop = 1,
    S2CpuPowerOff = 2,
    S3SuspendToRam = 3,
    S4Hibernate = 4,
    S5SoftOff = 5,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct SleepTypeConfig {
    pub slp_typa: u16,
    pub slp_typb: u16,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PowerError {
    InvalidSignature,
    InvalidLength,
    InvalidChecksum,
    TableNotFound,
    UnsupportedState,
    HardwareFailure,
}

/// ACPI Generic Address Structure (GAS) matching ACPI 2.0+ specifications.
#[repr(C, packed)]
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct GenericAddressStructure {
    pub address_space_id: u8,
    pub register_bit_width: u8,
    pub register_bit_offset: u8,
    pub access_size: u8,
    pub address: u64,
}

/// Extracted Power Management parameters from Fixed ACPI Description Table (FADT).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FadtInfo {
    pub pm1a_cnt_blk: u32,
    pub pm1b_cnt_blk: u32,
    pub smi_cmd: u32,
    pub acpi_enable: u8,
    pub acpi_disable: u8,
    pub reset_reg: GenericAddressStructure,
    pub reset_value: u8,
    pub dsdt_address: u64,
    pub flags: u32,
    pub century: u8,
    pub preferred_pm_profile: u8,
}

impl Default for FadtInfo {
    fn default() -> Self {
        // Standard QEMU Q35 / PC default fallback values
        Self {
            pm1a_cnt_blk: 0x0604,
            pm1b_cnt_blk: 0,
            smi_cmd: 0x00B2,
            acpi_enable: 0x02,
            acpi_disable: 0x03,
            reset_reg: GenericAddressStructure {
                address_space_id: GAS_ADDR_SPACE_SYS_IO,
                register_bit_width: 8,
                register_bit_offset: 0,
                access_size: 1,
                address: 0x0CF9,
            },
            reset_value: 0x06, // System reset via 0xCF9
            dsdt_address: 0,
            flags: 0,
            century: 0,
            preferred_pm_profile: 1, // Desktop
        }
    }
}

impl FadtInfo {
    /// Parses an ACPI FADT table from raw bytes.
    pub fn parse(raw: &[u8]) -> Result<Self, PowerError> {
        if raw.len() < 116 {
            return Err(PowerError::InvalidLength);
        }

        // Validate table signature (must be "FACP")
        if &raw[0..4] != FADT_SIGNATURE {
            return Err(PowerError::InvalidSignature);
        }

        let mut sum: u8 = 0;
        let declared_len = u32::from_le_bytes([raw[4], raw[5], raw[6], raw[7]]) as usize;
        let check_len = core::cmp::min(declared_len, raw.len());
        for &b in &raw[..check_len] {
            sum = sum.wrapping_add(b);
        }
        if sum != 0 {
            return Err(PowerError::InvalidChecksum);
        }

        let dsdt_32 = u32::from_le_bytes([raw[40], raw[41], raw[42], raw[43]]) as u64;
        let preferred_pm_profile = raw[45];
        let smi_cmd = u32::from_le_bytes([raw[48], raw[49], raw[50], raw[51]]);
        let acpi_enable = raw[52];
        let acpi_disable = raw[53];
        let pm1a_cnt_blk = u32::from_le_bytes([raw[64], raw[65], raw[66], raw[67]]);
        let pm1b_cnt_blk = u32::from_le_bytes([raw[68], raw[69], raw[70], raw[71]]);
        let century = raw[108];
        let flags = u32::from_le_bytes([raw[112], raw[113], raw[114], raw[115]]);

        let mut reset_reg = GenericAddressStructure {
            address_space_id: GAS_ADDR_SPACE_SYS_IO,
            register_bit_width: 8,
            register_bit_offset: 0,
            access_size: 1,
            address: 0x0CF9,
        };
        let mut reset_value = 0x06;
        let mut dsdt_address = dsdt_32;

        // ACPI 2.0+ fields (offset 116..244)
        if raw.len() >= 148 {
            let space_id = raw[116];
            let bit_width = raw[117];
            let bit_offset = raw[118];
            let access_size = raw[119];
            let addr = u64::from_le_bytes([
                raw[120], raw[121], raw[122], raw[123],
                raw[124], raw[125], raw[126], raw[127],
            ]);

            if addr != 0 {
                reset_reg = GenericAddressStructure {
                    address_space_id: space_id,
                    register_bit_width: bit_width,
                    register_bit_offset: bit_offset,
                    access_size,
                    address: addr,
                };
            }
            reset_value = raw[128];

            let x_dsdt = u64::from_le_bytes([
                raw[140], raw[141], raw[142], raw[143],
                raw[144], raw[145], raw[146], raw[147],
            ]);
            if x_dsdt != 0 {
                dsdt_address = x_dsdt;
            }
        }

        Ok(Self {
            pm1a_cnt_blk,
            pm1b_cnt_blk,
            smi_cmd,
            acpi_enable,
            acpi_disable,
            reset_reg,
            reset_value,
            dsdt_address,
            flags,
            century,
            preferred_pm_profile,
        })
    }

    /// Discovers system FADT via ACPI RSDP/XSDT or falls back to standard Q35 values.
    pub fn discover() -> Self {
        serial_write_str("[KNT-POWER] Discovering ACPI FADT power management table...\r\n");

        if let Some(rsdp_ptr) = unsafe { AcpiParser::find_rsdp() } {
            let rsdp = unsafe { &*rsdp_ptr };
            let fadt_header = unsafe {
                if rsdp.revision >= 2 && rsdp.xsdt_address != 0 {
                    AcpiParser::find_table_xsdt(rsdp.xsdt_address as *const AcpiHeader, FADT_SIGNATURE)
                } else if rsdp.rsdt_address != 0 {
                    AcpiParser::find_table_rsdt(rsdp.rsdt_address as *const AcpiHeader, FADT_SIGNATURE)
                } else {
                    None
                }
            };

            if let Some(fadt_ptr) = fadt_header {
                let length = unsafe { (*fadt_ptr).length as usize };
                let raw_slice = unsafe { core::slice::from_raw_parts(fadt_ptr as *const u8, length) };
                if let Ok(info) = Self::parse(raw_slice) {
                    serial_write_str("  * Located ACPI FADT (PM1a_CNT: 0x");
                    write_hex32(info.pm1a_cnt_blk);
                    serial_write_str(", SMI_CMD: 0x");
                    write_hex32(info.smi_cmd);
                    serial_write_str(", Reset: 0x");
                    write_hex32(info.reset_reg.address as u32);
                    serial_write_str(")\r\n");
                    return info;
                }
            }
        }

        serial_write_str("[KNT-POWER] Using default Q35 ACPI power management registers\r\n");
        Self::default()
    }
}

fn write_hex32(val: u32) {
    const HEX_CHARS: &[u8; 16] = b"0123456789ABCDEF";
    for i in (0..8).rev() {
        let nibble = ((val >> (i * 4)) & 0x0F) as usize;
        crate::serial_write_byte(HEX_CHARS[nibble]);
    }
}

/// Verification unit test ensuring ACPI FADT parser extracts registers correctly.
pub fn test_fadt_parser_and_power_structures() -> bool {
    // 1. Build a mock 148-byte ACPI 2.0+ FADT buffer
    let mut mock_fadt = [0u8; 148];

    // Signature "FACP"
    mock_fadt[0..4].copy_from_slice(FADT_SIGNATURE);
    // Length: 148 (0x00000094)
    let len_bytes = 148u32.to_le_bytes();
    mock_fadt[4..8].copy_from_slice(&len_bytes);
    mock_fadt[8] = 2; // Revision 2 (ACPI 2.0+)

    // DSDT 32-bit: 0x07FE0000
    mock_fadt[40..44].copy_from_slice(&0x07FE0000u32.to_le_bytes());
    mock_fadt[45] = 1; // Preferred PM Profile: Desktop

    // SMI_CMD: 0x00B2
    mock_fadt[48..52].copy_from_slice(&0x00B2u32.to_le_bytes());
    mock_fadt[52] = 0x02; // ACPI_ENABLE
    mock_fadt[53] = 0x03; // ACPI_DISABLE

    // PM1a_CNT_BLK: 0x0604
    mock_fadt[64..68].copy_from_slice(&0x0604u32.to_le_bytes());
    // PM1b_CNT_BLK: 0x0000
    mock_fadt[68..72].copy_from_slice(&0x0000u32.to_le_bytes());

    mock_fadt[108] = 0x32; // Century RTC offset (0x32 = 50)
    mock_fadt[112..116].copy_from_slice(&0x00000001u32.to_le_bytes()); // Flags

    // ACPI 2.0+ RESET_REG (offset 116..128)
    mock_fadt[116] = GAS_ADDR_SPACE_SYS_IO; // System I/O
    mock_fadt[117] = 8;                      // 8-bit width
    mock_fadt[118] = 0;                      // Bit offset 0
    mock_fadt[119] = 1;                      // Access size 1 byte
    mock_fadt[120..128].copy_from_slice(&0x0CF9u64.to_le_bytes()); // Address 0xCF9
    mock_fadt[128] = 0x06;                   // RESET_VALUE

    // X_DSDT 64-bit (offset 140..148): 0x0000_0000_07FE_1000
    mock_fadt[140..148].copy_from_slice(&0x07FE1000u64.to_le_bytes());

    // Compute checksum so sum across entire table is 0 mod 256
    let mut sum: u8 = 0;
    for (i, &b) in mock_fadt.iter().enumerate() {
        if i != 9 { // Skip checksum byte at offset 9
            sum = sum.wrapping_add(b);
        }
    }
    mock_fadt[9] = (!sum).wrapping_add(1);

    // 2. Parse mock buffer
    let info = match FadtInfo::parse(&mock_fadt) {
        Ok(i) => i,
        Err(_) => return false,
    };

    assert_eq!(info.pm1a_cnt_blk, 0x0604);
    assert_eq!(info.pm1b_cnt_blk, 0x0000);
    assert_eq!(info.smi_cmd, 0x00B2);
    assert_eq!(info.acpi_enable, 0x02);
    assert_eq!(info.acpi_disable, 0x03);
    let reset_addr = { info.reset_reg.address };
    assert_eq!(info.reset_reg.address_space_id, GAS_ADDR_SPACE_SYS_IO);
    assert_eq!(reset_addr, 0x0CF9);
    assert_eq!(info.reset_value, 0x06);
    assert_eq!(info.dsdt_address, 0x07FE1000);
    assert_eq!(info.century, 0x32);
    assert_eq!(info.preferred_pm_profile, 1);

    // 3. PowerState definitions verification
    assert_eq!(PowerState::S5SoftOff as u8, 5);
    assert_eq!(PowerState::S3SuspendToRam as u8, 3);

    true
}
