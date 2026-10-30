//! Unified Handheld PC Multi-Zone Haptic Force-Feedback Trigger & Active Resistance Motor Governor.
//!
//! Provides bare-metal force-feedback adaptive trigger motor control featuring:
//! - Multi-mode active resistance: Linear, Stepped/Clicky, Variable Stop, Frequency Rumble, Recoil Pulse Trains.
//! - Closed-loop PWM duty control with stall-current burnout and thermal overload protection.
//! - Dual-trigger position sensing with microsecond response curve interpolation.
//! - Failsafe neutral state handling during process exit or kernel panic.

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