//! Unified Game State Overlay Compositor & Zero-Copy Hardware Cursor Plane Subsystem.
//!
//! Provides an ultra-low-overhead in-game Quick-Access overlay compositor (triggered via
//! dedicated handheld buttons like Steam Deck '...' or ROG Ally Armoury Crate), hardware cursor
//! plane with zero-copy scanout composition, and quick settings management.
//!
//! Architecture:
//! - Hardware Display Plane Descriptors (Primary, Overlay, Cursor) with DRM FourCC formats
//! - Sub-pixel Zero-Copy Hardware Cursor Plane & SIMD Fixed-Point Alpha Blending Pipeline
//! - Quick Settings Engine (TDP 5W-30W, Framerate limiter 30/40/60/120Hz, Fan acoustic modes)
//! - Handheld QAM Button Trigger & Smooth Slide Transition Physics

extern crate alloc;

use alloc::vec::Vec;
use alloc::string::String;

// =============================================================================
// 1. DRM Pixel Formats & Plane Constants
// =============================================================================

pub const DRM_FORMAT_XRGB8888: u32 = 0x34325258; // FourCC XR24
pub const DRM_FORMAT_ARGB8888: u32 = 0x34325241; // FourCC AR24
pub const DRM_FORMAT_RGBA8888: u32 = 0x34324152; // FourCC RA24
pub const DRM_FORMAT_BGRA8888: u32 = 0x41524742; // FourCC BA24
pub const DRM_FORMAT_NV12: u32     = 0x3231564E; // FourCC NV12 (Y/CbCr 4:2:0)

pub const PLANE_ZPOS_PRIMARY: u32 = 0;
pub const PLANE_ZPOS_OVERLAY: u32 = 1;
pub const PLANE_ZPOS_CURSOR: u32  = 2;

pub const DEFAULT_QAM_WIDTH: u32 = 340;
pub const DEFAULT_CURSOR_SIZE: u32 = 64;

// -----------------------------------------------------------------------------
// Plane Geometry
// -----------------------------------------------------------------------------

/// 2D Integer Rectangle for display plane source and destination coordinates.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct PlaneRect {
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
}

impl PlaneRect {
    pub const fn new(x: i32, y: i32, width: u32, height: u32) -> Self {
        Self { x, y, width, height }
    }

    /// Checks if a 2D point lies within the rectangle bounds.
    pub fn contains(&self, px: i32, py: i32) -> bool {
        px >= self.x
            && px < self.x + self.width as i32
            && py >= self.y
            && py < self.y + self.height as i32
    }

    /// Checks whether this rectangle intersects with another.
    pub fn intersects(&self, other: &PlaneRect) -> bool {
        self.x < other.x + other.width as i32
            && self.x + self.width as i32 > other.x
            && self.y < other.y + other.height as i32
            && self.y + self.height as i32 > other.y
    }
}

// -----------------------------------------------------------------------------
// Hardware Plane Types & Descriptors
// -----------------------------------------------------------------------------

/// Hardware display plane classification in DRM/KMS scanout pipe.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u32)]
pub enum HardwarePlaneType {
    Primary = 0,
    Overlay = 1,
    Cursor = 2,
}

/// Hardware Overlay Plane Descriptor configured for zero-copy scanout composition.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OverlayPlaneDescriptor {
    pub plane_id: u32,
    pub plane_type: HardwarePlaneType,
    pub zpos: u32,
    pub enabled: bool,
    /// Plane-wide global alpha in DRM 16-bit format (0x0000 = transparent, 0xFFFF = fully opaque).
    pub global_alpha: u16,
    pub dest_rect: PlaneRect,
    pub src_rect: PlaneRect,
    pub pixel_format: u32,
    pub stride_bytes: usize,
    pub framebuffer_phys_addr: u64,
    pub buffer: Vec<u32>,
}

impl OverlayPlaneDescriptor {
    pub fn new(plane_id: u32, width: u32, height: u32, screen_width: u32, screen_height: u32) -> Self {
        let size = (width * height) as usize;
        let mut buffer = Vec::with_capacity(size);
        buffer.resize(size, 0); // Transparent black

        // Right-docked sidebar positioning
        let dest_x = (screen_width.saturating_sub(width)) as i32;
        let dest_rect = PlaneRect::new(dest_x, 0, width, screen_height);
        let src_rect = PlaneRect::new(0, 0, width, height);

        Self {
            plane_id,
            plane_type: HardwarePlaneType::Overlay,
            zpos: PLANE_ZPOS_OVERLAY,
            enabled: false,
            global_alpha: 0xD999, // ~85% opaque default
            dest_rect,
            src_rect,
            pixel_format: DRM_FORMAT_ARGB8888,
            stride_bytes: (width as usize) * 4,
            framebuffer_phys_addr: 0,
            buffer,
        }
    }

    /// Sets the on-screen global opacity (0.0 to 1.0).
    pub fn set_opacity_f32(&mut self, opacity: f32) {
        let clamped = opacity.clamp(0.0, 1.0);
        self.global_alpha = (clamped * 65535.0) as u16;
    }

    /// Returns the global opacity as a normalized float (0.0 to 1.0).
    pub fn opacity_f32(&self) -> f32 {
        self.global_alpha as f32 / 65535.0
    }
}

// =============================================================================
// 2. Handheld Quick-Access Menu (QAM) Trigger & Layout
// =============================================================================

/// Dedicated handheld physical quick-access buttons.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HandheldButton {
    /// Steam Deck dedicated Quick-Access Menu button ('...')
    SteamQuickAccess,
    /// ASUS ROG Ally dedicated Armoury Crate / Command Center button
    RogArmouryCrate,
    /// AYANEO Quick Menu button
    AyaneoQuickMenu,
    /// Lenovo Legion Go Legion 'L' / 'R' quick access button
    LegionQuickAccess,
}

/// Operational state of the overlay slide-in / slide-out animation.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum OverlayVisibilityState {
    Hidden,
    Opening { progress: f32 },
    Visible,
    Closing { progress: f32 },
}

/// Standard Quick-Access Menu tabs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QuickAccessTab {
    Performance,
    Display,
    Audio,
    Peripherals,
}

/// Interactive Quick-Access Menu setting item description.
#[derive(Debug, Clone, PartialEq)]
pub struct QuickAccessItem {
    pub id: u32,
    pub title: String,
    pub tab: QuickAccessTab,
    pub bounds: PlaneRect,
    pub is_highlighted: bool,
}

/// Quick-Access Menu layout geometry and animation controller.
#[derive(Debug, Clone)]
pub struct QuickAccessMenuLayout {
    pub screen_width: u32,
    pub screen_height: u32,
    pub menu_width: u32,
    pub current_tab: QuickAccessTab,
    pub visibility: OverlayVisibilityState,
    pub items: Vec<QuickAccessItem>,
    pub selected_item_idx: usize,
}

impl QuickAccessMenuLayout {
    pub fn new(screen_width: u32, screen_height: u32) -> Self {
        let menu_width = DEFAULT_QAM_WIDTH.min(screen_width);
        let mut layout = Self {
            screen_width,
            screen_height,
            menu_width,
            current_tab: QuickAccessTab::Performance,
            visibility: OverlayVisibilityState::Hidden,
            items: Vec::new(),
            selected_item_idx: 0,
        };
        layout.populate_default_items();
        layout
    }

    /// Toggles the quick access menu upon pressing a dedicated handheld hardware button.
    pub fn handle_button_press(&mut self, _btn: HandheldButton) {
        match self.visibility {
            OverlayVisibilityState::Hidden | OverlayVisibilityState::Closing { .. } => {
                self.visibility = OverlayVisibilityState::Opening { progress: 0.0 };
            }
            OverlayVisibilityState::Visible | OverlayVisibilityState::Opening { .. } => {
                self.visibility = OverlayVisibilityState::Closing { progress: 1.0 };
            }
        }
    }

    /// Advances the smooth cubic ease-out slide animation.
    ///
    /// Returns the current on-screen X offset for the overlay plane.
    pub fn tick_animation(&mut self, delta_seconds: f32) -> i32 {
        const ANIMATION_DURATION: f32 = 0.15; // 150ms smooth transition
        let step = delta_seconds / ANIMATION_DURATION;

        match self.visibility {
            OverlayVisibilityState::Hidden => self.screen_width as i32,
            OverlayVisibilityState::Visible => (self.screen_width - self.menu_width) as i32,
            OverlayVisibilityState::Opening { progress } => {
                let next = (progress + step).min(1.0);
                if next >= 1.0 {
                    self.visibility = OverlayVisibilityState::Visible;
                    (self.screen_width - self.menu_width) as i32
                } else {
                    self.visibility = OverlayVisibilityState::Opening { progress: next };
                    let ease = Self::cubic_ease_out(next);
                    let x_offset = (self.screen_width as f32) - (self.menu_width as f32 * ease);
                    x_offset as i32
                }
            }
            OverlayVisibilityState::Closing { progress } => {
                let next = (progress - step).max(0.0);
                if next <= 0.0 {
                    self.visibility = OverlayVisibilityState::Hidden;
                    self.screen_width as i32
                } else {
                    self.visibility = OverlayVisibilityState::Closing { progress: next };
                    let ease = Self::cubic_ease_out(next);
                    let x_offset = (self.screen_width as f32) - (self.menu_width as f32 * ease);
                    x_offset as i32
                }
            }
        }
    }

    /// Cubic ease-out polynomial curve: f(t) = 1 - (1 - t)^3.
    pub fn cubic_ease_out(t: f32) -> f32 {
        let inv = 1.0 - t.clamp(0.0, 1.0);
        1.0 - (inv * inv * inv)
    }

    /// Populates standard handheld performance settings items into the layout.
    fn populate_default_items(&mut self) {
        self.items.clear();
        let base_x = (self.screen_width - self.menu_width) as i32 + 16;
        let mut y = 60;

        let performance_titles = [
            "TDP Power Limit (W)",
            "Framerate Limiter (FPS)",
            "Fan Acoustic Profile",
            "Display Brightness (%)",
            "Audio Master Volume (%)",
            "Overlay Transparency (%)",
        ];

        for (i, title) in performance_titles.iter().enumerate() {
            self.items.push(QuickAccessItem {
                id: (i + 1) as u32,
                title: String::from(*title),
                tab: QuickAccessTab::Performance,
                bounds: PlaneRect::new(base_x, y, self.menu_width - 32, 44),
                is_highlighted: i == 0,
            });
            y += 52;
        }
    }

    /// Navigates selection up or down via gamepad D-Pad.
    pub fn select_previous(&mut self) {
        if self.selected_item_idx > 0 {
            self.items[self.selected_item_idx].is_highlighted = false;
            self.selected_item_idx -= 1;
            self.items[self.selected_item_idx].is_highlighted = true;
        }
    }

    pub fn select_next(&mut self) {
        if self.selected_item_idx + 1 < self.items.len() {
            self.items[self.selected_item_idx].is_highlighted = false;
            self.selected_item_idx += 1;
            self.items[self.selected_item_idx].is_highlighted = true;
        }
    }
}

// -----------------------------------------------------------------------------
// Unit Tests for Commit 1
// -----------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    pub fn test_overlay_plane_descriptors() {
        let overlay = OverlayPlaneDescriptor::new(10, 340, 800, 1280, 800);
        assert_eq!(overlay.plane_id, 10);
        assert_eq!(overlay.plane_type, HardwarePlaneType::Overlay);
        assert_eq!(overlay.zpos, PLANE_ZPOS_OVERLAY);
        assert_eq!(overlay.pixel_format, DRM_FORMAT_ARGB8888);
        assert_eq!(overlay.dest_rect.x, 1280 - 340);
        assert_eq!(overlay.dest_rect.width, 340);
        assert_eq!(overlay.dest_rect.height, 800);
        assert!((overlay.opacity_f32() - 0.85).abs() < 0.05);

        // Test PlaneRect hit testing
        let rect = PlaneRect::new(100, 50, 200, 150);
        assert!(rect.contains(150, 100));
        assert!(!rect.contains(99, 100));
        assert!(!rect.contains(301, 100));
    }

    #[test]
    pub fn test_quick_access_menu_layout_and_animation() {
        let mut layout = QuickAccessMenuLayout::new(1280, 800);
        assert_eq!(layout.menu_width, 340);
        assert_eq!(layout.visibility, OverlayVisibilityState::Hidden);

        // Press Steam '...' button to open QAM
        layout.handle_button_press(HandheldButton::SteamQuickAccess);
        assert_eq!(layout.visibility, OverlayVisibilityState::Opening { progress: 0.0 });

        // Tick 75ms (halfway)
        let x_half = layout.tick_animation(0.075);
        assert!(x_half < 1280 && x_half > 940);

        // Tick another 75ms (reach open)
        let x_full = layout.tick_animation(0.075);
        assert_eq!(x_full, 1280 - 340);
        assert_eq!(layout.visibility, OverlayVisibilityState::Visible);

        // D-Pad navigation
        assert_eq!(layout.selected_item_idx, 0);
        layout.select_next();
        assert_eq!(layout.selected_item_idx, 1);
        assert!(layout.items[1].is_highlighted);
        layout.select_previous();
        assert_eq!(layout.selected_item_idx, 0);

        // Close menu
        layout.handle_button_press(HandheldButton::RogArmouryCrate);
        assert!(matches!(layout.visibility, OverlayVisibilityState::Closing { .. }));
        let x_closed = layout.tick_animation(0.16);
        assert_eq!(x_closed, 1280);
        assert_eq!(layout.visibility, OverlayVisibilityState::Hidden);
    }
}
