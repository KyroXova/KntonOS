//! Unified Handheld PC Dual-Battery Balance Manager & Fast USB-PD 3.1 Extended Power Profile Engine.
//!
//! Provides bare-metal power architecture for high-TDP handheld gaming PCs featuring:
//! - Dual-battery cell balancing architecture (Cell 1 & Cell 2 active balancing).
//! - SMBus / I2C multi-cell fuel gauge telemetry (BQ40Z50 / MAX17320 style).
//! - Active cell load-balancing, State-of-Charge (SoC) estimation, and thermal-throttled charge curves.
//! - Pass-through bypass power mode (wall power directly driving APU without cycling battery).
//! - USB-PD 3.1 Extended Power Range (EPR) contract negotiator supporting up to 140W (28V @ 5A).

/// Hardware I2C / SMBus fuel gauge and charge controller register definitions.
pub mod registers {
    /// Total battery pack terminal voltage in millivolts (mV).
    pub const REG_PACK_VOLTAGE: u8 = 0x09;
    /// Instantaneous pack current in milliamperes (mA, negative = discharging, positive = charging).
    pub const REG_PACK_CURRENT: u8 = 0x0A;
    /// Battery pack surface temperature in deci-Kelvin (0.1 K).
    pub const REG_TEMPERATURE: u8 = 0x08;
    /// Relative State of Charge percentage (0..100%).
    pub const REG_STATE_OF_CHARGE: u8 = 0x0D;
    /// Remaining capacity in milliampere-hours (mAh).
    pub const REG_REMAINING_CAPACITY: u8 = 0x0F;
    /// Full charge capacity in milliampere-hours (mAh).
    pub const REG_FULL_CHARGE_CAPACITY: u8 = 0x10;
    /// Accumulated charge/discharge cycle count.
    pub const REG_CYCLE_COUNT: u8 = 0x17;

    /// Cell 1 individual terminal voltage in millivolts (mV).
    pub const REG_CELL1_VOLTAGE: u8 = 0x3C;
    /// Cell 2 individual terminal voltage in millivolts (mV).
    pub const REG_CELL2_VOLTAGE: u8 = 0x3D;
    /// Cell balancing active shunt control register.
    pub const REG_CELL_BALANCE_CTRL: u8 = 0x48;
    /// Pass-through power path bypass mode switch register.
    pub const REG_POWER_PATH_CTRL: u8 = 0x50;
    /// USB-PD Extended Power Range (EPR) status register.
    pub const REG_USB_PD_EPR_STATUS: u8 = 0x60;

    // Control flags for REG_CELL_BALANCE_CTRL
    pub const BALANCE_CTRL_CELL1_SHUNT: u8 = 1 << 0;
    pub const BALANCE_CTRL_CELL2_SHUNT: u8 = 1 << 1;
    pub const BALANCE_CTRL_AUTO_BALANCE: u8 = 1 << 2;

    // Control flags for REG_POWER_PATH_CTRL
    pub const POWER_PATH_BYPASS_ENABLE: u8 = 1 << 0;
    pub const POWER_PATH_BATT_ISOLATE: u8 = 1 << 1;
    pub const POWER_PATH_AC_DIRECT: u8 = 1 << 2;
}

/// Identifies an individual battery cell within the dual-cell pack.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CellId {
    Cell1 = 1,
    Cell2 = 2,
}

/// Individual battery cell operational descriptor.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BatteryCellDescriptor {
    /// Cell identifier.
    pub cell_id: CellId,
    /// Cell terminal voltage in millivolts (mV) (nominal 3000 to 4350 mV).
    pub voltage_mv: u16,
    /// Instantaneous current flowing through cell in milliamperes (mA).
    pub current_ma: i32,
    /// Individual cell temperature in degrees Celsius (°C).
    pub temperature_c: f32,
    /// Estimated internal AC/DC resistance in milli-Ohms (mOhm).
    pub internal_resistance_mohm: u16,
    /// Individual cell State-of-Charge percentage (0..100%).
    pub state_of_charge_pct: u8,
    /// True if active balancing bypass shunt resistor is engaged.
    pub balance_shunt_active: bool,
}

impl BatteryCellDescriptor {
    /// Creates a default healthy cell descriptor.
    pub const fn new(cell_id: CellId, voltage_mv: u16) -> Self {
        Self {
            cell_id,
            voltage_mv,
            current_ma: 0,
            temperature_c: 25.0,
            internal_resistance_mohm: 20, // 20 mOhm nominal for high-drain 21700 / pouch cell
            state_of_charge_pct: 50,
            balance_shunt_active: false,
        }
    }
}

/// Dual-battery cell pack state representation.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DualBatteryPack {
    /// Cell 1 descriptor.
    pub cell1: BatteryCellDescriptor,
    /// Cell 2 descriptor.
    pub cell2: BatteryCellDescriptor,
    /// Total pack series voltage in millivolts (mV).
    pub pack_voltage_mv: u32,
    /// Total pack current in milliamperes (mA).
    pub pack_current_ma: i32,
    /// Combined full charge design capacity in milliampere-hours (mAh) (e.g. 5200 mAh).
    pub design_capacity_mah: u32,
    /// Accumulated full charge cycles.
    pub cycle_count: u16,
}

impl DualBatteryPack {
    /// Creates a new dual-battery pack with default nominal cell voltages.
    pub const fn new(cell1_mv: u16, cell2_mv: u16) -> Self {
        Self {
            cell1: BatteryCellDescriptor::new(CellId::Cell1, cell1_mv),
            cell2: BatteryCellDescriptor::new(CellId::Cell2, cell2_mv),
            pack_voltage_mv: (cell1_mv as u32) + (cell2_mv as u32),
            pack_current_ma: 0,
            design_capacity_mah: 5200, // 40Wh pack at 7.7V nominal
            cycle_count: 12,
        }
    }

    /// Computes the inter-cell voltage delta in millivolts: |V1 - V2|.
    #[inline(always)]
    pub fn cell_delta_mv(&self) -> u16 {
        let v1 = self.cell1.voltage_mv as i32;
        let v2 = self.cell2.voltage_mv as i32;
        (v1 - v2).abs() as u16
    }

    /// Computes average State of Charge across both cells.
    #[inline(always)]
    pub fn average_soc_pct(&self) -> u8 {
        ((self.cell1.state_of_charge_pct as u16 + self.cell2.state_of_charge_pct as u16) / 2) as u8
    }

    /// Returns the maximum temperature between Cell 1 and Cell 2.
    #[inline(always)]
    pub fn max_cell_temp_c(&self) -> f32 {
        if self.cell1.temperature_c > self.cell2.temperature_c {
            self.cell1.temperature_c
        } else {
            self.cell2.temperature_c
        }
    }
}

/// USB Power Delivery (USB-PD) power range tier.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PdPowerRange {
    /// Standard Power Range (SPR): Fixed voltages 5V, 9V, 15V, 20V up to 100W (20V @ 5A).
    StandardPowerRange,
    /// Extended Power Range (EPR): Added in USB-PD 3.1; voltages 28V, 36V, 48V up to 240W.
    ExtendedPowerRange,
}

/// Power Data Object (PDO) descriptor describing an advertised USB-PD power profile.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PdPowerProfile {
    /// Nominal bus voltage in millivolts (mV) (e.g. 5000, 9000, 15000, 20000, 28000).
    pub voltage_mv: u32,
    /// Maximum continuous current in milliamperes (mA) (e.g. 3000, 5000).
    pub max_current_ma: u32,
    /// Total continuous power capability in Watts.
    pub power_watts: u16,
    /// Power range tier (SPR or EPR).
    pub range: PdPowerRange,
}

impl PdPowerProfile {
    /// Creates a 15W standard SPR profile (5V @ 3A).
    pub const fn spr_15w() -> Self {
        Self {
            voltage_mv: 5000,
            max_current_ma: 3000,
            power_watts: 15,
            range: PdPowerRange::StandardPowerRange,
        }
    }

    /// Creates a 65W standard SPR profile (20V @ 3.25A).
    pub const fn spr_65w() -> Self {
        Self {
            voltage_mv: 20000,
            max_current_ma: 3250,
            power_watts: 65,
            range: PdPowerRange::StandardPowerRange,
        }
    }

    /// Creates a 100W standard SPR profile (20V @ 5.0A).
    pub const fn spr_100w() -> Self {
        Self {
            voltage_mv: 20000,
            max_current_ma: 5000,
            power_watts: 100,
            range: PdPowerRange::StandardPowerRange,
        }
    }

    /// Creates a 140W USB-PD 3.1 Extended Power Range (EPR) profile (28V @ 5.0A).
    pub const fn epr_140w() -> Self {
        Self {
            voltage_mv: 28000,
            max_current_ma: 5000,
            power_watts: 140,
            range: PdPowerRange::ExtendedPowerRange,
        }
    }

    /// Returns true if this profile is an Extended Power Range (EPR) contract.
    #[inline(always)]
    pub const fn is_epr(&self) -> bool {
        matches!(self.range, PdPowerRange::ExtendedPowerRange)
    }
}

/// Verification helper for Commit 1 structures and profile definitions.
pub fn verify_battery_descriptors_and_pd_profiles() -> bool {
    // 1. Verify Dual-Battery Pack Cell Delta Calculation
    let pack = DualBatteryPack::new(3850, 3810);
    if pack.cell_delta_mv() != 40 {
        return false;
    }
    if pack.pack_voltage_mv != 7660 {
        return false;
    }

    // 2. Verify Average SoC Calculation
    let mut pack_soc = pack;
    pack_soc.cell1.state_of_charge_pct = 70;
    pack_soc.cell2.state_of_charge_pct = 60;
    if pack_soc.average_soc_pct() != 65 {
        return false;
    }

    // 3. Verify Maximum Cell Temperature
    pack_soc.cell1.temperature_c = 32.5;
    pack_soc.cell2.temperature_c = 36.8;
    if (pack_soc.max_cell_temp_c() - 36.8).abs() > 0.01 {
        return false;
    }

    // 4. Verify USB-PD 3.1 EPR 140W Profile
    let p_epr = PdPowerProfile::epr_140w();
    if p_epr.voltage_mv != 28000 || p_epr.max_current_ma != 5000 || p_epr.power_watts != 140 {
        return false;
    }
    if !p_epr.is_epr() {
        return false;
    }

    // 5. Verify 100W SPR Profile
    let p_spr = PdPowerProfile::spr_100w();
    if p_spr.voltage_mv != 20000 || p_spr.max_current_ma != 5000 || p_spr.power_watts != 100 {
        return false;
    }
    if p_spr.is_epr() {
        return false;
    }

    true
}
