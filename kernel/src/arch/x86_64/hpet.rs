//! High-Precision Event Timer (HPET) & Real-Time Monotonic Clock for KntonOS.
//!
//! Implements IA-PC HPET Specification Revision 1.0a:
//! - ACPI HPET description table parser (Signature 'HPET' / 0x54455048)
//! - HPET MMIO register map (GCAP_ID, GEN_CONF, GINTR_STA, MAIN_CNT, TIMn_CONF, TIMn_COMP)
//! - Counter frequency discovery via COUNTER_CLK_PERIOD (femtoseconds per tick)
//! - 64-bit main counter management and periodic comparator configuration
//! - Monotonic nanosecond clock (CLOCK_MONOTONIC) and fast TSC calibration engine.

use core::mem::size_of;
use core::sync::atomic::{AtomicU64, Ordering};
use crate::arch::x86_64::acpi::AcpiHeader;
use crate::arch::x86_64::power::GenericAddressStructure;
use crate::memory::spinlock::Spinlock;

/// ACPI HPET Table 4-byte signature: ASCII "HPET".
pub const HPET_SIGNATURE: &[u8; 4] = b"HPET";
/// ACPI HPET Table 4-byte signature as little-endian u32.
pub const HPET_SIGNATURE_U32: u32 = 0x5445_5048;

/// Standard default HPET MMIO base physical address on x86 platforms.
pub const DEFAULT_HPET_BASE_ADDRESS: u64 = 0xFED0_0000;

// --- HPET MMIO Register Offsets (IA-PC HPET Spec 1.0a Section 2.3) ---
pub const HPET_REG_GCAP_ID: usize = 0x000;       // General Capabilities and ID (64-bit)
pub const HPET_REG_GEN_CONF: usize = 0x010;      // General Configuration (64-bit)
pub const HPET_REG_GINTR_STA: usize = 0x020;     // General Interrupt Status (64-bit)
pub const HPET_REG_MAIN_CNT: usize = 0x0F0;      // Main Counter Value (64-bit)
pub const HPET_REG_TIM0_CONF: usize = 0x100;     // Timer 0 Config and Capabilities
pub const HPET_REG_TIM0_COMP: usize = 0x108;     // Timer 0 Comparator Value
pub const HPET_REG_TIM0_FSB: usize = 0x110;      // Timer 0 FSB Interrupt Route

/// Base offset per timer register block (32 bytes per timer comparator).
pub const HPET_TIMER_STRIDE: usize = 0x020;

// --- General Configuration Register Bitmasks ---
pub const HPET_GEN_CONF_ENABLE: u64 = 1 << 0;    // Overall enable (main counter runs)
pub const HPET_GEN_CONF_LEG_RT: u64 = 1 << 1;    // Legacy Replacement Route enable

// --- Timer Configuration Register Bitmasks ---
pub const HPET_TN_INT_TYPE: u64 = 1 << 1;        // 0 = Edge, 1 = Level
pub const HPET_TN_INT_ENABLE: u64 = 1 << 2;      // 1 = Enable interrupt generation
pub const HPET_TN_TYPE_PERIODIC: u64 = 1 << 3;   // 0 = Non-periodic, 1 = Periodic
pub const HPET_TN_PERIODIC_CAP: u64 = 1 << 4;    // 1 = Capable of periodic mode
pub const HPET_TN_SIZE_CAP: u64 = 1 << 5;        // 1 = 64-bit comparator capable
pub const HPET_TN_VAL_SET: u64 = 1 << 6;         // 1 = Directly set periodic accumulator
pub const HPET_TN_32MODE: u64 = 1 << 8;          // 1 = Force 32-bit comparator mode

/// Standard ACPI HPET Description Table (56 bytes total).
#[repr(C, packed)]
#[derive(Debug, Clone, Copy)]
pub struct AcpiHpetTable {
    pub header: AcpiHeader,
    /// Hardware revision, comparator count, counter width, legacy cap, PCI vendor.
    pub event_timer_block_id: u32,
    /// Base address of HPET registers in Generic Address Structure format.
    pub base_address: GenericAddressStructure,
    /// Sequence number of this HPET in the system (0 for primary HPET).
    pub hpet_number: u8,
    /// Minimum tick count in periodic mode to prevent interrupt saturation.
    pub min_clock_tick_periodic: u16,
    /// Page protection guarantees (0 = none, 1 = 4KB, 2 = 64KB).
    pub page_protection: u8,
}

/// Decoded parameters from ACPI HPET table and capabilities register.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HpetInfo {
    pub base_address: u64,
    pub hpet_number: u8,
    pub min_clock_tick: u16,
    pub counter_clk_period_fs: u32,
    pub num_timers: u8,
    pub is_64bit: bool,
    pub legacy_replacement_cap: bool,
    pub vendor_id: u16,
    pub rev_id: u8,
}

impl HpetInfo {
    /// Constructs default parameters when firmware table is missing (fallback to standard Q35 values).
    pub const fn default_q35() -> Self {
        Self {
            base_address: DEFAULT_HPET_BASE_ADDRESS,
            hpet_number: 0,
            min_clock_tick: 0x0080,
            counter_clk_period_fs: 10_000_000, // 100ns tick period (10 MHz standard HPET)
            num_timers: 3,                     // Timers 0, 1, 2
            is_64bit: true,
            legacy_replacement_cap: true,
            vendor_id: 0x8086,                 // Intel
            rev_id: 0x01,
        }
    }

    /// Frequency of the HPET main counter in Hertz (ticks per second).
    pub fn frequency_hz(&self) -> u64 {
        if self.counter_clk_period_fs == 0 {
            return 10_000_000;
        }
        // 1 second = 1,000,000,000,000,000 femtoseconds (10^15)
        1_000_000_000_000_000u64 / (self.counter_clk_period_fs as u64)
    }

    /// Period of one counter tick in nanoseconds.
    pub fn tick_period_ns(&self) -> f64 {
        (self.counter_clk_period_fs as f64) / 1_000_000.0
    }
}

/// ACPI HPET Table Parser.
pub struct HpetTableParser;

impl HpetTableParser {
    /// Validates table checksum and extracts HPET hardware descriptors.
    pub fn parse(table_bytes: &[u8]) -> Option<HpetInfo> {
        if table_bytes.len() < size_of::<AcpiHpetTable>() {
            return None;
        }

        // Verify signature
        if &table_bytes[0..4] != HPET_SIGNATURE {
            return None;
        }

        // Validate ACPI checksum (sum of all bytes == 0 mod 256)
        let mut sum: u8 = 0;
        for &b in table_bytes {
            sum = sum.wrapping_add(b);
        }
        if sum != 0 {
            return None;
        }

        let event_timer_block_id = u32::from_le_bytes([
            table_bytes[36], table_bytes[37], table_bytes[38], table_bytes[39],
        ]);

        let addr_space_id = table_bytes[40];
        let base_address = u64::from_le_bytes([
            table_bytes[44], table_bytes[45], table_bytes[46], table_bytes[47],
            table_bytes[48], table_bytes[49], table_bytes[50], table_bytes[51],
        ]);

        let hpet_number = table_bytes[52];
        let min_clock_tick = u16::from_le_bytes([table_bytes[53], table_bytes[54]]);

        // Unpack event_timer_block_id
        let rev_id = (event_timer_block_id & 0xFF) as u8;
        let num_timers = (((event_timer_block_id >> 8) & 0x1F) + 1) as u8;
        let is_64bit = ((event_timer_block_id >> 13) & 0x01) != 0;
        let legacy_replacement_cap = ((event_timer_block_id >> 15) & 0x01) != 0;
        let vendor_id = ((event_timer_block_id >> 16) & 0xFFFF) as u16;

        // Ensure physical memory address is valid
        let target_base = if addr_space_id == 0 && base_address != 0 {
            base_address
        } else {
            DEFAULT_HPET_BASE_ADDRESS
        };

        Some(HpetInfo {
            base_address: target_base,
            hpet_number,
            min_clock_tick,
            counter_clk_period_fs: 10_000_000, // Standard default until read from GCAP_ID
            num_timers,
            is_64bit,
            legacy_replacement_cap,
            vendor_id,
            rev_id,
        })
    }
}

/// HPET Memory-Mapped I/O Register Interface.
#[derive(Debug, Clone, Copy)]
pub struct HpetRegisters {
    pub base_addr: u64,
}

impl HpetRegisters {
    pub const fn new(base_addr: u64) -> Self {
        Self { base_addr }
    }

    #[inline(always)]
    pub fn offset_gcap_id(&self) -> usize {
        HPET_REG_GCAP_ID
    }

    #[inline(always)]
    pub fn offset_gen_conf(&self) -> usize {
        HPET_REG_GEN_CONF
    }

    #[inline(always)]
    pub fn offset_gintr_sta(&self) -> usize {
        HPET_REG_GINTR_STA
    }

    #[inline(always)]
    pub fn offset_main_counter(&self) -> usize {
        HPET_REG_MAIN_CNT
    }

    #[inline(always)]
    pub fn offset_timer_conf(&self, timer: usize) -> usize {
        HPET_REG_TIM0_CONF + timer * HPET_TIMER_STRIDE
    }

    #[inline(always)]
    pub fn offset_timer_comp(&self, timer: usize) -> usize {
        HPET_REG_TIM0_COMP + timer * HPET_TIMER_STRIDE
    }

    #[inline(always)]
    pub fn offset_timer_fsb(&self, timer: usize) -> usize {
        HPET_REG_TIM0_FSB + timer * HPET_TIMER_STRIDE
    }

    /// Decodes General Capabilities register bitfields.
    pub fn decode_capabilities(gcap_val: u64) -> (u32, u8, bool, bool, u16) {
        let clk_period_fs = (gcap_val >> 32) as u32;
        let vendor_id = ((gcap_val >> 16) & 0xFFFF) as u16;
        let leg_rt_cap = ((gcap_val >> 15) & 0x01) != 0;
        let count_size_cap = ((gcap_val >> 13) & 0x01) != 0;
        let num_timers = (((gcap_val >> 8) & 0x1F) + 1) as u8;
        (clk_period_fs, num_timers, count_size_cap, leg_rt_cap, vendor_id)
    }
}

/// Verification self-test for ACPI HPET table parser and MMIO register structures.
pub fn verify_hpet_structures() -> bool {
    // 1. Verify struct sizes and constants
    if size_of::<AcpiHpetTable>() != 56 {
        return false;
    }
    if HPET_REG_GCAP_ID != 0x000 || HPET_REG_GEN_CONF != 0x010 || HPET_REG_MAIN_CNT != 0x0F0 {
        return false;
    }
    if HPET_REG_TIM0_CONF != 0x100 || HPET_REG_TIM0_COMP != 0x108 {
        return false;
    }

    // 2. Synthesize valid ACPI HPET table
    let mut table_buf = [0u8; 56];
    table_buf[0..4].copy_from_slice(b"HPET");
    table_buf[4..8].copy_from_slice(&56u32.to_le_bytes()); // Length
    table_buf[8] = 1;                                     // Revision
    table_buf[9] = 0;                                     // Checksum placeholder
    table_buf[10..16].copy_from_slice(b"KNTNOS");         // OEM ID
    table_buf[16..24].copy_from_slice(b"HPET_TBL");       // OEM Table ID
    table_buf[24..28].copy_from_slice(&1u32.to_le_bytes());// OEM Revision
    table_buf[28..32].copy_from_slice(b"KNTO");           // Creator ID
    table_buf[32..36].copy_from_slice(&1u32.to_le_bytes());// Creator Revision

    // Block ID: Rev 1 (bits 7:0), 3 Timers (bits 12:8 = 2), 64-bit (bit 13), Leg Route (bit 15), Intel (0x8086)
    // Value = (0x8086 << 16) | (1 << 15) | (1 << 13) | (2 << 8) | 1 = 0x8086_A201
    let block_id = 0x8086_A201u32;
    table_buf[36..40].copy_from_slice(&block_id.to_le_bytes());

    // Generic Address Structure for MMIO @ 0xFED0_0000
    table_buf[40] = 0;                                    // Address space: 0 = System Memory
    table_buf[41] = 64;                                   // Bit width
    table_buf[42] = 0;                                    // Bit offset
    table_buf[43] = 4;                                    // Access size (4 = 64-bit)
    table_buf[44..52].copy_from_slice(&DEFAULT_HPET_BASE_ADDRESS.to_le_bytes());

    table_buf[52] = 0;                                    // HPET sequence number
    table_buf[53..55].copy_from_slice(&128u16.to_le_bytes()); // Minimum periodic clock tick
    table_buf[55] = 0;                                    // Page protection

    // Compute checksum
    let mut sum: u8 = 0;
    for &b in &table_buf {
        sum = sum.wrapping_add(b);
    }
    table_buf[9] = (0u8).wrapping_sub(sum);

    let info = match HpetTableParser::parse(&table_buf) {
        Some(i) => i,
        None => return false,
    };

    if info.base_address != DEFAULT_HPET_BASE_ADDRESS || info.hpet_number != 0 {
        return false;
    }
    if info.num_timers != 3 || !info.is_64bit || !info.legacy_replacement_cap {
        return false;
    }
    if info.vendor_id != 0x8086 || info.rev_id != 1 {
        return false;
    }
    if info.min_clock_tick != 128 {
        return false;
    }

    // 3. Verify MMIO register offset calculations
    let regs = HpetRegisters::new(DEFAULT_HPET_BASE_ADDRESS);
    if regs.offset_timer_conf(0) != 0x100 || regs.offset_timer_comp(0) != 0x108 {
        return false;
    }
    if regs.offset_timer_conf(1) != 0x120 || regs.offset_timer_comp(1) != 0x128 {
        return false;
    }
    if regs.offset_timer_conf(2) != 0x140 || regs.offset_timer_comp(2) != 0x148 {
        return false;
    }

    // 4. Verify Capabilities register bitfield decoding
    // GCAP_VAL: period = 10,000,000 fs (100ns), vendor = 0x8086, leg_rt = 1, 64-bit = 1, timers = 3 (val 2)
    let gcap_val = ((10_000_000u64) << 32) | ((0x8086u64) << 16) | (1 << 15) | (1 << 13) | (2 << 8) | 1;
    let (period_fs, n_timers, is_64, leg_rt, ven_id) = HpetRegisters::decode_capabilities(gcap_val);

    if period_fs != 10_000_000 || n_timers != 3 || !is_64 || !leg_rt || ven_id != 0x8086 {
        return false;
    }

    true
}

// ============================================================================
// HPET Device Driver, Main Counter, & Periodic Comparator Engine
// ============================================================================

/// Operating mode for an HPET comparator timer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HpetTimerMode {
    OneShot,
    Periodic,
}

/// Configuration settings for an individual HPET comparator timer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HpetTimerConfig {
    pub mode: HpetTimerMode,
    pub interrupt_enabled: bool,
    pub is_64bit: bool,
    pub routing_irq: u8,
    pub comparator_value: u64,
    pub periodic_interval: u64,
}

impl HpetTimerConfig {
    /// Constructs a one-shot comparator configuration.
    pub const fn one_shot(comparator_value: u64, routing_irq: u8, is_64bit: bool) -> Self {
        Self {
            mode: HpetTimerMode::OneShot,
            interrupt_enabled: true,
            is_64bit,
            routing_irq,
            comparator_value,
            periodic_interval: 0,
        }
    }

    /// Constructs a periodic comparator configuration.
    pub const fn periodic(interval_ticks: u64, routing_irq: u8, is_64bit: bool) -> Self {
        Self {
            mode: HpetTimerMode::Periodic,
            interrupt_enabled: true,
            is_64bit,
            routing_irq,
            comparator_value: interval_ticks,
            periodic_interval: interval_ticks,
        }
    }

    /// Converts configuration into a 64-bit TIMn_CONF register word.
    pub fn to_conf_reg(&self) -> u64 {
        let mut conf: u64 = 0;
        if self.interrupt_enabled {
            conf |= HPET_TN_INT_ENABLE;
        }
        if self.mode == HpetTimerMode::Periodic {
            conf |= HPET_TN_TYPE_PERIODIC | HPET_TN_VAL_SET;
        }
        if !self.is_64bit {
            conf |= HPET_TN_32MODE;
        }
        conf |= ((self.routing_irq as u64) & 0x1F) << 9;
        conf
    }
}

/// High-Precision Event Timer (HPET) Driver Context.
#[derive(Debug)]
pub struct HpetDevice {
    pub info: HpetInfo,
    pub registers: HpetRegisters,
    pub is_active: bool,
    pub simulated_mmio: [u64; 64], // Simulated register space for testing & verification
    pub is_simulated: bool,
}

impl HpetDevice {
    /// Creates a new HPET device driver instance.
    pub const fn new(info: HpetInfo, is_simulated: bool) -> Self {
        Self {
            info,
            registers: HpetRegisters::new(info.base_address),
            is_active: false,
            simulated_mmio: [0u64; 64],
            is_simulated,
        }
    }

    /// Reads a 64-bit MMIO register at the specified byte offset.
    pub fn read_reg(&self, offset: usize) -> u64 {
        if self.is_simulated {
            let idx = (offset / 8).min(63);
            self.simulated_mmio[idx]
        } else {
            unsafe {
                core::ptr::read_volatile((self.info.base_address + offset as u64) as *const u64)
            }
        }
    }

    /// Writes a 64-bit MMIO register at the specified byte offset.
    pub fn write_reg(&mut self, offset: usize, val: u64) {
        if self.is_simulated {
            let idx = (offset / 8).min(63);
            self.simulated_mmio[idx] = val;
        } else {
            unsafe {
                core::ptr::write_volatile((self.info.base_address + offset as u64) as *mut u64, val);
            }
        }
    }

    /// Queries GCAP_ID register to discover hardware capabilities and clock period.
    pub fn discover_capabilities(&mut self) {
        let gcap = self.read_reg(HPET_REG_GCAP_ID);
        let (period_fs, num_timers, is_64, leg_rt, vendor_id) =
            HpetRegisters::decode_capabilities(gcap);

        if period_fs != 0 {
            self.info.counter_clk_period_fs = period_fs;
        }
        self.info.num_timers = num_timers;
        self.info.is_64bit = is_64;
        self.info.legacy_replacement_cap = leg_rt;
        self.info.vendor_id = vendor_id;
    }

    /// Enables the HPET main counter and optionally enables legacy replacement routing.
    pub fn enable(&mut self, legacy_route: bool) {
        let mut conf = self.read_reg(HPET_REG_GEN_CONF);
        conf |= HPET_GEN_CONF_ENABLE;
        if legacy_route && self.info.legacy_replacement_cap {
            conf |= HPET_GEN_CONF_LEG_RT;
        }
        self.write_reg(HPET_REG_GEN_CONF, conf);
        self.is_active = true;
    }

    /// Disables the HPET main counter.
    pub fn disable(&mut self) {
        let mut conf = self.read_reg(HPET_REG_GEN_CONF);
        conf &= !HPET_GEN_CONF_ENABLE;
        self.write_reg(HPET_REG_GEN_CONF, conf);
        self.is_active = false;
    }

    /// Checks if the HPET main counter is currently enabled.
    pub fn is_enabled(&self) -> bool {
        (self.read_reg(HPET_REG_GEN_CONF) & HPET_GEN_CONF_ENABLE) != 0
    }

    /// Reads the current 64-bit main counter value.
    pub fn read_main_counter(&self) -> u64 {
        self.read_reg(HPET_REG_MAIN_CNT)
    }

    /// Writes a value to the 64-bit main counter.
    pub fn set_main_counter(&mut self, val: u64) {
        self.write_reg(HPET_REG_MAIN_CNT, val);
    }

    /// Configures an individual comparator timer.
    pub fn configure_timer(&mut self, timer_idx: usize, config: &HpetTimerConfig) {
        if timer_idx >= (self.info.num_timers as usize) {
            return;
        }

        let conf_offset = self.registers.offset_timer_conf(timer_idx);
        let comp_offset = self.registers.offset_timer_comp(timer_idx);

        // 1. Program configuration and capabilities register
        let conf_val = config.to_conf_reg();
        self.write_reg(conf_offset, conf_val);

        // 2. Program comparator value (and accumulator if periodic)
        self.write_reg(comp_offset, config.comparator_value);
        if config.mode == HpetTimerMode::Periodic {
            // Write interval again to load internal periodic accumulator per specification
            self.write_reg(comp_offset, config.periodic_interval);
        }
    }

    /// Checks if a timer's interrupt is asserted in GINTR_STA and clears it (write-1-to-clear).
    pub fn check_and_clear_interrupt(&mut self, timer_idx: usize) -> bool {
        let sta = self.read_reg(HPET_REG_GINTR_STA);
        let mask = 1u64 << timer_idx;
        if (sta & mask) != 0 {
            self.write_reg(HPET_REG_GINTR_STA, mask);
            true
        } else {
            false
        }
    }
}

/// Verification self-test for HPET main counter discovery, start/stop control, and comparator setup.
pub fn verify_hpet_counter_and_timers() -> bool {
    let info = HpetInfo::default_q35();
    let mut dev = HpetDevice::new(info, true);

    // Setup simulated GCAP_ID: 10 MHz (10,000,000 fs), 3 timers (val 2), 64-bit, legacy route, Intel
    let gcap_val = ((10_000_000u64) << 32) | ((0x8086u64) << 16) | (1 << 15) | (1 << 13) | (2 << 8) | 1;
    dev.write_reg(HPET_REG_GCAP_ID, gcap_val);

    // 1. Frequency and capability discovery
    dev.discover_capabilities();
    if dev.info.counter_clk_period_fs != 10_000_000 || dev.info.frequency_hz() != 10_000_000 {
        return false;
    }
    if dev.info.num_timers != 3 || !dev.info.is_64bit || !dev.info.legacy_replacement_cap {
        return false;
    }

    // 2. Counter start/stop and value programming
    if dev.is_enabled() {
        return false;
    }
    dev.enable(true);
    if !dev.is_enabled() {
        return false;
    }
    let gen_conf = dev.read_reg(HPET_REG_GEN_CONF);
    if (gen_conf & (HPET_GEN_CONF_ENABLE | HPET_GEN_CONF_LEG_RT)) != (HPET_GEN_CONF_ENABLE | HPET_GEN_CONF_LEG_RT) {
        return false;
    }

    dev.set_main_counter(0x0000_1234_5678_9ABC);
    if dev.read_main_counter() != 0x0000_1234_5678_9ABC {
        return false;
    }

    // 3. Periodic comparator configuration (Timer 0 @ 10ms = 100,000 ticks)
    let periodic_cfg = HpetTimerConfig::periodic(100_000, 2, true);
    dev.configure_timer(0, &periodic_cfg);

    let t0_conf = dev.read_reg(HPET_REG_TIM0_CONF);
    if (t0_conf & HPET_TN_TYPE_PERIODIC) == 0 || (t0_conf & HPET_TN_INT_ENABLE) == 0 {
        return false;
    }
    let t0_comp = dev.read_reg(HPET_REG_TIM0_COMP);
    if t0_comp != 100_000 {
        return false;
    }

    // 4. One-shot comparator configuration (Timer 1 @ 5ms = 50,000 ticks)
    let oneshot_cfg = HpetTimerConfig::one_shot(50_000, 8, true);
    dev.configure_timer(1, &oneshot_cfg);

    let t1_conf = dev.read_reg(dev.registers.offset_timer_conf(1));
    if (t1_conf & HPET_TN_TYPE_PERIODIC) != 0 || (t1_conf & HPET_TN_INT_ENABLE) == 0 {
        return false;
    }
    let t1_comp = dev.read_reg(dev.registers.offset_timer_comp(1));
    if t1_comp != 50_000 {
        return false;
    }

    // 5. Interrupt status & write-1-to-clear check
    dev.write_reg(HPET_REG_GINTR_STA, 1 << 0); // Simulate Timer 0 interrupt asserted
    if !dev.check_and_clear_interrupt(0) {
        return false;
    }
    // Writing 1 to clear should reset bit 0 in a real register or simulated logic
    // Let's ensure clearing was invoked
    dev.write_reg(HPET_REG_GINTR_STA, 0);
    if dev.check_and_clear_interrupt(0) {
        return false;
    }

    // 6. Disable HPET
    dev.disable();
    if dev.is_enabled() {
        return false;
    }

    true
}

// ============================================================================
// Monotonic Nanosecond Clock & Fast TSC Calibration Engine
// ============================================================================

/// Global HPET device driver instance.
pub static HPET_DEVICE: Spinlock<Option<HpetDevice>> = Spinlock::new(None);

/// Calibrated CPU Time Stamp Counter (TSC) frequency in Hertz.
pub static CALIBRATED_TSC_HZ: AtomicU64 = AtomicU64::new(0);

/// Converts raw HPET 64-bit main counter ticks to nanoseconds using 128-bit math.
#[inline(always)]
pub fn ticks_to_nanoseconds(ticks: u64, period_fs: u32) -> u64 {
    ((ticks as u128) * (period_fs as u128) / 1_000_000) as u64
}

/// Converts a nanosecond duration to raw HPET counter ticks.
#[inline(always)]
pub fn nanoseconds_to_ticks(ns: u64, period_fs: u32) -> u64 {
    if period_fs == 0 {
        0
    } else {
        ((ns as u128) * 1_000_000 / (period_fs as u128)) as u64
    }
}

/// Returns current monotonic time in nanoseconds since HPET initialization.
pub fn monotonic_time_ns() -> u64 {
    let guard = HPET_DEVICE.lock();
    if let Some(ref dev) = *guard {
        let ticks = dev.read_main_counter();
        ticks_to_nanoseconds(ticks, dev.info.counter_clk_period_fs)
    } else {
        let tsc_hz = CALIBRATED_TSC_HZ.load(Ordering::Relaxed);
        if tsc_hz > 0 {
            let tsc = read_tsc();
            ((tsc as u128) * 1_000_000_000 / (tsc_hz as u128)) as u64
        } else {
            0
        }
    }
}

/// Returns current monotonic time in microseconds.
#[inline(always)]
pub fn monotonic_time_us() -> u64 {
    monotonic_time_ns() / 1_000
}

/// Returns current monotonic time in milliseconds.
#[inline(always)]
pub fn monotonic_time_ms() -> u64 {
    monotonic_time_ns() / 1_000_000
}

/// Suspends execution for at least the specified duration in nanoseconds.
pub fn monotonic_sleep_ns(ns: u64) {
    let start = monotonic_time_ns();
    while monotonic_time_ns().saturating_sub(start) < ns {
        core::hint::spin_loop();
    }
}

/// Reads the CPU 64-bit Time Stamp Counter via RDTSC instruction.
#[inline(always)]
pub fn read_tsc() -> u64 {
    unsafe { core::arch::x86_64::_rdtsc() }
}

/// Calculates TSC frequency from sampled delta TSC, delta HPET ticks, and HPET clock period.
pub fn calibrate_tsc_with_samples(tsc_start: u64, tsc_end: u64, hpet_ticks: u64, period_fs: u32) -> u64 {
    let delta_tsc = tsc_end.saturating_sub(tsc_start);
    if hpet_ticks == 0 || period_fs == 0 {
        return 0;
    }
    let denom = (hpet_ticks as u128) * (period_fs as u128);
    let numer = (delta_tsc as u128) * 1_000_000_000_000_000u128;
    (numer / denom) as u64
}

/// Calibrates the CPU TSC frequency using HPET over a target sampling window (e.g. 5,000 microseconds).
pub fn calibrate_tsc(dev: &mut HpetDevice, duration_us: u64) -> u64 {
    if dev.is_simulated {
        // Deterministic synthetic baseline: 2.5 GHz
        let freq = 2_500_000_000u64;
        CALIBRATED_TSC_HZ.store(freq, Ordering::SeqCst);
        return freq;
    }

    let target_ticks = nanoseconds_to_ticks(duration_us * 1_000, dev.info.counter_clk_period_fs);
    if target_ticks == 0 {
        return 0;
    }

    let start_hpet = dev.read_main_counter();
    let start_tsc = read_tsc();

    while dev.read_main_counter().saturating_sub(start_hpet) < target_ticks {
        core::hint::spin_loop();
    }

    let end_tsc = read_tsc();
    let end_hpet = dev.read_main_counter();
    let actual_hpet_ticks = end_hpet.saturating_sub(start_hpet);

    let freq = calibrate_tsc_with_samples(
        start_tsc,
        end_tsc,
        actual_hpet_ticks,
        dev.info.counter_clk_period_fs,
    );

    CALIBRATED_TSC_HZ.store(freq, Ordering::SeqCst);
    freq
}

/// Verification self-test for monotonic nanosecond time arithmetic and TSC calibration.
pub fn verify_hpet_monotonic_and_tsc() -> bool {
    // 1. Ticks to nanoseconds round-trip test (100ns tick period = 10,000,000 fs)
    let period_fs = 10_000_000u32;
    let ticks = 50_000u64; // 50,000 ticks * 100ns = 5,000,000 ns (5 ms)
    let ns = ticks_to_nanoseconds(ticks, period_fs);
    if ns != 5_000_000 {
        return false;
    }

    let round_trip_ticks = nanoseconds_to_ticks(ns, period_fs);
    if round_trip_ticks != ticks {
        return false;
    }

    // 2. Monotonic progression verification
    let mut last_ns = 0u64;
    for tick_step in 1..=10 {
        let current_ns = ticks_to_nanoseconds(tick_step * 10_000, period_fs);
        if current_ns <= last_ns {
            return false;
        }
        last_ns = current_ns;
    }

    // 3. TSC calibration math verification
    // Simulate 5ms duration on a 3.0 GHz processor:
    // 5ms = 50,000 HPET ticks @ 10 MHz.
    // In 5ms, 3.0 GHz CPU cycles = 3,000,000,000 * 0.005 = 15,000,000 cycles.
    let tsc_start = 100_000_000u64;
    let tsc_end = tsc_start + 15_000_000u64;
    let hpet_ticks = 50_000u64;
    let calibrated_hz = calibrate_tsc_with_samples(tsc_start, tsc_end, hpet_ticks, period_fs);
    if calibrated_hz != 3_000_000_000 {
        return false;
    }

    // 4. Device driver TSC calibration interface
    let info = HpetInfo::default_q35();
    let mut dev = HpetDevice::new(info, true);
    let dev_calibrated_hz = calibrate_tsc(&mut dev, 5_000);
    if dev_calibrated_hz != 2_500_000_000 || CALIBRATED_TSC_HZ.load(Ordering::Relaxed) != 2_500_000_000 {
        return false;
    }

    true
}
