//! Handheld PC Multi-Zone RGB Lighting Driver: Color Math, Gamma Correction & Layout.
//!
//! Provides color primitives, 2.2 perceptual gamma correction, HSV color space
//! conversion, effect generators, game-state reactive aura synchronization,
//! and Embedded Controller (EC) / I2C hardware register mapping for handheld gaming PCs.

use crate::graphics::{sin_f32, TWO_PI};

// -----------------------------------------------------------------------------
// Gamma 2.2 Perceptual Correction Lookup Table
// -----------------------------------------------------------------------------

/// Precomputed 256-entry lookup table for 2.2 power-law gamma correction.
///
/// Maps linear 8-bit input intensity values to perceptual LED PWM duty cycles:
/// PWM = round(255 * (input / 255.0)^2.2).
pub static GAMMA_2_2_TABLE: [u8; 256] = [
      0,   0,   0,   0,   0,   0,   0,   0,   0,   0,   0,   0,   0,   0,   0,   1,
      1,   1,   1,   1,   1,   1,   1,   1,   1,   2,   2,   2,   2,   2,   2,   2,
      3,   3,   3,   3,   3,   4,   4,   4,   4,   5,   5,   5,   5,   6,   6,   6,
      6,   7,   7,   7,   8,   8,   8,   9,   9,   9,  10,  10,  11,  11,  11,  12,
     12,  13,  13,  13,  14,  14,  15,  15,  16,  16,  17,  17,  18,  18,  19,  19,
     20,  20,  21,  22,  22,  23,  23,  24,  25,  25,  26,  26,  27,  28,  28,  29,
     30,  30,  31,  32,  33,  33,  34,  35,  35,  36,  37,  38,  39,  39,  40,  41,
     42,  43,  43,  44,  45,  46,  47,  48,  49,  49,  50,  51,  52,  53,  54,  55,
     56,  57,  58,  59,  60,  61,  62,  63,  64,  65,  66,  67,  68,  69,  70,  71,
     73,  74,  75,  76,  77,  78,  79,  81,  82,  83,  84,  85,  87,  88,  89,  90,
     91,  93,  94,  95,  97,  98,  99, 100, 102, 103, 105, 106, 107, 109, 110, 111,
    113, 114, 116, 117, 119, 120, 121, 123, 124, 126, 127, 129, 130, 132, 133, 135,
    137, 138, 140, 141, 143, 145, 146, 148, 149, 151, 153, 154, 156, 158, 159, 161,
    163, 165, 166, 168, 170, 172, 173, 175, 177, 179, 181, 182, 184, 186, 188, 190,
    192, 194, 196, 197, 199, 201, 203, 205, 207, 209, 211, 213, 215, 217, 219, 221,
    223, 225, 227, 229, 231, 234, 236, 238, 240, 242, 244, 246, 248, 251, 253, 255,
];

/// Applies 2.2 gamma correction to an 8-bit color channel.
#[inline]
pub fn apply_gamma_2_2(channel: u8) -> u8 {
    GAMMA_2_2_TABLE[channel as usize]
}

// -----------------------------------------------------------------------------
// Rgb8 Color Structure
// -----------------------------------------------------------------------------

/// 24-bit TrueColor RGB color representation (8 bits per channel).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Rgb8 {
    /// Red channel intensity [0..255].
    pub r: u8,
    /// Green channel intensity [0..255].
    pub g: u8,
    /// Blue channel intensity [0..255].
    pub b: u8,
}

impl Rgb8 {
    pub const BLACK: Self = Self::new(0, 0, 0);
    pub const WHITE: Self = Self::new(255, 255, 255);
    pub const RED: Self = Self::new(255, 0, 0);
    pub const GREEN: Self = Self::new(0, 255, 0);
    pub const BLUE: Self = Self::new(0, 0, 255);
    pub const YELLOW: Self = Self::new(255, 255, 0);
    pub const CYAN: Self = Self::new(0, 255, 255);
    pub const MAGENTA: Self = Self::new(255, 0, 255);
    pub const ORANGE: Self = Self::new(255, 128, 0);
    pub const PURPLE: Self = Self::new(128, 0, 255);
    pub const AMBER: Self = Self::new(255, 191, 0);
    pub const GOLD: Self = Self::new(255, 215, 0);

    /// Constructs a new Rgb8 color.
    pub const fn new(r: u8, g: u8, b: u8) -> Self {
        Self { r, g, b }
    }

    /// Performs linear interpolation between self and other by parameter `t` [0.0 .. 1.0].
    pub fn lerp(self, other: Self, t: f32) -> Self {
        let t_clamped = t.clamp(0.0, 1.0);
        let r = ((self.r as f32) + ((other.r as f32) - (self.r as f32)) * t_clamped) + 0.5;
        let g = ((self.g as f32) + ((other.g as f32) - (self.g as f32)) * t_clamped) + 0.5;
        let b = ((self.b as f32) + ((other.b as f32) - (self.b as f32)) * t_clamped) + 0.5;
        Self {
            r: (r as u32).min(255) as u8,
            g: (g as u32).min(255) as u8,
            b: (b as u32).min(255) as u8,
        }
    }

    /// Scales color intensity by a fractional brightness factor [0.0 .. 1.0].
    pub fn scale(self, brightness: f32) -> Self {
        let b_clamped = brightness.clamp(0.0, 1.0);
        Self {
            r: (((self.r as f32) * b_clamped) + 0.5) as u8,
            g: (((self.g as f32) * b_clamped) + 0.5) as u8,
            b: (((self.b as f32) * b_clamped) + 0.5) as u8,
        }
    }

    /// Transforms linear color channels through the 2.2 gamma correction table.
    pub fn apply_gamma(self) -> Self {
        Self {
            r: apply_gamma_2_2(self.r),
            g: apply_gamma_2_2(self.g),
            b: apply_gamma_2_2(self.b),
        }
    }
}

// -----------------------------------------------------------------------------
// HSV Color Space
// -----------------------------------------------------------------------------

/// HSV (Hue, Saturation, Value) cylindrical color space representation.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct HsvColor {
    /// Hue angle in degrees [0.0 .. 360.0).
    pub h: f32,
    /// Saturation fraction [0.0 .. 1.0].
    pub s: f32,
    /// Value/brightness fraction [0.0 .. 1.0].
    pub v: f32,
}

impl HsvColor {
    pub const fn new(h: f32, s: f32, v: f32) -> Self {
        Self { h, s, v }
    }

    /// Converts HSV color to 24-bit TrueColor Rgb8.
    pub fn to_rgb(&self) -> Rgb8 {
        let v_clamped = self.v.clamp(0.0, 1.0);
        let s_clamped = self.s.clamp(0.0, 1.0);

        if s_clamped <= 0.0001 {
            let gray = (v_clamped * 255.0 + 0.5) as u8;
            return Rgb8::new(gray, gray, gray);
        }

        let mut hue = self.h % 360.0;
        if hue < 0.0 {
            hue += 360.0;
        }

        let c = v_clamped * s_clamped;
        let h_prime = hue / 60.0;
        let x = c * (1.0 - (((h_prime as i32) % 2) as f32 + (h_prime - (h_prime as i32 as f32)) - 1.0).abs());
        let m = v_clamped - c;

        let (r1, g1, b1) = if h_prime < 1.0 {
            (c, x, 0.0)
        } else if h_prime < 2.0 {
            (x, c, 0.0)
        } else if h_prime < 3.0 {
            (0.0, c, x)
        } else if h_prime < 4.0 {
            (0.0, x, c)
        } else if h_prime < 5.0 {
            (x, 0.0, c)
        } else {
            (c, 0.0, x)
        };

        Rgb8 {
            r: (((r1 + m) * 255.0) + 0.5).clamp(0.0, 255.0) as u8,
            g: (((g1 + m) * 255.0) + 0.5).clamp(0.0, 255.0) as u8,
            b: (((b1 + m) * 255.0) + 0.5).clamp(0.0, 255.0) as u8,
        }
    }

    /// Converts 24-bit TrueColor Rgb8 to HSV color space.
    pub fn from_rgb(rgb: Rgb8) -> Self {
        let r = (rgb.r as f32) / 255.0;
        let g = (rgb.g as f32) / 255.0;
        let b = (rgb.b as f32) / 255.0;

        let max = r.max(g).max(b);
        let min = r.min(g).min(b);
        let delta = max - min;

        let v = max;
        let s = if max > 0.0 { delta / max } else { 0.0 };

        let h = if delta <= 0.0001 {
            0.0
        } else if (max - r).abs() < 0.0001 {
            60.0 * (((g - b) / delta) % 6.0)
        } else if (max - g).abs() < 0.0001 {
            60.0 * (((b - r) / delta) + 2.0)
        } else {
            60.0 * (((r - g) / delta) + 4.0)
        };

        let h_final = if h < 0.0 { h + 360.0 } else { h };
        Self { h: h_final, s, v }
    }
}

// -----------------------------------------------------------------------------
// Handheld Multi-Zone Layout
// -----------------------------------------------------------------------------

/// Identifies physical RGB lighting zones on a handheld gaming PC.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LedZone {
    /// Left analog joystick illuminated aura ring.
    LeftStickRing = 0,
    /// Right analog joystick illuminated aura ring.
    RightStickRing = 1,
    /// Directional pad perimeter backlighting accent.
    DpadAccent = 2,
    /// Left rear ergonomic grip accent strip.
    LeftGrip = 3,
    /// Right rear ergonomic grip accent strip.
    RightGrip = 4,
    /// Chassis illuminated branding badge / emblem.
    ChassisLogo = 5,
}

/// Total count of physical hardware lighting zones supported.
pub const LED_ZONE_COUNT: usize = 6;

/// All zones in array index order.
pub const ALL_LED_ZONES: [LedZone; LED_ZONE_COUNT] = [
    LedZone::LeftStickRing,
    LedZone::RightStickRing,
    LedZone::DpadAccent,
    LedZone::LeftGrip,
    LedZone::RightGrip,
    LedZone::ChassisLogo,
];

/// Spatial perimeter phase offsets (in degrees) for continuous console perimeter flow.
pub const ZONE_SPATIAL_PHASE: [f32; LED_ZONE_COUNT] = [
    0.0,   // LeftStickRing
    180.0, // RightStickRing
    60.0,  // DpadAccent
    300.0, // LeftGrip
    240.0, // RightGrip
    120.0, // ChassisLogo
];

/// Physical and logical descriptor for a handheld hardware lighting zone.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ZoneDescriptor {
    pub zone: LedZone,
    pub name: &'static str,
    pub led_count: u8,
    pub default_color: Rgb8,
}

/// Returns hardware descriptor metadata for a given lighting zone.
pub const fn get_zone_descriptor(zone: LedZone) -> ZoneDescriptor {
    match zone {
        LedZone::LeftStickRing => ZoneDescriptor {
            zone: LedZone::LeftStickRing,
            name: "Left Joystick Aura Ring",
            led_count: 8,
            default_color: Rgb8::CYAN,
        },
        LedZone::RightStickRing => ZoneDescriptor {
            zone: LedZone::RightStickRing,
            name: "Right Joystick Aura Ring",
            led_count: 8,
            default_color: Rgb8::CYAN,
        },
        LedZone::DpadAccent => ZoneDescriptor {
            zone: LedZone::DpadAccent,
            name: "D-Pad Perimeter Accent",
            led_count: 4,
            default_color: Rgb8::PURPLE,
        },
        LedZone::LeftGrip => ZoneDescriptor {
            zone: LedZone::LeftGrip,
            name: "Left Grip LED Strip",
            led_count: 6,
            default_color: Rgb8::BLUE,
        },
        LedZone::RightGrip => ZoneDescriptor {
            zone: LedZone::RightGrip,
            name: "Right Grip LED Strip",
            led_count: 6,
            default_color: Rgb8::BLUE,
        },
        LedZone::ChassisLogo => ZoneDescriptor {
            zone: LedZone::ChassisLogo,
            name: "Chassis Badge Emblem",
            led_count: 2,
            default_color: Rgb8::WHITE,
        },
    }
}

// -----------------------------------------------------------------------------
// Lighting Effect Generators
// -----------------------------------------------------------------------------

/// Configurable dynamic lighting effects for handheld console illumination.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum LedEffect {
    /// Static single color.
    Static(Rgb8),
    /// Smooth sinusoidal breathing pulsation between min and max brightness.
    Breathing {
        color: Rgb8,
        period_ms: u32,
        min_brightness: f32,
        max_brightness: f32,
    },
    /// Dynamic continuous 360-degree rainbow spectrum cycle.
    RainbowCycle {
        period_ms: u32,
        spatial_phase_deg: f32,
    },
    /// Smooth color wave oscillating between two distinct hues.
    ColorWave {
        color_a: Rgb8,
        color_b: Rgb8,
        period_ms: u32,
        spatial_phase_deg: f32,
    },
    /// Battery fuel gauge indicator (Red critical, Amber low, Green nominal, Pulsing if charging).
    BatteryIndicator {
        charge_percent: u8,
        is_charging: bool,
    },
    /// Turn off LEDs in this zone.
    Off,
}

impl LedEffect {
    /// Evaluates the effect at a specific timestamp in milliseconds.
    pub fn render(&self, elapsed_ms: u64) -> Rgb8 {
        match *self {
            Self::Static(color) => color,
            Self::Breathing {
                color,
                period_ms,
                min_brightness,
                max_brightness,
            } => {
                let period = if period_ms == 0 { 1000 } else { period_ms };
                let phase = (elapsed_ms % (period as u64)) as f32 / (period as f32);
                let sin_norm = (sin_f32(phase * TWO_PI) + 1.0) * 0.5;
                let b = min_brightness + (max_brightness - min_brightness) * sin_norm;
                color.scale(b)
            }
            Self::RainbowCycle {
                period_ms,
                spatial_phase_deg,
            } => {
                let period = if period_ms == 0 { 3000 } else { period_ms };
                let time_frac = (elapsed_ms % (period as u64)) as f32 / (period as f32);
                let hue = (time_frac * 360.0 + spatial_phase_deg) % 360.0;
                let hue_pos = if hue < 0.0 { hue + 360.0 } else { hue };
                HsvColor::new(hue_pos, 1.0, 1.0).to_rgb()
            }
            Self::ColorWave {
                color_a,
                color_b,
                period_ms,
                spatial_phase_deg,
            } => {
                let period = if period_ms == 0 { 2000 } else { period_ms };
                let time_frac = (elapsed_ms % (period as u64)) as f32 / (period as f32);
                let phase_rad = time_frac * TWO_PI + (spatial_phase_deg * (TWO_PI / 360.0));
                let t = (sin_f32(phase_rad) + 1.0) * 0.5;
                color_a.lerp(color_b, t)
            }
            Self::BatteryIndicator {
                charge_percent,
                is_charging,
            } => {
                let base_color = if charge_percent < 20 {
                    Rgb8::RED
                } else if charge_percent < 50 {
                    Rgb8::AMBER
                } else {
                    Rgb8::GREEN
                };

                if is_charging {
                    // 1200ms charging pulse
                    let phase = (elapsed_ms % 1200) as f32 / 1200.0;
                    let sin_norm = (sin_f32(phase * TWO_PI) + 1.0) * 0.5;
                    let b = 0.35 + 0.65 * sin_norm;
                    base_color.scale(b)
                } else if charge_percent < 10 {
                    // Low battery fast blink (400ms: 200ms ON / 200ms OFF)
                    if (elapsed_ms % 400) < 200 {
                        base_color
                    } else {
                        Rgb8::BLACK
                    }
                } else {
                    base_color
                }
            }
            Self::Off => Rgb8::BLACK,
        }
    }
}

// -----------------------------------------------------------------------------
// Aura Sync Controller
// -----------------------------------------------------------------------------

/// Synchronization coordination mode across handheld lighting zones.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuraSyncMode {
    /// Each zone executes its own assigned effect independently.
    Independent,
    /// All zones locked in synchronous phase.
    Synchronized,
    /// Perimeter flow around handheld console with progressive spatial phase offsets.
    FlowingPerimeter,
}

/// Central multi-zone RGB lighting controller for handheld gaming consoles.
pub struct RgbLightingController {
    /// Current effect assigned to each zone.
    effects: [LedEffect; LED_ZONE_COUNT],
    /// Current rendered output color for each zone (gamma-corrected).
    current_colors: [Rgb8; LED_ZONE_COUNT],
    /// Global master brightness scale factor [0.0 .. 1.0].
    master_brightness: f32,
    /// Master power / enable switch.
    master_enabled: bool,
    /// Active Aura Sync synchronization mode.
    sync_mode: AuraSyncMode,
    /// Cumulative elapsed operational time in milliseconds.
    elapsed_time_ms: u64,
}

impl RgbLightingController {
    /// Creates a new RGB lighting controller with default handheld presets.
    pub fn new() -> Self {
        let mut controller = Self {
            effects: [LedEffect::Off; LED_ZONE_COUNT],
            current_colors: [Rgb8::BLACK; LED_ZONE_COUNT],
            master_brightness: 1.0,
            master_enabled: true,
            sync_mode: AuraSyncMode::Independent,
            elapsed_time_ms: 0,
        };

        // Initialize default zone effects
        for &zone in &ALL_LED_ZONES {
            let desc = get_zone_descriptor(zone);
            controller.set_zone_effect(zone, LedEffect::Static(desc.default_color));
        }

        controller
    }

    /// Sets the lighting effect for a specific zone.
    pub fn set_zone_effect(&mut self, zone: LedZone, effect: LedEffect) {
        self.effects[zone as usize] = effect;
    }

    /// Applies a uniform lighting effect across all hardware zones.
    pub fn set_all_zones_effect(&mut self, effect: LedEffect) {
        for slot in &mut self.effects {
            *slot = effect;
        }
    }

    /// Configures the Aura Sync synchronization mode.
    pub fn set_sync_mode(&mut self, mode: AuraSyncMode) {
        self.sync_mode = mode;
    }

    /// Sets global master brightness scale [0.0 .. 1.0].
    pub fn set_master_brightness(&mut self, brightness: f32) {
        self.master_brightness = brightness.clamp(0.0, 1.0);
    }

    /// Enables or disables master LED illumination.
    pub fn set_master_enabled(&mut self, enabled: bool) {
        self.master_enabled = enabled;
    }

    /// Returns master brightness scale.
    pub fn master_brightness(&self) -> f32 {
        self.master_brightness
    }

    /// Returns master enabled status.
    pub fn is_master_enabled(&self) -> bool {
        self.master_enabled
    }

    /// Returns the current rendered color for a specific zone.
    pub fn get_zone_color(&self, zone: LedZone) -> Rgb8 {
        self.current_colors[zone as usize]
    }

    /// Returns reference to all current zone colors.
    pub fn current_colors(&self) -> &[Rgb8; LED_ZONE_COUNT] {
        &self.current_colors
    }

    /// Advances lighting animation timeline and computes next gamma-corrected frame buffer.
    pub fn update(&mut self, delta_ms: u32) -> &[Rgb8; LED_ZONE_COUNT] {
        self.elapsed_time_ms += delta_ms as u64;

        if !self.master_enabled || self.master_brightness <= 0.0 {
            for slot in &mut self.current_colors {
                *slot = Rgb8::BLACK;
            }
            return &self.current_colors;
        }

        for (i, _) in ALL_LED_ZONES.iter().enumerate() {
            let base_effect = self.effects[i];
            let effective_effect = match (self.sync_mode, base_effect) {
                (AuraSyncMode::FlowingPerimeter, LedEffect::RainbowCycle { period_ms, .. }) => {
                    LedEffect::RainbowCycle {
                        period_ms,
                        spatial_phase_deg: ZONE_SPATIAL_PHASE[i],
                    }
                }
                (AuraSyncMode::FlowingPerimeter, LedEffect::ColorWave { color_a, color_b, period_ms, .. }) => {
                    LedEffect::ColorWave {
                        color_a,
                        color_b,
                        period_ms,
                        spatial_phase_deg: ZONE_SPATIAL_PHASE[i],
                    }
                }
                (AuraSyncMode::Synchronized, LedEffect::RainbowCycle { period_ms, .. }) => {
                    LedEffect::RainbowCycle {
                        period_ms,
                        spatial_phase_deg: 0.0,
                    }
                }
                _ => base_effect,
            };

            let linear_color = effective_effect.render(self.elapsed_time_ms);
            let scaled_color = linear_color.scale(self.master_brightness);
            self.current_colors[i] = scaled_color.apply_gamma();
        }

        &self.current_colors
    }
}

// -----------------------------------------------------------------------------
// In-Game Reactive Lighting Synchronization Engine
// -----------------------------------------------------------------------------

/// Telemetry metrics streamed from the active game engine or runtime governor.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GameTelemetry {
    /// Player health percentage [0..100].
    pub health_percent: u8,
    /// Player shield/armor percentage [0..100].
    pub shield_percent: u8,
    /// APU/GPU silicon junction temperature in Celsius [0.0 .. 110.0].
    pub gpu_temp_celsius: f32,
    /// Cooling blower fan rotational speed in RPM [0 .. 8000].
    pub fan_rpm: u16,
}

impl Default for GameTelemetry {
    fn default() -> Self {
        Self {
            health_percent: 100,
            shield_percent: 100,
            gpu_temp_celsius: 45.0,
            fan_rpm: 2200,
        }
    }
}

/// Evaluates APU temperature and maps to a dynamic thermal gradient color.
///
/// - <= 45°C: Deep Sapphire Blue (0, 80, 255)
/// - 45°C..65°C: Neon Cyan to Radiant Gold (0, 255, 255 -> 255, 215, 0)
/// - >= 85°C: Blazing Crimson Thermal Red (255, 0, 0)
pub fn evaluate_thermal_heat_gradient(temp_celsius: f32) -> Rgb8 {
    let t = temp_celsius.clamp(35.0, 90.0);
    if t <= 50.0 {
        let frac = (t - 35.0) / 15.0;
        Rgb8::new(0, 60, 255).lerp(Rgb8::new(0, 255, 255), frac)
    } else if t <= 70.0 {
        let frac = (t - 50.0) / 20.0;
        Rgb8::new(0, 255, 255).lerp(Rgb8::new(255, 215, 0), frac)
    } else {
        let frac = ((t - 70.0) / 20.0).min(1.0);
        Rgb8::new(255, 215, 0).lerp(Rgb8::new(255, 0, 0), frac)
    }
}

/// Dynamic game-state reactive lighting governor applying real-time telemetry overlays.
pub struct GameStateAuraSync {
    /// Current live game telemetry.
    pub telemetry: GameTelemetry,
    /// Remaining duration of critical damage flash in milliseconds.
    damage_flash_remaining_ms: u32,
    /// Peak intensity factor of active damage flash.
    damage_flash_peak: f32,
    /// Internal accumulator for low-health heartbeat pulse.
    heartbeat_phase_ms: u32,
}

impl GameStateAuraSync {
    /// Initializes reactive game state governor with default nominal telemetry.
    pub fn new() -> Self {
        Self {
            telemetry: GameTelemetry::default(),
            damage_flash_remaining_ms: 0,
            damage_flash_peak: 0.0,
            heartbeat_phase_ms: 0,
        }
    }

    /// Triggers an immediate critical damage flash overlay (decays over ~200ms).
    pub fn trigger_damage_flash(&mut self, intensity: f32) {
        self.damage_flash_remaining_ms = 200;
        self.damage_flash_peak = intensity.clamp(0.0, 1.0);
    }

    /// Updates active game telemetry metrics.
    pub fn update_telemetry(&mut self, telemetry: GameTelemetry) {
        self.telemetry = telemetry;
    }

    /// Applies real-time game state reactive overlays on top of the base rendered frame.
    pub fn apply_reactive_overlay(
        &mut self,
        base_frame: &[Rgb8; LED_ZONE_COUNT],
        delta_ms: u32,
        out_frame: &mut [Rgb8; LED_ZONE_COUNT],
    ) {
        *out_frame = *base_frame;

        // 1. Thermal heat gradient on rear ergonomic grips
        let heat_color = evaluate_thermal_heat_gradient(self.telemetry.gpu_temp_celsius).apply_gamma();
        out_frame[LedZone::LeftGrip as usize] = heat_color;
        out_frame[LedZone::RightGrip as usize] = heat_color;

        // 2. Health-dependent Aura Ring pulsing
        if self.telemetry.health_percent < 25 {
            // Urgent critical low-health heartbeat throb (500ms period)
            self.heartbeat_phase_ms = (self.heartbeat_phase_ms + delta_ms) % 500;
            let phase_frac = (self.heartbeat_phase_ms as f32) / 500.0;
            let pulse = (sin_f32(phase_frac * TWO_PI) + 1.0) * 0.5;
            let critical_red = Rgb8::RED.scale(0.20 + 0.80 * pulse).apply_gamma();
            out_frame[LedZone::LeftStickRing as usize] = critical_red;
            out_frame[LedZone::RightStickRing as usize] = critical_red;
        } else if self.telemetry.health_percent < 50 {
            // Warning state: amber breathing
            let warning_amber = Rgb8::AMBER.scale(0.65).apply_gamma();
            out_frame[LedZone::LeftStickRing as usize] = warning_amber;
            out_frame[LedZone::RightStickRing as usize] = warning_amber;
        }

        // 3. Highest-priority damage flash overlay
        if self.damage_flash_remaining_ms > 0 {
            self.damage_flash_remaining_ms = self.damage_flash_remaining_ms.saturating_sub(delta_ms);
            if self.damage_flash_remaining_ms > 0 {
                let decay = (self.damage_flash_remaining_ms as f32) / 200.0;
                let flash_color = Rgb8::WHITE.lerp(Rgb8::RED, 0.35).scale(self.damage_flash_peak * decay);
                for slot in out_frame.iter_mut() {
                    *slot = slot.lerp(flash_color, decay * self.damage_flash_peak);
                }
            }
        }
    }
}

// -----------------------------------------------------------------------------
// Embedded Controller (EC) / I2C Hardware Register Mapping
// -----------------------------------------------------------------------------

/// Hardware register addresses for Handheld Embedded Controller (EC) / I2C RGB LED Driver.
pub mod ec_regs {
    /// I2C 7-bit bus slave address for handheld LED microcontroller.
    pub const EC_I2C_SLAVE_ADDR: u8 = 0x5A;
    /// Hardware Chip Identification register (returns 0xA5).
    pub const REG_CHIP_ID: u8 = 0x00;
    /// Master power and configuration control register.
    pub const REG_GLOBAL_CONFIG: u8 = 0x01;
    /// Master brightness scaling register (0..255).
    pub const REG_MASTER_BRIGHTNESS: u8 = 0x02;
    /// Starting register address for per-zone RGB PWM channels.
    pub const REG_ZONE_BASE: u8 = 0x10;
    /// Number of bytes allocated per lighting zone in EC register space (R, G, B, flags).
    pub const BYTES_PER_ZONE: usize = 4;
    /// Hardware frame latch / commit command register (immediately follows 24 bytes of zone PWMs).
    pub const REG_COMMIT_LATCH: u8 = 0x28;
    /// Hardware status and fault monitoring register.
    pub const REG_STATUS: u8 = 0x29;

    /// Command token to commit and latch pending PWM registers to physical LED pins.
    pub const CMD_LATCH_UPDATE: u8 = 0x01;
}

/// Serializes a 6-zone RGB frame buffer into an EC I2C register transaction packet.
///
/// Output packet layout:
/// - [0]: Target start register address (REG_ZONE_BASE: 0x10)
/// - [1..24]: 6 zones * 4 bytes (Red PWM, Green PWM, Blue PWM, Zone flags)
/// - [25]: Commit command byte (0x01) at REG_COMMIT_LATCH (0x28)
pub fn serialize_ec_frame(
    frame: &[Rgb8; LED_ZONE_COUNT],
    out_buf: &mut [u8],
) -> usize {
    if out_buf.len() < 26 {
        return 0;
    }

    out_buf[0] = ec_regs::REG_ZONE_BASE;
    let mut idx = 1;
    for &color in frame {
        out_buf[idx] = color.r;
        out_buf[idx + 1] = color.g;
        out_buf[idx + 2] = color.b;
        out_buf[idx + 3] = 0x01; // Zone enabled flag
        idx += ec_regs::BYTES_PER_ZONE;
    }

    out_buf[idx] = ec_regs::CMD_LATCH_UPDATE;
    idx + 1
}

/// Simulated Handheld Embedded Controller (EC) I2C peripheral for testing hardware register interactions.
pub struct MockEcDevice {
    /// 64-byte hardware register bank.
    pub registers: [u8; 64],
    /// Latched flag set when commit command is received.
    pub latched_frame_count: u32,
}

impl MockEcDevice {
    pub fn new() -> Self {
        let mut dev = Self {
            registers: [0u8; 64],
            latched_frame_count: 0,
        };
        dev.registers[ec_regs::REG_CHIP_ID as usize] = 0xA5;
        dev.registers[ec_regs::REG_GLOBAL_CONFIG as usize] = 0x01;
        dev
    }

    /// Ingests a raw I2C burst write starting at `start_reg`.
    pub fn write_i2c_burst(&mut self, start_reg: u8, data: &[u8]) {
        let start = start_reg as usize;
        for (i, &byte) in data.iter().enumerate() {
            let addr = start + i;
            if addr < self.registers.len() {
                self.registers[addr] = byte;
                if addr == ec_regs::REG_COMMIT_LATCH as usize && byte == ec_regs::CMD_LATCH_UPDATE {
                    self.latched_frame_count += 1;
                }
            }
        }
    }

    /// Reads duty cycle PWM values (R, G, B) for a specific zone from EC register memory.
    pub fn get_zone_pwm(&self, zone: LedZone) -> (u8, u8, u8) {
        let offset = ec_regs::REG_ZONE_BASE as usize + (zone as usize) * ec_regs::BYTES_PER_ZONE;
        (self.registers[offset], self.registers[offset + 1], self.registers[offset + 2])
    }
}

// -----------------------------------------------------------------------------
// Unit Tests & Verification
// -----------------------------------------------------------------------------

/// Unit test verifying Rgb8 color mathematics, HSV conversions, gamma table, and multi-zone layout.
pub fn verify_rgb_math_and_gamma() -> bool {
    // 1. Verify Rgb8 constants and linear interpolation
    let black = Rgb8::BLACK;
    let white = Rgb8::WHITE;
    let mid_gray = black.lerp(white, 0.5);
    if mid_gray.r < 127 || mid_gray.r > 128 || mid_gray.g != mid_gray.r || mid_gray.b != mid_gray.r {
        return false;
    }

    // Scaling
    let scaled_red = Rgb8::RED.scale(0.5);
    if scaled_red.r < 127 || scaled_red.r > 128 || scaled_red.g != 0 || scaled_red.b != 0 {
        return false;
    }

    // 2. Verify HSV to RGB conversion
    // Red (H=0, S=1, V=1)
    let red_hsv = HsvColor::new(0.0, 1.0, 1.0).to_rgb();
    if red_hsv != Rgb8::RED {
        return false;
    }

    // Green (H=120, S=1, V=1)
    let green_hsv = HsvColor::new(120.0, 1.0, 1.0).to_rgb();
    if green_hsv != Rgb8::GREEN {
        return false;
    }

    // Blue (H=240, S=1, V=1)
    let blue_hsv = HsvColor::new(240.0, 1.0, 1.0).to_rgb();
    if blue_hsv != Rgb8::BLUE {
        return false;
    }

    // Yellow (H=60, S=1, V=1)
    let yellow_hsv = HsvColor::new(60.0, 1.0, 1.0).to_rgb();
    if yellow_hsv != Rgb8::YELLOW {
        return false;
    }

    // 3. Verify RGB to HSV roundtrip
    let orig = Rgb8::new(255, 128, 0); // Orange
    let hsv = HsvColor::from_rgb(orig);
    let roundtrip = hsv.to_rgb();
    if (roundtrip.r as i16 - orig.r as i16).abs() > 1
        || (roundtrip.g as i16 - orig.g as i16).abs() > 1
        || (roundtrip.b as i16 - orig.b as i16).abs() > 1
    {
        return false;
    }

    // 4. Verify 2.2 Gamma correction lookup table
    // 0 -> 0
    if apply_gamma_2_2(0) != 0 {
        return false;
    }
    // 255 -> 255
    if apply_gamma_2_2(255) != 255 {
        return false;
    }
    // 128 -> 56 (+- 1)
    let mid_pwm = apply_gamma_2_2(128);
    if mid_pwm < 55 || mid_pwm > 57 {
        return false;
    }
    // Monotonic non-decreasing check across all 256 entries
    for i in 0..255 {
        if GAMMA_2_2_TABLE[i] > GAMMA_2_2_TABLE[i + 1] {
            return false;
        }
    }

    // 5. Verify Multi-Zone Layout descriptors
    for &zone in &ALL_LED_ZONES {
        let desc = get_zone_descriptor(zone);
        if desc.zone != zone || desc.led_count == 0 || desc.name.is_empty() {
            return false;
        }
    }

    true
}

/// Unit test verifying effect generators (breathing, rainbow, battery) and Aura Sync synchronization.
pub fn verify_effects_and_aura_sync() -> bool {
    let mut controller = RgbLightingController::new();

    // 1. Static effect and master brightness scaling
    controller.set_zone_effect(LedZone::ChassisLogo, LedEffect::Static(Rgb8::WHITE));
    controller.set_master_brightness(1.0);
    controller.update(16);
    let c_full = controller.get_zone_color(LedZone::ChassisLogo);
    if c_full != Rgb8::WHITE {
        return false;
    }

    // At 50% master brightness, 255 * 0.5 = 128 -> gamma 2.2 = 56
    controller.set_master_brightness(0.5);
    controller.update(16);
    let c_half = controller.get_zone_color(LedZone::ChassisLogo);
    if c_half.r < 55 || c_half.r > 57 || c_half.g != c_half.r || c_half.b != c_half.r {
        return false;
    }
    controller.set_master_brightness(1.0);

    // 2. Breathing effect sinusoidal progression
    // Period = 1000ms, Red color, min=0.0, max=1.0
    let breathing = LedEffect::Breathing {
        color: Rgb8::RED,
        period_ms: 1000,
        min_brightness: 0.0,
        max_brightness: 1.0,
    };
    // At t = 250ms (peak): sin(PI/2) = 1.0 -> 100% brightness (Red=255)
    let peak_color = breathing.render(250);
    if peak_color.r != 255 || peak_color.g != 0 || peak_color.b != 0 {
        return false;
    }
    // At t = 750ms (trough): sin(3PI/2) = -1.0 -> 0% brightness (Red=0)
    let trough_color = breathing.render(750);
    if trough_color.r != 0 || trough_color.g != 0 || trough_color.b != 0 {
        return false;
    }

    // 3. Rainbow cycle progression
    // Period = 3600ms -> 100ms per 10 degrees
    let rainbow = LedEffect::RainbowCycle {
        period_ms: 3600,
        spatial_phase_deg: 0.0,
    };
    let c_0 = rainbow.render(0); // Hue 0 = Red
    if c_0 != Rgb8::RED {
        return false;
    }
    let c_120 = rainbow.render(1200); // Hue 120 = Green
    if c_120 != Rgb8::GREEN {
        return false;
    }
    let c_240 = rainbow.render(2400); // Hue 240 = Blue
    if c_240 != Rgb8::BLUE {
        return false;
    }

    // 4. Battery indicator validation
    let batt_crit = LedEffect::BatteryIndicator {
        charge_percent: 15,
        is_charging: false,
    }.render(0);
    if batt_crit != Rgb8::RED {
        return false;
    }

    let batt_ok = LedEffect::BatteryIndicator {
        charge_percent: 85,
        is_charging: false,
    }.render(0);
    if batt_ok != Rgb8::GREEN {
        return false;
    }

    // 5. Flowing perimeter Aura Sync spatial offset check
    controller.set_all_zones_effect(LedEffect::RainbowCycle {
        period_ms: 3600,
        spatial_phase_deg: 0.0,
    });
    controller.set_sync_mode(AuraSyncMode::FlowingPerimeter);
    // At elapsed time 0:
    // LeftStick is at 0.0 deg (Red), RightStick is at 180.0 deg (Cyan)
    controller.update(0);
    let left_stick = controller.get_zone_color(LedZone::LeftStickRing);
    let right_stick = controller.get_zone_color(LedZone::RightStickRing);
    // Left stick must have dominant Red, Right stick must have dominant Green/Blue (Cyan)
    if left_stick.r < 200 || left_stick.g != 0 || left_stick.b != 0 {
        return false;
    }
    if right_stick.r != 0 || right_stick.g < 200 || right_stick.b < 200 {
        return false;
    }

    true
}

/// Unit test verifying game-state reactive lighting triggers and hardware EC/I2C register mapping.
pub fn verify_game_state_reactive_and_ec_mapping() -> bool {
    // 1. Verify Thermal Heat Gradient mapping
    let cold_color = evaluate_thermal_heat_gradient(35.0);
    if cold_color.b < 200 || cold_color.r > 20 {
        return false;
    }

    let hot_color = evaluate_thermal_heat_gradient(90.0);
    if hot_color.r != 255 || hot_color.b != 0 {
        return false;
    }

    // 2. Verify Game-State Reactive Sync and Health Heartbeat
    let mut sync = GameStateAuraSync::new();
    let base_frame = [Rgb8::WHITE; LED_ZONE_COUNT];
    let mut out_frame = [Rgb8::BLACK; LED_ZONE_COUNT];

    // Low health: 15% -> Critical red heartbeat on stick rings
    sync.update_telemetry(GameTelemetry {
        health_percent: 15,
        shield_percent: 0,
        gpu_temp_celsius: 40.0,
        fan_rpm: 3000,
    });
    sync.apply_reactive_overlay(&base_frame, 16, &mut out_frame);
    let stick_red = out_frame[LedZone::LeftStickRing as usize];
    if stick_red.r == 0 || stick_red.g != 0 || stick_red.b != 0 {
        return false;
    }

    // 3. Verify Damage Flash Trigger & Temporal Decay
    sync.trigger_damage_flash(1.0);
    sync.apply_reactive_overlay(&base_frame, 0, &mut out_frame);
    // Immediate frame: dominant flash
    let flash_stick = out_frame[LedZone::ChassisLogo as usize];
    if flash_stick.r < 200 {
        return false;
    }

    // Step 250ms -> Damage flash decays to 0
    sync.apply_reactive_overlay(&base_frame, 250, &mut out_frame);
    let normal_logo = out_frame[LedZone::ChassisLogo as usize];
    if normal_logo != Rgb8::WHITE {
        return false;
    }

    // 4. Verify EC / I2C Register Serialization & Mock EC Device
    let mut test_frame = [Rgb8::BLACK; LED_ZONE_COUNT];
    test_frame[LedZone::LeftStickRing as usize] = Rgb8::new(200, 100, 50);
    test_frame[LedZone::RightStickRing as usize] = Rgb8::new(0, 255, 128);

    let mut packet = [0u8; 32];
    let packet_len = serialize_ec_frame(&test_frame, &mut packet);
    if packet_len != 26 {
        return false;
    }
    if packet[0] != ec_regs::REG_ZONE_BASE {
        return false;
    }

    // Ingest into Mock EC
    let mut mock_ec = MockEcDevice::new();
    mock_ec.write_i2c_burst(packet[0], &packet[1..packet_len]);

    // Check that Mock EC latched the frame
    if mock_ec.latched_frame_count != 1 {
        return false;
    }

    // Check written zone PWMs in Mock EC register memory
    let (r_left, g_left, b_left) = mock_ec.get_zone_pwm(LedZone::LeftStickRing);
    if r_left != 200 || g_left != 100 || b_left != 50 {
        return false;
    }

    let (r_right, g_right, b_right) = mock_ec.get_zone_pwm(LedZone::RightStickRing);
    if r_right != 0 || g_right != 255 || b_right != 128 {
        return false;
    }

    true
}
