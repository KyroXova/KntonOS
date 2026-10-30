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
