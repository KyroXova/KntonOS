//! Intel RAPL (Running Average Power Limit) & AMD SMU Energy Interface.
//! Implements hardware power and energy metering for x86-64 processors:

// --- Intel RAPL Model-Specific Registers (MSRs) ---
pub const MSR_RAPL_POWER_UNIT: u32 = 0x0000_0606;
pub const MSR_PKG_POWER_LIMIT: u32 = 0x0000_0610;
pub const MSR_PKG_ENERGY_STATUS: u32 = 0x0000_0611;
pub const MSR_PKG_PERF_STATUS: u32 = 0x0000_0613;
pub const MSR_DRAM_POWER_LIMIT: u32 = 0x0000_0618;
pub const MSR_DRAM_ENERGY_STATUS: u32 = 0x0000_0619;
pub const MSR_PP0_ENERGY_STATUS: u32 = 0x0000_0639; // Core energy status

// --- AMD SMU Mailbox Message Codes ---
pub const AMD_SMU_MSG_TEST_MESSAGE: u32 = 0x01;
pub const AMD_SMU_MSG_GET_PACKAGE_POWER: u32 = 0x14;
pub const AMD_SMU_MSG_GET_CORE_TEMP: u32 = 0x15;
pub const AMD_SMU_MSG_SET_FAST_PPT_LIMIT: u32 = 0x20;
pub const AMD_SMU_MSG_SET_SLOW_PPT_LIMIT: u32 = 0x21;

// --- AMD SMU Response Status ---
pub const AMD_SMU_RES_OK: u32 = 0x01;
pub const AMD_SMU_RES_FAILED: u32 = 0xFF;
pub const AMD_SMU_RES_UNKNOWN_CMD: u32 = 0xFE;
pub const AMD_SMU_RES_BUSY: u32 = 0x00;

/// Processor Power Telemetry Architecture.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CpuPowerVendor {
    IntelRapl,
    AmdSmu,
    GenericSimulated,
}

/// Parsed units from MSR_RAPL_POWER_UNIT (0x606).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RaplUnits {
    /// Power unit exponent (Watts = 1 / (2^power_unit_exp)).
    pub power_unit_exp: u8,
    /// Energy status unit exponent (Joules = 1 / (2^energy_unit_exp)).
    pub energy_unit_exp: u8,
    /// Time unit exponent (Seconds = 1 / (2^time_unit_exp)).
    pub time_unit_exp: u8,
}

impl RaplUnits {
    /// Constructs default Intel client processor units (PU=3 -> 0.125W, ESU=16 -> ~15.3uJ, TU=10 -> 976us).
    pub const fn default_intel() -> Self {
        Self {
            power_unit_exp: 3,
            energy_unit_exp: 16,
            time_unit_exp: 10,
        }
    }

    /// Decodes units from MSR 0x606 value.
    pub fn from_msr(msr_val: u64) -> Self {
        Self {
            power_unit_exp: (msr_val & 0x0F) as u8,
            energy_unit_exp: ((msr_val >> 8) & 0x1F) as u8,
            time_unit_exp: ((msr_val >> 16) & 0x0F) as u8,
        }
    }

    /// Encodes units into MSR 0x606 representation.
    pub fn to_msr(&self) -> u64 {
        ((self.power_unit_exp as u64) & 0x0F)
            | (((self.energy_unit_exp as u64) & 0x1F) << 8)
            | (((self.time_unit_exp as u64) & 0x0F) << 16)
    }

    /// Multiplier to convert raw energy ticks into microjoules (10^-6 J).
    #[inline(always)]
    pub fn raw_ticks_to_microjoules(&self, ticks: u32) -> u64 {
        // Energy in Joules = ticks / (2^energy_unit_exp)
        // Energy in microjoules = (ticks * 1_000_000) / (2^energy_unit_exp)
        let numerator = (ticks as u128) * 1_000_000u128;
        let denominator = 1u128 << (self.energy_unit_exp as usize);
        (numerator / denominator) as u64
    }

    /// Converts raw integer power limit field (bits 14:0) into milliwatts (mW).
    #[inline(always)]
    pub fn raw_power_to_milliwatts(&self, raw: u16) -> u32 {
        let numerator = (raw as u64) * 1_000u64;
        let denominator = 1u64 << (self.power_unit_exp as usize);
        (numerator / denominator) as u32
    }

    /// Converts milliwatts (mW) to raw integer power limit field.
    #[inline(always)]
    pub fn milliwatts_to_raw_power(&self, mw: u32) -> u16 {
        let val = ((mw as u64) * (1u64 << (self.power_unit_exp as usize))) / 1_000u64;
        (val & 0x7FFF) as u16
    }
}

/// Package Power Limit Configuration (MSR_PKG_POWER_LIMIT / 0x610).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PkgPowerLimit {
    /// Long-term power limit (PL1) in milliwatts.
    pub pl1_milliwatts: u32,
    pub pl1_enabled: bool,
    pub pl1_clamped: bool,
    /// Short-term power limit (PL2) in milliwatts.
    pub pl2_milliwatts: u32,
    pub pl2_enabled: bool,
    pub pl2_clamped: bool,
    /// Configuration locked until next hard reset.
    pub locked: bool,
}

impl PkgPowerLimit {
    /// Constructs default TDP settings (e.g. 15W PL1, 25W PL2).
    pub const fn new(pl1_mw: u32, pl2_mw: u32) -> Self {
        Self {
            pl1_milliwatts: pl1_mw,
            pl1_enabled: true,
            pl1_clamped: false,
            pl2_milliwatts: pl2_mw,
            pl2_enabled: true,
            pl2_clamped: false,
            locked: false,
        }
    }

    /// Decodes MSR 0x610 using the provided units.
    pub fn from_msr(val: u64, units: &RaplUnits) -> Self {
        let pl1_raw = (val & 0x7FFF) as u16;
        let pl1_enabled = (val & (1 << 15)) != 0;
        let pl1_clamped = (val & (1 << 16)) != 0;

        let pl2_raw = ((val >> 32) & 0x7FFF) as u16;
        let pl2_enabled = (val & (1 << 47)) != 0;
        let pl2_clamped = (val & (1 << 48)) != 0;

        let locked = (val & (1 << 63)) != 0;

        Self {
            pl1_milliwatts: units.raw_power_to_milliwatts(pl1_raw),
            pl1_enabled,
            pl1_clamped,
            pl2_milliwatts: units.raw_power_to_milliwatts(pl2_raw),
            pl2_enabled,
            pl2_clamped,
            locked,
        }
    }

    /// Encodes structure into MSR 0x610 bitfield representation.
    pub fn to_msr(&self, units: &RaplUnits) -> u64 {
        let pl1_raw = units.milliwatts_to_raw_power(self.pl1_milliwatts) as u64;
        let mut msr = pl1_raw & 0x7FFF;
        if self.pl1_enabled {
            msr |= 1 << 15;
        }
        if self.pl1_clamped {
            msr |= 1 << 16;
        }

        let pl2_raw = units.milliwatts_to_raw_power(self.pl2_milliwatts) as u64;
        msr |= (pl2_raw & 0x7FFF) << 32;
        if self.pl2_enabled {
            msr |= 1 << 47;
        }
        if self.pl2_clamped {
            msr |= 1 << 48;
        }

        if self.locked {
            msr |= 1 << 63;
        }
        msr
    }
}

/// Cumulative 32-bit Energy Counter Tracker with Wrap-Around Handling.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EnergyCounter {
    pub last_raw: u32,
    pub total_wraps: u64,
    pub cumulative_microjoules: u64,
    pub is_initialized: bool,
}

impl EnergyCounter {
    pub const fn new() -> Self {
        Self {
            last_raw: 0,
            total_wraps: 0,
            cumulative_microjoules: 0,
            is_initialized: false,
        }
    }

    /// Samples the hardware 32-bit counter, handles wrap-arounds, and returns delta microjoules.
    pub fn update(&mut self, current_raw: u32, units: &RaplUnits) -> u64 {
        if !self.is_initialized {
            self.last_raw = current_raw;
            self.is_initialized = true;
            return 0;
        }

        // 32-bit unsigned wrapping subtraction handles counter overflow cleanly
        let delta_ticks = current_raw.wrapping_sub(self.last_raw);
        if current_raw < self.last_raw {
            self.total_wraps = self.total_wraps.wrapping_add(1);
        }

        self.last_raw = current_raw;
        let delta_uj = units.raw_ticks_to_microjoules(delta_ticks);
        self.cumulative_microjoules = self.cumulative_microjoules.wrapping_add(delta_uj);
        delta_uj
    }
}

/// AMD SMU (System Management Unit) Mailbox Interface.
#[derive(Debug, Clone)]
pub struct AmdSmuMailbox {
    pub last_message: u32,
    pub last_response: u32,
    pub args: [u32; 6],
    pub is_simulated: bool,
}

impl AmdSmuMailbox {
    pub const fn new(is_simulated: bool) -> Self {
        Self {
            last_message: 0,
            last_response: AMD_SMU_RES_OK,
            args: [0u32; 6],
            is_simulated,
        }
    }

    /// Dispatches a message to the AMD SMU and retrieves response arguments.
    pub fn send_message(&mut self, msg: u32, input_args: &[u32]) -> Result<u32, u32> {
        self.last_message = msg;
        for i in 0..6 {
            self.args[i] = if i < input_args.len() { input_args[i] } else { 0 };
        }

        if self.is_simulated {
            // Emulate SMU response according to command
            match msg {
                AMD_SMU_MSG_TEST_MESSAGE => {
                    self.last_response = AMD_SMU_RES_OK;
                    Ok(1)
                }
                AMD_SMU_MSG_GET_PACKAGE_POWER => {
                    // Return simulated power (e.g., 15000 mW = 15W)
                    self.args[0] = 15_000;
                    self.last_response = AMD_SMU_RES_OK;
                    Ok(self.args[0])
                }
                AMD_SMU_MSG_GET_CORE_TEMP => {
                    // Return simulated junction temp (e.g. 52000 mC = 52.0 C)
                    self.args[0] = 52_000;
                    self.last_response = AMD_SMU_RES_OK;
                    Ok(self.args[0])
                }
                AMD_SMU_MSG_SET_FAST_PPT_LIMIT | AMD_SMU_MSG_SET_SLOW_PPT_LIMIT => {
                    self.last_response = AMD_SMU_RES_OK;
                    Ok(0)
                }
                _ => {
                    self.last_response = AMD_SMU_RES_UNKNOWN_CMD;
                    Err(AMD_SMU_RES_UNKNOWN_CMD)
                }
            }
        } else {
            // Hardware port communication placeholder
            Ok(0)
        }
    }
}

/// Verification self-test for RAPL MSR parsing, power unit conversions, and SMU mailbox.
pub fn verify_rapl_and_smu_structures() -> bool {
    // 1. Verify default Intel RAPL unit decoding (PU=3 -> 1/8 W, ESU=16 -> 1/65536 J, TU=10 -> 1/1024 s)
    // Encoded: (10 << 16) | (16 << 8) | 3 = 0x000A_1003
    let units = RaplUnits::from_msr(0x000A_1003);
    if units.power_unit_exp != 3 || units.energy_unit_exp != 16 || units.time_unit_exp != 10 {
        return false;
    }
    if units.to_msr() != 0x000A_1003 {
        return false;
    }

    // 2. Verify energy tick to microjoules conversion
    // 65536 ticks @ ESU=16 must equal 1,000,000 microjoules (1 Joule)
    let uj = units.raw_ticks_to_microjoules(65536);
    if uj != 1_000_000 {
        return false;
    }

    // 3. Verify power limit encoding & decoding (15,000 mW = 15W)
    let raw_15w = units.milliwatts_to_raw_power(15_000);
    // 15 * 8 = 120
    if raw_15w != 120 {
        return false;
    }
    let mw_recovered = units.raw_power_to_milliwatts(raw_15w);
    if mw_recovered != 15_000 {
        return false;
    }

    let pl = PkgPowerLimit::new(15_000, 25_000);
    let msr_pl = pl.to_msr(&units);
    let pl_decoded = PkgPowerLimit::from_msr(msr_pl, &units);
    if pl_decoded.pl1_milliwatts != 15_000 || pl_decoded.pl2_milliwatts != 25_000 {
        return false;
    }
    if !pl_decoded.pl1_enabled || !pl_decoded.pl2_enabled {
        return false;
    }

    // 4. Verify 32-bit energy counter wrap-around tracking
    let mut energy_tracker = EnergyCounter::new();
    energy_tracker.update(0xFFFF_0000, &units);

    // Advance by 0x0002_0000 ticks across the 32-bit wrap boundary (0xFFFF_0000 -> 0x0001_0000)
    let delta_uj = energy_tracker.update(0x0001_0000, &units);
    let expected_delta_ticks = 0x0002_0000u32; // 131,072 ticks = 2 Joules = 2,000,000 uJ
    let expected_delta_uj = units.raw_ticks_to_microjoules(expected_delta_ticks);

    if delta_uj != expected_delta_uj || delta_uj != 2_000_000 {
        return false;
    }
    if energy_tracker.total_wraps != 1 || energy_tracker.cumulative_microjoules != 2_000_000 {
        return false;
    }

    // 5. Verify AMD SMU mailbox message dispatch
    let mut smu = AmdSmuMailbox::new(true);
    let power_mw = match smu.send_message(AMD_SMU_MSG_GET_PACKAGE_POWER, &[]) {
        Ok(p) => p,
        Err(_) => return false,
    };
    if power_mw != 15_000 || smu.last_response != AMD_SMU_RES_OK {
        return false;
    }

    let temp_mc = match smu.send_message(AMD_SMU_MSG_GET_CORE_TEMP, &[]) {
        Ok(t) => t,
        Err(_) => return false,
    };
    if temp_mc != 52_000 {
        return false;
    }

    true
}
