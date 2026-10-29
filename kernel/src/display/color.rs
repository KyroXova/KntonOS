//! 32-bit RGBA/BGRA color primitives for KntonOS display pipelines.

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct Color {
    pub r: u8,
    pub g: u8,
    pub b: u8,
    pub a: u8,
}

impl Color {
    pub const fn rgb(r: u8, g: u8, b: u8) -> Self {
        Self { r, g, b, a: 255 }
    }

    pub const fn rgba(r: u8, g: u8, b: u8, a: u8) -> Self {
        Self { r, g, b, a }
    }

    pub const BLACK: Self = Self::rgb(0, 0, 0);
    pub const WHITE: Self = Self::rgb(255, 255, 255);
    
    // Windows 11 Aesthetic Palette
    pub const WIN11_BG: Self = Self::rgb(28, 32, 40);         // Deep slate dark mode wallpaper
    pub const WIN11_TASKBAR: Self = Self::rgba(32, 38, 48, 230); // Mica-style translucent taskbar
    pub const WIN11_CARD_BG: Self = Self::rgb(45, 52, 64);      // Window client area
    pub const WIN11_CARD_BORDER: Self = Self::rgb(60, 70, 85);  // Subtle window border
    pub const WIN11_TITLEBAR: Self = Self::rgb(36, 42, 54);     // Window title bar
    pub const WIN11_ACCENT: Self = Self::rgb(0, 120, 215);      // Windows Fluent Blue accent
    pub const WIN11_TEXT_PRIMARY: Self = Self::rgb(240, 244, 250);
    pub const WIN11_TEXT_SECONDARY: Self = Self::rgb(160, 172, 190);
    pub const WIN11_CLOSE_RED: Self = Self::rgb(232, 17, 35);
    pub const WIN11_HOVER: Self = Self::rgb(55, 64, 78);

    #[inline(always)]
    pub fn to_u32_bgr(&self) -> u32 {
        ((self.r as u32) << 16) | ((self.g as u32) << 8) | (self.b as u32)
    }

    #[inline(always)]
    pub fn to_u32_rgb(&self) -> u32 {
        ((self.b as u32) << 16) | ((self.g as u32) << 8) | (self.r as u32)
    }
}
