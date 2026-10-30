//! DMI and ACPI chassis and display panel orientation detection.

use super::drm::{
    DRM_MODE_ROTATE_0, DRM_MODE_ROTATE_90, DRM_MODE_ROTATE_180, DRM_MODE_ROTATE_270,
};

/// Hardware Display Panel Native Physical Orientation relative to device chassis.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[repr(u8)]
pub enum PanelOrientation {
    /// Upright / standard orientation (0 degrees).
    #[default]
    Normal = 0,
    /// Upside down (180 degrees rotation).
    BottomUp = 1,
    /// Native portrait panel mounted rotated clockwise (requires 90 deg rotation for upright).
    RightUp = 2,
    /// Native portrait panel mounted rotated counter-clockwise (requires 270 deg rotation for upright).
    LeftUp = 3,
}

impl PanelOrientation {
    /// Returns rotation angle in degrees clockwise.
    #[inline(always)]
    pub const fn to_degrees(&self) -> u32 {
        match self {
            PanelOrientation::Normal => 0,
            PanelOrientation::BottomUp => 180,
            PanelOrientation::RightUp => 90,
            PanelOrientation::LeftUp => 270,
        }
    }

    /// Converts integer degrees (0, 90, 180, 270) to `PanelOrientation`.
    pub const fn from_degrees(deg: u32) -> Option<Self> {
        match deg % 360 {
            0 => Some(PanelOrientation::Normal),
            90 => Some(PanelOrientation::RightUp),
            180 => Some(PanelOrientation::BottomUp),
            270 => Some(PanelOrientation::LeftUp),
            _ => None,
        }
    }

    /// Converts to DRM KMS rotation bitmask (`DRM_MODE_ROTATE_*`).
    #[inline(always)]
    pub const fn to_drm_rotation(&self) -> u32 {
        match self {
            PanelOrientation::Normal => DRM_MODE_ROTATE_0,
            PanelOrientation::BottomUp => DRM_MODE_ROTATE_180,
            PanelOrientation::RightUp => DRM_MODE_ROTATE_90,
            PanelOrientation::LeftUp => DRM_MODE_ROTATE_270,
        }
    }

    /// Returns `true` if this orientation swaps horizontal width and vertical height dimensions.
    #[inline(always)]
    pub const fn is_swapped_dimensions(&self) -> bool {
        matches!(self, PanelOrientation::RightUp | PanelOrientation::LeftUp)
    }
}

/// SMBIOS / DMI Chassis Type enumeration.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ChassisType {
    #[default]
    Unknown = 0,
    Desktop = 0x03,
    Laptop = 0x0A,
    Tablet = 0x1E,
    Convertible = 0x1F,
    Handheld = 0x20, // Common extension for PC gaming handhelds
}

/// System identity information queried from SMBIOS / DMI tables.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DmiSystemInfo {
    pub sys_vendor: &'static str,
    pub product_name: &'static str,
    pub product_version: &'static str,
    pub board_vendor: &'static str,
    pub board_name: &'static str,
    pub chassis_type: ChassisType,
}

impl Default for DmiSystemInfo {
    fn default() -> Self {
        Self {
            sys_vendor: "Generic",
            product_name: "PC",
            product_version: "1.0",
            board_vendor: "Generic",
            board_name: "Motherboard",
            chassis_type: ChassisType::Unknown,
        }
    }
}

/// Quirk database entry for handhelds with portrait mobile display panels.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HandheldPanelQuirk {
    pub sys_vendor_match: &'static str,
    pub product_name_match: &'static str,
    pub native_width: u32,
    pub native_height: u32,
    pub native_orientation: PanelOrientation,
    pub panel_name: &'static str,
}

/// Handheld Device Quirks Database for Native Portrait Gaming Panels.
pub static HANDHELD_PANEL_QUIRKS: &[HandheldPanelQuirk] = &[
    // Valve Steam Deck LCD (800x1280 60Hz MIPI/eDP portrait panel)
    HandheldPanelQuirk {
        sys_vendor_match: "Valve",
        product_name_match: "Jupiter",
        native_width: 800,
        native_height: 1280,
        native_orientation: PanelOrientation::RightUp,
        panel_name: "Steam Deck LCD 7\" 800x1280",
    },
    // Valve Steam Deck OLED (800x1280 90Hz HDR portrait panel)
    HandheldPanelQuirk {
        sys_vendor_match: "Valve",
        product_name_match: "Galileo",
        native_width: 800,
        native_height: 1280,
        native_orientation: PanelOrientation::RightUp,
        panel_name: "Steam Deck OLED 7.4\" 800x1280 90Hz",
    },
    // ASUS ROG Ally (1080x1920 120Hz FreeSync portrait panel)
    HandheldPanelQuirk {
        sys_vendor_match: "ASUSTeK COMPUTER INC.",
        product_name_match: "RC71L",
        native_width: 1080,
        native_height: 1920,
        native_orientation: PanelOrientation::RightUp,
        panel_name: "ASUS ROG Ally 7\" 1080x1920 120Hz",
    },
    // ASUS ROG Ally X (1080x1920 120Hz FreeSync portrait panel)
    HandheldPanelQuirk {
        sys_vendor_match: "ASUSTeK COMPUTER INC.",
        product_name_match: "RC72LA",
        native_width: 1080,
        native_height: 1920,
        native_orientation: PanelOrientation::RightUp,
        panel_name: "ASUS ROG Ally X 7\" 1080x1920 120Hz",
    },
    // Lenovo Legion Go (1600x2560 144Hz 8.8\" portrait panel)
    HandheldPanelQuirk {
        sys_vendor_match: "LENOVO",
        product_name_match: "83E1",
        native_width: 1600,
        native_height: 2560,
        native_orientation: PanelOrientation::RightUp,
        panel_name: "Lenovo Legion Go 8.8\" 1600x2560 144Hz",
    },
    // GPD Win 4 (1080x1920 60Hz 6\" portrait panel)
    HandheldPanelQuirk {
        sys_vendor_match: "GPD",
        product_name_match: "G1618-04",
        native_width: 1080,
        native_height: 1920,
        native_orientation: PanelOrientation::RightUp,
        panel_name: "GPD Win 4 6\" 1080x1920",
    },
    // GPD Win Mini (1080x1920 120Hz 7\" portrait panel)
    HandheldPanelQuirk {
        sys_vendor_match: "GPD",
        product_name_match: "G1617-01",
        native_width: 1080,
        native_height: 1920,
        native_orientation: PanelOrientation::RightUp,
        panel_name: "GPD Win Mini 7\" 1080x1920 120Hz",
    },
    // AYANEO 2 (1200x1920 60Hz 7\" portrait panel)
    HandheldPanelQuirk {
        sys_vendor_match: "AYANEO",
        product_name_match: "AYANEO 2",
        native_width: 1200,
        native_height: 1920,
        native_orientation: PanelOrientation::RightUp,
        panel_name: "AYANEO 2 7\" 1200x1920",
    },
    // AYANEO GEEK (800x1280 60Hz 7\" portrait panel)
    HandheldPanelQuirk {
        sys_vendor_match: "AYANEO",
        product_name_match: "GEEK",
        native_width: 800,
        native_height: 1280,
        native_orientation: PanelOrientation::RightUp,
        panel_name: "AYANEO GEEK 7\" 800x1280",
    },
    // OneXPlayer Mini (1200x1920 60Hz 7\" portrait panel)
    HandheldPanelQuirk {
        sys_vendor_match: "One-Netbook",
        product_name_match: "ONEXPLAYER Mini",
        native_width: 1200,
        native_height: 1920,
        native_orientation: PanelOrientation::RightUp,
        panel_name: "OneXPlayer Mini 7\" 1200x1920",
    },
    // AOKZOE A1 (1200x1920 60Hz 8\" portrait panel)
    HandheldPanelQuirk {
        sys_vendor_match: "AOKZOE",
        product_name_match: "AOKZOE A1",
        native_width: 1200,
        native_height: 1920,
        native_orientation: PanelOrientation::RightUp,
        panel_name: "AOKZOE A1 8\" 1200x1920",
    },
];

/// Fully resolved display panel descriptor.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DisplayPanelDescriptor {
    pub name: &'static str,
    /// Physical width of the raw panel in pixels.
    pub physical_width: u32,
    /// Physical height of the raw panel in pixels.
    pub physical_height: u32,
    /// Logical width of the desktop / compositor framebuffer.
    pub logical_width: u32,
    /// Logical height of the desktop / compositor framebuffer.
    pub logical_height: u32,
    /// Required hardware scanout rotation to present an upright display.
    pub orientation: PanelOrientation,
    /// Panel refresh rate in Hz.
    pub refresh_rate_hz: u32,
    /// Whether the panel is physically portrait (height > width).
    pub is_portrait_native: bool,
}

impl DisplayPanelDescriptor {
    /// Creates a standard landscape panel descriptor (no rotation required).
    pub const fn new_landscape(name: &'static str, width: u32, height: u32, refresh_hz: u32) -> Self {
        Self {
            name,
            physical_width: width,
            physical_height: height,
            logical_width: width,
            logical_height: height,
            orientation: PanelOrientation::Normal,
            refresh_rate_hz: refresh_hz,
            is_portrait_native: false,
        }
    }

    /// Creates a rotated portrait panel descriptor.
    pub const fn new_rotated_portrait(
        name: &'static str,
        phys_width: u32,
        phys_height: u32,
        orientation: PanelOrientation,
        refresh_hz: u32,
    ) -> Self {
        let (log_w, log_h) = if orientation.is_swapped_dimensions() {
            (phys_height, phys_width)
        } else {
            (phys_width, phys_height)
        };

        Self {
            name,
            physical_width: phys_width,
            physical_height: phys_height,
            logical_width: log_w,
            logical_height: log_h,
            orientation,
            refresh_rate_hz: refresh_hz,
            is_portrait_native: phys_height > phys_width,
        }
    }
}

/// Display Panel Orientation Detector.
pub struct PanelDetector;

impl PanelDetector {
    /// Matches DMI system strings against the handheld panel quirks database.
    pub fn detect_from_dmi(dmi: &DmiSystemInfo) -> Option<DisplayPanelDescriptor> {
        for quirk in HANDHELD_PANEL_QUIRKS {
            if str_equals_ignore_case(dmi.sys_vendor, quirk.sys_vendor_match)
                && str_contains_ignore_case(dmi.product_name, quirk.product_name_match)
            {
                return Some(DisplayPanelDescriptor::new_rotated_portrait(
                    quirk.panel_name,
                    quirk.native_width,
                    quirk.native_height,
                    quirk.native_orientation,
                    60,
                ));
            }
        }
        None
    }

    /// Resolves display panel orientation given DMI info or fallback EDID dimensions.
    pub fn detect_or_fallback(
        dmi: Option<&DmiSystemInfo>,
        edid_width: u32,
        edid_height: u32,
        refresh_hz: u32,
    ) -> DisplayPanelDescriptor {
        if let Some(dmi_info) = dmi {
            if let Some(desc) = Self::detect_from_dmi(dmi_info) {
                return desc;
            }
        }

        // Check if raw panel EDID dimensions indicate native portrait (H > W)
        if edid_height > edid_width && edid_width > 0 {
            // Unidentified handheld / tablet portrait panel: standard 90 deg clockwise rotation
            DisplayPanelDescriptor::new_rotated_portrait(
                "Generic Portrait Mobile Panel",
                edid_width,
                edid_height,
                PanelOrientation::RightUp,
                refresh_hz,
            )
        } else {
            DisplayPanelDescriptor::new_landscape(
                "Standard Landscape Display",
                edid_width,
                edid_height,
                refresh_hz,
            )
        }
    }
}

// String helper functions for no_std DMI matching.

fn str_equals_ignore_case(a: &str, b: &str) -> bool {
    if a.len() != b.len() {
        return false;
    }
    a.bytes().zip(b.bytes()).all(|(x, y)| x.to_ascii_lowercase() == y.to_ascii_lowercase())
}

fn str_contains_ignore_case(haystack: &str, needle: &str) -> bool {
    let h_bytes = haystack.as_bytes();
    let n_bytes = needle.as_bytes();
    if n_bytes.is_empty() {
        return true;
    }
    if h_bytes.len() < n_bytes.len() {
        return false;
    }

    for i in 0..=(h_bytes.len() - n_bytes.len()) {
        let mut matched = true;
        for j in 0..n_bytes.len() {
            if h_bytes[i + j].to_ascii_lowercase() != n_bytes[j].to_ascii_lowercase() {
                matched = false;
                break;
            }
        }
        if matched {
            return true;
        }
    }
    false
}

// Touchscreen and digitizer coordinate normalization transform.

/// Touchscreen Coordinate Transformation Matrix (CTM) for digitizer normalization.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TouchCalibrationMatrix {
    pub c: [f32; 6],
}

impl Default for TouchCalibrationMatrix {
    fn default() -> Self {
        Self::IDENTITY
    }
}

impl TouchCalibrationMatrix {
    /// Identity calibration matrix (0 degrees rotation).
    pub const IDENTITY: Self = Self {
        c: [1.0, 0.0, 0.0, 0.0, 1.0, 0.0],
    };

    pub const fn new(c0: f32, c1: f32, c2: f32, c3: f32, c4: f32, c5: f32) -> Self {
        Self {
            c: [c0, c1, c2, c3, c4, c5],
        }
    }

    /// Constructs the touch CTM matching the hardware panel orientation.
    pub fn for_orientation(orientation: PanelOrientation) -> Self {
        match orientation {
            PanelOrientation::Normal => Self::IDENTITY,
            PanelOrientation::RightUp => {
                // 90 deg clockwise: X' = Y, Y' = 1.0 - X
                Self::new(0.0, 1.0, 0.0, -1.0, 0.0, 1.0)
            }
            PanelOrientation::BottomUp => {
                // 180 deg upside-down: X' = 1.0 - X, Y' = 1.0 - Y
                Self::new(-1.0, 0.0, 1.0, 0.0, -1.0, 1.0)
            }
            PanelOrientation::LeftUp => {
                // 270 deg counter-clockwise: X' = 1.0 - Y, Y' = X
                Self::new(0.0, -1.0, 1.0, 1.0, 0.0, 0.0)
            }
        }
    }

    /// Applies matrix transformation to normalized coordinates [0.0, 1.0].
    pub fn transform_normalized(&self, norm_x: f32, norm_y: f32) -> (f32, f32) {
        let x_out = self.c[0] * norm_x + self.c[1] * norm_y + self.c[2];
        let y_out = self.c[3] * norm_x + self.c[4] * norm_y + self.c[5];

        // Clamp to [0.0, 1.0] viewport boundary
        let clamped_x = if x_out < 0.0 { 0.0 } else if x_out > 1.0 { 1.0 } else { x_out };
        let clamped_y = if y_out < 0.0 { 0.0 } else if y_out > 1.0 { 1.0 } else { y_out };

        (clamped_x, clamped_y)
    }
}

/// Raw touch contact input received directly from I2C / USB HID digitizer hardware.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct RawTouchInput {
    /// Multi-touch contact / tracking ID (0..15).
    pub contact_id: u32,
    /// Raw X coordinate in digitizer units.
    pub raw_x: u32,
    /// Raw Y coordinate in digitizer units.
    pub raw_y: u32,
    /// Contact pressure (0..4095).
    pub pressure: u16,
    /// Whether contact is currently touching the surface.
    pub touch_down: bool,
}

/// Calibrated, orientation-transformed touch event ready for desktop compositor dispatch.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct TouchPoint {
    /// Multi-touch contact ID.
    pub contact_id: u32,
    /// Raw hardware X.
    pub raw_x: u32,
    /// Raw hardware Y.
    pub raw_y: u32,
    /// Normalized transformed X in range [0.0, 1.0].
    pub norm_x: f32,
    /// Normalized transformed Y in range [0.0, 1.0].
    pub norm_y: f32,
    /// Transformed desktop pixel X coordinate.
    pub screen_x: u32,
    /// Transformed desktop pixel Y coordinate.
    pub screen_y: u32,
    /// Contact pressure.
    pub pressure: u16,
    /// Touch contact active state.
    pub touch_down: bool,
}

/// Touchscreen Coordinate Normalization & Orientation Transform Pipeline.
pub struct TouchTransformPipeline {
    pub orientation: PanelOrientation,
    pub raw_max_x: u32,
    pub raw_max_y: u32,
    pub screen_w: u32,
    pub screen_h: u32,
    pub matrix: TouchCalibrationMatrix,
}

impl TouchTransformPipeline {
    pub fn new(
        orientation: PanelOrientation,
        raw_max_x: u32,
        raw_max_y: u32,
        screen_w: u32,
        screen_h: u32,
    ) -> Self {
        let matrix = TouchCalibrationMatrix::for_orientation(orientation);
        Self {
            orientation,
            raw_max_x: raw_max_x.max(1),
            raw_max_y: raw_max_y.max(1),
            screen_w: screen_w.max(1),
            screen_h: screen_h.max(1),
            matrix,
        }
    }

    /// Updates panel orientation and recomputes the calibration matrix.
    pub fn set_orientation(&mut self, orientation: PanelOrientation) {
        self.orientation = orientation;
        self.matrix = TouchCalibrationMatrix::for_orientation(orientation);
    }

    /// Transforms a raw digitizer input packet into calibrated desktop screen coordinates.
    pub fn transform(&self, input: &RawTouchInput) -> TouchPoint {
        // 1. Normalize raw hardware coordinates into [0.0, 1.0]
        let nx_raw = (input.raw_x as f32) / (self.raw_max_x as f32);
        let ny_raw = (input.raw_y as f32) / (self.raw_max_y as f32);

        // 2. Apply orientation transformation matrix
        let (nx_trans, ny_trans) = self.matrix.transform_normalized(nx_raw, ny_raw);

        // 3. Scale to desktop compositor pixel dimensions
        let max_screen_x = self.screen_w.saturating_sub(1) as f32;
        let max_screen_y = self.screen_h.saturating_sub(1) as f32;

        let sx = (nx_trans * max_screen_x + 0.5) as u32;
        let sy = (ny_trans * max_screen_y + 0.5) as u32;

        TouchPoint {
            contact_id: input.contact_id,
            raw_x: input.raw_x,
            raw_y: input.raw_y,
            norm_x: nx_trans,
            norm_y: ny_trans,
            screen_x: sx,
            screen_y: sy,
            pressure: input.pressure,
            touch_down: input.touch_down,
        }
    }
}