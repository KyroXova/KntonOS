//! USB Type-C Power Delivery (USB-PD) Contract Negotiator & CCCV Smart Charger Engine.
//!
//! Provides comprehensive charging management for handheld gaming PCs:
//! - USB-PD 3.0 contract negotiation (5V, 9V, 15V, 20V up to 100W)
//! - Multi-stage CCCV (Constant Current, Constant Voltage) charging state machine
//! - Direct AC bypass charging (power pass-through) eliminating battery heat during gaming
//! - Thermal charging current throttling protecting lithium cells against degradation

// -----------------------------------------------------------------------------
// USB-PD Contracts & Power Delivery
// -----------------------------------------------------------------------------

/// Standard USB Type-C Power Delivery voltage rails.
#[repr(u32)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UsbPdVoltage {
    V5_0 = 5000,
    V9_0 = 9000,
    V15_0 = 15000,
    V20_0 = 20000,
}

/// A Power Data Object (PDO) offered by an AC power adapter or dock.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UsbPdContract {
    pub voltage_mv: u32,
    pub max_current_ma: u32,
    pub max_power_mw: u32,
}

impl UsbPdContract {
    pub const fn new(voltage_mv: u32, max_current_ma: u32) -> Self {
        let max_power_mw = (voltage_mv as u64 * max_current_ma as u64 / 1000) as u32;
        Self {
            voltage_mv,
            max_current_ma,
            max_power_mw,
        }
    }
}

// -----------------------------------------------------------------------------
// Charging State Machine & Profiles
// -----------------------------------------------------------------------------

/// Stages of the smart battery charging lifecycle.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChargingState {
    /// Discharging: No external AC power connected.
    Discharging,
    /// Pre-charge / Trickle: Cell voltage deeply depleted (< 3.0V/cell); gentle 0.1C current.
    TrickleCharge,
    /// Constant Current (CC): Bulk charging stage; maximum negotiated current.
    ConstantCurrent,
    /// Constant Voltage (CV): Terminal voltage held at 4.2V/cell; current decays towards cutoff.
    ConstantVoltage,
    /// Float / Charge Complete: Battery at 100%; charging cutoff reached.
    ChargeComplete,
    /// Direct AC Bypass: External power directly supplies system APU; battery current is zero.
    BypassCharging,
    /// Thermal Throttle / Suspended: Battery temperature out-of-spec; charging suspended.
    ThermalSuspended,
}

/// Output decision returned by the charger controller step.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ChargerStepResult {
    pub state: ChargingState,
    pub negotiated_voltage_mv: u32,
    pub charge_current_ma: u32,
    pub system_power_supplied_mw: u32,
    pub is_bypass_active: bool,
}

/// Unified Handheld Smart Charger Controller.
#[derive(Debug, Clone)]
pub struct SmartChargerController {
    pub cell_count: u8,
    pub active_contract: Option<UsbPdContract>,
    pub charging_state: ChargingState,
    pub bypass_mode_enabled: bool,
    pub max_bulk_current_ma: u32,     // e.g. 3000 mA (approx 22W-25W into 2S pack)
    pub cv_cutoff_current_ma: u32,    // e.g. 150 mA termination threshold
    pub target_cell_voltage_mv: u16,  // 4200 mV (4.2V per cell)
}

impl SmartChargerController {
    pub fn new(cell_count: u8) -> Self {
        Self {
            cell_count: cell_count.max(1),
            active_contract: None,
            charging_state: ChargingState::Discharging,
            bypass_mode_enabled: false,
            max_bulk_current_ma: 3000,
            cv_cutoff_current_ma: 150,
            target_cell_voltage_mv: 4200,
        }
    }

    /// Negotiates the highest power USB-PD contract from offered source PDOs (up to 100W).
    pub fn connect_ac_power(&mut self, source_pdos: &[UsbPdContract]) -> Result<UsbPdContract, &'static str> {
        if source_pdos.is_empty() {
            return Err("No USB-PD source PDOs provided");
        }

        // Find candidate offering the highest power up to 100W (100,000 mW)
        let mut best: Option<UsbPdContract> = None;
        for pdo in source_pdos {
            if pdo.max_power_mw <= 100_000 {
                if let Some(current_best) = best {
                    if pdo.max_power_mw > current_best.max_power_mw {
                        best = Some(*pdo);
                    }
                } else {
                    best = Some(*pdo);
                }
            }
        }

        let selected = best.ok_or("No compatible USB-PD contract found")?;
        self.active_contract = Some(selected);
        self.charging_state = ChargingState::ConstantCurrent;
        Ok(selected)
    }

    /// Disconnects external AC power and returns controller to Discharging state.
    pub fn disconnect_ac_power(&mut self) {
        self.active_contract = None;
        self.charging_state = ChargingState::Discharging;
        self.bypass_mode_enabled = false;
    }

    /// Configures direct AC bypass charging (power pass-through).
    pub fn set_bypass_mode(&mut self, enabled: bool) {
        self.bypass_mode_enabled = enabled;
    }

    /// Advances the charger state machine and computes operating parameters.
    pub fn step(
        &mut self,
        soc_permille: u16,
        pack_voltage_mv: u32,
        battery_temp_dci: i16,
        apu_load_mw: u32,
    ) -> ChargerStepResult {
        let contract = match self.active_contract {
            Some(c) => c,
            None => {
                self.charging_state = ChargingState::Discharging;
                return ChargerStepResult {
                    state: ChargingState::Discharging,
                    negotiated_voltage_mv: 0,
                    charge_current_ma: 0,
                    system_power_supplied_mw: 0,
                    is_bypass_active: false,
                };
            }
        };

        let temp_c = battery_temp_dci / 10;
        let cell_mv = (pack_voltage_mv / self.cell_count as u32) as u16;

        // 1. Thermal protection check
        if temp_c >= 55 || temp_c < 0 {
            self.charging_state = ChargingState::ThermalSuspended;
            return ChargerStepResult {
                state: ChargingState::ThermalSuspended,
                negotiated_voltage_mv: contract.voltage_mv,
                charge_current_ma: 0,
                system_power_supplied_mw: apu_load_mw.min(contract.max_power_mw),
                is_bypass_active: true, // System still runs on AC, but battery doesn't charge
            };
        }

        // 2. Direct AC Bypass Charging check (active during gaming when requested)
        if self.bypass_mode_enabled && contract.max_power_mw >= apu_load_mw {
            self.charging_state = ChargingState::BypassCharging;
            return ChargerStepResult {
                state: ChargingState::BypassCharging,
                negotiated_voltage_mv: contract.voltage_mv,
                charge_current_ma: 0,
                system_power_supplied_mw: apu_load_mw,
                is_bypass_active: true,
            };
        }

        // 3. Multi-stage CCCV state machine
        let mut max_current = self.max_bulk_current_ma;

        // Thermal throttling: at 45°C - 55°C, throttle charging current to 1000 mA
        if temp_c >= 45 {
            max_current = max_current.min(1000);
        }

        let (state, charge_current) = if cell_mv < 3000 {
            // Trickle / Pre-charge stage for deeply discharged cells
            (ChargingState::TrickleCharge, 500)
        } else if cell_mv < self.target_cell_voltage_mv {
            // Constant Current (CC) bulk charging
            (ChargingState::ConstantCurrent, max_current)
        } else if soc_permille >= 995 {
            // Charge complete
            (ChargingState::ChargeComplete, 0)
        } else {
            // Constant Voltage (CV) absorption stage: current tapers down
            // Linear approximation of exponential current decay based on SoC (95% -> 100%)
            let remaining_permille = 1000 - soc_permille;
            let cv_current = ((max_current as u32 * remaining_permille as u32) / 50).max(self.cv_cutoff_current_ma);
            if cv_current <= self.cv_cutoff_current_ma {
                (ChargingState::ChargeComplete, 0)
            } else {
                (ChargingState::ConstantVoltage, cv_current)
            }
        };

        self.charging_state = state;

        ChargerStepResult {
            state,
            negotiated_voltage_mv: contract.voltage_mv,
            charge_current_ma: charge_current,
            system_power_supplied_mw: contract.max_power_mw.min(apu_load_mw + (charge_current * pack_voltage_mv / 1000)),
            is_bypass_active: false,
        }
    }
}

// -----------------------------------------------------------------------------
// Unit Tests & Verification
// -----------------------------------------------------------------------------

/// Unit test verifying USB-PD contract negotiation, CCCV state transitions, bypass charging, and thermal throttling.
pub fn verify_usb_pd_and_cccv_charging() -> bool {
    let mut charger = SmartChargerController::new(2); // 2S pack (8.4V full)

    // 1. Verify USB-PD contract negotiation
    let offered_pdos = [
        UsbPdContract::new(5000, 3000),   // 15W
        UsbPdContract::new(9000, 3000),   // 27W
        UsbPdContract::new(15000, 3000),  // 45W
        UsbPdContract::new(20000, 3250),  // 65W
        UsbPdContract::new(20000, 5000),  // 100W
    ];

    let contract = match charger.connect_ac_power(&offered_pdos) {
        Ok(c) => c,
        Err(_) => return false,
    };

    // Should choose 100W contract (20V @ 5A = 100,000 mW)
    if contract.voltage_mv != 20000 || contract.max_current_ma != 5000 || contract.max_power_mw != 100000 {
        return false;
    }

    // 2. Test CCCV Stage 1: Trickle / Pre-charge stage (< 3000 mV/cell = < 6000 mV for 2S)
    let res_trickle = charger.step(10, 5800, 250, 15000);
    if res_trickle.state != ChargingState::TrickleCharge || res_trickle.charge_current_ma != 500 {
        return false;
    }

    // 3. Test CCCV Stage 2: Constant Current bulk charging (3.0V..4.2V/cell)
    // 7600 mV total (3800 mV/cell) at 50% SoC, 30°C
    let res_cc = charger.step(500, 7600, 300, 15000);
    if res_cc.state != ChargingState::ConstantCurrent || res_cc.charge_current_ma != 3000 {
        return false;
    }

    // 4. Test CCCV Stage 3: Constant Voltage absorption stage (4.2V/cell = 8400 mV total) at 97% SoC
    let res_cv = charger.step(970, 8400, 320, 15000);
    if res_cv.state != ChargingState::ConstantVoltage || res_cv.charge_current_ma >= 3000 {
        return false;
    }

    // 5. Test CCCV Stage 4: Charge Complete (100% SoC)
    let res_full = charger.step(1000, 8400, 280, 15000);
    if res_full.state != ChargingState::ChargeComplete || res_full.charge_current_ma != 0 {
        return false;
    }

    // 6. Test Direct AC Bypass Charging (Gaming Pass-Through)
    charger.set_bypass_mode(true);
    let res_bypass = charger.step(700, 7800, 350, 25000); // 25W gaming load
    if res_bypass.state != ChargingState::BypassCharging
        || !res_bypass.is_bypass_active
        || res_bypass.charge_current_ma != 0
        || res_bypass.system_power_supplied_mw != 25000
    {
        return false;
    }
    charger.set_bypass_mode(false);

    // 7. Test Thermal Charging Throttling
    // 7a. Warm Throttling at 48°C (480 dci) -> charge current throttled to 1000 mA
    let res_warm = charger.step(500, 7600, 480, 15000);
    if res_warm.charge_current_ma != 1000 {
        return false;
    }

    // 7b. Over-Temperature Suspended at 58°C (580 dci) -> charging suspended (0 mA)
    let res_hot = charger.step(500, 7600, 580, 15000);
    if res_hot.state != ChargingState::ThermalSuspended || res_hot.charge_current_ma != 0 {
        return false;
    }

    // 7c. Sub-zero Freezing Pack at -5°C (-50 dci) -> charging suspended (0 mA)
    let res_cold = charger.step(500, 7600, -50, 15000);
    if res_cold.state != ChargingState::ThermalSuspended || res_cold.charge_current_ma != 0 {
        return false;
    }

    // 8. Test AC unplug
    charger.disconnect_ac_power();
    let res_unplug = charger.step(500, 7600, 250, 15000);
    if res_unplug.state != ChargingState::Discharging || res_unplug.charge_current_ma != 0 {
        return false;
    }

    true
}
