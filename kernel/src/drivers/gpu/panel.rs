//! DMI / ACPI Chassis and Display Panel Orientation Detection for Handheld Mobile Panels.
//!
//! Provides automatic detection and compensation for native portrait mobile display panels
//! commonly mounted in PC gaming handhelds (Steam Deck, ASUS ROG Ally, Lenovo Legion Go,
//! GPD Win, AYANEO, OneXPlayer) and convertibles.

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

// -----------------------------------------------------------------------------
// String Helper Functions for no_std DMI Matching
// -----------------------------------------------------------------------------

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
