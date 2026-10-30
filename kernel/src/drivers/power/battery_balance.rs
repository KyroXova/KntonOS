//! Handheld PC dual-battery balance manager and USB-PD 3.1 power engine.

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


/// Threshold in millivolts where active cell balancing engages.
pub const BALANCE_THRESHOLD_MV: u16 = 30;
/// Threshold in millivolts where active cell balancing disengages (hysteresis).
pub const BALANCE_HYSTERESIS_MV: u16 = 10;

/// Active balancing state across the dual-cell battery pack.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BalanceState {
    /// Both cells balanced within threshold; shunts disabled.
    Idle,
    /// Cell 1 voltage exceeds Cell 2; bleeding Cell 1 via active shunt.
    BalancingCell1,
    /// Cell 2 voltage exceeds Cell 1; bleeding Cell 2 via active shunt.
    BalancingCell2,
}

/// Thermal throttle zones for handheld battery charging safety.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ThermalThrottleZone {
    /// Below 0°C: Lithium plating hazard, charge strictly disabled.
    ColdCutoff,
    /// 0°C - 15°C: Sub-optimal cell temperature, 50% charge current.
    CoolReduced,
    /// 15°C - 40°C: Optimal operating window, 100% full-rate charge.
    Normal,
    /// 40°C - 45°C: Warm pack during heavy gameplay, 75% throttled current.
    WarmThrottled,
    /// 45°C - 50°C: Hot pack, 40% throttled current.
    HotThrottled,
    /// Above 50°C: Thermal runaway prevention cutoff, 0% current.
    CriticalCutoff,
}

/// Evaluates inter-cell voltage imbalance and manages active balance shunts.
pub fn evaluate_cell_balance(pack: &mut DualBatteryPack, is_charging: bool) -> BalanceState {
    let delta = pack.cell_delta_mv();

    if delta > BALANCE_THRESHOLD_MV {
        if pack.cell1.voltage_mv > pack.cell2.voltage_mv {
            pack.cell1.balance_shunt_active = true;
            pack.cell2.balance_shunt_active = false;
            BalanceState::BalancingCell1
        } else {
            pack.cell1.balance_shunt_active = false;
            pack.cell2.balance_shunt_active = true;
            BalanceState::BalancingCell2
        }
    } else if delta <= BALANCE_HYSTERESIS_MV || !is_charging {
        pack.cell1.balance_shunt_active = false;
        pack.cell2.balance_shunt_active = false;
        BalanceState::Idle
    } else {
        if pack.cell1.balance_shunt_active {
            BalanceState::BalancingCell1
        } else if pack.cell2.balance_shunt_active {
            BalanceState::BalancingCell2
        } else {
            BalanceState::Idle
        }
    }
}

/// OCV (Open Circuit Voltage) lookup table for Li-ion / LiPo pouch cells.
const OCV_CURVE: &[(u16, u8)] = &[
    (4200, 100),
    (4100, 90),
    (4000, 80),
    (3900, 70),
    (3820, 60),
    (3760, 50),
    (3720, 40),
    (3680, 30),
    (3640, 20),
    (3580, 10),
    (3300, 0),
];

/// Estimates cell State-of-Charge (0..100%) using load-compensated Open Circuit Voltage (OCV).
pub fn estimate_cell_soc(terminal_mv: u16, current_ma: i32, internal_res_mohm: u16) -> u8 {
    let ir_drop_mv = (current_ma as i64 * internal_res_mohm as i64) / 1000;
    let ocv_mv = (terminal_mv as i64 - ir_drop_mv).clamp(3000, 4400) as u16;

    if ocv_mv >= OCV_CURVE[0].0 {
        return 100;
    }
    let last_idx = OCV_CURVE.len() - 1;
    if ocv_mv <= OCV_CURVE[last_idx].0 {
        return 0;
    }

    for i in 0..last_idx {
        let (v_high, soc_high) = OCV_CURVE[i];
        let (v_low, soc_low) = OCV_CURVE[i + 1];

        if ocv_mv <= v_high && ocv_mv >= v_low {
            let v_span = (v_high - v_low) as u32;
            let soc_span = (soc_high - soc_low) as u32;
            let offset = (ocv_mv - v_low) as u32;
            return (soc_low as u32 + (offset * soc_span) / v_span) as u8;
        }
    }

    50
}

/// Updates SoC estimates for both cells in the pack using current telemetry.
pub fn update_pack_soc(pack: &mut DualBatteryPack) {
    pack.cell1.state_of_charge_pct = estimate_cell_soc(
        pack.cell1.voltage_mv,
        pack.cell1.current_ma,
        pack.cell1.internal_resistance_mohm,
    );
    pack.cell2.state_of_charge_pct = estimate_cell_soc(
        pack.cell2.voltage_mv,
        pack.cell2.current_ma,
        pack.cell2.internal_resistance_mohm,
    );
}

/// Computes the safe thermal charge current limit and throttle zone.
pub fn calculate_thermal_charge_limit(max_temp_c: f32, max_current_ma: u32) -> (ThermalThrottleZone, u32) {
    if max_temp_c < 0.0 {
        (ThermalThrottleZone::ColdCutoff, 0)
    } else if max_temp_c < 15.0 {
        (ThermalThrottleZone::CoolReduced, max_current_ma / 2)
    } else if max_temp_c <= 40.0 {
        (ThermalThrottleZone::Normal, max_current_ma)
    } else if max_temp_c <= 45.0 {
        (ThermalThrottleZone::WarmThrottled, (max_current_ma * 3) / 4)
    } else if max_temp_c <= 50.0 {
        (ThermalThrottleZone::HotThrottled, (max_current_ma * 2) / 5)
    } else {
        (ThermalThrottleZone::CriticalCutoff, 0)
    }
}

/// Unit test verifying cell load balancing, SoC estimation, and thermal curves.
pub fn verify_cell_balancing_and_thermal_curves() -> bool {
    // 1. Verify Active Cell Load Balancing
    let mut pack = DualBatteryPack::new(3950, 3900); // 50 mV delta > 30 mV
    let state = evaluate_cell_balance(&mut pack, true);
    if state != BalanceState::BalancingCell1 || !pack.cell1.balance_shunt_active || pack.cell2.balance_shunt_active {
        return false;
    }

    // Cell 2 higher
    let mut pack2 = DualBatteryPack::new(3880, 3930); // 50 mV delta
    let state2 = evaluate_cell_balance(&mut pack2, true);
    if state2 != BalanceState::BalancingCell2 || pack2.cell1.balance_shunt_active || !pack2.cell2.balance_shunt_active {
        return false;
    }

    // Balanced within hysteresis (<= 10 mV)
    let mut pack_balanced = DualBatteryPack::new(3905, 3900); // 5 mV delta
    let state_b = evaluate_cell_balance(&mut pack_balanced, true);
    if state_b != BalanceState::Idle || pack_balanced.cell1.balance_shunt_active || pack_balanced.cell2.balance_shunt_active {
        return false;
    }

    // 2. Verify SoC Estimation with Load Compensation
    // At rest (0 mA), 4200 mV -> 100%
    let soc_full = estimate_cell_soc(4200, 0, 20);
    if soc_full != 100 {
        return false;
    }

    // At rest (0 mA), 3760 mV -> 50%
    let soc_half = estimate_cell_soc(3760, 0, 20);
    if soc_half != 50 {
        return false;
    }

    // Under heavy load (discharging -2000 mA), IR drop = (-2000 * 20)/1000 = -40 mV
    // V_terminal = 3720 mV -> V_ocv = 3720 - (-40) = 3760 mV -> 50%
    let soc_loaded = estimate_cell_soc(3720, -2000, 20);
    if soc_loaded != 50 {
        return false;
    }

    // 3. Verify Thermal Throttle Curves
    let (zone_norm, i_norm) = calculate_thermal_charge_limit(35.0, 5000);
    if zone_norm != ThermalThrottleZone::Normal || i_norm != 5000 {
        return false;
    }

    let (zone_warm, i_warm) = calculate_thermal_charge_limit(42.0, 5000);
    if zone_warm != ThermalThrottleZone::WarmThrottled || i_warm != 3750 {
        return false;
    }

    let (zone_hot, i_hot) = calculate_thermal_charge_limit(48.0, 5000);
    if zone_hot != ThermalThrottleZone::HotThrottled || i_hot != 2000 {
        return false;
    }

    let (zone_crit, i_crit) = calculate_thermal_charge_limit(52.5, 5000);
    if zone_crit != ThermalThrottleZone::CriticalCutoff || i_crit != 0 {
        return false;
    }

    let (zone_cold, i_cold) = calculate_thermal_charge_limit(-5.0, 5000);
    if zone_cold != ThermalThrottleZone::ColdCutoff || i_cold != 0 {
        return false;
    }

    true
}

/// Operational power path routing modes for handheld hardware.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PowerPathMode {
    /// No AC attached; system runs entirely from battery pack discharge.
    BatteryDischarge,
    /// AC adapter attached; charging battery while powering system.
    BatteryCharging,
    /// Pass-through bypass active: AC adapter powers APU directly; battery is isolated (0 mA).
    PassThroughBypass,
}

/// Evaluates hardware power path routing based on AC capability, system load, and battery SoC.
pub fn evaluate_power_path(
    ac_profile: Option<PdPowerProfile>,
    apu_load_watts: u16,
    pack: &DualBatteryPack,
    bypass_target_soc: u8,
) -> (PowerPathMode, u16) {
    let profile = match ac_profile {
        Some(p) => p,
        None => return (PowerPathMode::BatteryDischarge, 0),
    };

    // If AC power is less than APU load, system must draw from battery (hybrid discharge)
    if profile.power_watts < apu_load_watts {
        return (PowerPathMode::BatteryDischarge, 0);
    }

    let surplus_watts = profile.power_watts - apu_load_watts;
    let avg_soc = pack.average_soc_pct();

    // If battery is at or above bypass target SoC (e.g. 80% battery health protection or 100% full),
    // engage Pass-Through Bypass mode: AC drives APU directly, zero current cycles battery.
    if avg_soc >= bypass_target_soc {
        (PowerPathMode::PassThroughBypass, 0)
    } else {
        // Battery needs charging: allocate remaining headroom up to pack voltage current limit
        let pack_v = (pack.pack_voltage_mv / 1000).max(6) as u16;
        let available_charge_current_ma = ((surplus_watts as u32 * 1000) / pack_v as u32) as u16;
        (PowerPathMode::BatteryCharging, available_charge_current_ma.min(5000))
    }
}

/// USB-PD 3.1 Extended Power Range (EPR) protocol negotiation state machine.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PdNegotiationState {
    Disconnected,
    SourceCapabilitiesReceived,
    EprModeEntering,
    EprModeActive,
    ContractEstablished,
    HardReset,
}

/// USB-PD 3.1 Extended Power Range (EPR) contract negotiator.
pub struct UsbPdEprNegotiator {
    pub state: PdNegotiationState,
    pub advertised_pdos: [Option<PdPowerProfile>; 6],
    pub advertised_count: usize,
    pub active_contract: Option<PdPowerProfile>,
}

impl UsbPdEprNegotiator {
    /// Creates a new unattached negotiator instance.
    pub const fn new() -> Self {
        Self {
            state: PdNegotiationState::Disconnected,
            advertised_pdos: [None; 6],
            advertised_count: 0,
            active_contract: None,
        }
    }

    /// Ingests advertised Source_Capabilities Power Data Objects (PDOs).
    pub fn on_source_capabilities_received(&mut self, pdos: &[PdPowerProfile]) {
        self.advertised_count = 0;
        for (i, pdo) in pdos.iter().take(6).enumerate() {
            self.advertised_pdos[i] = Some(*pdo);
            self.advertised_count += 1;
        }
        self.state = PdNegotiationState::SourceCapabilitiesReceived;
    }

    /// Selects the highest available power profile, preferring EPR 140W > SPR 100W > SPR 65W > SPR 15W.
    pub fn select_optimal_profile(&self) -> Option<PdPowerProfile> {
        let mut best: Option<PdPowerProfile> = None;
        for i in 0..self.advertised_count {
            if let Some(profile) = self.advertised_pdos[i] {
                match best {
                    None => best = Some(profile),
                    Some(b) => {
                        if profile.power_watts > b.power_watts {
                            best = Some(profile);
                        } else if profile.power_watts == b.power_watts && profile.is_epr() {
                            best = Some(profile);
                        }
                    }
                }
            }
        }
        best
    }

    /// Advances the USB-PD 3.1 contract negotiation state machine.
    pub fn step_negotiation(&mut self) -> Result<PdPowerProfile, &'static str> {
        match self.state {
            PdNegotiationState::Disconnected => Err("No Type-C source attached"),
            PdNegotiationState::SourceCapabilitiesReceived => {
                let optimal = self.select_optimal_profile().ok_or("No valid PDOs in source capabilities")?;
                if optimal.is_epr() {
                    self.state = PdNegotiationState::EprModeEntering;
                    self.state = PdNegotiationState::EprModeActive;
                }
                self.active_contract = Some(optimal);
                self.state = PdNegotiationState::ContractEstablished;
                Ok(optimal)
            }
            PdNegotiationState::EprModeEntering => {
                self.state = PdNegotiationState::EprModeActive;
                let optimal = self.select_optimal_profile().ok_or("No valid PDOs")?;
                self.active_contract = Some(optimal);
                self.state = PdNegotiationState::ContractEstablished;
                Ok(optimal)
            }
            PdNegotiationState::EprModeActive | PdNegotiationState::ContractEstablished => {
                self.active_contract.ok_or("No active contract")
            }
            PdNegotiationState::HardReset => {
                self.active_contract = None;
                self.advertised_count = 0;
                self.state = PdNegotiationState::Disconnected;
                Err("Negotiator in HardReset; link reset to 5V SPR")
            }
        }
    }
}

/// Unit test verifying Pass-Through Bypass mode and USB-PD 3.1 140W EPR negotiation.
pub fn verify_bypass_mode_and_epr_negotiation() -> bool {
    // 1. Verify Pass-Through Bypass Mode Logic
    let mut pack = DualBatteryPack::new(4100, 4100);
    pack.cell1.state_of_charge_pct = 85;
    pack.cell2.state_of_charge_pct = 85;

    // High TDP gameplay (35W APU) on 100W AC adapter, SoC at 85% >= 80% bypass target
    let (mode_bypass, charge_current) = evaluate_power_path(
        Some(PdPowerProfile::spr_100w()),
        35,
        &pack,
        80,
    );
    if mode_bypass != PowerPathMode::PassThroughBypass || charge_current != 0 {
        return false;
    }

    // Battery needs charging: SoC at 40% < 80% bypass target, APU 25W on 65W AC
    pack.cell1.state_of_charge_pct = 40;
    pack.cell2.state_of_charge_pct = 40;
    let (mode_charge, charge_current_act) = evaluate_power_path(
        Some(PdPowerProfile::spr_65w()),
        25,
        &pack,
        80,
    );
    if mode_charge != PowerPathMode::BatteryCharging || charge_current_act == 0 {
        return false;
    }

    // AC disconnected -> BatteryDischarge
    let (mode_batt, _) = evaluate_power_path(None, 25, &pack, 80);
    if mode_batt != PowerPathMode::BatteryDischarge {
        return false;
    }

    // 2. Verify USB-PD 3.1 140W EPR Negotiator
    let mut negotiator = UsbPdEprNegotiator::new();
    let pdos = [
        PdPowerProfile::spr_15w(),
        PdPowerProfile::spr_65w(),
        PdPowerProfile::spr_100w(),
        PdPowerProfile::epr_140w(),
    ];

    negotiator.on_source_capabilities_received(&pdos);
    if negotiator.state != PdNegotiationState::SourceCapabilitiesReceived {
        return false;
    }

    let optimal = match negotiator.select_optimal_profile() {
        Some(p) => p,
        None => return false,
    };
    if optimal.voltage_mv != 28000 || optimal.power_watts != 140 || !optimal.is_epr() {
        return false;
    }

    let contract = match negotiator.step_negotiation() {
        Ok(c) => c,
        Err(_) => return false,
    };
    if contract.power_watts != 140 || contract.voltage_mv != 28000 || !contract.is_epr() {
        return false;
    }
    if negotiator.state != PdNegotiationState::ContractEstablished {
        return false;
    }

    true
}