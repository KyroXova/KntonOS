//! Handheld PC backlight governor and flicker-free hybrid DC dimming.

use crate::drivers::display::pwm::{PwmModulator, PWM_DUTY_MAX};

// CIE 1931 perceptual luminance curve.

/// Converts perceived lightness L* [0.0..1.0] to linear physical luminance Y based on CIE 1931.
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

// Display technology and dimming modes.

/// Display panel technology profile.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PanelType {
    /// Transmissive LCD panel with LED edge-lit / direct backlight.
    Lcd,
    /// Self-emissive OLED panel with per-pixel emission and ABL governor.
    Oled,
}

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

// Fade pacer and clamping.

/// Temporal fade pacer providing smooth, gradual brightness transitions.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FadePacer {
    /// Current interpolated perceived brightness level [0.0..1.0].
    pub current_level: f32,
    /// Target perceived brightness level [0.0..1.0].
    pub target_level: f32,
    /// Transition rate when increasing brightness (units / second, default 2.5).
    pub ramp_up_rate_per_sec: f32,
    /// Transition rate when dimming brightness for comfortable dark adaptation (units / second, default 1.5).
    pub ramp_down_rate_per_sec: f32,
}

impl FadePacer {
    pub const fn new(initial_level: f32) -> Self {
        Self {
            current_level: initial_level,
            target_level: initial_level,
            ramp_up_rate_per_sec: 2.5,
            ramp_down_rate_per_sec: 1.5,
        }
    }

    /// Advances the smooth fade pacer by `dt_sec` seconds.
    pub fn step(&mut self, dt_sec: f32) -> f32 {
        if dt_sec <= 0.0 {
            return self.current_level;
        }

        let delta = self.target_level - self.current_level;
        if delta.abs() < 1e-4 {
            self.current_level = self.target_level;
            return self.current_level;
        }

        let rate = if delta > 0.0 {
            self.ramp_up_rate_per_sec
        } else {
            self.ramp_down_rate_per_sec
        };

        let max_step = rate * dt_sec;
        if delta.abs() <= max_step {
            self.current_level = self.target_level;
        } else if delta > 0.0 {
            self.current_level += max_step;
        } else {
            self.current_level -= max_step;
        }

        self.current_level
    }

    /// Returns true if currently actively transitioning towards target.
    pub fn is_fading(&self) -> bool {
        (self.target_level - self.current_level).abs() > 1e-4
    }
}

// Backlight configuration and output state.

/// Backlight governor configuration parameters.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BacklightConfig {
    /// Display panel technology (LCD vs OLED).
    pub panel_type: PanelType,
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
    /// Ambient minimum perceived brightness clamp in pitch-dark conditions (default 0.03).
    pub min_ambient_clamp: f32,
    /// Minimum OLED ABL power scaling multiplier under 100% full-screen white (default 0.60).
    pub oled_min_abl_multiplier: f32,
}

impl BacklightConfig {
    pub const fn default_handheld() -> Self {
        Self {
            panel_type: PanelType::Lcd,
            dimming_mode: DimmingMode::HybridDc,
            hybrid_threshold: 0.25,
            hybrid_hysteresis: 0.02,
            max_luminance_nits: 500.0,
            min_luminance_nits: 2.0,
            min_ambient_clamp: 0.03,
            oled_min_abl_multiplier: 0.60,
        }
    }

    pub const fn default_handheld_oled() -> Self {
        Self {
            panel_type: PanelType::Oled,
            dimming_mode: DimmingMode::HybridDc,
            hybrid_threshold: 0.25,
            hybrid_hysteresis: 0.02,
            max_luminance_nits: 600.0,
            min_luminance_nits: 1.5,
            min_ambient_clamp: 0.02,
            oled_min_abl_multiplier: 0.60,
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
    /// Auto-brightness limiter (ABL) scaling multiplier applied [0.0..1.0].
    pub abl_scaling: f32,
}

// Backlight governor.

/// Unified backlight governor managing PWM modulation, flicker-free DC dimming,
/// fade pacing, and OLED ABL scaling.
#[derive(Debug, Clone)]
pub struct BacklightGovernor {
    /// Operational configuration.
    pub config: BacklightConfig,
    /// Current hybrid operating domain.
    pub current_domain: HybridDomain,
    /// Smooth temporal fade pacer.
    pub pacer: FadePacer,
    /// Battery saver maximum power clamp (e.g. Some(0.70)).
    pub battery_saver_clamp: Option<f32>,
    /// Display blanked state (true = display sleep / screen off).
    pub display_blanked: bool,
    /// Current Average Picture Level (APL) for OLED panels [0.0..1.0].
    pub average_picture_level: f32,
}

impl BacklightGovernor {
    /// Creates a new backlight governor with the specified configuration.
    pub fn new(config: BacklightConfig) -> Self {
        Self {
            config,
            current_domain: HybridDomain::DcRegulated,
            pacer: FadePacer::new(1.0),
            battery_saver_clamp: None,
            display_blanked: false,
            average_picture_level: 0.5,
        }
    }

    /// Sets the requested perceived brightness target.
    pub fn set_target(&mut self, target: f32) {
        self.pacer.target_level = target.clamp(0.0, 1.0);
    }

    /// Sets battery saver maximum luminance clamp.
    pub fn set_battery_saver_clamp(&mut self, clamp: Option<f32>) {
        self.battery_saver_clamp = clamp.map(|c| c.clamp(0.0, 1.0));
    }

    /// Sets display blanked state (screen off / sleep mode).
    pub fn set_display_blanked(&mut self, blanked: bool) {
        self.display_blanked = blanked;
    }

    /// Updates Average Picture Level (APL) for OLED ABL governor.
    pub fn set_average_picture_level(&mut self, apl: f32) {
        self.average_picture_level = apl.clamp(0.0, 1.0);
    }

    /// Computes Auto-Brightness Limiting (ABL) scaling multiplier based on APL.
    pub fn compute_abl_multiplier(&self, apl: f32) -> f32 {
        if self.config.panel_type != PanelType::Oled {
            return 1.0;
        }

        // OLED APL: scales down linearly to oled_min_abl_multiplier at full-field white (APL=1.0).
        if apl <= 0.30 {
            1.0
        } else {
            let progress = (apl - 0.30) / 0.70;
            1.0 - progress * (1.0 - self.config.oled_min_abl_multiplier)
        }
    }

    /// Applies near-black shadow detail enhancement for OLED dark themes.
    pub fn apply_near_black_boost(&self, linear_lum: f32, apl: f32) -> f32 {
        if self.config.panel_type == PanelType::Oled && apl < 0.20 {
            if linear_lum > 0.0 && linear_lum < 0.05 {
                // Subtle 15% gamma boost to near-black tones, keeping 0.0 true black
                (linear_lum * 1.15).min(0.05)
            } else {
                linear_lum
            }
        } else {
            linear_lum
        }
    }

    /// Evaluates hardware drive state for the current interpolated fade level.
    pub fn evaluate(&mut self, perceived_brightness: f32) -> BacklightOutputState {
        // 1. If display is blanked (sleep mode), drive outputs are 0
        if self.display_blanked {
            return BacklightOutputState {
                perceived_brightness: 0.0,
                linear_luminance: 0.0,
                output_nits: 0.0,
                dc_drive_permille: 0,
                pwm_duty_16: 0,
                active_domain: self.current_domain,
                abl_scaling: 1.0,
            };
        }

        // 2. Apply ambient minimum clamp and battery saver clamp
        let mut clamped_perceived = perceived_brightness.clamp(0.0, 1.0);
        clamped_perceived = clamped_perceived.max(self.config.min_ambient_clamp);
        if let Some(saver_max) = self.battery_saver_clamp {
            clamped_perceived = clamped_perceived.min(saver_max);
        }

        // 3. Apply CIE 1931 perceptual luminance curve
        let mut linear_luminance = cie1931_perceived_to_linear(clamped_perceived);

        // 4. Apply OLED dark-mode shadow boost
        linear_luminance = self.apply_near_black_boost(linear_luminance, self.average_picture_level);

        // 5. Apply OLED Auto-Brightness Limiting (ABL) scaling
        let abl_scaling = self.compute_abl_multiplier(self.average_picture_level);
        let effective_linear = (linear_luminance * abl_scaling).clamp(0.0, 1.0);
        let output_nits = effective_linear * self.config.max_luminance_nits;

        // 6. Evaluate hybrid DC / PWM drive values
        let (dc_permille, pwm_duty, domain) = match self.config.dimming_mode {
            DimmingMode::PureDc => {
                let dc = ((effective_linear * 1000.0) + 0.5) as u16;
                let pwm = if effective_linear > 0.0 { PWM_DUTY_MAX } else { 0 };
                (dc.min(1000), pwm, HybridDomain::DcRegulated)
            }
            DimmingMode::HighFreqPwm => {
                let pwm = ((effective_linear * (PWM_DUTY_MAX as f32)) + 0.5) as u16;
                let dc = if effective_linear > 0.0 { 1000 } else { 0 };
                (dc, pwm, HybridDomain::PwmModulated)
            }
            DimmingMode::HybridDc => {
                // Update domain using hysteresis band
                let high_thresh = self.config.hybrid_threshold + self.config.hybrid_hysteresis;
                let low_thresh = self.config.hybrid_threshold - self.config.hybrid_hysteresis;

                match self.current_domain {
                    HybridDomain::DcRegulated => {
                        if effective_linear < low_thresh {
                            self.current_domain = HybridDomain::PwmModulated;
                        }
                    }
                    HybridDomain::PwmModulated => {
                        if effective_linear > high_thresh {
                            self.current_domain = HybridDomain::DcRegulated;
                        }
                    }
                }

                if effective_linear == 0.0 {
                    (0, 0, self.current_domain)
                } else if self.current_domain == HybridDomain::DcRegulated {
                    // Medium-to-high luminance: DC regulation.
                    // PWM is locked at 100% duty (continuous non-pulsing current, zero strobe flicker).
                    let dc = ((effective_linear * 1000.0) + 0.5) as u16;
                    (dc.min(1000), PWM_DUTY_MAX, HybridDomain::DcRegulated)
                } else {
                    // Low luminance: PWM modulation below threshold.
                    // DC drive is locked at the hybrid threshold permille (e.g. 250 = 25% current)
                    let dc_floor_permille = ((self.config.hybrid_threshold * 1000.0) + 0.5) as u16;
                    let pwm_fraction = (effective_linear / self.config.hybrid_threshold).clamp(0.0, 1.0);
                    let pwm = ((pwm_fraction * (PWM_DUTY_MAX as f32)) + 0.5) as u16;
                    (dc_floor_permille, pwm, HybridDomain::PwmModulated)
                }
            }
        };

        BacklightOutputState {
            perceived_brightness: clamped_perceived,
            linear_luminance: effective_linear,
            output_nits,
            dc_drive_permille: dc_permille,
            pwm_duty_16: pwm_duty,
            active_domain: domain,
            abl_scaling,
        }
    }

    /// Advances the smooth fade pacer by `dt_sec` and evaluates the new output state.
    pub fn tick_fade(&mut self, dt_sec: f32) -> BacklightOutputState {
        let current = self.pacer.step(dt_sec);
        self.evaluate(current)
    }

    /// Dispatches hardware updates directly to an active PWM modulator.
    pub fn apply_to_pwm(&mut self, perceived: f32, modulator: &mut PwmModulator) -> BacklightOutputState {
        let state = self.evaluate(perceived);
        modulator.set_duty_cycle_16(state.pwm_duty_16);
        modulator.commit_shadow_registers();
        state
    }
}

// Unit tests and verification.

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
    let mut config = BacklightConfig::default_handheld();
    config.min_ambient_clamp = 0.0; // Disable min clamp for transition testing
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

    // Hysteresis test: linear at 0.24 remains in PwmModulated below (0.25 + 0.02 = 0.27).
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

/// Unit test verifying smooth fade pacing, ambient minimum clamp, and OLED dark-mode / ABL scaling.
pub fn verify_fade_pacing_ambient_clamp_and_oled_scaling() -> bool {
    // 1. Verify Fade Pacer
    let mut pacer = FadePacer::new(0.2);
    pacer.target_level = 0.7; // delta = +0.5
    // With ramp_up_rate = 2.5/s, a 0.1s step should advance level by 0.25
    let step_1 = pacer.step(0.1);
    if (step_1 - 0.45).abs() > 0.01 {
        return false;
    }
    if !pacer.is_fading() {
        return false;
    }
    // Advance another 0.15s: should reach 0.45 + (2.5 * 0.15) = 0.45 + 0.375 = 0.825 -> clamped to 0.7
    let step_2 = pacer.step(0.15);
    if (step_2 - 0.7).abs() > 0.001 {
        return false;
    }
    if pacer.is_fading() {
        return false;
    }

    // 2. Verify Ambient Minimum Clamp & Battery Saver
    let config = BacklightConfig::default_handheld(); // min_ambient_clamp = 0.03
    let mut gov = BacklightGovernor::new(config);

    // Requesting 0.0 perceived should be clamped to 0.03
    let state_clamped = gov.evaluate(0.0);
    if (state_clamped.perceived_brightness - 0.03).abs() > 0.001 {
        return false;
    }

    // If display is blanked (sleep mode), clamp is bypassed and output is 0
    gov.set_display_blanked(true);
    let state_blank = gov.evaluate(0.5);
    if state_blank.linear_luminance != 0.0 || state_blank.dc_drive_permille != 0 || state_blank.pwm_duty_16 != 0 {
        return false;
    }
    gov.set_display_blanked(false);

    // Battery saver clamp: 70% max
    gov.set_battery_saver_clamp(Some(0.70));
    let state_saver = gov.evaluate(1.0);
    if (state_saver.perceived_brightness - 0.70).abs() > 0.001 {
        return false;
    }
    gov.set_battery_saver_clamp(None);

    // 3. Verify OLED Dark-Mode & Auto-Brightness Limiting (ABL) Scaling
    let oled_config = BacklightConfig::default_handheld_oled();
    let oled_gov = BacklightGovernor::new(oled_config);

    // 3a. LCD panel: ABL is inactive (multiplier always 1.0)
    if gov.compute_abl_multiplier(1.0) != 1.0 {
        return false;
    }

    // 3b. OLED panel at low APL (<= 0.30): multiplier is 1.0
    if oled_gov.compute_abl_multiplier(0.15) != 1.0 {
        return false;
    }
    if oled_gov.compute_abl_multiplier(0.30) != 1.0 {
        return false;
    }

    // 3c. OLED panel at 100% full-screen white (APL = 1.00): throttles to 60%
    let oled_abl_full = oled_gov.compute_abl_multiplier(1.0);
    if (oled_abl_full - 0.60).abs() > 0.01 {
        return false;
    }

    // 3d. OLED near-black shadow detail boost:
    // For low APL (< 0.20), near-black (0.02) gets boosted by 15%
    let boosted = oled_gov.apply_near_black_boost(0.02, 0.10);
    if (boosted - (0.02 * 1.15)).abs() > 0.001 {
        return false;
    }
    // True black (0.0) must NOT be boosted
    if oled_gov.apply_near_black_boost(0.0, 0.10) != 0.0 {
        return false;
    }

    true
}
