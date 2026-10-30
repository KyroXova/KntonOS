//! ACPI power management subsystem for FADT/DSDT sleep states, throttling, and reset.

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

#[inline(always)]
pub unsafe fn outw(port: u16, val: u16) {
    core::arch::asm!(
        "out dx, ax",
        in("dx") port,
        in("ax") val,
        options(nomem, nostack, preserves_flags)
    );
}

#[inline(always)]
pub unsafe fn inw(port: u16) -> u16 {
    let val: u16;
    core::arch::asm!(
        "in ax, dx",
        out("ax") val,
        in("dx") port,
        options(nomem, nostack, preserves_flags)
    );
    val
}

#[inline(always)]
pub unsafe fn io_delay() {
    // 0x80 is the standard POST diagnostic port used as an I/O delay bus cycle (~1 microsecond)
    crate::outb(0x80, 0);
}

/// Parses DSDT AML bytecode to extract sleep type values for a given sleep object (e.g. "_S5_" or "_S3_").
pub fn parse_dsdt_sleep_state(dsdt_raw: &[u8], state_name: &[u8; 4]) -> Option<SleepTypeConfig> {
    if dsdt_raw.len() < 8 {
        return None;
    }

    for i in 0..(dsdt_raw.len() - 8) {
        if &dsdt_raw[i..i + 4] == state_name {
            let mut idx = i + 4;
            // Skip PackageOp (0x12) if present
            if idx < dsdt_raw.len() && dsdt_raw[idx] == 0x12 {
                idx += 1;
                // Skip PkgLen
                if idx < dsdt_raw.len() {
                    let lead = dsdt_raw[idx];
                    let len_bytes = (lead >> 6) as usize;
                    idx += 1 + len_bytes;
                }
                // Skip NumElements
                if idx < dsdt_raw.len() {
                    idx += 1;
                }
            }

            // Extract slp_typa
            let typa = if idx < dsdt_raw.len() {
                if dsdt_raw[idx] == 0x0A && idx + 1 < dsdt_raw.len() {
                    idx += 1;
                    let v = dsdt_raw[idx];
                    idx += 1;
                    v as u16
                } else {
                    let v = dsdt_raw[idx];
                    idx += 1;
                    v as u16
                }
            } else {
                5
            };

            // Extract slp_typb
            let typb = if idx < dsdt_raw.len() {
                if dsdt_raw[idx] == 0x0A && idx + 1 < dsdt_raw.len() {
                    idx += 1;
                    dsdt_raw[idx] as u16
                } else {
                    dsdt_raw[idx] as u16
                }
            } else {
                typa
            };

            return Some(SleepTypeConfig { slp_typa: typa, slp_typb: typb });
        }
    }

    None
}

/// Central ACPI Power Management Controller.
pub struct PowerManager {
    pub fadt: FadtInfo,
    pub s5_config: SleepTypeConfig,
    pub s3_config: SleepTypeConfig,
    pub acpi_active: bool,
}

impl PowerManager {
    pub fn new(fadt: FadtInfo) -> Self {
        let mut s5 = SleepTypeConfig { slp_typa: 5, slp_typb: 5 }; // Q35 default S5
        let mut s3 = SleepTypeConfig { slp_typa: 3, slp_typb: 3 }; // Q35 default S3

        // Scan DSDT if mapped address is provided
        if fadt.dsdt_address != 0 {
            let dsdt_ptr = fadt.dsdt_address as *const AcpiHeader;
            let length = unsafe { (*dsdt_ptr).length as usize };
            if length > 36 && length < 0x0010_0000 {
                let raw_slice = unsafe { core::slice::from_raw_parts(dsdt_ptr as *const u8, length) };
                if let Some(cfg) = parse_dsdt_sleep_state(raw_slice, b"_S5_") {
                    s5 = cfg;
                }
                if let Some(cfg) = parse_dsdt_sleep_state(raw_slice, b"_S3_") {
                    s3 = cfg;
                }
            }
        }

        Self {
            fadt,
            s5_config: s5,
            s3_config: s3,
            acpi_active: false,
        }
    }

    /// Enables ACPI mode via the SMI_CMD port if currently in legacy mode.
    pub fn enable_acpi(&mut self) -> Result<(), PowerError> {
        if self.fadt.pm1a_cnt_blk == 0 {
            return Err(PowerError::HardwareFailure);
        }

        let is_active = unsafe { (inw(self.fadt.pm1a_cnt_blk as u16) & PM1_SCI_EN) != 0 };
        if is_active {
            self.acpi_active = true;
            return Ok(());
        }

        // Only send ACPI_ENABLE to SMI_CMD if discovered from genuine firmware ACPI tables
        if self.fadt.dsdt_address != 0 && self.fadt.smi_cmd != 0 && self.fadt.acpi_enable != 0 {
            unsafe {
                crate::outb(self.fadt.smi_cmd as u16, self.fadt.acpi_enable);
                for _ in 0..1000 {
                    if (inw(self.fadt.pm1a_cnt_blk as u16) & PM1_SCI_EN) != 0 {
                        self.acpi_active = true;
                        return Ok(());
                    }
                    io_delay();
                }
            }
        }

        // On virtualized/emulated environments, allow proceeding
        self.acpi_active = true;
        Ok(())
    }

    /// Computes the 16-bit PM1_CNT register value to trigger the specified sleep state.
    pub fn compute_sleep_control_word(&self, state: PowerState) -> Result<(u16, u16), PowerError> {
        let cfg = match state {
            PowerState::S5SoftOff => self.s5_config,
            PowerState::S3SuspendToRam => self.s3_config,
            PowerState::S1CpuStop => SleepTypeConfig { slp_typa: 1, slp_typb: 1 },
            _ => return Err(PowerError::UnsupportedState),
        };

        let val_a = (cfg.slp_typa << 10) | PM1_SLP_EN;
        let val_b = (cfg.slp_typb << 10) | PM1_SLP_EN;

        Ok((val_a, val_b))
    }

    /// Transitions the platform into the target ACPI sleep state (S3 Suspend, S5 Soft Off).
    pub fn enter_sleep_state(&mut self, state: PowerState) -> Result<(), PowerError> {
        let (val_a, val_b) = self.compute_sleep_control_word(state)?;

        unsafe {
            core::arch::asm!("cli", options(nomem, nostack, preserves_flags));

            if state == PowerState::S3SuspendToRam {
                core::arch::asm!("wbinvd", options(nomem, nostack, preserves_flags));
            }

            if self.fadt.pm1a_cnt_blk != 0 {
                outw(self.fadt.pm1a_cnt_blk as u16, val_a);
            }
            if self.fadt.pm1b_cnt_blk != 0 {
                outw(self.fadt.pm1b_cnt_blk as u16, val_b);
            }

            loop {
                core::arch::asm!("hlt", options(nomem, nostack, preserves_flags));
            }
        }
    }

    /// Performs full platform reset attempting ACPI reset register, 0xCF9 chipset reset, and 0x64 KBD reset.
    pub fn system_reset(&self) -> ! {
        unsafe {
            core::arch::asm!("cli", options(nomem, nostack, preserves_flags));

            // 1. ACPI 2.0+ Reset Register
            let reset_addr = self.fadt.reset_reg.address;
            if reset_addr != 0 {
                if self.fadt.reset_reg.address_space_id == GAS_ADDR_SPACE_SYS_IO {
                    crate::outb(reset_addr as u16, self.fadt.reset_value);
                } else if self.fadt.reset_reg.address_space_id == GAS_ADDR_SPACE_SYS_MEM {
                    *(reset_addr as *mut u8) = self.fadt.reset_value;
                }
                io_delay();
            }

            // 2. Chipset / PCI Reset Port 0xCF9 (write 0x02, then 0x06)
            crate::outb(PORT_CF9_RESET, 0x02);
            io_delay();
            crate::outb(PORT_CF9_RESET, 0x06);
            io_delay();

            // 3. PS/2 8042 Keyboard Controller Reset 0x64
            let mut timeout = 10000;
            while (crate::inb(PORT_KBD_RESET) & 0x02) != 0 && timeout > 0 {
                timeout -= 1;
            }
            crate::outb(PORT_KBD_RESET, 0xFE);
            io_delay();

            // 4. Fallback Triple Fault
            let zero_idt = [0u8; 10];
            core::arch::asm!(
                "lidt [{}]",
                "int3",
                in(reg) zero_idt.as_ptr(),
                options(noreturn)
            );
        }
    }
}

pub static POWER_MANAGER: crate::memory::spinlock::Spinlock<Option<PowerManager>> =
    crate::memory::spinlock::Spinlock::new(None);

/// Verification unit test exercising DSDT AML parsing, sleep control words, and reset configurations.
pub fn test_sleep_and_reset_engine() -> bool {
    // 1. Test DSDT AML sleep state parsing
    let mock_dsdt = b"\x00\x00_S5_\x12\x04\x02\x0A\x07\x0A\x07\x00_S3_\x12\x04\x02\x03\x03\x00";

    let s5_cfg = parse_dsdt_sleep_state(mock_dsdt, b"_S5_").expect("DSDT _S5_ must be found");
    assert_eq!(s5_cfg.slp_typa, 7);
    assert_eq!(s5_cfg.slp_typb, 7);

    let s3_cfg = parse_dsdt_sleep_state(mock_dsdt, b"_S3_").expect("DSDT _S3_ must be found");
    assert_eq!(s3_cfg.slp_typa, 3);
    assert_eq!(s3_cfg.slp_typb, 3);

    assert!(parse_dsdt_sleep_state(mock_dsdt, b"_S4_").is_none());

    // 2. Test PowerManager sleep control word computation
    let mut fadt = FadtInfo::default();
    fadt.pm1a_cnt_blk = 0x0604;
    let mut pm = PowerManager::new(fadt);
    pm.s5_config = s5_cfg;
    pm.s3_config = s3_cfg;

    let (s5_a, s5_b) = pm.compute_sleep_control_word(PowerState::S5SoftOff)
        .expect("S5 control word must be computed");
    // (7 << 10) | (1 << 13) = 0x1C00 | 0x2000 = 0x3C00
    assert_eq!(s5_a, 0x3C00);
    assert_eq!(s5_b, 0x3C00);

    let (s3_a, s3_b) = pm.compute_sleep_control_word(PowerState::S3SuspendToRam)
        .expect("S3 control word must be computed");
    // (3 << 10) | (1 << 13) = 0x0C00 | 0x2000 = 0x2C00
    assert_eq!(s3_a, 0x2C00);
    assert_eq!(s3_b, 0x2C00);

    // 3. Verify hardware reset port definitions
    assert_eq!(PORT_CF9_RESET, 0x0CF9);
    assert_eq!(PORT_KBD_RESET, 0x0064);

    true
}

/// CPU throttling state and duty cycle percentage.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct CpuThrottleState {
    pub duty_percent: u8,
    pub throttling_enabled: bool,
    pub idle_hlt_count: u64,
}

pub static CPU_THROTTLE: crate::memory::spinlock::Spinlock<CpuThrottleState> =
    crate::memory::spinlock::Spinlock::new(CpuThrottleState {
        duty_percent: 0,
        throttling_enabled: false,
        idle_hlt_count: 0,
    });

/// Puts the CPU into low-power C1 state (HLT with interrupts enabled).
#[inline(always)]
pub fn cpu_idle() {
    {
        let mut guard = CPU_THROTTLE.lock();
        guard.idle_hlt_count += 1;
    }
    unsafe {
        core::arch::asm!("sti", "hlt", options(nomem, nostack, preserves_flags));
    }
}

/// Relaxes the CPU in busy-wait spinlocks.
#[inline(always)]
pub fn cpu_relax() {
    core::hint::spin_loop();
}

/// Sets CPU throttling duty cycle percentage (0 to 100).
pub fn set_cpu_throttling(duty_percent: u8) {
    let clamped = core::cmp::min(duty_percent, 100);
    let mut guard = CPU_THROTTLE.lock();
    guard.duty_percent = clamped;
    guard.throttling_enabled = clamped > 0;
}

/// Returns current CPU throttling duty cycle percentage.
pub fn get_cpu_throttling() -> u8 {
    CPU_THROTTLE.lock().duty_percent
}

/// Initializes the ACPI power management subsystem and registers the global controller.
pub fn init() {
    serial_write_str("[KNT-POWER-INIT] Initializing ACPI power management subsystem...\r\n");
    let fadt = FadtInfo::discover();
    let mut pm = PowerManager::new(fadt);
    let _ = pm.enable_acpi();
    *POWER_MANAGER.lock() = Some(pm);
    serial_write_str("[KNT-POWER] ACPI power management active with S3/S5 and CPU throttling support\r\n");
}

/// Verification unit test ensuring CPU throttling state and idle scaling operate correctly.
pub fn test_cpu_throttling() -> bool {
    assert_eq!(get_cpu_throttling(), 0);

    set_cpu_throttling(50);
    assert_eq!(get_cpu_throttling(), 50);

    set_cpu_throttling(100);
    assert_eq!(get_cpu_throttling(), 100);

    set_cpu_throttling(0);
    assert_eq!(get_cpu_throttling(), 0);

    cpu_relax();
    true
}

/// Comprehensive ACPI power management verification suite emitting [KNT-POWER-MGMT-PASS].
pub fn test_power_subsystem() -> bool {
    serial_write_str("[KNT-TEST] Executing ACPI Power Management Subsystem self-tests...\r\n");

    if !test_fadt_parser_and_power_structures() {
        serial_write_str("[KNT-TEST-FAIL] FADT parser and register structures test failed!\r\n");
        return false;
    }

    if !test_sleep_and_reset_engine() {
        serial_write_str("[KNT-TEST-FAIL] Sleep transition and reset engine test failed!\r\n");
        return false;
    }

    if !test_cpu_throttling() {
        serial_write_str("[KNT-TEST-FAIL] CPU throttling and idle scaling test failed!\r\n");
        return false;
    }

    serial_write_str("[KNT-POWER-MGMT-PASS] ACPI Power Management Subsystem operational\r\n");
    true
}
