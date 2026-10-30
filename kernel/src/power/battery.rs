//! Smart Battery System (SBS / ACPI _BST, _BIX) & Coulomb Counting Fuel Gauge.
//!
//! Provides high-precision handheld battery monitoring:
//! - ACPI `_BST` (Battery Status) and `_BIX` (Battery Information Extended) structures
//! - Coulomb counting charge integration with microampere-hour accumulation
//! - State of Charge (SoC %) and State of Health (SoH %) estimators
//! - Open-Circuit Voltage (OCV) calibration curves for zero-current drift correction

// -----------------------------------------------------------------------------
// ACPI Battery Structures & Status Flags
// -----------------------------------------------------------------------------

/// ACPI `_BST` state bitflags.
#[repr(transparent)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BatteryStateFlags(pub u32);

impl BatteryStateFlags {
    pub const NOT_PRESENT: Self = Self(0);
    pub const DISCHARGING: Self = Self(1 << 0);
    pub const CHARGING: Self = Self(1 << 1);
    pub const CRITICAL: Self = Self(1 << 2);

    #[inline(always)]
    pub const fn bits(&self) -> u32 {
        self.0
    }

    #[inline(always)]
    pub const fn is_charging(&self) -> bool {
        (self.0 & Self::CHARGING.0) != 0
    }

    #[inline(always)]
    pub const fn is_discharging(&self) -> bool {
        (self.0 & Self::DISCHARGING.0) != 0
    }

    #[inline(always)]
    pub const fn is_critical(&self) -> bool {
        (self.0 & Self::CRITICAL.0) != 0
    }
}

/// Dynamic Battery Status (`_BST`) matching ACPI specification.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BatteryStatus {
    /// Operating state flags (Charging, Discharging, Critical).
    pub state: BatteryStateFlags,
    /// Instantaneous current rate in mA (negative for discharge, positive for charge).
    pub present_rate_ma: i32,
    /// Remaining capacity in milliampere-hours (mAh).
    pub remaining_capacity_mah: u32,
    /// Present battery terminal voltage in millivolts (mV).
    pub present_voltage_mv: u32,
    /// Internal battery cell temperature in tenths of a degree Celsius (e.g. 250 = 25.0°C).
    pub temperature_dci: i16,
}

/// Static Battery Information Extended (`_BIX`) matching ACPI 4.0+ specification.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BatteryInfoExtended {
    /// Factory design capacity in mAh.
    pub design_capacity_mah: u32,
    /// Last full charge capacity in mAh (degrades with battery aging).
    pub last_full_capacity_mah: u32,
    /// Battery chemistry (0 = Li-Ion / Li-Polymer, 1 = NiMH, 2 = Other).
    pub chemistry: u8,
    /// Nominal design terminal voltage in mV (e.g. 7400 mV for 2S, 11100 mV for 3S).
    pub design_voltage_mv: u32,
    /// Low-battery warning threshold in mAh (~10% SoC).
    pub warning_capacity_mah: u32,
    /// Critical low-battery threshold in mAh (~3% SoC - hibernate trigger).
    pub low_capacity_mah: u32,
    /// Cumulative charge/discharge cycle count.
    pub cycle_count: u32,
}

impl BatteryInfoExtended {
    pub const fn new_handheld_2s(design_capacity_mah: u32, last_full_capacity_mah: u32) -> Self {
        Self {
            design_capacity_mah,
            last_full_capacity_mah,
            chemistry: 0, // Li-Ion / LiPo
            design_voltage_mv: 7400, // 2S nominal 7.4V (2x 3.7V)
            warning_capacity_mah: (last_full_capacity_mah * 10) / 100, // 10%
            low_capacity_mah: (last_full_capacity_mah * 3) / 100,       // 3%
            cycle_count: 1,
        }
    }
}

// -----------------------------------------------------------------------------
// High-Precision Coulomb Counter & Fuel Gauge
// -----------------------------------------------------------------------------

/// Open-Circuit Voltage (OCV) calibration point (Cell mV -> SoC permille).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OcvPoint {
    pub cell_mv: u16,
    pub soc_permille: u16,
}

/// Standard Li-Ion/LiPo OCV lookup table for 1S equivalent cell (3.0V empty to 4.2V full).
pub const LI_ION_OCV_TABLE: [OcvPoint; 11] = [
    OcvPoint { cell_mv: 3000, soc_permille: 0 },
    OcvPoint { cell_mv: 3450, soc_permille: 50 },
    OcvPoint { cell_mv: 3680, soc_permille: 100 },
    OcvPoint { cell_mv: 3740, soc_permille: 200 },
    OcvPoint { cell_mv: 3790, soc_permille: 350 },
    OcvPoint { cell_mv: 3840, soc_permille: 500 },
    OcvPoint { cell_mv: 3910, soc_permille: 650 },
    OcvPoint { cell_mv: 3980, soc_permille: 800 },
    OcvPoint { cell_mv: 4080, soc_permille: 900 },
    OcvPoint { cell_mv: 4150, soc_permille: 960 },
    OcvPoint { cell_mv: 4200, soc_permille: 1000 },
];

/// Smart battery fuel gauge controller with Coulomb counting charge integration.
#[derive(Debug, Clone)]
pub struct SmartBatteryGauge {
    pub info: BatteryInfoExtended,
    pub status: BatteryStatus,
    /// Accumulates fractional milliampere-milliseconds (mA * ms) to prevent integration rounding drift.
    pub coulomb_accumulator_ma_ms: i64,
    /// High-precision State of Charge in permille (0..1000 = 0.0% .. 100.0%).
    pub soc_permille: u16,
    /// State of Health in permille (0..1000 = last_full / design).
    pub soh_permille: u16,
    /// Cell series count (e.g. 2 for 2S, 3 for 3S).
    pub cell_series_count: u8,
}

impl SmartBatteryGauge {
    pub fn new(design_capacity_mah: u32, last_full_capacity_mah: u32, cell_series_count: u8) -> Self {
        let info = BatteryInfoExtended::new_handheld_2s(design_capacity_mah, last_full_capacity_mah);
        let soh = if design_capacity_mah > 0 {
            ((last_full_capacity_mah as u64 * 1000) / design_capacity_mah as u64).min(1000) as u16
        } else {
            1000
        };

        let initial_status = BatteryStatus {
            state: BatteryStateFlags::DISCHARGING,
            present_rate_ma: -1500,
            remaining_capacity_mah: last_full_capacity_mah,
            present_voltage_mv: (cell_series_count as u32) * 3840, // ~50% nominal cell
            temperature_dci: 280, // 28.0°C
        };

        Self {
            info,
            status: initial_status,
            coulomb_accumulator_ma_ms: 0,
            soc_permille: 1000, // Starts at 100% full
            soh_permille: soh,
            cell_series_count: cell_series_count.max(1),
        }
    }

    /// Advances Coulomb counter integration over `elapsed_ms` with current rate in mA.
    ///
    /// 1 mAh = 3,600,000 mA * ms.
    pub fn update(&mut self, current_ma: i32, voltage_mv: u32, temp_dci: i16, elapsed_ms: u32) {
        self.status.present_rate_ma = current_ma;
        self.status.present_voltage_mv = voltage_mv;
        self.status.temperature_dci = temp_dci;

        // Update state flags
        if current_ma > 50 {
            self.status.state = BatteryStateFlags::CHARGING;
        } else if current_ma < -50 {
            self.status.state = BatteryStateFlags::DISCHARGING;
        } else {
            self.status.state = BatteryStateFlags::NOT_PRESENT;
        }

        // Integrate charge: delta_q = current_ma * elapsed_ms
        let delta_ma_ms = (current_ma as i64) * (elapsed_ms as i64);
        self.coulomb_accumulator_ma_ms += delta_ma_ms;

        // Extract whole milliampere-hours (3,600,000 mA*ms per mAh)
        let mah_steps = self.coulomb_accumulator_ma_ms / 3_600_000;
        if mah_steps != 0 {
            self.coulomb_accumulator_ma_ms %= 3_600_000;
            let new_cap = (self.status.remaining_capacity_mah as i64) + mah_steps;
            let clamped_cap = new_cap.clamp(0, self.info.last_full_capacity_mah as i64) as u32;
            self.status.remaining_capacity_mah = clamped_cap;
        }

        // Compute State of Charge (SoC)
        if self.info.last_full_capacity_mah > 0 {
            let soc = (self.status.remaining_capacity_mah as u64 * 1000) / (self.info.last_full_capacity_mah as u64);
            self.soc_permille = soc.min(1000) as u16;
        }

        // Check critical low battery flag
        if self.status.remaining_capacity_mah <= self.info.low_capacity_mah {
            self.status.state = BatteryStateFlags(self.status.state.bits() | BatteryStateFlags::CRITICAL.bits());
        }
    }

    /// Recalibrates remaining capacity using Open-Circuit Voltage (OCV) when current is near zero (< 50mA).
    pub fn recalibrate_with_ocv(&mut self, voltage_mv: u32) -> bool {
        if self.status.present_rate_ma.abs() > 50 {
            return false; // Cannot recalibrate under active load
        }

        let cell_mv = (voltage_mv / self.cell_series_count as u32) as u16;

        let estimated_soc = if cell_mv <= LI_ION_OCV_TABLE[0].cell_mv {
            0
        } else if cell_mv >= LI_ION_OCV_TABLE[LI_ION_OCV_TABLE.len() - 1].cell_mv {
            1000
        } else {
            let mut soc = 0;
            for i in 0..(LI_ION_OCV_TABLE.len() - 1) {
                let p0 = LI_ION_OCV_TABLE[i];
                let p1 = LI_ION_OCV_TABLE[i + 1];
                if cell_mv >= p0.cell_mv && cell_mv <= p1.cell_mv {
                    let span_mv = (p1.cell_mv - p0.cell_mv) as i32;
                    let delta_mv = (cell_mv - p0.cell_mv) as i32;
                    let delta_soc = (p1.soc_permille - p0.soc_permille) as i32;
                    soc = (p0.soc_permille as i32 + (delta_mv * delta_soc) / span_mv) as u16;
                    break;
                }
            }
            soc
        };

        self.soc_permille = estimated_soc;
        self.status.remaining_capacity_mah =
            ((self.info.last_full_capacity_mah as u64 * estimated_soc as u64) / 1000) as u32;
        self.coulomb_accumulator_ma_ms = 0;
        true
    }

    #[inline(always)]
    pub fn soc_percent(&self) -> u8 {
        (self.soc_permille / 10) as u8
    }

    #[inline(always)]
    pub fn soh_percent(&self) -> u8 {
        (self.soh_permille / 10) as u8
    }
}

// -----------------------------------------------------------------------------
// Unit Tests & Verification
// -----------------------------------------------------------------------------

/// Unit test verifying battery gauge initialization, Coulomb counting integration, and OCV recalibration.
pub fn verify_battery_gauge_and_coulomb_counter() -> bool {
    // 1. Initialize 2S battery pack: 5000 mAh design, 4800 mAh actual full
    let mut gauge = SmartBatteryGauge::new(5000, 4800, 2);

    // Initial SoH: 4800 / 5000 = 96.0% (960 permille)
    if gauge.soh_permille != 960 || gauge.soh_percent() != 96 {
        return false;
    }
    // Initial SoC: 1000 permille (100%)
    if gauge.soc_permille != 1000 || gauge.soc_percent() != 100 {
        return false;
    }
    if gauge.status.remaining_capacity_mah != 4800 {
        return false;
    }

    // 2. Simulate steady discharge under gaming load: -2500 mA (approx 18.5W on 7.4V)
    // Over 360,000 ms (6 minutes = 0.1 hr):
    // Discharge = 2500 mA * 0.1 hr = 250 mAh.
    // Remaining should be 4800 - 250 = 4550 mAh.
    gauge.update(-2500, 7600, 320, 360_000);

    if gauge.status.remaining_capacity_mah != 4550 {
        return false;
    }
    // Expected SoC: 4550 / 4800 * 1000 = 947 permille (94%)
    if gauge.soc_permille != 947 || gauge.soc_percent() != 94 {
        return false;
    }
    if !gauge.status.state.is_discharging() {
        return false;
    }

    // 3. Simulate high-power USB-PD charging at +3000 mA
    // Over 120,000 ms (2 minutes = 1/30 hr = 0.0333 hr):
    // Charge = 3000 mA * (2 / 60 hr) = 100 mAh.
    // Remaining should be 4550 + 100 = 4650 mAh.
    gauge.update(3000, 8100, 350, 120_000);

    if gauge.status.remaining_capacity_mah != 4650 {
        return false;
    }
    // Expected SoC: 4650 / 4800 * 1000 = 968 permille (96%)
    if gauge.soc_permille != 968 || gauge.soc_percent() != 96 {
        return false;
    }
    if !gauge.status.state.is_charging() {
        return false;
    }

    // 4. Test OCV recalibration when resting (current = 0 mA)
    // Cell voltage 3840 mV (7680 mV total for 2S) corresponds to 500 permille (50% SoC) in LI_ION_OCV_TABLE
    gauge.update(0, 7680, 250, 1000);
    if !gauge.recalibrate_with_ocv(7680) {
        return false;
    }
    if gauge.soc_permille != 500 || gauge.soc_percent() != 50 {
        return false;
    }
    if gauge.status.remaining_capacity_mah != 2400 {
        return false; // 50% of 4800 mAh
    }

    // 5. Test active load rejection for OCV
    gauge.update(-1200, 7500, 300, 1000);
    if gauge.recalibrate_with_ocv(7500) {
        return false; // Must reject OCV recalibration under load
    }

    true
}
