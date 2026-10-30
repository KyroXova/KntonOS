//! Handheld PC multi-zone haptic force-feedback trigger and active resistance motor governor.

/// Wire format header magic for Trigger Motor Command Packets ("TMGR").
pub const TRIGGER_PACKET_MAGIC: u32 = 0x544D_4752;

/// Identifies an individual force-feedback trigger motor.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TriggerId {
    Left = 0,
    Right = 1,
}

/// Active resistance profile modes for adaptive trigger mechanisms.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResistanceProfile {
    /// Motor disengaged (0% duty cycle, free floating travel).
    Off,
    /// Progressive spring resistance (force proportional to pull depth).
    Linear,
    /// Tactile detent / two-stage click (simulating a firearm sear or camera shutter).
    SteppedClick,
    /// Solid mechanical stop barrier (prohibits trigger pull past specified threshold).
    VariableStop,
    /// High-frequency rumble / vibration buzz (e.g. ABS braking or road surface vibration).
    FrequencyBuzz,
    /// Repetitive recoil impulse pulse train (simulating automatic weapon discharge).
    MachineGunRecoil,
}

/// Operational state descriptor for an individual trigger micromotor.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TriggerMotorDescriptor {
    /// Trigger identifier (Left or Right).
    pub trigger_id: TriggerId,
    /// Active resistance profile.
    pub profile: ResistanceProfile,
    /// Start position threshold along trigger pull (0..255).
    pub start_pos: u8,
    /// End position threshold along trigger pull (0..255).
    pub end_pos: u8,
    /// Target force magnitude / resistance intensity (0..255).
    pub force: u8,
    /// Vibration or recoil frequency in Hz (0..500 Hz).
    pub frequency_hz: u16,
    /// Instantaneous applied PWM duty cycle (0..255).
    pub current_pwm: u8,
    /// True if motor is actively driven.
    pub active: bool,
}

impl TriggerMotorDescriptor {
    /// Creates a new default trigger motor descriptor in neutral/off state.
    pub const fn new(trigger_id: TriggerId) -> Self {
        Self {
            trigger_id,
            profile: ResistanceProfile::Off,
            start_pos: 0,
            end_pos: 255,
            force: 0,
            frequency_hz: 0,
            current_pwm: 0,
            active: false,
        }
    }
}

/// Serialized PWM command packet wire format for embedded trigger motor controller (MCU/EC).
#[repr(C, packed)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TriggerMotorPacket {
    /// Magic header identifier (0x544D_4752).
    pub magic: u32,
    /// Target trigger (0 = Left, 1 = Right).
    pub trigger_index: u8,
    /// Resistance profile mode ID.
    pub profile_id: u8,
    /// Start position threshold (0..255).
    pub start_pos: u8,
    /// End position threshold (0..255).
    pub end_pos: u8,
    /// Force magnitude (0..255).
    pub force: u8,
    /// Frequency low byte.
    pub freq_lo: u8,
    /// Frequency high byte.
    pub freq_hi: u8,
    /// Packet simple XOR checksum.
    pub checksum: u8,
}

impl TriggerMotorPacket {
    /// Encodes a motor command packet from descriptor state.
    pub fn encode(desc: &TriggerMotorDescriptor) -> Self {
        let profile_id = match desc.profile {
            ResistanceProfile::Off => 0,
            ResistanceProfile::Linear => 1,
            ResistanceProfile::SteppedClick => 2,
            ResistanceProfile::VariableStop => 3,
            ResistanceProfile::FrequencyBuzz => 4,
            ResistanceProfile::MachineGunRecoil => 5,
        };

        let freq_lo = (desc.frequency_hz & 0xFF) as u8;
        let freq_hi = ((desc.frequency_hz >> 8) & 0xFF) as u8;
        let trigger_idx = desc.trigger_id as u8;

        let mut pkt = Self {
            magic: TRIGGER_PACKET_MAGIC,
            trigger_index: trigger_idx,
            profile_id,
            start_pos: desc.start_pos,
            end_pos: desc.end_pos,
            force: desc.force,
            freq_lo,
            freq_hi,
            checksum: 0,
        };
        pkt.checksum = pkt.compute_checksum();
        pkt
    }

    /// Computes XOR checksum over all packet payload bytes.
    pub fn compute_checksum(&self) -> u8 {
        let m = self.magic;
        let b0 = (m & 0xFF) as u8;
        let b1 = ((m >> 8) & 0xFF) as u8;
        let b2 = ((m >> 16) & 0xFF) as u8;
        let b3 = ((m >> 24) & 0xFF) as u8;

        b0 ^ b1 ^ b2 ^ b3
            ^ self.trigger_index
            ^ self.profile_id
            ^ self.start_pos
            ^ self.end_pos
            ^ self.force
            ^ self.freq_lo
            ^ self.freq_hi
    }

    /// Validates packet magic and checksum integrity.
    pub fn is_valid(&self) -> bool {
        self.magic == TRIGGER_PACKET_MAGIC && self.checksum == self.compute_checksum()
    }
}

/// Verification helper for Commit 1 structures, profiles, and wire packets.
pub fn verify_trigger_motor_descriptors_and_wire_format() -> bool {
    // 1. Verify Descriptor Defaults
    let left = TriggerMotorDescriptor::new(TriggerId::Left);
    if left.trigger_id != TriggerId::Left || left.profile != ResistanceProfile::Off || left.active {
        return false;
    }

    // 2. Configure Variable Stop Profile on Right Trigger
    let mut right = TriggerMotorDescriptor::new(TriggerId::Right);
    right.profile = ResistanceProfile::VariableStop;
    right.start_pos = 180;
    right.end_pos = 255;
    right.force = 240;
    right.frequency_hz = 0;

    // 3. Encode to Wire Format Packet
    let pkt = TriggerMotorPacket::encode(&right);
    if !pkt.is_valid() {
        return false;
    }
    if pkt.trigger_index != 1 || pkt.profile_id != 3 || pkt.start_pos != 180 || pkt.force != 240 {
        return false;
    }

    // 4. Test Recoil Frequency Serialization
    let mut recoil = TriggerMotorDescriptor::new(TriggerId::Right);
    recoil.profile = ResistanceProfile::MachineGunRecoil;
    recoil.force = 255;
    recoil.frequency_hz = 280; // > 255 (spans hi/lo byte)
    let pkt_recoil = TriggerMotorPacket::encode(&recoil);
    if !pkt_recoil.is_valid() || pkt_recoil.profile_id != 5 {
        return false;
    }
    let reconstructed_freq = (pkt_recoil.freq_lo as u16) | ((pkt_recoil.freq_hi as u16) << 8);
    if reconstructed_freq != 280 {
        return false;
    }

    true
}
/// Stall current threshold in milliamperes where micromotor rotor lockup is detected.
pub const STALL_CURRENT_THRESHOLD_MA: u16 = 600;
/// Maximum ticks a motor can remain in stall before current protection engages (150ms at 100Hz).
pub const STALL_DURATION_LIMIT_TICKS: u32 = 15;
/// Maximum clamped PWM duty allowed while stall protection is active (to prevent coil burnout).
pub const SAFE_HOLD_DUTY_LIMIT: u8 = 75;
/// Critical coil temperature limit in °C where thermal throttling begins.
pub const COIL_THERMAL_LIMIT_C: f32 = 75.0;

/// Computes the raw target PWM duty cycle for a given profile and trigger position.
pub fn calculate_profile_pwm(
    desc: &TriggerMotorDescriptor,
    current_pos: u8,
    time_ms: u32,
) -> u8 {
    match desc.profile {
        ResistanceProfile::Off => 0,

        ResistanceProfile::Linear => {
            if current_pos <= desc.start_pos {
                0
            } else if current_pos >= desc.end_pos {
                desc.force
            } else {
                let span = (desc.end_pos - desc.start_pos) as u32;
                let offset = (current_pos - desc.start_pos) as u32;
                ((offset * desc.force as u32) / span) as u8
            }
        }

        ResistanceProfile::SteppedClick => {
            // Tactile detent: light take-up -> high resistance peak -> crisp snap-through break
            if current_pos < desc.start_pos.saturating_sub(15) {
                // Pre-travel light spring resistance (20% force)
                (desc.force as u16 / 5) as u8
            } else if current_pos <= desc.start_pos {
                // Detent wall / sear resistance: full peak force
                desc.force
            } else {
                // Post-break follow-through: drop to 10% force
                (desc.force as u16 / 10) as u8
            }
        }

        ResistanceProfile::VariableStop => {
            // Solid barrier: 0 before start_pos, full blocking counter-force at/past start_pos
            if current_pos >= desc.start_pos {
                desc.force
            } else {
                0
            }
        }

        ResistanceProfile::FrequencyBuzz => {
            if current_pos >= desc.start_pos && current_pos <= desc.end_pos {
                let freq = desc.frequency_hz.max(10) as u32;
                let period_ms = (1000 / freq).max(2);
                let phase = time_ms % period_ms;
                if phase < (period_ms / 2) {
                    desc.force
                } else {
                    0
                }
            } else {
                0
            }
        }

        ResistanceProfile::MachineGunRecoil => {
            if current_pos >= desc.start_pos {
                let freq = desc.frequency_hz.clamp(5, 25) as u32; // 5 to 25 shots/sec
                let cycle_ms = 1000 / freq;
                let phase_ms = time_ms % cycle_ms;
                // High-force kick for first 25ms of each cycle, followed by relaxation
                if phase_ms < 25 {
                    desc.force
                } else {
                    0
                }
            } else {
                0
            }
        }
    }
}

/// Active Resistance Motor Governor with closed-loop stall protection and thermal throttling.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TriggerMotorGovernor {
    /// Descriptors for Left and Right trigger motors.
    pub descriptors: [TriggerMotorDescriptor; 2],
    /// Consecutive stall ticks per trigger.
    pub stall_ticks: [u32; 2],
    /// True if stall burnout protection is actively clamping PWM duty.
    pub stall_tripped: [bool; 2],
    /// Estimated coil temperature in °C per trigger.
    pub coil_temp_c: [f32; 2],
}

impl TriggerMotorGovernor {
    /// Creates a new governor with default initialized motors.
    pub const fn new() -> Self {
        Self {
            descriptors: [
                TriggerMotorDescriptor::new(TriggerId::Left),
                TriggerMotorDescriptor::new(TriggerId::Right),
            ],
            stall_ticks: [0, 0],
            stall_tripped: [false, false],
            coil_temp_c: [30.0, 30.0],
        }
    }

    /// Evaluates trigger position and motor current to compute safe output PWM duty.
    pub fn update_motor(
        &mut self,
        trigger: TriggerId,
        current_pos: u8,
        measured_current_ma: u16,
        time_ms: u32,
    ) -> u8 {
        let idx = trigger as usize;
        let desc = &mut self.descriptors[idx];

        // 1. Calculate raw target PWM from profile curve
        let raw_pwm = calculate_profile_pwm(desc, current_pos, time_ms);

        // 2. Stall-current burnout protection logic
        if measured_current_ma >= STALL_CURRENT_THRESHOLD_MA && raw_pwm > 0 {
            self.stall_ticks[idx] += 1;
            if self.stall_ticks[idx] >= STALL_DURATION_LIMIT_TICKS {
                self.stall_tripped[idx] = true;
            }
        } else {
            // Recover when current drops or motor commanded off
            if self.stall_ticks[idx] > 0 {
                self.stall_ticks[idx] = self.stall_ticks[idx].saturating_sub(2);
            }
            if measured_current_ma < (STALL_CURRENT_THRESHOLD_MA - 150) {
                self.stall_tripped[idx] = false;
            }
        }

        // 3. Thermal coil estimator (Joulean heating I^2 * R approximation)
        if raw_pwm > 0 {
            self.coil_temp_c[idx] += 0.05 * (raw_pwm as f32 / 255.0);
        } else {
            self.coil_temp_c[idx] = (self.coil_temp_c[idx] - 0.1).max(25.0);
        }

        // 4. Apply protections to final PWM
        let mut final_pwm = raw_pwm;

        // Apply stall current clamping
        if self.stall_tripped[idx] {
            final_pwm = final_pwm.min(SAFE_HOLD_DUTY_LIMIT);
        }

        // Apply thermal overload throttling
        if self.coil_temp_c[idx] >= COIL_THERMAL_LIMIT_C {
            final_pwm = (final_pwm as u16 / 2) as u8;
        }

        desc.current_pwm = final_pwm;
        desc.active = final_pwm > 0;
        final_pwm
    }
}

/// Verification helper for Commit 2 governor curves and stall protection.
pub fn verify_active_resistance_governor_and_stall_protection() -> bool {
    let mut gov = TriggerMotorGovernor::new();

    // 1. Verify Linear Profile Curve
    gov.descriptors[0].profile = ResistanceProfile::Linear;
    gov.descriptors[0].start_pos = 50;
    gov.descriptors[0].end_pos = 200;
    gov.descriptors[0].force = 150;

    // Below start_pos (pos 30) -> 0
    let pwm_pre = gov.update_motor(TriggerId::Left, 30, 100, 0);
    if pwm_pre != 0 {
        return false;
    }

    // Midpoint (pos 125 = exactly 50% between 50 and 200) -> 75
    let pwm_mid = gov.update_motor(TriggerId::Left, 125, 100, 0);
    if pwm_mid != 75 {
        return false;
    }

    // At or past end_pos (pos 210) -> full force 150
    let pwm_end = gov.update_motor(TriggerId::Left, 210, 100, 0);
    if pwm_end != 150 {
        return false;
    }

    // 2. Verify Stepped Click Profile (Sear break detent)
    gov.descriptors[1].profile = ResistanceProfile::SteppedClick;
    gov.descriptors[1].start_pos = 120;
    gov.descriptors[1].force = 200;

    // Pre-travel (pos 80) -> light spring (200 / 5 = 40)
    let pwm_takeup = gov.update_motor(TriggerId::Right, 80, 50, 0);
    if pwm_takeup != 40 {
        return false;
    }

    // Detent peak at wall (pos 120) -> 200
    let pwm_sear = gov.update_motor(TriggerId::Right, 120, 100, 0);
    if pwm_sear != 200 {
        return false;
    }

    // Post-break follow-through (pos 130) -> crisp drop (200 / 10 = 20)
    let pwm_break = gov.update_motor(TriggerId::Right, 130, 50, 0);
    if pwm_break != 20 {
        return false;
    }

    // 3. Verify Stall-Current Burnout Protection
    let mut stall_gov = TriggerMotorGovernor::new();
    stall_gov.descriptors[0].profile = ResistanceProfile::VariableStop;
    stall_gov.descriptors[0].start_pos = 100;
    stall_gov.descriptors[0].force = 255;

    // Normal operation before stall trips
    let p_init = stall_gov.update_motor(TriggerId::Left, 120, 700, 0);
    if p_init != 255 || stall_gov.stall_tripped[0] {
        return false;
    }

    // Simulate sustained stall current (> 600mA for >= 15 ticks)
    for t in 1..=16 {
        stall_gov.update_motor(TriggerId::Left, 120, 750, t);
    }
    // Now stall should be tripped and PWM clamped to SAFE_HOLD_DUTY_LIMIT (75)
    let p_stalled = stall_gov.update_motor(TriggerId::Left, 120, 750, 20);
    if !stall_gov.stall_tripped[0] || p_stalled != SAFE_HOLD_DUTY_LIMIT {
        return false;
    }

    true
}
/// Calibration parameters for normalizing raw Hall/ADC trigger inputs into unified 0..255 range.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TriggerCalibration {
    /// Minimum raw ADC value at physical resting stop.
    pub min_raw: u16,
    /// Maximum raw ADC value at physical bottom-out.
    pub max_raw: u16,
    /// Deadzone threshold at resting position (0..255).
    pub rest_deadzone: u8,
    /// Deadzone threshold at full pull (0..255).
    pub full_deadzone: u8,
}

impl TriggerCalibration {
    /// Creates a typical calibration for handheld Hall-effect trigger sensors.
    pub const fn new(min_raw: u16, max_raw: u16) -> Self {
        Self {
            min_raw,
            max_raw,
            rest_deadzone: 8,
            full_deadzone: 248,
        }
    }

    /// Normalizes a raw ADC sensor reading into a calibrated 0..255 position with deadzone filtering.
    pub fn normalize_reading(&self, raw: u16) -> u8 {
        if raw <= self.min_raw {
            return 0;
        }
        if raw >= self.max_raw {
            return 255;
        }

        let span = (self.max_raw - self.min_raw) as u32;
        let offset = (raw - self.min_raw) as u32;
        let scaled = ((offset * 255) / span) as u8;

        if scaled <= self.rest_deadzone {
            0
        } else if scaled >= self.full_deadzone {
            255
        } else {
            let active_span = (self.full_deadzone - self.rest_deadzone) as u32;
            let active_offset = (scaled - self.rest_deadzone) as u32;
            ((active_offset * 255) / active_span) as u8
        }
    }
}

/// High-level game force-feedback trigger effect presets.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GameTriggerEffect {
    /// Unforced standard trigger travel.
    DefaultOff,
    /// Semi-automatic handgun: Crisp sear wall at 65% pull, followed by snap-through.
    HandgunSingleAction,
    /// Heavy shotgun: Stiff mechanical wall at 50% pull requiring high break force.
    ShotgunHeavyWall,
    /// Fully automatic rifle: 12 Hz recoil pulse train once pull passes 35%.
    AutomaticRifle,
    /// Archery bow draw: Progressive exponential tension resisting deeper pull.
    BowAndArrow,
    /// Racing ABS braking: 80 Hz haptic buzz when trigger pull exceeds 80%.
    AbsBrakingVibration,
}

/// Game Force-Feedback Synthesizer coordinating dual-trigger profiles and safety neutralization.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TriggerEffectSynthesizer {
    /// Active motor governor.
    pub governor: TriggerMotorGovernor,
    /// Calibration profiles for left and right triggers.
    pub calibration: [TriggerCalibration; 2],
    /// Active high-level game effect preset per trigger.
    pub active_effects: [GameTriggerEffect; 2],
    /// True if all motors have been neutralized to failsafe neutral state.
    pub neutralized: bool,
}

impl TriggerEffectSynthesizer {
    /// Creates a new synthesizer instance with calibrated defaults.
    pub const fn new() -> Self {
        Self {
            governor: TriggerMotorGovernor::new(),
            calibration: [
                TriggerCalibration::new(500, 3500),
                TriggerCalibration::new(500, 3500),
            ],
            active_effects: [GameTriggerEffect::DefaultOff, GameTriggerEffect::DefaultOff],
            neutralized: false,
        }
    }

    /// Configures an active game-defined force-feedback effect onto the target trigger.
    pub fn apply_game_effect(&mut self, trigger: TriggerId, effect: GameTriggerEffect) {
        let idx = trigger as usize;
        self.active_effects[idx] = effect;
        self.neutralized = false;

        let desc = &mut self.governor.descriptors[idx];

        match effect {
            GameTriggerEffect::DefaultOff => {
                desc.profile = ResistanceProfile::Off;
                desc.force = 0;
            }
            GameTriggerEffect::HandgunSingleAction => {
                desc.profile = ResistanceProfile::SteppedClick;
                desc.start_pos = 160; // 63% travel
                desc.end_pos = 255;
                desc.force = 220;
                desc.frequency_hz = 0;
            }
            GameTriggerEffect::ShotgunHeavyWall => {
                desc.profile = ResistanceProfile::VariableStop;
                desc.start_pos = 128; // 50% travel
                desc.end_pos = 255;
                desc.force = 255; // Maximum resistance barrier
                desc.frequency_hz = 0;
            }
            GameTriggerEffect::AutomaticRifle => {
                desc.profile = ResistanceProfile::MachineGunRecoil;
                desc.start_pos = 90; // 35% travel
                desc.end_pos = 255;
                desc.force = 240;
                desc.frequency_hz = 12; // 12 shots per second
            }
            GameTriggerEffect::BowAndArrow => {
                desc.profile = ResistanceProfile::Linear;
                desc.start_pos = 20;
                desc.end_pos = 240;
                desc.force = 230;
                desc.frequency_hz = 0;
            }
            GameTriggerEffect::AbsBrakingVibration => {
                desc.profile = ResistanceProfile::FrequencyBuzz;
                desc.start_pos = 200; // 80% travel
                desc.end_pos = 255;
                desc.force = 200;
                desc.frequency_hz = 80; // 80 Hz ABS vibration
            }
        }
    }

    /// Failsafe neutral state: immediately de-energizes all micromotors to 0% PWM.
    pub fn neutralize_all(&mut self) {
        for i in 0..2 {
            self.governor.descriptors[i].profile = ResistanceProfile::Off;
            self.governor.descriptors[i].current_pwm = 0;
            self.governor.descriptors[i].force = 0;
            self.governor.descriptors[i].active = false;
            self.active_effects[i] = GameTriggerEffect::DefaultOff;
        }
        self.neutralized = true;
    }

    /// Updates motor output from raw ADC sensor inputs.
    pub fn update_from_raw_adc(
        &mut self,
        trigger: TriggerId,
        raw_adc: u16,
        measured_current_ma: u16,
        time_ms: u32,
    ) -> u8 {
        if self.neutralized {
            return 0;
        }
        let idx = trigger as usize;
        let normalized_pos = self.calibration[idx].normalize_reading(raw_adc);
        self.governor.update_motor(trigger, normalized_pos, measured_current_ma, time_ms)
    }
}

/// Verification helper for Commit 3 calibration and game effect synthesizer.
pub fn verify_calibration_and_effect_synthesizer() -> bool {
    // 1. Verify Trigger Calibration and Deadzone Normalization
    let calib = TriggerCalibration::new(500, 3500);

    // Below min_raw -> 0
    if calib.normalize_reading(450) != 0 {
        return false;
    }
    // Rest deadzone area -> 0
    if calib.normalize_reading(550) != 0 {
        return false;
    }
    // Midpoint reading ~2000 -> approximately 128
    let mid_pos = calib.normalize_reading(2000);
    if mid_pos < 120 || mid_pos > 136 {
        return false;
    }
    // Full deadzone reading ~3450 -> 255
    if calib.normalize_reading(3450) != 255 {
        return false;
    }

    // 2. Verify Game Effect Synthesis
    let mut synth = TriggerEffectSynthesizer::new();

    // Configure Shotgun effect on Right trigger (hard stop at 50% = pos 128)
    synth.apply_game_effect(TriggerId::Right, GameTriggerEffect::ShotgunHeavyWall);
    if synth.governor.descriptors[1].profile != ResistanceProfile::VariableStop {
        return false;
    }
    if synth.governor.descriptors[1].start_pos != 128 || synth.governor.descriptors[1].force != 255 {
        return false;
    }

    // Test raw ADC update: raw 1000 (pre-wall) -> 0 PWM
    let pwm_pre_wall = synth.update_from_raw_adc(TriggerId::Right, 1000, 100, 0);
    if pwm_pre_wall != 0 {
        return false;
    }
    // Raw 2800 (past 50% wall) -> 255 PWM
    let pwm_post_wall = synth.update_from_raw_adc(TriggerId::Right, 2800, 100, 0);
    if pwm_post_wall != 255 {
        return false;
    }

    // 3. Verify Failsafe Neutral State
    synth.neutralize_all();
    if !synth.neutralized {
        return false;
    }
    if synth.governor.descriptors[1].profile != ResistanceProfile::Off || synth.governor.descriptors[1].current_pwm != 0 {
        return false;
    }
    let pwm_neutral = synth.update_from_raw_adc(TriggerId::Right, 2800, 100, 0);
    if pwm_neutral != 0 {
        return false;
    }

    true
}