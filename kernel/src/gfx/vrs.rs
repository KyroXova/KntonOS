//! Direct3D 12 Variable Rate Shading (VRS Tier 2) Subsystem.
//!
//! Implements hardware-accelerated Variable Rate Shading (VRS) Tier 2 specifications:
//! - Discrete shading rates (1x1, 1x2, 2x1, 2x2, 2x4, 4x2, 4x4)
//! - Combiner modes (Passthrough, Override, Min, Max, Sum)
//! - Screen-space Shading Rate Image (SRI) tile map generation and sampling (8x8, 16x16, 32x32)
//! - Per-draw command list shading rate configuration and multi-source rate resolution.

/// Direct3D 12 discrete coarse pixel shading rates.
///
/// Follows D3D12_SHADING_RATE enumeration bitmasks where the lower 2 bits represent
/// vertical coarse sizing and bits [3:2] represent horizontal coarse sizing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[repr(u8)]
pub enum ShadingRate {
    /// 1x1: Full shading rate (1 pixel shader invocation per pixel).
    Rate1x1 = 0x0,
    /// 1x2: 1 horizontal pixel, 2 vertical pixels per invocation.
    Rate1x2 = 0x1,
    /// 2x1: 2 horizontal pixels, 1 vertical pixel per invocation.
    Rate2x1 = 0x4,
    /// 2x2: 2x2 coarse pixel (1 invocation per 4 pixels).
    Rate2x2 = 0x5,
    /// 2x4: 2 horizontal pixels, 4 vertical pixels (1 invocation per 8 pixels).
    Rate2x4 = 0x6,
    /// 4x2: 4 horizontal pixels, 2 vertical pixels (1 invocation per 8 pixels).
    Rate4x2 = 0x9,
    /// 4x4: 4x4 coarse pixel (1 invocation per 16 pixels).
    Rate4x4 = 0xa,
}

impl ShadingRate {
    /// Returns the horizontal pixel dimension (1, 2, or 4).
    #[inline]
    pub const fn width(self) -> u8 {
        match self {
            Self::Rate1x1 | Self::Rate1x2 => 1,
            Self::Rate2x1 | Self::Rate2x2 | Self::Rate2x4 => 2,
            Self::Rate4x2 | Self::Rate4x4 => 4,
        }
    }

    /// Returns the vertical pixel dimension (1, 2, or 4).
    #[inline]
    pub const fn height(self) -> u8 {
        match self {
            Self::Rate1x1 | Self::Rate2x1 => 1,
            Self::Rate1x2 | Self::Rate2x2 | Self::Rate4x2 => 2,
            Self::Rate2x4 | Self::Rate4x4 => 4,
        }
    }

    /// Returns the coarse pixel area (width * height), i.e., number of pixels per shader sample.
    #[inline]
    pub const fn pixel_area(self) -> u8 {
        self.width() * self.height()
    }

    /// Creates a `ShadingRate` from given dimensions, returning None if dimensions are invalid.
    pub const fn from_dimensions(width: u8, height: u8) -> Option<Self> {
        match (width, height) {
            (1, 1) => Some(Self::Rate1x1),
            (1, 2) => Some(Self::Rate1x2),
            (2, 1) => Some(Self::Rate2x1),
            (2, 2) => Some(Self::Rate2x2),
            (2, 4) => Some(Self::Rate2x4),
            (4, 2) => Some(Self::Rate4x2),
            (4, 4) => Some(Self::Rate4x4),
            _ => None,
        }
    }

    /// Encodes to D3D12 raw 8-bit shading rate byte value.
    #[inline]
    pub const fn to_d3d12(self) -> u8 {
        self as u8
    }

    /// Decodes from D3D12 raw 8-bit shading rate byte value.
    pub const fn from_d3d12(val: u8) -> Option<Self> {
        match val {
            0x0 => Some(Self::Rate1x1),
            0x1 => Some(Self::Rate1x2),
            0x4 => Some(Self::Rate2x1),
            0x5 => Some(Self::Rate2x2),
            0x6 => Some(Self::Rate2x4),
            0x9 => Some(Self::Rate4x2),
            0xa => Some(Self::Rate4x4),
            _ => None,
        }
    }

    /// Descriptive name.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Rate1x1 => "1x1 (Full)",
            Self::Rate1x2 => "1x2",
            Self::Rate2x1 => "2x1",
            Self::Rate2x2 => "2x2",
            Self::Rate2x4 => "2x4",
            Self::Rate4x2 => "4x2",
            Self::Rate4x4 => "4x4 (Maximum Coarseness)",
        }
    }
}

/// Direct3D 12 coarse pixel shading rate combiner mode (`D3D12_SHADING_RATE_COMBINER`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum ShadingRateCombiner {
    /// Passthrough: The incoming rate A is kept unchanged; rate B is ignored.
    Passthrough = 0,
    /// Override: The incoming rate B completely replaces rate A.
    Override = 1,
    /// Min: Chooses the finer rate (min width and min height).
    Min = 2,
    /// Max: Chooses the coarser rate (max width and max height).
    Max = 3,
    /// Sum: Combines the coarse rates additively (dimensions multiplied, saturating at 4x4).
    Sum = 4,
}

/// Hardware Variable Rate Shading tier capabilities.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
#[repr(u8)]
pub enum VrsTier {
    /// Variable rate shading not supported.
    NotSupported = 0,
    /// Tier 1: Per-draw command list shading rate only.
    Tier1 = 1,
    /// Tier 2: Per-draw, screen-space Shading Rate Image (SRI) tile map, and per-primitive rates.
    Tier2 = 2,
}

/// Shading Rate Image (SRI) tile dimension granularity in pixels.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u32)]
pub enum TileGranularity {
    /// 8x8 pixels per SRI tile.
    Tile8x8 = 8,
    /// 16x16 pixels per SRI tile.
    Tile16x16 = 16,
    /// 32x32 pixels per SRI tile.
    Tile32x32 = 32,
}

impl TileGranularity {
    /// Returns tile size in pixels.
    #[inline]
    pub const fn pixel_size(self) -> u32 {
        self as u32
    }
}

/// Evaluates two shading rates using a given combiner rule.
pub fn combine_shading_rates(
    rate_a: ShadingRate,
    rate_b: ShadingRate,
    combiner: ShadingRateCombiner,
) -> ShadingRate {
    match combiner {
        ShadingRateCombiner::Passthrough => rate_a,
        ShadingRateCombiner::Override => rate_b,
        ShadingRateCombiner::Min => {
            let w = core::cmp::min(rate_a.width(), rate_b.width());
            let h = core::cmp::min(rate_a.height(), rate_b.height());
            ShadingRate::from_dimensions(w, h).unwrap_or(ShadingRate::Rate1x1)
        }
        ShadingRateCombiner::Max => {
            let w = core::cmp::max(rate_a.width(), rate_b.width());
            let h = core::cmp::max(rate_a.height(), rate_b.height());
            ShadingRate::from_dimensions(w, h).unwrap_or(ShadingRate::Rate4x4)
        }
        ShadingRateCombiner::Sum => {
            // In D3D12, Sum multiplies coarse axes (e.g., 2x1 combined with 1x2 yields 2x2),
            // clamping at maximum 4 on each axis.
            let w = core::cmp::min(4, rate_a.width() * rate_b.width());
            let h = core::cmp::min(4, rate_a.height() * rate_b.height());
            // Round to valid power of two (1, 2, or 4)
            let w_valid = if w >= 4 { 4 } else if w >= 2 { 2 } else { 1 };
            let h_valid = if h >= 4 { 4 } else if h >= 2 { 2 } else { 1 };
            ShadingRate::from_dimensions(w_valid, h_valid).unwrap_or(ShadingRate::Rate4x4)
        }
    }
}

/// Per-draw command list shading rate state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PerDrawShadingState {
    /// Command list base shading rate.
    pub base_rate: ShadingRate,
    /// Combiner 0: Combines base_rate and screen-space SRI tile map.
    pub combiner_draw_vs_image: ShadingRateCombiner,
    /// Combiner 1: Combines the previous result with per-primitive SV_ShadingRate.
    pub combiner_image_vs_primitive: ShadingRateCombiner,
}

impl Default for PerDrawShadingState {
    fn default() -> Self {
        Self {
            base_rate: ShadingRate::Rate1x1,
            combiner_draw_vs_image: ShadingRateCombiner::Passthrough,
            combiner_image_vs_primitive: ShadingRateCombiner::Passthrough,
        }
    }
}

impl PerDrawShadingState {
    /// Creates a new per-draw shading state with specified base rate and combiners.
    pub const fn new(
        base_rate: ShadingRate,
        combiner_draw_vs_image: ShadingRateCombiner,
        combiner_image_vs_primitive: ShadingRateCombiner,
    ) -> Self {
        Self {
            base_rate,
            combiner_draw_vs_image,
            combiner_image_vs_primitive,
        }
    }

    /// Sets the command list shading rate and combiners (`RSSetShadingRate`).
    pub fn set_shading_rate(
        &mut self,
        rate: ShadingRate,
        combiners: [ShadingRateCombiner; 2],
    ) {
        self.base_rate = rate;
        self.combiner_draw_vs_image = combiners[0];
        self.combiner_image_vs_primitive = combiners[1];
    }
}
