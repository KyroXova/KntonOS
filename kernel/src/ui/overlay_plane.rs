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

// =============================================================================
// 3. Hardware Cursor Plane & Sub-Pixel Scanout Composition
// =============================================================================

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CursorType {
    Arrow,
    Hand,
    TextIBeam,
    Crosshair,
}

#[derive(Debug, Clone)]
pub struct HardwareCursorPlane {
    pub plane_id: u32,
    pub width: u32,
    pub height: u32,
    pub subpixel_x: f32,
    pub subpixel_y: f32,
    pub hotspot_x: u32,
    pub hotspot_y: u32,
    pub enabled: bool,
    pub cursor_type: CursorType,
    pub buffer: Vec<u32>, // e.g. 64x64 ARGB8888
    pub framebuffer_phys_addr: u64,
}

impl HardwareCursorPlane {
    pub fn new(plane_id: u32, size: u32) -> Self {
        let count = (size * size) as usize;
        let mut buffer = Vec::with_capacity(count);
        buffer.resize(count, 0);

        let mut cursor = Self {
            plane_id,
            width: size,
            height: size,
            subpixel_x: 0.0,
            subpixel_y: 0.0,
            hotspot_x: 0,
            hotspot_y: 0,
            enabled: true,
            cursor_type: CursorType::Arrow,
            buffer,
            framebuffer_phys_addr: 0,
        };
        cursor.render_default_arrow_sprite();
        cursor
    }

    /// Sets cursor on-screen sub-pixel position.
    pub fn set_subpixel_position(&mut self, x: f32, y: f32) {
        self.subpixel_x = x;
        self.subpixel_y = y;
    }

    /// Integer screen coordinates considering hotspot offset.
    pub fn screen_coords(&self) -> (i32, i32) {
        let rx = if self.subpixel_x >= 0.0 { (self.subpixel_x + 0.5) as i32 } else { (self.subpixel_x - 0.5) as i32 };
        let ry = if self.subpixel_y >= 0.0 { (self.subpixel_y + 0.5) as i32 } else { (self.subpixel_y - 0.5) as i32 };
        (rx - self.hotspot_x as i32, ry - self.hotspot_y as i32)
    }

    /// Sets the cursor shape/type and updates sprite & hotspot.
    pub fn set_cursor_type(&mut self, c_type: CursorType) {
        self.cursor_type = c_type;
        self.buffer.fill(0);
        match c_type {
            CursorType::Arrow => {
                self.hotspot_x = 0;
                self.hotspot_y = 0;
                self.render_default_arrow_sprite();
            }
            CursorType::Crosshair => {
                self.hotspot_x = self.width / 2;
                self.hotspot_y = self.height / 2;
                self.render_crosshair_sprite();
            }
            CursorType::Hand => {
                self.hotspot_x = 4;
                self.hotspot_y = 0;
                self.render_default_arrow_sprite();
            }
            CursorType::TextIBeam => {
                self.hotspot_x = self.width / 2;
                self.hotspot_y = self.height / 2;
                self.render_ibeam_sprite();
            }
        }
    }

    fn render_default_arrow_sprite(&mut self) {
        for y in 0..16 {
            for x in 0..=y {
                if x < self.width as usize && y < self.height as usize {
                    let is_border = x == 0 || x == y || y == 15;
                    let color = if is_border {
                        0xFF000000 // Opaque black
                    } else {
                        0xFFFFFFFF // Opaque white
                    };
                    self.buffer[y * (self.width as usize) + x] = color;
                }
            }
        }
    }

    fn render_crosshair_sprite(&mut self) {
        let cx = (self.width / 2) as usize;
        let cy = (self.height / 2) as usize;
        let w = self.width as usize;
        for i in 0..self.width as usize {
            if i < self.buffer.len() {
                self.buffer[cy * w + i] = 0xFF00FF00;
                self.buffer[i * w + cx] = 0xFF00FF00;
            }
        }
    }

    fn render_ibeam_sprite(&mut self) {
        let cx = (self.width / 2) as usize;
        let w = self.width as usize;
        for y in 2..18 {
            if y < self.height as usize {
                self.buffer[y * w + cx] = 0xFFFFFFFF;
            }
        }
        for x in (cx.saturating_sub(4))..=(cx + 4).min(w - 1) {
            self.buffer[2 * w + x] = 0xFFFFFFFF;
            self.buffer[17 * w + x] = 0xFFFFFFFF;
        }
    }
}

// =============================================================================
// 4. Alpha Blending Pipeline (Porter-Duff Over Fixed-Point Engine)
// =============================================================================

pub struct AlphaBlendPipeline;

impl AlphaBlendPipeline {
    /// Fixed-point Porter-Duff Over blend of foreground ARGB8888 onto background XRGB/ARGB8888.
    #[inline(always)]
    pub fn blend_pixel(bg: u32, fg: u32, plane_global_alpha: u16) -> u32 {
        let fg_alpha = ((fg >> 24) & 0xFF) as u32;
        if fg_alpha == 0 {
            return bg;
        }

        let eff_alpha = (fg_alpha * ((plane_global_alpha >> 8) as u32) + 128) / 255;
        if eff_alpha == 0 {
            return bg;
        }
        if eff_alpha >= 255 {
            return fg;
        }

        let bg_r = (bg >> 16) & 0xFF;
        let bg_g = (bg >> 8) & 0xFF;
        let bg_b = bg & 0xFF;

        let fg_r = (fg >> 16) & 0xFF;
        let fg_g = (fg >> 8) & 0xFF;
        let fg_b = fg & 0xFF;

        let inv_alpha = 255 - eff_alpha;

        let out_r = (fg_r * eff_alpha + bg_r * inv_alpha + 128) / 255;
        let out_g = (fg_g * eff_alpha + bg_g * inv_alpha + 128) / 255;
        let out_b = (fg_b * eff_alpha + bg_b * inv_alpha + 128) / 255;

        0xFF000000 | ((out_r & 0xFF) << 16) | ((out_g & 0xFF) << 8) | (out_b & 0xFF)
    }

    /// Composes the 3 hardware planes (Primary, Overlay, Cursor) into an output frame.
    pub fn compose_3planes(
        primary: &[u32],
        overlay: &OverlayPlaneDescriptor,
        cursor: &HardwareCursorPlane,
        out: &mut [u32],
        width: usize,
        height: usize,
    ) {
        let total_pixels = width * height;
        let copy_len = primary.len().min(out.len()).min(total_pixels);
        out[..copy_len].copy_from_slice(&primary[..copy_len]);

        // 1. Blend Overlay Plane if enabled
        if overlay.enabled {
            let ox = overlay.dest_rect.x;
            let oy = overlay.dest_rect.y;
            let ow = overlay.dest_rect.width as usize;
            let oh = overlay.dest_rect.height as usize;

            for row in 0..oh {
                let screen_y = oy + row as i32;
                if screen_y < 0 || screen_y >= height as i32 {
                    continue;
                }
                for col in 0..ow {
                    let screen_x = ox + col as i32;
                    if screen_x < 0 || screen_x >= width as i32 {
                        continue;
                    }
                    let overlay_idx = row * ow + col;
                    if overlay_idx < overlay.buffer.len() {
                        let fg = overlay.buffer[overlay_idx];
                        let out_idx = (screen_y as usize) * width + (screen_x as usize);
                        if out_idx < out.len() {
                            let bg = out[out_idx];
                            out[out_idx] = Self::blend_pixel(bg, fg, overlay.global_alpha);
                        }
                    }
                }
            }
        }

        // 2. Blend Hardware Cursor Plane if enabled
        if cursor.enabled {
            let (cx, cy) = cursor.screen_coords();
            let cw = cursor.width as usize;
            let ch = cursor.height as usize;

            for row in 0..ch {
                let screen_y = cy + row as i32;
                if screen_y < 0 || screen_y >= height as i32 {
                    continue;
                }
                for col in 0..cw {
                    let screen_x = cx + col as i32;
                    if screen_x < 0 || screen_x >= width as i32 {
                        continue;
                    }
                    let cursor_idx = row * cw + col;
                    if cursor_idx < cursor.buffer.len() {
                        let fg = cursor.buffer[cursor_idx];
                        let out_idx = (screen_y as usize) * width + (screen_x as usize);
                        if out_idx < out.len() {
                            let bg = out[out_idx];
                            out[out_idx] = Self::blend_pixel(bg, fg, 0xFFFF);
                        }
                    }
                }
            }
        }
    }
}

// =============================================================================
// 5. Quick Settings Manager (TDP, Framerate, Fan, Brightness, Audio, Opacity)
// =============================================================================

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FpsLimit {
    Uncapped = 0,
    Fps30 = 30,
    Fps40 = 40,
    Fps60 = 60,
    Fps120 = 120,
}

impl FpsLimit {
    pub const fn target_frametime_us(&self) -> u32 {
        match self {
            Self::Uncapped => 0,
            Self::Fps30 => 33333,
            Self::Fps40 => 25000,
            Self::Fps60 => 16666,
            Self::Fps120 => 8333,
        }
    }

    pub fn cycle_next(&self) -> Self {
        match self {
            Self::Uncapped => Self::Fps30,
            Self::Fps30 => Self::Fps40,
            Self::Fps40 => Self::Fps60,
            Self::Fps60 => Self::Fps120,
            Self::Fps120 => Self::Uncapped,
        }
    }

    pub fn cycle_prev(&self) -> Self {
        match self {
            Self::Uncapped => Self::Fps120,
            Self::Fps30 => Self::Uncapped,
            Self::Fps40 => Self::Fps30,
            Self::Fps60 => Self::Fps40,
            Self::Fps120 => Self::Fps60,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FanAcousticMode {
    Quiet,
    Balanced,
    Performance,
}

impl FanAcousticMode {
    pub fn target_dba(&self) -> u32 {
        match self {
            Self::Quiet => 28,
            Self::Balanced => 35,
            Self::Performance => 45,
        }
    }

    pub fn cycle_next(&self) -> Self {
        match self {
            Self::Quiet => Self::Balanced,
            Self::Balanced => Self::Performance,
            Self::Performance => Self::Quiet,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QuickSettingField {
    TdpPowerLimit = 0,
    FpsLimiter = 1,
    FanProfile = 2,
    Brightness = 3,
    Volume = 4,
    OverlayOpacity = 5,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QuickSettingAction {
    SelectNext,
    SelectPrev,
    AdjustLeft,  // Decrease slider / cycle backward
    AdjustRight, // Increase slider / cycle forward
    Toggle,
}

#[derive(Debug, Clone)]
pub struct QuickSettingsManager {
    pub tdp_watts: u8,               // 5 to 30 W (default 15 W)
    pub fps_limit: FpsLimit,         // 30, 40, 60, 120, Uncapped
    pub fan_mode: FanAcousticMode,   // Quiet, Balanced, Performance
    pub brightness_percent: u8,     // 0 to 100%
    pub volume_percent: u8,         // 0 to 100%
    pub overlay_opacity_percent: u8, // 20 to 100% (default 85%)
    pub active_field: QuickSettingField,
}

impl QuickSettingsManager {
    pub fn new() -> Self {
        Self {
            tdp_watts: 15,
            fps_limit: FpsLimit::Fps60,
            fan_mode: FanAcousticMode::Balanced,
            brightness_percent: 80,
            volume_percent: 75,
            overlay_opacity_percent: 85,
            active_field: QuickSettingField::TdpPowerLimit,
        }
    }

    pub fn set_tdp(&mut self, watts: u8) {
        self.tdp_watts = watts.clamp(5, 30);
    }

    pub fn set_brightness(&mut self, percent: u8) {
        self.brightness_percent = percent.min(100);
    }

    pub fn set_volume(&mut self, percent: u8) {
        self.volume_percent = percent.min(100);
    }

    pub fn set_overlay_opacity(&mut self, percent: u8) {
        self.overlay_opacity_percent = percent.clamp(20, 100);
    }

    /// Converts brightness percentage to 8-bit display backlight PWM value (0..255).
    pub fn brightness_pwm_value(&self) -> u8 {
        ((self.brightness_percent as u32 * 255) / 100) as u8
    }

    /// Computes 16-bit DRM global alpha from overlay opacity percentage.
    pub fn global_alpha_u16(&self) -> u16 {
        ((self.overlay_opacity_percent as u32 * 65535) / 100) as u16
    }

    /// Handles gamepad navigation and adjustments across quick settings items.
    pub fn handle_action(&mut self, action: QuickSettingAction) {
        match action {
            QuickSettingAction::SelectNext => {
                self.active_field = match self.active_field {
                    QuickSettingField::TdpPowerLimit => QuickSettingField::FpsLimiter,
                    QuickSettingField::FpsLimiter => QuickSettingField::FanProfile,
                    QuickSettingField::FanProfile => QuickSettingField::Brightness,
                    QuickSettingField::Brightness => QuickSettingField::Volume,
                    QuickSettingField::Volume => QuickSettingField::OverlayOpacity,
                    QuickSettingField::OverlayOpacity => QuickSettingField::TdpPowerLimit,
                };
            }
            QuickSettingAction::SelectPrev => {
                self.active_field = match self.active_field {
                    QuickSettingField::TdpPowerLimit => QuickSettingField::OverlayOpacity,
                    QuickSettingField::FpsLimiter => QuickSettingField::TdpPowerLimit,
                    QuickSettingField::FanProfile => QuickSettingField::FpsLimiter,
                    QuickSettingField::Brightness => QuickSettingField::FanProfile,
                    QuickSettingField::Volume => QuickSettingField::Brightness,
                    QuickSettingField::OverlayOpacity => QuickSettingField::Volume,
                };
            }
            QuickSettingAction::AdjustLeft => match self.active_field {
                QuickSettingField::TdpPowerLimit => {
                    self.tdp_watts = self.tdp_watts.saturating_sub(1).max(5);
                }
                QuickSettingField::FpsLimiter => {
                    self.fps_limit = self.fps_limit.cycle_prev();
                }
                QuickSettingField::FanProfile => {
                    self.fan_mode = match self.fan_mode {
                        FanAcousticMode::Quiet => FanAcousticMode::Performance,
                        FanAcousticMode::Balanced => FanAcousticMode::Quiet,
                        FanAcousticMode::Performance => FanAcousticMode::Balanced,
                    };
                }
                QuickSettingField::Brightness => {
                    self.brightness_percent = self.brightness_percent.saturating_sub(5);
                }
                QuickSettingField::Volume => {
                    self.volume_percent = self.volume_percent.saturating_sub(5);
                }
                QuickSettingField::OverlayOpacity => {
                    self.overlay_opacity_percent = self.overlay_opacity_percent.saturating_sub(5).max(20);
                }
            },
            QuickSettingAction::AdjustRight => match self.active_field {
                QuickSettingField::TdpPowerLimit => {
                    self.tdp_watts = (self.tdp_watts + 1).min(30);
                }
                QuickSettingField::FpsLimiter => {
                    self.fps_limit = self.fps_limit.cycle_next();
                }
                QuickSettingField::FanProfile => {
                    self.fan_mode = self.fan_mode.cycle_next();
                }
                QuickSettingField::Brightness => {
                    self.brightness_percent = (self.brightness_percent + 5).min(100);
                }
                QuickSettingField::Volume => {
                    self.volume_percent = (self.volume_percent + 5).min(100);
                }
                QuickSettingField::OverlayOpacity => {
                    self.overlay_opacity_percent = (self.overlay_opacity_percent + 5).min(100);
                }
            },
            QuickSettingAction::Toggle => match self.active_field {
                QuickSettingField::FpsLimiter => {
                    self.fps_limit = self.fps_limit.cycle_next();
                }
                QuickSettingField::FanProfile => {
                    self.fan_mode = self.fan_mode.cycle_next();
                }
                _ => {}
            },
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
    #[test]
    pub fn test_cursor_and_alpha_blending() {
        let mut cursor = HardwareCursorPlane::new(30, 64);
        assert_eq!(cursor.plane_id, 30);
        assert_eq!(cursor.width, 64);

        // Sub-pixel movement
        cursor.set_subpixel_position(100.4, 200.6);
        let (cx, cy) = cursor.screen_coords();
        assert_eq!(cx, 100);
        assert_eq!(cy, 201);

        // Crosshair hotspot check
        cursor.set_cursor_type(CursorType::Crosshair);
        assert_eq!(cursor.hotspot_x, 32);
        assert_eq!(cursor.hotspot_y, 32);
        let (chx, chy) = cursor.screen_coords();
        assert_eq!(chx, 100 - 32);
        assert_eq!(chy, 201 - 32);

        // Alpha blending test: 50% white over black = ~128 gray
        let bg = 0xFF000000;
        let fg = 0x80FFFFFF;
        let blended = AlphaBlendPipeline::blend_pixel(bg, fg, 0xFFFF);
        let r = (blended >> 16) & 0xFF;
        assert!(r >= 126 && r <= 130);
    }
    #[test]
    pub fn test_quick_settings_manager() {
        let mut qsm = QuickSettingsManager::new();
        assert_eq!(qsm.tdp_watts, 15);
        assert_eq!(qsm.fps_limit, FpsLimit::Fps60);
        assert_eq!(qsm.fps_limit.target_frametime_us(), 16666);
        assert_eq!(qsm.fan_mode, FanAcousticMode::Balanced);
        assert_eq!(qsm.fan_mode.target_dba(), 35);

        // Adjust TDP slider
        qsm.handle_action(QuickSettingAction::AdjustRight);
        assert_eq!(qsm.tdp_watts, 16);
        qsm.set_tdp(35); // Test clamp
        assert_eq!(qsm.tdp_watts, 30);
        qsm.set_tdp(2);  // Test min clamp
        assert_eq!(qsm.tdp_watts, 5);

        // Navigate to FPS Limiter and cycle
        qsm.handle_action(QuickSettingAction::SelectNext);
        assert_eq!(qsm.active_field, QuickSettingField::FpsLimiter);
        qsm.handle_action(QuickSettingAction::AdjustRight); // 60 -> 120
        assert_eq!(qsm.fps_limit, FpsLimit::Fps120);
        assert_eq!(qsm.fps_limit.target_frametime_us(), 8333);
        qsm.handle_action(QuickSettingAction::AdjustRight); // 120 -> Uncapped
        assert_eq!(qsm.fps_limit, FpsLimit::Uncapped);
        assert_eq!(qsm.fps_limit.target_frametime_us(), 0);

        // Brightness PWM
        qsm.set_brightness(100);
        assert_eq!(qsm.brightness_pwm_value(), 255);
        qsm.set_brightness(50);
        assert_eq!(qsm.brightness_pwm_value(), 127);

        // Global alpha
        qsm.set_overlay_opacity(85);
        let alpha = qsm.global_alpha_u16();
        assert!(alpha > 50000);
    }
}
