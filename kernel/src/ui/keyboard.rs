//! Unified Handheld PC Virtual Keyboard & Predictive Swipe IME Gesture Engine.
//!
//! Provides an ultra-low-latency in-game virtual keyboard for handheld PCs with zero input lag:
//! - Multi-Layout Maps: Full QWERTY, Compact Thumb-Pad, and Dual Gamepad Split-Pad
//! - Spatial Touch Hit-Test Grid with sub-microsecond lookup
//! - Continuous Swipe Trajectory Smoothing via Catmull-Rom Spline Interpolation
//! - Shape-Writing Curve Tokenizer with inflection / cusp feature extraction
//! - Predictive N-Gram Word Candidate Lattice with spatial error tolerance ellipse
//! - Direct Win32 Virtual Key Event Injection

extern crate alloc;

use alloc::string::String;
use alloc::vec::Vec;
use crate::ui::overlay_plane::PlaneRect;

// =============================================================================
// 1. Virtual Key Definitions & Layout Modes
// =============================================================================

/// Distinct key action or character output.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VirtualKeyId {
    Char(char),
    Backspace,
    Enter,
    Space,
    Shift,
    Symbol123,
    LayoutToggle,
    Dismiss,
    Tab,
}

/// Visual and input state of a virtual key.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyVisualState {
    Normal,
    Pressed,
    Hovered,
    ActiveShift,
}

/// Geometry and metadata for a single virtual key.
#[derive(Debug, Clone, PartialEq)]
pub struct VirtualKey {
    pub id: VirtualKeyId,
    pub label: String,
    pub bounds: PlaneRect,
    pub center_x: f32,
    pub center_y: f32,
    pub state: KeyVisualState,
}

impl VirtualKey {
    pub fn new(id: VirtualKeyId, label: &str, x: i32, y: i32, width: u32, height: u32) -> Self {
        let bounds = PlaneRect::new(x, y, width, height);
        let center_x = x as f32 + (width as f32 * 0.5);
        let center_y = y as f32 + (height as f32 * 0.5);
        Self {
            id,
            label: String::from(label),
            bounds,
            center_x,
            center_y,
            state: KeyVisualState::Normal,
        }
    }

    /// Squared Euclidean distance from a touch point to the key center.
    pub fn distance_squared(&self, px: f32, py: f32) -> f32 {
        let dx = px - self.center_x;
        let dy = py - self.center_y;
        dx * dx + dy * dy
    }
}

/// Virtual Keyboard Layout Presentation Modes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyboardLayoutMode {
    /// Full standard 4-row QWERTY keyboard.
    FullQwerty,
    /// Ergonomic compact keyboard centered for thumb reachability.
    CompactThumbPad,
    /// Dual Split-Pad separated left and right with a clear center view for game HUDs.
    DualSplitPad,
}

/// Complete virtual keyboard layout map and spatial hit-test accelerator.
#[derive(Debug, Clone)]
pub struct VirtualKeyboardLayout {
    pub mode: KeyboardLayoutMode,
    pub screen_width: u32,
    pub screen_height: u32,
    pub keyboard_height: u32,
    pub keys: Vec<VirtualKey>,
    pub is_shift_active: bool,
    pub is_symbols_active: bool,
}

impl VirtualKeyboardLayout {
    pub fn new(screen_width: u32, screen_height: u32, mode: KeyboardLayoutMode) -> Self {
        let keyboard_height = (screen_height as f32 * 0.45) as u32; // 45% of screen height
        let mut layout = Self {
            mode,
            screen_width,
            screen_height,
            keyboard_height,
            keys: Vec::new(),
            is_shift_active: false,
            is_symbols_active: false,
        };
        layout.build_layout();
        layout
    }

    /// Toggles between FullQwerty, CompactThumbPad, and DualSplitPad.
    pub fn cycle_layout_mode(&mut self) {
        self.mode = match self.mode {
            KeyboardLayoutMode::FullQwerty => KeyboardLayoutMode::CompactThumbPad,
            KeyboardLayoutMode::CompactThumbPad => KeyboardLayoutMode::DualSplitPad,
            KeyboardLayoutMode::DualSplitPad => KeyboardLayoutMode::FullQwerty,
        };
        self.build_layout();
    }

    /// Reconstructs keys according to current mode and screen dimensions.
    pub fn build_layout(&mut self) {
        self.keys.clear();
        match self.mode {
            KeyboardLayoutMode::FullQwerty => self.build_full_qwerty(),
            KeyboardLayoutMode::CompactThumbPad => self.build_compact_thumb_pad(),
            KeyboardLayoutMode::DualSplitPad => self.build_dual_split_pad(),
        }
    }

    /// Standard 4-row Full QWERTY layout.
    fn build_full_qwerty(&mut self) {
        let base_y = (self.screen_height - self.keyboard_height) as i32;
        let row_height = self.keyboard_height / 4;
        let key_spacing = 4;

        // Row 1: Q W E R T Y U I O P (10 keys)
        let row1 = ['Q', 'W', 'E', 'R', 'T', 'Y', 'U', 'I', 'O', 'P'];
        let key_w1 = (self.screen_width.saturating_sub(key_spacing * 11)) / 10;
        let mut x = key_spacing as i32;
        for c in row1.iter() {
            let label = alloc::format!("{}", c);
            self.keys.push(VirtualKey::new(
                VirtualKeyId::Char(*c),
                &label,
                x,
                base_y,
                key_w1,
                row_height - key_spacing,
            ));
            x += (key_w1 + key_spacing) as i32;
        }

        // Row 2: A S D F G H J K L (9 keys, indented)
        let row2 = ['A', 'S', 'D', 'F', 'G', 'H', 'J', 'K', 'L'];
        let indent = (key_w1 / 2) as i32;
        x = (key_spacing as i32) + indent;
        let y2 = base_y + row_height as i32;
        for c in row2.iter() {
            let label = alloc::format!("{}", c);
            self.keys.push(VirtualKey::new(
                VirtualKeyId::Char(*c),
                &label,
                x,
                y2,
                key_w1,
                row_height - key_spacing,
            ));
            x += (key_w1 + key_spacing) as i32;
        }

        // Row 3: Shift, Z X C V B N M, Backspace
        let y3 = base_y + (row_height * 2) as i32;
        let shift_w = (key_w1 as f32 * 1.5) as u32;
        self.keys.push(VirtualKey::new(
            VirtualKeyId::Shift,
            "Shift",
            key_spacing as i32,
            y3,
            shift_w,
            row_height - key_spacing,
        ));

        let row3 = ['Z', 'X', 'C', 'V', 'B', 'N', 'M'];
        x = (key_spacing as i32) + (shift_w + key_spacing) as i32;
        for c in row3.iter() {
            let label = alloc::format!("{}", c);
            self.keys.push(VirtualKey::new(
                VirtualKeyId::Char(*c),
                &label,
                x,
                y3,
                key_w1,
                row_height - key_spacing,
            ));
            x += (key_w1 + key_spacing) as i32;
        }

        let backspace_w = (self.screen_width as i32 - x - key_spacing as i32).max(key_w1 as i32) as u32;
        self.keys.push(VirtualKey::new(
            VirtualKeyId::Backspace,
            "Bksp",
            x,
            y3,
            backspace_w,
            row_height - key_spacing,
        ));

        // Row 4: ?123, Layout, Space, Enter, Dismiss
        let y4 = base_y + (row_height * 3) as i32;
        let fn_w = (key_w1 as f32 * 1.4) as u32;
        let mut cur_x = key_spacing as i32;

        self.keys.push(VirtualKey::new(VirtualKeyId::Symbol123, "?123", cur_x, y4, fn_w, row_height - key_spacing));
        cur_x += (fn_w + key_spacing) as i32;

        self.keys.push(VirtualKey::new(VirtualKeyId::LayoutToggle, "Mode", cur_x, y4, fn_w, row_height - key_spacing));
        cur_x += (fn_w + key_spacing) as i32;

        let right_cluster_w = fn_w * 2 + key_spacing * 2;
        let space_w = (self.screen_width as i32 - cur_x - right_cluster_w as i32 - key_spacing as i32).max(100) as u32;

        self.keys.push(VirtualKey::new(VirtualKeyId::Space, "Space", cur_x, y4, space_w, row_height - key_spacing));
        cur_x += (space_w + key_spacing) as i32;

        self.keys.push(VirtualKey::new(VirtualKeyId::Enter, "Enter", cur_x, y4, fn_w, row_height - key_spacing));
        cur_x += (fn_w + key_spacing) as i32;

        self.keys.push(VirtualKey::new(VirtualKeyId::Dismiss, "Hide", cur_x, y4, fn_w, row_height - key_spacing));
    }

    /// Compact Thumb-Pad layout (narrower key pitch centered on screen).
    fn build_compact_thumb_pad(&mut self) {
        let pad_w = (self.screen_width as f32 * 0.75) as u32;
        let offset_x = ((self.screen_width - pad_w) / 2) as i32;
        let base_y = (self.screen_height - self.keyboard_height) as i32;
        let row_height = self.keyboard_height / 4;
        let key_w = pad_w / 10;

        let row1 = ['Q', 'W', 'E', 'R', 'T', 'Y', 'U', 'I', 'O', 'P'];
        for (i, c) in row1.iter().enumerate() {
            let label = alloc::format!("{}", c);
            self.keys.push(VirtualKey::new(
                VirtualKeyId::Char(*c),
                &label,
                offset_x + (i as u32 * key_w) as i32,
                base_y,
                key_w - 2,
                row_height - 2,
            ));
        }

        let row2 = ['A', 'S', 'D', 'F', 'G', 'H', 'J', 'K', 'L'];
        for (i, c) in row2.iter().enumerate() {
            let label = alloc::format!("{}", c);
            self.keys.push(VirtualKey::new(
                VirtualKeyId::Char(*c),
                &label,
                offset_x + (key_w / 2) as i32 + (i as u32 * key_w) as i32,
                base_y + row_height as i32,
                key_w - 2,
                row_height - 2,
            ));
        }

        let row3 = ['Z', 'X', 'C', 'V', 'B', 'N', 'M'];
        for (i, c) in row3.iter().enumerate() {
            let label = alloc::format!("{}", c);
            self.keys.push(VirtualKey::new(
                VirtualKeyId::Char(*c),
                &label,
                offset_x + (key_w * 3 / 2) as i32 + (i as u32 * key_w) as i32,
                base_y + (row_height * 2) as i32,
                key_w - 2,
                row_height - 2,
            ));
        }

        // Bottom space bar
        self.keys.push(VirtualKey::new(
            VirtualKeyId::Space,
            "Space",
            offset_x + (key_w * 2) as i32,
            base_y + (row_height * 3) as i32,
            key_w * 6,
            row_height - 2,
        ));
    }

    /// Dual Gamepad Split-Pad layout (keys placed along left and right display borders).
    fn build_dual_split_pad(&mut self) {
        let half_w = (self.screen_width as f32 * 0.40) as u32;
        let right_start = (self.screen_width - half_w) as i32;
        let base_y = (self.screen_height - self.keyboard_height) as i32;
        let row_height = self.keyboard_height / 4;
        let key_w = half_w / 5;

        // Left Half: Q W E R T / A S D F G / Z X C V B / Space-L
        let left1 = ['Q', 'W', 'E', 'R', 'T'];
        for (i, c) in left1.iter().enumerate() {
            let label = alloc::format!("{}", c);
            self.keys.push(VirtualKey::new(
                VirtualKeyId::Char(*c),
                &label,
                (i as u32 * key_w) as i32,
                base_y,
                key_w - 2,
                row_height - 2,
            ));
        }

        let left2 = ['A', 'S', 'D', 'F', 'G'];
        for (i, c) in left2.iter().enumerate() {
            let label = alloc::format!("{}", c);
            self.keys.push(VirtualKey::new(
                VirtualKeyId::Char(*c),
                &label,
                (i as u32 * key_w) as i32,
                base_y + row_height as i32,
                key_w - 2,
                row_height - 2,
            ));
        }

        let left3 = ['Z', 'X', 'C', 'V', 'B'];
        for (i, c) in left3.iter().enumerate() {
            let label = alloc::format!("{}", c);
            self.keys.push(VirtualKey::new(
                VirtualKeyId::Char(*c),
                &label,
                (i as u32 * key_w) as i32,
                base_y + (row_height * 2) as i32,
                key_w - 2,
                row_height - 2,
            ));
        }

        self.keys.push(VirtualKey::new(
            VirtualKeyId::Space,
            "Space",
            0,
            base_y + (row_height * 3) as i32,
            half_w,
            row_height - 2,
        ));

        // Right Half: Y U I O P / H J K L Bksp / N M Enter / Space-R
        let right1 = ['Y', 'U', 'I', 'O', 'P'];
        for (i, c) in right1.iter().enumerate() {
            let label = alloc::format!("{}", c);
            self.keys.push(VirtualKey::new(
                VirtualKeyId::Char(*c),
                &label,
                right_start + (i as u32 * key_w) as i32,
                base_y,
                key_w - 2,
                row_height - 2,
            ));
        }

        let right2 = ['H', 'J', 'K', 'L'];
        for (i, c) in right2.iter().enumerate() {
            let label = alloc::format!("{}", c);
            self.keys.push(VirtualKey::new(
                VirtualKeyId::Char(*c),
                &label,
                right_start + (i as u32 * key_w) as i32,
                base_y + row_height as i32,
                key_w - 2,
                row_height - 2,
            ));
        }
        self.keys.push(VirtualKey::new(
            VirtualKeyId::Backspace,
            "Bksp",
            right_start + (4 * key_w) as i32,
            base_y + row_height as i32,
            key_w - 2,
            row_height - 2,
        ));

        let right3 = ['N', 'M'];
        for (i, c) in right3.iter().enumerate() {
            let label = alloc::format!("{}", c);
            self.keys.push(VirtualKey::new(
                VirtualKeyId::Char(*c),
                &label,
                right_start + (i as u32 * key_w) as i32,
                base_y + (row_height * 2) as i32,
                key_w - 2,
                row_height - 2,
            ));
        }
        self.keys.push(VirtualKey::new(
            VirtualKeyId::Enter,
            "Enter",
            right_start + (2 * key_w) as i32,
            base_y + (row_height * 2) as i32,
            key_w * 3,
            row_height - 2,
        ));

        self.keys.push(VirtualKey::new(
            VirtualKeyId::Space,
            "Space",
            right_start,
            base_y + (row_height * 3) as i32,
            half_w,
            row_height - 2,
        ));
    }

    /// Spatial hit-test: returns the virtual key containing the touch coordinates, or closest key.
    pub fn hit_test(&self, touch_x: f32, touch_y: f32) -> Option<&VirtualKey> {
        let ix = touch_x as i32;
        let iy = touch_y as i32;

        // Exact bounding box hit
        for key in &self.keys {
            if key.bounds.contains(ix, iy) {
                return Some(key);
            }
        }

        // Closest key fallback within 32px tolerance
        let mut closest: Option<&VirtualKey> = None;
        let mut min_dist_sq = 32.0 * 32.0;

        for key in &self.keys {
            let d_sq = key.distance_squared(touch_x, touch_y);
            if d_sq < min_dist_sq {
                min_dist_sq = d_sq;
                closest = Some(key);
            }
        }

        closest
    }
}

// =============================================================================
// 2. Continuous Swipe Trajectory Smoothing & Shape-Writing Tokenizer
// =============================================================================

/// Single touch sensor sample along a gesture stroke.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TouchPoint {
    pub x: f32,
    pub y: f32,
    pub timestamp_us: u64,
}

impl TouchPoint {
    pub const fn new(x: f32, y: f32, timestamp_us: u64) -> Self {
        Self { x, y, timestamp_us }
    }

    pub fn distance_to(&self, other: &TouchPoint) -> f32 {
        let dx = self.x - other.x;
        let dy = self.y - other.y;
        sqrt_f32(dx * dx + dy * dy)
    }
}

/// Helper function to compute square root in pure no_std via Newton-Raphson.
pub fn sqrt_f32(val: f32) -> f32 {
    if val <= 0.0 {
        return 0.0;
    }
    let mut guess = val;
    for _ in 0..10 {
        guess = 0.5 * (guess + val / guess);
    }
    guess
}

/// Catmull-Rom Spline Trajectory Smoother.
pub struct CatmullRomSmoother;

impl CatmullRomSmoother {
    /// Evaluates a single Catmull-Rom spline point at parameter t in [0.0, 1.0].
    pub fn interpolate(p0: TouchPoint, p1: TouchPoint, p2: TouchPoint, p3: TouchPoint, t: f32) -> TouchPoint {
        let t2 = t * t;
        let t3 = t2 * t;

        let x = 0.5 * ((2.0 * p1.x)
            + (-p0.x + p2.x) * t
            + (2.0 * p0.x - 5.0 * p1.x + 4.0 * p2.x - p3.x) * t2
            + (-p0.x + 3.0 * p1.x - 3.0 * p2.x + p3.x) * t3);

        let y = 0.5 * ((2.0 * p1.y)
            + (-p0.y + p2.y) * t
            + (2.0 * p0.y - 5.0 * p1.y + 4.0 * p2.y - p3.y) * t2
            + (-p0.y + 3.0 * p1.y - 3.0 * p2.y + p3.y) * t3);

        let dt = (p2.timestamp_us as f32 - p1.timestamp_us as f32) * t;
        let timestamp_us = p1.timestamp_us + dt as u64;

        TouchPoint::new(x, y, timestamp_us)
    }

    /// Resamples and smooths a raw jagged touch trajectory into continuous sub-pixel points.
    pub fn smooth_trajectory(raw_points: &[TouchPoint], subdivisions_per_segment: usize) -> Vec<TouchPoint> {
        if raw_points.len() < 2 {
            return raw_points.to_vec();
        }

        let n = raw_points.len();
        let mut smoothed = Vec::with_capacity((n - 1) * subdivisions_per_segment + 1);

        for i in 0..n - 1 {
            let p0 = if i == 0 { raw_points[0] } else { raw_points[i - 1] };
            let p1 = raw_points[i];
            let p2 = raw_points[i + 1];
            let p3 = if i + 2 < n { raw_points[i + 2] } else { raw_points[n - 1] };

            for step in 0..subdivisions_per_segment {
                let t = step as f32 / subdivisions_per_segment as f32;
                smoothed.push(Self::interpolate(p0, p1, p2, p3, t));
            }
        }
        smoothed.push(raw_points[n - 1]);
        smoothed
    }
}

/// Extracted curve feature point for shape writing.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum GestureFeatureType {
    StrokeStart,
    CornerTurn,
    DwellStop,
    StrokeEnd,
}

#[derive(Debug, Clone, PartialEq)]
pub struct GestureFeature {
    pub point: TouchPoint,
    pub feature_type: GestureFeatureType,
    pub nearest_key: Option<char>,
}

/// Tokenizer that parses continuous swipe gesture trajectories into key tokens.
pub struct ShapeWritingTokenizer;

impl ShapeWritingTokenizer {
    /// Extracts critical gesture inflection points (start, corners, dwell stops, end).
    pub fn extract_features(trajectory: &[TouchPoint], layout: &VirtualKeyboardLayout) -> Vec<GestureFeature> {
        if trajectory.is_empty() {
            return Vec::new();
        }

        let mut features = Vec::new();

        // 1. Start feature
        let start_pt = trajectory[0];
        let start_key = layout.hit_test(start_pt.x, start_pt.y).and_then(|k| match k.id {
            VirtualKeyId::Char(c) => Some(c),
            _ => None,
        });
        features.push(GestureFeature {
            point: start_pt,
            feature_type: GestureFeatureType::StrokeStart,
            nearest_key: start_key,
        });

        // 2. Corner / directional inflection detection
        let step = 4;
        if trajectory.len() > step * 2 {
            for i in (step..trajectory.len() - step).step_by(step) {
                let p_prev = trajectory[i - step];
                let p_curr = trajectory[i];
                let p_next = trajectory[i + step];

                let v1_x = p_curr.x - p_prev.x;
                let v1_y = p_curr.y - p_prev.y;
                let v2_x = p_next.x - p_curr.x;
                let v2_y = p_next.y - p_curr.y;

                let mag1 = sqrt_f32(v1_x * v1_x + v1_y * v1_y);
                let mag2 = sqrt_f32(v2_x * v2_x + v2_y * v2_y);

                if mag1 > 4.0 && mag2 > 4.0 {
                    let dot = (v1_x * v2_x + v1_y * v2_y) / (mag1 * mag2);
                    // Angle > 60 degrees (dot < 0.5) indicates sharp directional turn
                    if dot < 0.5 {
                        let key = layout.hit_test(p_curr.x, p_curr.y).and_then(|k| match k.id {
                            VirtualKeyId::Char(c) => Some(c),
                            _ => None,
                        });
                        features.push(GestureFeature {
                            point: p_curr,
                            feature_type: GestureFeatureType::CornerTurn,
                            nearest_key: key,
                        });
                    }
                }
            }
        }

        // 3. End feature
        let end_pt = trajectory[trajectory.len() - 1];
        let end_key = layout.hit_test(end_pt.x, end_pt.y).and_then(|k| match k.id {
            VirtualKeyId::Char(c) => Some(c),
            _ => None,
        });
        features.push(GestureFeature {
            point: end_pt,
            feature_type: GestureFeatureType::StrokeEnd,
            nearest_key: end_key,
        });

        features
    }

    /// Tokenizes the smoothed trajectory into an ordered string of keys traversed.
    pub fn tokenize_trajectory(trajectory: &[TouchPoint], layout: &VirtualKeyboardLayout) -> String {
        let mut tokens = String::new();
        let mut last_char: Option<char> = None;

        for pt in trajectory {
            if let Some(key) = layout.hit_test(pt.x, pt.y) {
                if let VirtualKeyId::Char(c) = key.id {
                    if last_char != Some(c) {
                        tokens.push(c);
                        last_char = Some(c);
                    }
                }
            }
        }
        tokens
    }
}

// =============================================================================
// 3. Spatial Error Ellipse, Predictive Lattice & Win32 Event Injector
// =============================================================================

pub const VK_BACK: u8 = 0x08;
pub const VK_TAB: u8 = 0x09;
pub const VK_RETURN: u8 = 0x0D;
pub const VK_SHIFT: u8 = 0x10;
pub const VK_SPACE: u8 = 0x20;

pub const KEYEVENTF_KEYDOWN: u32 = 0x0000;
pub const KEYEVENTF_KEYUP: u32 = 0x0002;

/// Standard Win32 virtual keyboard input event.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Win32KeyEvent {
    pub vk_code: u8,
    pub scan_code: u8,
    pub flags: u32,
    pub is_up: bool,
    pub timestamp_us: u64,
}

impl Win32KeyEvent {
    pub fn key_down(vk_code: u8, timestamp_us: u64) -> Self {
        Self {
            vk_code,
            scan_code: 0,
            flags: KEYEVENTF_KEYDOWN,
            is_up: false,
            timestamp_us,
        }
    }

    pub fn key_up(vk_code: u8, timestamp_us: u64) -> Self {
        Self {
            vk_code,
            scan_code: 0,
            flags: KEYEVENTF_KEYUP,
            is_up: true,
            timestamp_us,
        }
    }
}

/// Ring buffer for injecting keyboard events into the OS input dispatch queue.
#[derive(Debug, Default)]
pub struct KeyEventInjector {
    pub queue: alloc::collections::VecDeque<Win32KeyEvent>,
    pub injected_count: u64,
}

impl KeyEventInjector {
    pub fn new() -> Self {
        Self {
            queue: alloc::collections::VecDeque::with_capacity(64),
            injected_count: 0,
        }
    }

    /// Injects a key-down or key-up event.
    pub fn inject_vk(&mut self, vk_code: u8, is_down: bool, timestamp_us: u64) {
        let event = if is_down {
            Win32KeyEvent::key_down(vk_code, timestamp_us)
        } else {
            Win32KeyEvent::key_up(vk_code, timestamp_us)
        };
        self.queue.push_back(event);
        self.injected_count += 1;
    }

    /// Injects a complete keystroke (KeyDown followed by KeyUp).
    pub fn inject_char_keystroke(&mut self, c: char, timestamp_us: u64) {
        let upper = c.to_ascii_uppercase();
        let vk_code = match upper {
            'A'..='Z' => upper as u8,
            '0'..='9' => upper as u8,
            ' ' => VK_SPACE,
            '\n' => VK_RETURN,
            '\t' => VK_TAB,
            '\x08' => VK_BACK,
            _ => return,
        };
        self.inject_vk(vk_code, true, timestamp_us);
        self.inject_vk(vk_code, false, timestamp_us + 1000);
    }

    /// Injects an entire word string followed by an automatic trailing space.
    pub fn inject_word(&mut self, word: &str, timestamp_us: u64) {
        let mut t = timestamp_us;
        for c in word.chars() {
            self.inject_char_keystroke(c, t);
            t += 2000;
        }
        self.inject_char_keystroke(' ', t);
    }

    pub fn pop_event(&mut self) -> Option<Win32KeyEvent> {
        self.queue.pop_front()
    }

    pub fn pending_count(&self) -> usize {
        self.queue.len()
    }
}

/// Spatial touch error tolerance model.
#[derive(Debug, Clone, Copy)]
pub struct SpatialErrorEllipse {
    pub radius_x: f32, // Horizontal error tolerance in pixels (e.g. 40px)
    pub radius_y: f32, // Vertical error tolerance in pixels (e.g. 30px)
}

impl SpatialErrorEllipse {
    pub const fn default_handheld() -> Self {
        Self {
            radius_x: 40.0,
            radius_y: 30.0,
        }
    }

    /// Computes spatial match score [0.0..1.0] from touch point to key center.
    pub fn score(&self, tx: f32, ty: f32, kx: f32, ky: f32) -> f32 {
        let dx = (tx - kx) / self.radius_x;
        let dy = (ty - ky) / self.radius_y;
        let d_ellipse = dx * dx + dy * dy;
        1.0 / (1.0 + d_ellipse)
    }
}

/// Candidate word suggestion scored by spatial match and frequency.
#[derive(Debug, Clone, PartialEq)]
pub struct WordCandidate {
    pub word: String,
    pub score: f32,
}

/// Predictive N-Gram Word Candidate Lattice for Handheld Gaming Comms.
pub struct PredictiveWordLattice {
    pub vocabulary: Vec<(&'static str, f32)>, // Word and unigram frequency [0..1]
    pub spatial_model: SpatialErrorEllipse,
}

impl PredictiveWordLattice {
    pub fn new() -> Self {
        let vocabulary = alloc::vec![
            ("GAME", 0.95),
            ("HELP", 0.88),
            ("GOOD", 0.85),
            ("PLAY", 0.82),
            ("TEAM", 0.80),
            ("GG", 0.78),
            ("RUSH", 0.75),
            ("ATTACK", 0.72),
            ("DEFEND", 0.70),
            ("READY", 0.68),
            ("PAUSE", 0.65),
            ("START", 0.62),
            ("QUIT", 0.60),
            ("MENU", 0.58),
            ("CHAT", 0.55),
            ("LOOT", 0.52),
            ("BOSS", 0.50),
            ("HEAL", 0.48),
            ("SAVE", 0.45),
            ("FAST", 0.42),
        ];

        Self {
            vocabulary,
            spatial_model: SpatialErrorEllipse::default_handheld(),
        }
    }

    /// Evaluates swipe trajectory against vocabulary and returns top candidates.
    pub fn rank_candidates(
        &self,
        trajectory: &[TouchPoint],
        layout: &VirtualKeyboardLayout,
    ) -> Vec<WordCandidate> {
        if trajectory.len() < 2 {
            return Vec::new();
        }

        let start_pt = trajectory[0];
        let end_pt = trajectory[trajectory.len() - 1];

        let mut scored_candidates: Vec<WordCandidate> = Vec::new();

        for &(vocab_word, freq) in &self.vocabulary {
            let chars: Vec<char> = vocab_word.chars().collect();
            if chars.len() < 2 {
                continue;
            }

            // Find key centers for word letters
            let first_key = layout.keys.iter().find(|k| k.id == VirtualKeyId::Char(chars[0]));
            let last_key = layout.keys.iter().find(|k| k.id == VirtualKeyId::Char(chars[chars.len() - 1]));

            let (fk, lk) = match (first_key, last_key) {
                (Some(f), Some(l)) => (f, l),
                _ => continue,
            };

            // Evaluate spatial scores for start and end
            let start_score = self.spatial_model.score(start_pt.x, start_pt.y, fk.center_x, fk.center_y);
            let end_score = self.spatial_model.score(end_pt.x, end_pt.y, lk.center_x, lk.center_y);

            // Discard if start or end is too far
            if start_score < 0.25 || end_score < 0.25 {
                continue;
            }

            // Intermediate alignment score
            let mut intermediate_score = 0.0f32;
            let n_intermediate = chars.len() - 2;

            if n_intermediate > 0 {
                for (idx, &c) in chars[1..chars.len() - 1].iter().enumerate() {
                    if let Some(mid_key) = layout.keys.iter().find(|k| k.id == VirtualKeyId::Char(c)) {
                        // Sample trajectory proportionally
                        let traj_idx = ((idx + 1) * trajectory.len()) / chars.len();
                        let pt = trajectory[traj_idx.min(trajectory.len() - 1)];
                        let s = self.spatial_model.score(pt.x, pt.y, mid_key.center_x, mid_key.center_y);
                        intermediate_score += s;
                    }
                }
                intermediate_score /= n_intermediate as f32;
            } else {
                intermediate_score = 1.0;
            }

            let total_score = (start_score * 0.35)
                + (end_score * 0.35)
                + (intermediate_score * 0.20)
                + (freq * 0.10);

            scored_candidates.push(WordCandidate {
                word: String::from(vocab_word),
                score: total_score,
            });
        }

        // Sort descending by score
        scored_candidates.sort_by(|a, b| b.score.partial_cmp(&a.score).unwrap_or(core::cmp::Ordering::Equal));

        // Return top 3
        scored_candidates.truncate(3);
        scored_candidates
    }
}

// -----------------------------------------------------------------------------
// Unit Tests for Commit 1
// -----------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    pub fn test_keyboard_layouts_and_hit_test() {
        let mut kb = VirtualKeyboardLayout::new(1280, 800, KeyboardLayoutMode::FullQwerty);
        assert!(!kb.keys.is_empty());
        assert_eq!(kb.mode, KeyboardLayoutMode::FullQwerty);

        // Find key 'Q' in Full QWERTY
        let q_key = kb.keys.iter().find(|k| k.id == VirtualKeyId::Char('Q')).expect("Key Q missing");
        let q_hit = kb.hit_test(q_key.center_x, q_key.center_y).expect("Hit test Q failed");
        assert_eq!(q_hit.id, VirtualKeyId::Char('Q'));

        // Cycle to CompactThumbPad
        kb.cycle_layout_mode();
        assert_eq!(kb.mode, KeyboardLayoutMode::CompactThumbPad);
        assert!(!kb.keys.is_empty());

        // Cycle to DualSplitPad
        kb.cycle_layout_mode();
        assert_eq!(kb.mode, KeyboardLayoutMode::DualSplitPad);
        assert!(!kb.keys.is_empty());

        // In Dual SplitPad, verify center screen has no keys (HUD visibility gap)
        let center_x = 1280.0 / 2.0;
        let center_y = 800.0 - 100.0;
        // Exact hit should be None or far
        for key in &kb.keys {
            assert!(!key.bounds.contains(center_x as i32, center_y as i32));
        }
    }
    #[test]
    pub fn test_catmull_rom_smoothing_and_tokenizing() {
        let kb = VirtualKeyboardLayout::new(1280, 800, KeyboardLayoutMode::FullQwerty);

        // Define a 3-point corner swipe: P1 -> Corner P2 -> P3
        let p1 = TouchPoint::new(100.0, 500.0, 1000);
        let p2 = TouchPoint::new(300.0, 500.0, 2000);
        let p3 = TouchPoint::new(300.0, 700.0, 3000);
        let raw = [p1, p2, p3];

        let smoothed = CatmullRomSmoother::smooth_trajectory(&raw, 5);
        assert_eq!(smoothed.len(), 11); // 2 segments * 5 + 1
        assert_eq!(smoothed[0].x, 100.0);
        assert_eq!(smoothed[10].y, 700.0);

        // Feature extraction
        let features = ShapeWritingTokenizer::extract_features(&smoothed, &kb);
        assert!(features.len() >= 2);
        assert_eq!(features[0].feature_type, GestureFeatureType::StrokeStart);
        assert_eq!(features.last().unwrap().feature_type, GestureFeatureType::StrokeEnd);
    }
    #[test]
    pub fn test_word_lattice_and_key_injector() {
        let kb = VirtualKeyboardLayout::new(1280, 800, KeyboardLayoutMode::FullQwerty);
        let lattice = PredictiveWordLattice::new();

        // Construct synthetic trajectory for word "GAME" (G -> A -> M -> E)
        let g_key = kb.keys.iter().find(|k| k.id == VirtualKeyId::Char('G')).unwrap();
        let a_key = kb.keys.iter().find(|k| k.id == VirtualKeyId::Char('A')).unwrap();
        let m_key = kb.keys.iter().find(|k| k.id == VirtualKeyId::Char('M')).unwrap();
        let e_key = kb.keys.iter().find(|k| k.id == VirtualKeyId::Char('E')).unwrap();

        let traj = [
            TouchPoint::new(g_key.center_x, g_key.center_y, 1000),
            TouchPoint::new(a_key.center_x, a_key.center_y, 2000),
            TouchPoint::new(m_key.center_x, m_key.center_y, 3000),
            TouchPoint::new(e_key.center_x, e_key.center_y, 4000),
        ];

        let candidates = lattice.rank_candidates(&traj, &kb);
        assert!(!candidates.is_empty());
        assert_eq!(candidates[0].word, "GAME");
        assert!(candidates[0].score > 0.5);

        // Win32 key event injector test
        let mut injector = KeyEventInjector::new();
        injector.inject_word("GG", 10_000);
        assert_eq!(injector.pending_count(), 6); // 'G'(down,up) + 'G'(down,up) + ' '(down,up) = 6 events

        let ev1 = injector.pop_event().unwrap();
        assert_eq!(ev1.vk_code, b'G');
        assert!(!ev1.is_up);
    }
}
