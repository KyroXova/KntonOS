//! Handheld PC Multi-Zone RGB Lighting Driver: Color Math, Gamma Correction & Layout.
//!
//! Provides color primitives, 2.2 perceptual gamma correction, HSV color space
//! conversion, and multi-zone physical layout configurations for handheld gaming PCs.

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
