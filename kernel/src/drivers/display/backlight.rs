//! Unified Handheld PC Backlight Governor & Flicker-Free Hybrid DC Dimming.
//!
//! Provides advanced luminance control and eye comfort protection:
//! - Hybrid DC-dimming governor: dynamically switches between pure analog DC current regulation
//!   at medium-to-high luminance (>25%) for zero-strobe eye comfort, and high-frequency PWM
//!   at low luminance (<25%) to maintain LED color uniformity and prevent diode extinction.
//! - CIE 1931 perceptual luminance gamma mapping (L* -> Y) matching human ocular response.
//! - Hysteresis band preventing rapid mode oscillation near the 25% transition threshold.

use crate::drivers::display::pwm::{PwmModulator, PWM_DUTY_MAX};

// -----------------------------------------------------------------------------
// CIE 1931 Perceptual Luminance Curve
// -----------------------------------------------------------------------------

/// Converts perceived lightness L* in [0.0..1.0] (e.g. linear user brightness slider)
/// to linear physical luminance / radiant flux ratio Y in [0.0..1.0] based on the
/// international CIE 1931 lightness standard.
///
/// Mathematical formulation:
///   L*_100 = L* * 100.0
///   If L*_100 <= 8.0:
///     Y = (L*_100 / 903.3) / 100.0 = L* / 9.033
///   Else:
///     Y = ((L*_100 + 16.0) / 116.0)^3
pub fn cie1931_perceived_to_linear(perceived: f32) -> f32 {
    let p = perceived.clamp(0.0, 1.0);
    let l_star = p * 100.0;

    if l_star <= 8.0 {
        p / 9.033
    } else {
        let t = (l_star + 16.0) / 116.0;
        t * t * t
    }
}

/// Converts linear physical luminance ratio Y in [0.0..1.0] to perceived lightness L*
/// in [0.0..1.0] using the inverse CIE 1931 formula.
pub fn cie1931_linear_to_perceived(linear: f32) -> f32 {
    let y = linear.clamp(0.0, 1.0);
    if y <= 0.008856 {
        (y * 9.033).clamp(0.0, 1.0)
    } else {
        // Approximate cube root via Newton-Raphson
        let mut x = y;
        for _ in 0..6 {
            let x3 = x * x * x;
            let deriv = 3.0 * x * x;
            if deriv > 1e-6 {
                x -= (x3 - y) / deriv;
            }
        }
        let l_star = 116.0 * x - 16.0;
        (l_star / 100.0).clamp(0.0, 1.0)
    }
}

// -----------------------------------------------------------------------------
// Dimming Modes & Operating Domain
// -----------------------------------------------------------------------------

/// Backlight dimming operating mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DimmingMode {
    /// Pure continuous analog DC current regulation across entire range.
    PureDc,
    /// High-frequency PWM duty modulation across entire range.
    HighFreqPwm,
    /// Hybrid DC dimming: DC regulation above threshold, high-freq PWM below threshold.
    HybridDc,
}

/// Active operating domain when in Hybrid DC mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HybridDomain {
    /// Operating in analog DC voltage/current regulation domain (zero strobe flicker).
    DcRegulated,
    /// Operating in ultrasonic PWM duty modulation domain (preserving color consistency).
    PwmModulated,
}

/// Backlight governor configuration parameters.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BacklightConfig {
    /// Dimming operational strategy.
    pub dimming_mode: DimmingMode,
    /// Transition threshold for hybrid mode (linear ratio, default 0.25 = 25%).
    pub hybrid_threshold: f32,
    /// Hysteresis delta around hybrid threshold (default 0.02 = +/-2%).
    pub hybrid_hysteresis: f32,
    /// Maximum display panel luminance in nits (cd/m^2).
    pub max_luminance_nits: f32,
    /// Minimum calibrated readable display luminance in nits.
    pub min_luminance_nits: f32,
}

impl BacklightConfig {
    pub const fn default_handheld() -> Self {
        Self {
            dimming_mode: DimmingMode::HybridDc,
            hybrid_threshold: 0.25,
            hybrid_hysteresis: 0.02,
            max_luminance_nits: 500.0,
            min_luminance_nits: 2.0,
        }
    }
}

/// Output drive control values dispatched to display hardware.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BacklightOutputState {
    /// Normalized user perceived brightness [0.0..1.0].
    pub perceived_brightness: f32,
    /// Normalized linear physical luminance ratio [0.0..1.0].
    pub linear_luminance: f32,
    /// Absolute panel luminance in nits.
    pub output_nits: f32,
    /// Analog DC drive permille [0..1000] for LED current DAC.
    pub dc_drive_permille: u16,
    /// 16-bit PWM duty cycle [0..65535] for backlight timer.
    pub pwm_duty_16: u16,
    /// Active hybrid domain state.
    pub active_domain: HybridDomain,
}

// -----------------------------------------------------------------------------
// Backlight Governor
// -----------------------------------------------------------------------------

/// Unified backlight governor managing PWM modulation and flicker-free DC dimming.
#[derive(Debug, Clone)]
pub struct BacklightGovernor {
    /// Operational configuration.
    pub config: BacklightConfig,
    /// Current hybrid operating domain.
    pub current_domain: HybridDomain,
    /// Current user perceived brightness target [0.0..1.0].
    pub current_perceived: f32,
}

impl BacklightGovernor {
    /// Creates a new backlight governor with the specified configuration.
    pub fn new(config: BacklightConfig) -> Self {
        Self {
            config,
            current_domain: HybridDomain::DcRegulated,
            current_perceived: 1.0,
        }
    }

    /// Evaluates hardware drive state for a requested perceived brightness level [0.0..1.0].
    pub fn evaluate(&mut self, perceived_brightness: f32) -> BacklightOutputState {
        let perceived = perceived_brightness.clamp(0.0, 1.0);
        self.current_perceived = perceived;

        // Apply CIE 1931 perceptual luminance curve
        let linear_luminance = cie1931_perceived_to_linear(perceived);
        let output_nits = linear_luminance * self.config.max_luminance_nits;

        let (dc_permille, pwm_duty, domain) = match self.config.dimming_mode {
            DimmingMode::PureDc => {
                let dc = ((linear_luminance * 1000.0) + 0.5) as u16;
                let pwm = if linear_luminance > 0.0 { PWM_DUTY_MAX } else { 0 };
                (dc.min(1000), pwm, HybridDomain::DcRegulated)
            }
            DimmingMode::HighFreqPwm => {
                let pwm = ((linear_luminance * (PWM_DUTY_MAX as f32)) + 0.5) as u16;
                let dc = if linear_luminance > 0.0 { 1000 } else { 0 };
                (dc, pwm, HybridDomain::PwmModulated)
            }
            DimmingMode::HybridDc => {
                // Update domain using hysteresis band
                let high_thresh = self.config.hybrid_threshold + self.config.hybrid_hysteresis;
                let low_thresh = self.config.hybrid_threshold - self.config.hybrid_hysteresis;

                match self.current_domain {
                    HybridDomain::DcRegulated => {
                        if linear_luminance < low_thresh {
                            self.current_domain = HybridDomain::PwmModulated;
                        }
                    }
                    HybridDomain::PwmModulated => {
                        if linear_luminance > high_thresh {
                            self.current_domain = HybridDomain::DcRegulated;
                        }
                    }
                }

                if linear_luminance == 0.0 {
                    (0, 0, self.current_domain)
                } else if self.current_domain == HybridDomain::DcRegulated {
                    // Medium-to-high luminance: DC regulation.
                    // PWM is locked at 100% duty (continuous non-pulsing current, zero strobe flicker).
                    let dc = ((linear_luminance * 1000.0) + 0.5) as u16;
                    (dc.min(1000), PWM_DUTY_MAX, HybridDomain::DcRegulated)
                } else {
                    // Low luminance: PWM modulation below threshold.
                    // DC drive is locked at the hybrid threshold permille (e.g. 250 = 25% current)
                    // preventing color shift and diode knee extinction.
                    let dc_floor_permille = ((self.config.hybrid_threshold * 1000.0) + 0.5) as u16;
                    let pwm_fraction = (linear_luminance / self.config.hybrid_threshold).clamp(0.0, 1.0);
                    let pwm = ((pwm_fraction * (PWM_DUTY_MAX as f32)) + 0.5) as u16;
                    (dc_floor_permille, pwm, HybridDomain::PwmModulated)
                }
            }
        };

        BacklightOutputState {
            perceived_brightness: perceived,
            linear_luminance,
            output_nits,
            dc_drive_permille: dc_permille,
            pwm_duty_16: pwm_duty,
            active_domain: domain,
        }
    }

    /// Dispatches hardware updates directly to an active PWM modulator.
    pub fn apply_to_pwm(&mut self, perceived: f32, modulator: &mut PwmModulator) -> BacklightOutputState {
        let state = self.evaluate(perceived);
        modulator.set_duty_cycle_16(state.pwm_duty_16);
        modulator.commit_shadow_registers();
        state
    }
}

// -----------------------------------------------------------------------------
// Unit Tests & Verification
// -----------------------------------------------------------------------------

/// Unit test verifying hybrid DC dimming transition governor and CIE 1931 perceptual curve.
pub fn verify_hybrid_dc_dimming_and_cie1931() -> bool {
    // 1. Verify CIE 1931 Perceptual Curve
    // At L* = 0.0 -> Y = 0.0
    if cie1931_perceived_to_linear(0.0) != 0.0 {
        return false;
    }
    // At L* = 1.0 (100%) -> Y = 1.0 (100%)
    if (cie1931_perceived_to_linear(1.0) - 1.0).abs() > 0.0001 {
        return false;
    }
    // At L* = 0.5 (50% perceived lightness) -> Y should be ~18.4% (middle grey)
    let y_50 = cie1931_perceived_to_linear(0.5);
    if (y_50 - 0.184).abs() > 0.01 {
        return false;
    }
    // At L* = 0.08 (low threshold boundary) -> Y = 0.08 / 9.033 ~ 0.008856
    let y_low = cie1931_perceived_to_linear(0.08);
    if (y_low - 0.008856).abs() > 0.001 {
        return false;
    }

    // 2. Verify Inverse CIE 1931 Curve
    let p_back = cie1931_linear_to_perceived(y_50);
    if (p_back - 0.5).abs() > 0.02 {
        return false;
    }

    // 3. Verify Hybrid DC Dimming Governor Transitions
    let config = BacklightConfig::default_handheld();
    let mut governor = BacklightGovernor::new(config);

    // 3a. High luminance test: Perceived 1.0 -> Linear 1.0 -> DC domain
    let state_100 = governor.evaluate(1.0);
    if state_100.active_domain != HybridDomain::DcRegulated {
        return false;
    }
    // DC drive should be 1000 permille, PWM locked at 100% (65535)
    if state_100.dc_drive_permille != 1000 || state_100.pwm_duty_16 != PWM_DUTY_MAX {
        return false;
    }

    // 3b. Mid luminance test: At linear 0.50 (where perceived is ~0.74) -> DC domain
    // Perceived 0.80 -> linear ~ 0.575 > 0.25 -> DC domain
    let state_80 = governor.evaluate(0.80);
    if state_80.active_domain != HybridDomain::DcRegulated {
        return false;
    }
    if state_80.pwm_duty_16 != PWM_DUTY_MAX {
        return false; // PWM must remain 100% in DC domain (zero flicker!)
    }

    // 3c. Low luminance test: Perceived 0.30 -> linear = ((30+16)/116)^3 = (0.3965)^3 ~ 0.0623 < 0.23 -> PWM domain
    let state_30 = governor.evaluate(0.30);
    if state_30.active_domain != HybridDomain::PwmModulated {
        return false;
    }
    // DC drive must be held at floor (250 permille)
    if state_30.dc_drive_permille != 250 {
        return false;
    }
    // PWM duty should be scaled relative to 0.25: linear / 0.25 ~ 0.0623 / 0.25 ~ 0.249 -> PWM duty ~ 16330
    if state_30.pwm_duty_16 > 20000 || state_30.pwm_duty_16 < 14000 {
        return false;
    }

    // 3d. Hysteresis test:
    // With current_domain = PwmModulated, if linear rises to 0.24 (below 0.25 + 0.02 = 0.27),
    // it MUST stay in PwmModulated!
    // Perceived ~0.56 gives linear ~ 0.245
    let state_hyst = governor.evaluate(0.56);
    if state_hyst.active_domain != HybridDomain::PwmModulated {
        return false;
    }

    // Once perceived reaches 0.62 -> linear ~ 0.31 > 0.27 -> transitions to DcRegulated
    let state_dc_trans = governor.evaluate(0.62);
    if state_dc_trans.active_domain != HybridDomain::DcRegulated {
        return false;
    }

    true
}
