//! Kernel-Direct Dual-Motor Rumble Controller & Playback Sequence Queue.
//!
//! Controls low-frequency (LF / heavy mass) and high-frequency (HF / light mass)
//! gamepad rumble motors, manages active vibration step sequences, executes
//! real-time timeline mixing, and evaluates motor output values (0-255).

use alloc::vec::Vec;
use crate::drivers::input::waveform::WaveformSynthesizer;

// -----------------------------------------------------------------------------
// Dual-Motor State & Step Sequence
// -----------------------------------------------------------------------------

/// Output vibration intensity for dual-motor gamepads.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct MotorSpeeds {
    /// Strong motor / Low Frequency (LF) - heavy weight for deep rumble (0 to 255).
    pub strong_lf: u8,
    /// Weak motor / High Frequency (HF) - light weight for crisp buzz/click (0 to 255).
    pub weak_hf: u8,
}

impl MotorSpeeds {
    pub const fn new(strong_lf: u8, weak_hf: u8) -> Self {
        Self { strong_lf, weak_hf }
    }

    pub const fn off() -> Self {
        Self { strong_lf: 0, weak_hf: 0 }
    }

    pub fn is_active(&self) -> bool {
        self.strong_lf > 0 || self.weak_hf > 0
    }
}

/// Single step or layer in a haptic playback sequence.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RumbleEffectStep {
    /// Effect identifier for cancellation or tracking.
    pub effect_id: u32,
    /// Delay before playback starts (in milliseconds).
    pub start_delay_ms: u16,
    /// Total active duration of this effect (in milliseconds).
    pub duration_ms: u16,
    /// Low Frequency motor synthesizer.
    pub strong_synth: Option<WaveformSynthesizer>,
    /// High Frequency motor synthesizer.
    pub weak_synth: Option<WaveformSynthesizer>,
    /// Elapsed playback time in milliseconds.
    pub elapsed_ms: u16,
    /// Whether this step has finished executing.
    pub finished: bool,
}

impl RumbleEffectStep {
    pub fn new(
        effect_id: u32,
        start_delay_ms: u16,
        duration_ms: u16,
        strong_synth: Option<WaveformSynthesizer>,
        weak_synth: Option<WaveformSynthesizer>,
    ) -> Self {
        Self {
            effect_id,
            start_delay_ms,
            duration_ms,
            strong_synth,
            weak_synth,
            elapsed_ms: 0,
            finished: false,
        }
    }

    /// Evaluates motor speeds for the current elapsed time step.
    pub fn sample(&self) -> MotorSpeeds {
        if self.finished || self.elapsed_ms < self.start_delay_ms {
            return MotorSpeeds::off();
        }

        let active_t = self.elapsed_ms - self.start_delay_ms;
        if active_t >= self.duration_ms {
            return MotorSpeeds::off();
        }

        let lf = self.strong_synth.map_or(0, |s| s.sample_amplitude(active_t));
        let hf = self.weak_synth.map_or(0, |s| s.sample_amplitude(active_t));

        MotorSpeeds::new(lf, hf)
    }

    /// Advances playback timeline by `delta_ms`.
    pub fn advance(&mut self, delta_ms: u16) {
        if self.finished {
            return;
        }

        self.elapsed_ms = self.elapsed_ms.saturating_add(delta_ms);
        let total_lifetime = self.start_delay_ms.saturating_add(self.duration_ms);
        if self.elapsed_ms >= total_lifetime {
            self.finished = true;
        }
    }
}

// -----------------------------------------------------------------------------
// Playback Sequence Queue & Controller
// -----------------------------------------------------------------------------

/// Maximum concurrent active rumble effects allowed in playback queue.
pub const MAX_RUMBLE_QUEUE_CAPACITY: usize = 16;

/// Dual-Motor Rumble Controller managing playback sequence queue.
#[derive(Debug, Clone)]
pub struct DualMotorRumbleController {
    /// Active effect steps being mixed and played.
    pub active_effects: Vec<RumbleEffectStep>,
    /// Global master rumble gain (0 to 255).
    pub master_gain: u8,
    /// Current mixed motor output speeds.
    pub current_motors: MotorSpeeds,
    /// Monotonic total milliseconds processed.
    pub total_playback_ms: u64,
}

impl DualMotorRumbleController {
    pub fn new(master_gain: u8) -> Self {
        Self {
            active_effects: Vec::with_capacity(MAX_RUMBLE_QUEUE_CAPACITY),
            master_gain,
            current_motors: MotorSpeeds::off(),
            total_playback_ms: 0,
        }
    }

    /// Submits a new rumble effect step into the playback queue.
    pub fn enqueue_effect(&mut self, effect: RumbleEffectStep) -> bool {
        if self.active_effects.len() >= MAX_RUMBLE_QUEUE_CAPACITY {
            return false;
        }
        self.active_effects.push(effect);
        true
    }

    /// Cancels all active effects with the specified `effect_id`.
    pub fn cancel_effect(&mut self, effect_id: u32) {
        for effect in self.active_effects.iter_mut() {
            if effect.effect_id == effect_id {
                effect.finished = true;
            }
        }
        self.active_effects.retain(|e| !e.finished);
    }

    /// Stops all ongoing haptic feedback immediately.
    pub fn stop_all(&mut self) {
        self.active_effects.clear();
        self.current_motors = MotorSpeeds::off();
    }

    /// Updates playback by `delta_ms` and computes the combined dual-motor output.
    pub fn update(&mut self, delta_ms: u16) -> MotorSpeeds {
        self.total_playback_ms = self.total_playback_ms.saturating_add(delta_ms as u64);

        if self.active_effects.is_empty() || self.master_gain == 0 {
            self.current_motors = MotorSpeeds::off();
            return self.current_motors;
        }

        // Advance timelines and mix active layers using peak-hold / maximum mixing
        let mut max_lf: u8 = 0;
        let mut max_hf: u8 = 0;

        for effect in self.active_effects.iter_mut() {
            let sample = effect.sample();
            max_lf = max_lf.max(sample.strong_lf);
            max_hf = max_hf.max(sample.weak_hf);

            effect.advance(delta_ms);
        }

        // Clean up finished effects
        self.active_effects.retain(|e| !e.finished);

        // Apply master gain
        let final_lf = ((max_lf as u32 * self.master_gain as u32) / 255) as u8;
        let final_hf = ((max_hf as u32 * self.master_gain as u32) / 255) as u8;

        self.current_motors = MotorSpeeds::new(final_lf, final_hf);
        self.current_motors
    }
}

// -----------------------------------------------------------------------------
// Unit Tests & Verification
// -----------------------------------------------------------------------------

// -----------------------------------------------------------------------------
// USB HID & Gamepad Force Feedback Report Framing
// -----------------------------------------------------------------------------

/// Wire protocol for gamepad force feedback / vibration output report.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HapticReportProtocol {
    /// Xbox One / Series XINPUT vibration report (Report ID 0x09, 8 bytes).
    XboxOneGip,
    /// Standard USB HID PID (Physical Interface Device) force feedback output report.
    UsbHidPid,
    /// DualSense / DualShock 4 haptic output packet (Report ID 0x05 / 0x11).
    PlayStationDs,
}

/// Standard Xbox GIP Haptic Output Report (Report ID 0x09).
/// Format:
/// [0]: Report ID (0x09)
/// [1]: Sub-command / flags (0x00)
/// [2]: Left trigger motor (0-255)
/// [3]: Right trigger motor (0-255)
/// [4]: Left main LF motor (0-255)
/// [5]: Right main HF motor (0-255)
/// [6]: Duration in 10ms increments (0 = continuous)
/// [7]: Delay in 10ms increments
#[repr(C, packed)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct XboxHapticReport {
    pub report_id: u8,
    pub flags: u8,
    pub left_trigger: u8,
    pub right_trigger: u8,
    pub strong_lf: u8,
    pub weak_hf: u8,
    pub duration_10ms: u8,
    pub delay_10ms: u8,
}

impl XboxHapticReport {
    pub fn new(speeds: MotorSpeeds) -> Self {
        Self {
            report_id: 0x09,
            flags: 0x00,
            left_trigger: 0,
            right_trigger: 0,
            strong_lf: speeds.strong_lf,
            weak_hf: speeds.weak_hf,
            duration_10ms: 0xFF, // continuous until next report
            delay_10ms: 0x00,
        }
    }

    pub fn to_bytes(&self) -> [u8; 8] {
        [
            self.report_id,
            self.flags,
            self.left_trigger,
            self.right_trigger,
            self.strong_lf,
            self.weak_hf,
            self.duration_10ms,
            self.delay_10ms,
        ]
    }
}

/// Standard USB HID Force Feedback PID (Physical Interface Device) Report.
/// Report ID 0x01: Output Report with 16-bit magnitude values.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UsbHidPidReport {
    pub report_id: u8,
    pub strong_lf_magnitude: u16, // 0..=65535
    pub weak_hf_magnitude: u16,   // 0..=65535
}

impl UsbHidPidReport {
    pub fn new(speeds: MotorSpeeds) -> Self {
        Self {
            report_id: 0x01,
            strong_lf_magnitude: (speeds.strong_lf as u16) * 257, // 0..255 -> 0..65535
            weak_hf_magnitude: (speeds.weak_hf as u16) * 257,
        }
    }

    pub fn to_bytes(&self) -> [u8; 5] {
        [
            self.report_id,
            (self.strong_lf_magnitude & 0xFF) as u8,
            ((self.strong_lf_magnitude >> 8) & 0xFF) as u8,
            (self.weak_hf_magnitude & 0xFF) as u8,
            ((self.weak_hf_magnitude >> 8) & 0xFF) as u8,
        ]
    }
}

/// Formatter creating device-specific wire packets from motor speeds.
pub struct HapticReportFormatter;

impl HapticReportFormatter {
    /// Formats an output report byte buffer according to the target protocol.
    pub fn format_report(protocol: HapticReportProtocol, speeds: MotorSpeeds, out_buf: &mut [u8]) -> usize {
        match protocol {
            HapticReportProtocol::XboxOneGip => {
                if out_buf.len() < 8 {
                    return 0;
                }
                let report = XboxHapticReport::new(speeds);
                out_buf[..8].copy_from_slice(&report.to_bytes());
                8
            }
            HapticReportProtocol::UsbHidPid => {
                if out_buf.len() < 5 {
                    return 0;
                }
                let report = UsbHidPidReport::new(speeds);
                out_buf[..5].copy_from_slice(&report.to_bytes());
                5
            }
            HapticReportProtocol::PlayStationDs => {
                if out_buf.len() < 10 {
                    return 0;
                }
                // DualSense / DS4 compatible rumble frame
                // [0] = 0x05 (Report ID)
                // [1] = 0x07 (Motor flag enable)
                // [2] = 0x00
                // [3] = Weak HF motor (right)
                // [4] = Strong LF motor (left)
                out_buf[0] = 0x05;
                out_buf[1] = 0x07;
                out_buf[2] = 0x00;
                out_buf[3] = speeds.weak_hf;
                out_buf[4] = speeds.strong_lf;
                for i in 5..10 {
                    out_buf[i] = 0;
                }
                10
            }
        }
    }
}

/// Unit test verifying USB HID and Gamepad force feedback report packet formatting.
pub fn verify_haptic_report_formatting() -> bool {
    let speeds = MotorSpeeds::new(180, 220);

    // 1. Xbox GIP framing
    let mut xbox_buf = [0u8; 8];
    let len_xbox = HapticReportFormatter::format_report(HapticReportProtocol::XboxOneGip, speeds, &mut xbox_buf);
    if len_xbox != 8 || xbox_buf[0] != 0x09 || xbox_buf[4] != 180 || xbox_buf[5] != 220 {
        return false;
    }

    // 2. USB HID PID framing
    let mut pid_buf = [0u8; 5];
    let len_pid = HapticReportFormatter::format_report(HapticReportProtocol::UsbHidPid, speeds, &mut pid_buf);
    if len_pid != 5 || pid_buf[0] != 0x01 {
        return false;
    }
    let lf_16 = (pid_buf[1] as u16) | ((pid_buf[2] as u16) << 8);
    let hf_16 = (pid_buf[3] as u16) | ((pid_buf[4] as u16) << 8);
    if lf_16 != 180 * 257 || hf_16 != 220 * 257 {
        return false;
    }

    // 3. PlayStation DS framing
    let mut ds_buf = [0u8; 10];
    let len_ds = HapticReportFormatter::format_report(HapticReportProtocol::PlayStationDs, speeds, &mut ds_buf);
    if len_ds != 10 || ds_buf[0] != 0x05 || ds_buf[3] != 220 || ds_buf[4] != 180 {
        return false;
    }

    true
}

/// Unit test verifying dual-motor rumble controller, playback queue mixing, and timeline progression.
pub fn verify_dual_motor_controller() -> bool {
    use crate::drivers::input::waveform::{AdsrEnvelope, WaveformType};

    let mut controller = DualMotorRumbleController::new(255);

    // Create a 50ms LF strong rumble (Square wave)
    let lf_synth = WaveformSynthesizer::new(WaveformType::Square, 50, AdsrEnvelope::new(0, 0, 255, 0), 50, 255);
    // Create a 50ms HF weak rumble (Square wave)
    let hf_synth = WaveformSynthesizer::new(WaveformType::Square, 200, AdsrEnvelope::new(0, 0, 255, 0), 50, 255);

    let effect1 = RumbleEffectStep::new(1, 0, 50, Some(lf_synth), Some(hf_synth));
    if !controller.enqueue_effect(effect1) {
        return false;
    }

    // Step 1: Initial update at t=5ms
    let motors = controller.update(5);
    if motors.strong_lf == 0 || motors.weak_hf == 0 {
        return false;
    }

    // Step 2: Advance past 50ms duration (advance by 50ms => total 55ms)
    let motors_end = controller.update(50);
    if motors_end.is_active() {
        return false; // Should be off
    }
    if !controller.active_effects.is_empty() {
        return false; // Queue should be cleared
    }

    // Step 3: Cancellation test
    let effect2 = RumbleEffectStep::new(2, 0, 100, Some(lf_synth), None);
    controller.enqueue_effect(effect2);
    controller.cancel_effect(2);
    if !controller.active_effects.is_empty() {
        return false;
    }

    true
}
