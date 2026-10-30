//! Direct3D 12 Variable Rate Shading (VRS Tier 2) Subsystem.
//!
//! Implements hardware-accelerated Variable Rate Shading (VRS) Tier 2 specifications:
//! - Discrete shading rates (1x1, 1x2, 2x1, 2x2, 2x4, 4x2, 4x4)
//! - Combiner modes (Passthrough, Override, Min, Max, Sum)
//! - Screen-space Shading Rate Image (SRI) tile map generation and sampling (8x8, 16x16, 32x32)
//! - Per-draw command list shading rate configuration and multi-source rate resolution.

use alloc::vec;
use alloc::vec::Vec;

/// Errors returned by the Variable Rate Shading subsystem.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VrsError {
    /// Screen dimensions or tile parameters are invalid (e.g. zero width or height).
    InvalidDimensions,
    /// Tile coordinate or screen pixel coordinate is out of bounds.
    OutOfBounds,
    /// Feature requested is not supported by the configured VRS tier.
    TierNotSupported,
    /// Shading Rate Image buffer length does not match tile dimensions.
    TileBufferMismatch,
}

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
            // In D3D12, Sum multiplies coarse axes, clamping at maximum 4 on each axis.
            let w = core::cmp::min(4, rate_a.width() * rate_b.width());
            let h = core::cmp::min(4, rate_a.height() * rate_b.height());
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

/// Screen-space Shading Rate Image (SRI) tile map surface.
///
/// Divides the screen into tiles of `TileGranularity` (e.g. 8x8, 16x16, 32x32 pixels),
/// each specifying the coarse shading rate for rasterization in that tile.
#[derive(Debug, Clone)]
pub struct ShadingRateImage {
    /// Viewport width in pixels.
    width_pixels: u32,
    /// Viewport height in pixels.
    height_pixels: u32,
    /// Tile granularity (8x8, 16x16, 32x32).
    granularity: TileGranularity,
    /// Number of horizontal tiles.
    tiles_x: u32,
    /// Number of vertical tiles.
    tiles_y: u32,
    /// Array of shading rates for each tile (row-major order: index = y * tiles_x + x).
    tile_rates: Vec<ShadingRate>,
}

impl ShadingRateImage {
    /// Creates a new Shading Rate Image initialized to a default shading rate.
    pub fn new(
        width_pixels: u32,
        height_pixels: u32,
        granularity: TileGranularity,
        default_rate: ShadingRate,
    ) -> Result<Self, VrsError> {
        if width_pixels == 0 || height_pixels == 0 {
            return Err(VrsError::InvalidDimensions);
        }

        let tile_size = granularity.pixel_size();
        let tiles_x = (width_pixels + tile_size - 1) / tile_size;
        let tiles_y = (height_pixels + tile_size - 1) / tile_size;
        let total_tiles = (tiles_x as usize)
            .checked_mul(tiles_y as usize)
            .ok_or(VrsError::InvalidDimensions)?;

        let tile_rates = vec![default_rate; total_tiles];

        Ok(Self {
            width_pixels,
            height_pixels,
            granularity,
            tiles_x,
            tiles_y,
            tile_rates,
        })
    }

    /// Creates a Shading Rate Image from existing tile data.
    pub fn from_rates(
        width_pixels: u32,
        height_pixels: u32,
        granularity: TileGranularity,
        rates: Vec<ShadingRate>,
    ) -> Result<Self, VrsError> {
        if width_pixels == 0 || height_pixels == 0 {
            return Err(VrsError::InvalidDimensions);
        }

        let tile_size = granularity.pixel_size();
        let tiles_x = (width_pixels + tile_size - 1) / tile_size;
        let tiles_y = (height_pixels + tile_size - 1) / tile_size;
        let total_tiles = (tiles_x as usize)
            .checked_mul(tiles_y as usize)
            .ok_or(VrsError::InvalidDimensions)?;

        if rates.len() != total_tiles {
            return Err(VrsError::TileBufferMismatch);
        }

        Ok(Self {
            width_pixels,
            height_pixels,
            granularity,
            tiles_x,
            tiles_y,
            tile_rates: rates,
        })
    }

    /// Viewport width in pixels.
    #[inline]
    pub fn width_pixels(&self) -> u32 {
        self.width_pixels
    }

    /// Viewport height in pixels.
    #[inline]
    pub fn height_pixels(&self) -> u32 {
        self.height_pixels
    }

    /// Tile granularity.
    #[inline]
    pub fn granularity(&self) -> TileGranularity {
        self.granularity
    }

    /// Number of horizontal tiles.
    #[inline]
    pub fn tiles_x(&self) -> u32 {
        self.tiles_x
    }

    /// Number of vertical tiles.
    #[inline]
    pub fn tiles_y(&self) -> u32 {
        self.tiles_y
    }

    /// Total number of tiles in the image.
    #[inline]
    pub fn total_tiles(&self) -> usize {
        self.tile_rates.len()
    }

    /// Retrieves the shading rate for a given tile coordinate `(tile_x, tile_y)`.
    pub fn get_tile_rate(&self, tile_x: u32, tile_y: u32) -> Option<ShadingRate> {
        if tile_x >= self.tiles_x || tile_y >= self.tiles_y {
            return None;
        }
        let index = (tile_y as usize) * (self.tiles_x as usize) + (tile_x as usize);
        self.tile_rates.get(index).copied()
    }

    /// Sets the shading rate for a given tile coordinate `(tile_x, tile_y)`.
    pub fn set_tile_rate(&mut self, tile_x: u32, tile_y: u32, rate: ShadingRate) -> bool {
        if tile_x >= self.tiles_x || tile_y >= self.tiles_y {
            return false;
        }
        let index = (tile_y as usize) * (self.tiles_x as usize) + (tile_x as usize);
        if let Some(slot) = self.tile_rates.get_mut(index) {
            *slot = rate;
            true
        } else {
            false
        }
    }

    /// Samples the shading rate image at a given screen-space pixel coordinate `(x, y)`.
    pub fn sample_pixel(&self, x: u32, y: u32) -> ShadingRate {
        let tile_size = self.granularity.pixel_size();
        let tile_x = core::cmp::min(x / tile_size, self.tiles_x.saturating_sub(1));
        let tile_y = core::cmp::min(y / tile_size, self.tiles_y.saturating_sub(1));
        self.get_tile_rate(tile_x, tile_y).unwrap_or(ShadingRate::Rate1x1)
    }

    /// Encodes all tile rates to raw D3D12 bytes (1 byte per tile, `R8_UINT` format).
    pub fn to_raw_d3d12_bytes(&self) -> Vec<u8> {
        self.tile_rates.iter().map(|&r| r.to_d3d12()).collect()
    }

    /// Fills the entire SRI tile map with a single uniform rate.
    pub fn fill(&mut self, rate: ShadingRate) {
        for slot in self.tile_rates.iter_mut() {
            *slot = rate;
        }
    }
}

/// Statistics and telemetry for VRS rasterization profiling.
#[derive(Debug, Clone, Copy, Default)]
pub struct VrsMetrics {
    /// Total screen pixels represented across all shaded fragments.
    pub total_pixels_rasterized: u64,
    /// Effective pixel shader invocations dispatched to hardware execution units.
    pub effective_shader_invocations: u64,
    /// Counter for 1x1 full shading fragments.
    pub count_1x1: u64,
    /// Counter for 1x2 coarse shading fragments.
    pub count_1x2: u64,
    /// Counter for 2x1 coarse shading fragments.
    pub count_2x1: u64,
    /// Counter for 2x2 coarse shading fragments.
    pub count_2x2: u64,
    /// Counter for 2x4 coarse shading fragments.
    pub count_2x4: u64,
    /// Counter for 4x2 coarse shading fragments.
    pub count_4x2: u64,
    /// Counter for 4x4 maximum coarse shading fragments.
    pub count_4x4: u64,
}

impl VrsMetrics {
    /// Calculates shader invocation reduction percentage (0 to 99%).
    ///
    /// For example, if total_pixels is 1,000,000 and effective_invocations is 450,000,
    /// the reduction is 55%.
    pub fn invocation_reduction_percentage(&self) -> u32 {
        if self.total_pixels_rasterized == 0 {
            return 0;
        }
        let invocations = self.effective_shader_invocations;
        let total = self.total_pixels_rasterized;
        if invocations >= total {
            0
        } else {
            let reduction = (total - invocations) * 100 / total;
            reduction as u32
        }
    }
}

/// Direct3D 12 Variable Rate Shading Tier 2 Engine.
///
/// Orchestrates per-draw command list shading rates, screen-space Shading Rate Image (SRI)
/// tile maps, multi-stage combiner state resolution, and rasterizer shader invocation metrics.
#[derive(Debug)]
pub struct VariableRateShadingManager {
    /// Supported VRS hardware tier.
    tier: VrsTier,
    /// Configured tile granularity (8x8, 16x16, 32x32).
    granularity: TileGranularity,
    /// Active per-draw shading state (command list).
    draw_state: PerDrawShadingState,
    /// Active bound screen-space Shading Rate Image (Tier 2).
    bound_sri: Option<ShadingRateImage>,
    /// Profiling telemetry and metrics.
    metrics: VrsMetrics,
}

impl VariableRateShadingManager {
    /// Creates a new VariableRateShadingManager with the specified tier and tile granularity.
    pub fn new(tier: VrsTier, granularity: TileGranularity) -> Self {
        Self {
            tier,
            granularity,
            draw_state: PerDrawShadingState::default(),
            bound_sri: None,
            metrics: VrsMetrics::default(),
        }
    }

    /// Supported VRS tier.
    #[inline]
    pub fn tier(&self) -> VrsTier {
        self.tier
    }

    /// Supported tile granularity.
    #[inline]
    pub fn granularity(&self) -> TileGranularity {
        self.granularity
    }

    /// Active per-draw shading state.
    #[inline]
    pub fn draw_state(&self) -> &PerDrawShadingState {
        &self.draw_state
    }

    /// Sets the per-draw shading rate and combiners (`RSSetShadingRate`).
    pub fn set_shading_rate(
        &mut self,
        rate: ShadingRate,
        combiners: [ShadingRateCombiner; 2],
    ) {
        self.draw_state.set_shading_rate(rate, combiners);
    }

    /// Binds a screen-space Shading Rate Image (SRI) for Tier 2 rasterization.
    pub fn bind_shading_rate_image(&mut self, image: ShadingRateImage) -> Result<(), VrsError> {
        if self.tier < VrsTier::Tier2 {
            return Err(VrsError::TierNotSupported);
        }
        self.bound_sri = Some(image);
        Ok(())
    }

    /// Unbinds the current Shading Rate Image.
    pub fn unbind_shading_rate_image(&mut self) {
        self.bound_sri = None;
    }

    /// Reference to the currently bound SRI, if any.
    pub fn bound_sri(&self) -> Option<&ShadingRateImage> {
        self.bound_sri.as_ref()
    }

    /// Resolves the final effective coarse shading rate for a given screen pixel coordinate `(x, y)`,
    /// taking into account the per-draw base rate, the screen-space SRI, combiners, and optional
    /// per-primitive SV_ShadingRate.
    pub fn resolve_shading_rate(
        &self,
        screen_x: u32,
        screen_y: u32,
        primitive_rate: Option<ShadingRate>,
    ) -> ShadingRate {
        let base_rate = self.draw_state.base_rate;

        // Stage 1: Combine per-draw base rate with screen-space SRI tile map rate
        let stage1_rate = if let Some(sri) = &self.bound_sri {
            if self.tier >= VrsTier::Tier2 {
                let image_rate = sri.sample_pixel(screen_x, screen_y);
                combine_shading_rates(base_rate, image_rate, self.draw_state.combiner_draw_vs_image)
            } else {
                base_rate
            }
        } else {
            base_rate
        };

        // Stage 2: Combine stage 1 result with per-primitive rate (if provided)
        if let Some(prim_rate) = primitive_rate {
            if self.tier >= VrsTier::Tier2 {
                combine_shading_rates(
                    stage1_rate,
                    prim_rate,
                    self.draw_state.combiner_image_vs_primitive,
                )
            } else {
                stage1_rate
            }
        } else {
            stage1_rate
        }
    }

    /// Records rasterization of a coarse pixel fragment into profiling metrics.
    pub fn record_rasterization(&mut self, rate: ShadingRate) {
        self.metrics.total_pixels_rasterized += rate.pixel_area() as u64;
        self.metrics.effective_shader_invocations += 1;
        match rate {
            ShadingRate::Rate1x1 => self.metrics.count_1x1 += 1,
            ShadingRate::Rate1x2 => self.metrics.count_1x2 += 1,
            ShadingRate::Rate2x1 => self.metrics.count_2x1 += 1,
            ShadingRate::Rate2x2 => self.metrics.count_2x2 += 1,
            ShadingRate::Rate2x4 => self.metrics.count_2x4 += 1,
            ShadingRate::Rate4x2 => self.metrics.count_4x2 += 1,
            ShadingRate::Rate4x4 => self.metrics.count_4x4 += 1,
        }
    }

    /// Active telemetry metrics.
    #[inline]
    pub fn metrics(&self) -> &VrsMetrics {
        &self.metrics
    }

    /// Resets telemetry metrics.
    pub fn reset_metrics(&mut self) {
        self.metrics = VrsMetrics::default();
    }
}

// -----------------------------------------------------------------------------
// Direct3D 12 VRS Tier 2 Screen-Space Shading Rate Image (SRI) Descriptors
// -----------------------------------------------------------------------------

/// Raw D3D12 8-bit Shading Rate tile encoding constants.
pub const D3D12_VRS_SHADING_RATE_1X1: u8 = 0x0;
pub const D3D12_VRS_SHADING_RATE_1X2: u8 = 0x1;
pub const D3D12_VRS_SHADING_RATE_2X1: u8 = 0x4;
pub const D3D12_VRS_SHADING_RATE_2X2: u8 = 0x5;
pub const D3D12_VRS_SHADING_RATE_2X4: u8 = 0x6;
pub const D3D12_VRS_SHADING_RATE_4X2: u8 = 0x9;
pub const D3D12_VRS_SHADING_RATE_4X4: u8 = 0xa;

/// Screen-space 2D motion velocity vector for motion-adaptive shading rate calculation.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct MotionVector2D {
    /// Horizontal velocity in pixels per frame.
    pub vx: f32,
    /// Vertical velocity in pixels per frame.
    pub vy: f32,
}

impl MotionVector2D {
    /// Creates a new 2D motion vector.
    pub const fn new(vx: f32, vy: f32) -> Self {
        Self { vx, vy }
    }

    /// Squared magnitude of the motion vector.
    #[inline(always)]
    pub fn magnitude_sq(&self) -> f32 {
        self.vx * self.vx + self.vy * self.vy
    }

    /// Euclidean magnitude (speed in pixels per frame).
    #[inline(always)]
    pub fn magnitude(&self) -> f32 {
        let sq = self.magnitude_sq();
        if sq <= 0.0 {
            0.0
        } else {
            // Fast square root approximation
            let i = sq.to_bits();
            let i = 0x1fbd1df5 + (i >> 1);
            let mut x = f32::from_bits(i);
            x = 0.5 * (x + sq / x);
            x = 0.5 * (x + sq / x);
            x
        }
    }
}

/// Luminance statistics for a single screen-space tile used in contrast-adaptive VRS.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct LumaTile {
    /// Minimum luma value in `[0.0, 1.0]` across the tile.
    pub min_luma: f32,
    /// Maximum luma value in `[0.0, 1.0]` across the tile.
    pub max_luma: f32,
    /// Average luma value in `[0.0, 1.0]` across the tile.
    pub avg_luma: f32,
}

impl LumaTile {
    /// Creates a new luma tile descriptor.
    pub const fn new(min_luma: f32, max_luma: f32, avg_luma: f32) -> Self {
        Self { min_luma, max_luma, avg_luma }
    }

    /// Calculates normalized Michelson contrast: `(max - min) / (max + min + eps)`.
    pub fn contrast(&self) -> f32 {
        let diff = self.max_luma - self.min_luma;
        let sum = self.max_luma + self.min_luma;
        if sum < 1e-4 {
            0.0
        } else {
            diff / (sum + 1e-4)
        }
    }
}

/// Metadata and analysis record for an individual Shading Rate Image (SRI) tile.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SriTileDescriptor {
    /// Horizontal tile coordinate (column index).
    pub tile_x: u32,
    /// Vertical tile coordinate (row index).
    pub tile_y: u32,
    /// Coarse shading rate assigned to this tile.
    pub rate: ShadingRate,
    /// Local Michelson contrast metric `[0.0, 1.0]`.
    pub contrast_metric: f32,
    /// Local screen-space motion velocity magnitude in pixels/frame.
    pub motion_magnitude: f32,
    /// Radial distance from view center or gaze point in normalized screen units.
    pub foveal_distance: f32,
    /// Whether this tile contains high-frequency geometry/luma edges protected with 1x1.
    pub is_edge_protected: bool,
}

/// Configuration descriptor for generating a Shading Rate Image surface.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ShadingRateImageDescriptor {
    /// Viewport width in pixels.
    pub width_pixels: u32,
    /// Viewport height in pixels.
    pub height_pixels: u32,
    /// Tile granularity (8x8, 16x16, or 32x32 pixels).
    pub granularity: TileGranularity,
    /// Default shading rate applied to neutral tiles.
    pub default_rate: ShadingRate,
    /// Minimum allowed shading rate floor (e.g. Rate1x1).
    pub min_rate: ShadingRate,
    /// Maximum allowed coarse shading rate ceiling (e.g. Rate4x4).
    pub max_rate: ShadingRate,
}

/// Multi-source shading rate combiner configuration descriptor according to D3D12 Tier 2 specs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MultiSourceCombinerDescriptor {
    /// Stage 0: Combines per-draw pipeline base rate with screen-space SRI tile map.
    pub draw_vs_image: ShadingRateCombiner,
    /// Stage 1: Combines Stage 0 result with per-primitive SV_ShadingRate.
    pub image_vs_primitive: ShadingRateCombiner,
}

impl Default for MultiSourceCombinerDescriptor {
    fn default() -> Self {
        Self {
            draw_vs_image: ShadingRateCombiner::Passthrough,
            image_vs_primitive: ShadingRateCombiner::Passthrough,
        }
    }
}

impl MultiSourceCombinerDescriptor {
    /// Creates a new combiner descriptor with specified Stage 0 and Stage 1 rules.
    pub const fn new(
        draw_vs_image: ShadingRateCombiner,
        image_vs_primitive: ShadingRateCombiner,
    ) -> Self {
        Self {
            draw_vs_image,
            image_vs_primitive,
        }
    }

    /// Evaluates multi-source shading rates across both combiner stages.
    pub fn combine(
        &self,
        base_rate: ShadingRate,
        image_rate: ShadingRate,
        primitive_rate: Option<ShadingRate>,
    ) -> ShadingRate {
        let stage0 = combine_shading_rates(base_rate, image_rate, self.draw_vs_image);
        if let Some(prim) = primitive_rate {
            combine_shading_rates(stage0, prim, self.image_vs_primitive)
        } else {
            stage0
        }
    }
}

/// Tuning parameters for contrast-adaptive and motion-vector VRS tile evaluation.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ContrastSensitivityConfig {
    /// Contrast threshold above which full 1x1 shading is strictly enforced to protect geometry edges.
    pub edge_threshold: f32,
    /// Moderate contrast threshold below which 2x2 or 2x1/1x2 coarse shading is permitted.
    pub moderate_contrast_threshold: f32,
    /// Screen-space velocity magnitude (pixels/frame) above which motion blur allows 4x4 or 2x4/4x2 coarsening.
    pub motion_coarsening_threshold: f32,
    /// Whether directional motion stretching (e.g. 4x2 vs 2x4) is aligned to velocity axis.
    pub enable_directional_motion_shading: bool,
}

impl Default for ContrastSensitivityConfig {
    fn default() -> Self {
        Self {
            edge_threshold: 0.18,
            moderate_contrast_threshold: 0.08,
            motion_coarsening_threshold: 2.5,
            enable_directional_motion_shading: true,
        }
    }
}

// -----------------------------------------------------------------------------
// Contrast-Adaptive & Motion-Vector Shading Rate Image Generator
// -----------------------------------------------------------------------------

/// Direct3D 12 Contrast-Adaptive and Motion-Vector VRS Shading Rate Image Generator.
///
/// Implements dual-stream perceptual heuristics:
/// 1. Luminance Michelson Contrast: Identifies high-frequency geometry/specular edges
///    and pins them to `Rate1x1` to prevent aliasing.
/// 2. Motion Velocity Analysis: Identifies fast camera or object movement and coarsens
///    shading up to `Rate4x4` (or directional `Rate4x2`/`Rate2x4`) under motion blur.
#[derive(Debug, Clone)]
pub struct ContrastAdaptiveVrsGenerator {
    /// Configuration thresholds.
    pub config: ContrastSensitivityConfig,
}

impl ContrastAdaptiveVrsGenerator {
    /// Creates a new generator with the provided sensitivity configuration.
    pub const fn new(config: ContrastSensitivityConfig) -> Self {
        Self { config }
    }

    /// Evaluates the coarse shading rate based solely on tile luminance contrast.
    pub fn evaluate_contrast_rate(&self, luma: &LumaTile) -> (ShadingRate, bool) {
        let contrast = luma.contrast();

        if contrast >= self.config.edge_threshold {
            // High-contrast edge: full shading rate required to prevent geometric aliasing
            (ShadingRate::Rate1x1, true)
        } else if contrast >= self.config.moderate_contrast_threshold {
            // Moderate contrast: 2x2 coarse pixel shading
            (ShadingRate::Rate2x2, false)
        } else {
            // Low-contrast smooth region: maximum coarse pixel shading (4x4)
            (ShadingRate::Rate4x4, false)
        }
    }

    /// Evaluates the coarse shading rate based on screen-space motion velocity.
    pub fn evaluate_motion_rate(&self, motion: &MotionVector2D) -> ShadingRate {
        let speed = motion.magnitude();
        let thresh = self.config.motion_coarsening_threshold;

        if speed < thresh {
            // Low velocity: fine shading rate
            ShadingRate::Rate1x1
        } else if self.config.enable_directional_motion_shading {
            let ax = motion.vx.abs();
            let ay = motion.vy.abs();

            if ax >= 2.0 * ay {
                // Dominant horizontal motion: coarsen horizontally
                if speed >= 2.0 * thresh {
                    ShadingRate::Rate4x2
                } else {
                    ShadingRate::Rate2x1
                }
            } else if ay >= 2.0 * ax {
                // Dominant vertical motion: coarsen vertically
                if speed >= 2.0 * thresh {
                    ShadingRate::Rate2x4
                } else {
                    ShadingRate::Rate1x2
                }
            } else {
                // Isotropic motion
                if speed >= 2.0 * thresh {
                    ShadingRate::Rate4x4
                } else {
                    ShadingRate::Rate2x2
                }
            }
        } else {
            if speed >= 2.0 * thresh {
                ShadingRate::Rate4x4
            } else {
                ShadingRate::Rate2x2
            }
        }
    }

    /// Evaluates combined shading rate for a single tile.
    pub fn evaluate_tile(
        &self,
        tile_x: u32,
        tile_y: u32,
        luma: &LumaTile,
        motion: &MotionVector2D,
    ) -> SriTileDescriptor {
        let (contrast_rate, is_edge) = self.evaluate_contrast_rate(luma);
        let motion_rate = self.evaluate_motion_rate(motion);

        let final_rate = if is_edge {
            // Edge protection overrides motion coarsening unless motion is extremely rapid
            if motion.magnitude() >= self.config.motion_coarsening_threshold * 3.0 {
                combine_shading_rates(contrast_rate, motion_rate, ShadingRateCombiner::Min)
            } else {
                ShadingRate::Rate1x1
            }
        } else {
            // Non-edge region: coarsen based on maximum coarseness between contrast and motion
            combine_shading_rates(contrast_rate, motion_rate, ShadingRateCombiner::Max)
        };

        SriTileDescriptor {
            tile_x,
            tile_y,
            rate: final_rate,
            contrast_metric: luma.contrast(),
            motion_magnitude: motion.magnitude(),
            foveal_distance: 0.0,
            is_edge_protected: is_edge,
        }
    }

    /// Generates a full screen-space Shading Rate Image from input luma and motion vector grids.
    pub fn generate_sri(
        &self,
        desc: ShadingRateImageDescriptor,
        luma_grid: &[LumaTile],
        motion_grid: &[MotionVector2D],
    ) -> Result<(ShadingRateImage, Vec<SriTileDescriptor>), VrsError> {
        let tile_size = desc.granularity.pixel_size();
        let tiles_x = (desc.width_pixels + tile_size - 1) / tile_size;
        let tiles_y = (desc.height_pixels + tile_size - 1) / tile_size;
        let total_tiles = (tiles_x as usize) * (tiles_y as usize);

        if luma_grid.len() != total_tiles || motion_grid.len() != total_tiles {
            return Err(VrsError::TileBufferMismatch);
        }

        let mut descriptors = Vec::with_capacity(total_tiles);
        let mut rates = Vec::with_capacity(total_tiles);

        for y in 0..tiles_y {
            for x in 0..tiles_x {
                let idx = (y as usize) * (tiles_x as usize) + (x as usize);
                let tile_desc = self.evaluate_tile(x, y, &luma_grid[idx], &motion_grid[idx]);

                // Clamp to allowed range
                let clamped_rate = combine_shading_rates(tile_desc.rate, desc.min_rate, ShadingRateCombiner::Max);
                let clamped_rate = combine_shading_rates(clamped_rate, desc.max_rate, ShadingRateCombiner::Min);

                rates.push(clamped_rate);
                descriptors.push(SriTileDescriptor {
                    rate: clamped_rate,
                    ..tile_desc
                });
            }
        }

        let sri = ShadingRateImage::from_rates(desc.width_pixels, desc.height_pixels, desc.granularity, rates)?;
        Ok((sri, descriptors))
    }
}

// -----------------------------------------------------------------------------
// Foveated Peripheral Shading Rate Generator & Multi-Source Combiner Engine
// -----------------------------------------------------------------------------

/// Configuration for concentric radial foveated peripheral shading.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FoveatedVrsConfig {
    /// Screen-space gaze / view-center X coordinate in pixels.
    pub gaze_x: u32,
    /// Screen-space gaze / view-center Y coordinate in pixels.
    pub gaze_y: u32,
    /// Inner foveal radius in pixels: full 1x1 shading rate inside this disc.
    pub inner_radius: f32,
    /// Middle parafoveal radius in pixels: 2x2 coarse shading rate between inner and middle.
    pub middle_radius: f32,
    /// Outer peripheral coarse rate beyond middle radius (e.g. Rate4x4).
    pub peripheral_rate: ShadingRate,
}

impl FoveatedVrsConfig {
    /// Constructs default centered foveated config for a given viewport.
    pub fn default_for_viewport(width: u32, height: u32) -> Self {
        let cx = width / 2;
        let cy = height / 2;
        let min_dim = if width < height { width as f32 } else { height as f32 };

        Self {
            gaze_x: cx,
            gaze_y: cy,
            inner_radius: min_dim * 0.25,
            middle_radius: min_dim * 0.50,
            peripheral_rate: ShadingRate::Rate4x4,
        }
    }
}

/// Foveated Peripheral VRS Generator using concentric radial distance rings.
#[derive(Debug, Clone)]
pub struct FoveatedVrsGenerator {
    /// Active foveated configuration.
    pub config: FoveatedVrsConfig,
}

impl FoveatedVrsGenerator {
    /// Creates a new foveated generator with specified radial thresholds.
    pub const fn new(config: FoveatedVrsConfig) -> Self {
        Self { config }
    }

    /// Evaluates the foveated peripheral shading rate at pixel coordinate `(px, py)`.
    pub fn evaluate_pixel(&self, px: f32, py: f32) -> (ShadingRate, f32) {
        let dx = px - (self.config.gaze_x as f32);
        let dy = py - (self.config.gaze_y as f32);
        let dist_sq = dx * dx + dy * dy;

        let inner_sq = self.config.inner_radius * self.config.inner_radius;
        let middle_sq = self.config.middle_radius * self.config.middle_radius;

        // Fast square root approximation
        let dist = if dist_sq <= 0.0 {
            0.0
        } else {
            let i = dist_sq.to_bits();
            let i = 0x1fbd1df5 + (i >> 1);
            let mut x = f32::from_bits(i);
            x = 0.5 * (x + dist_sq / x);
            x = 0.5 * (x + dist_sq / x);
            x
        };

        let rate = if dist_sq <= inner_sq {
            ShadingRate::Rate1x1
        } else if dist_sq <= middle_sq {
            ShadingRate::Rate2x2
        } else {
            self.config.peripheral_rate
        };

        (rate, dist)
    }

    /// Evaluates the foveated peripheral shading rate for an entire tile.
    pub fn evaluate_tile(
        &self,
        tile_x: u32,
        tile_y: u32,
        granularity: TileGranularity,
    ) -> (ShadingRate, f32) {
        let tile_size = granularity.pixel_size() as f32;
        let center_x = (tile_x as f32) * tile_size + (tile_size * 0.5);
        let center_y = (tile_y as f32) * tile_size + (tile_size * 0.5);

        self.evaluate_pixel(center_x, center_y)
    }

    /// Generates a purely foveated Shading Rate Image surface.
    pub fn generate_sri(&self, desc: ShadingRateImageDescriptor) -> Result<ShadingRateImage, VrsError> {
        let tile_size = desc.granularity.pixel_size();
        let tiles_x = (desc.width_pixels + tile_size - 1) / tile_size;
        let tiles_y = (desc.height_pixels + tile_size - 1) / tile_size;
        let total_tiles = (tiles_x as usize) * (tiles_y as usize);

        let mut rates = Vec::with_capacity(total_tiles);

        for y in 0..tiles_y {
            for x in 0..tiles_x {
                let (rate, _) = self.evaluate_tile(x, y, desc.granularity);
                let clamped_rate = combine_shading_rates(rate, desc.min_rate, ShadingRateCombiner::Max);
                let clamped_rate = combine_shading_rates(clamped_rate, desc.max_rate, ShadingRateCombiner::Min);
                rates.push(clamped_rate);
            }
        }

        ShadingRateImage::from_rates(desc.width_pixels, desc.height_pixels, desc.granularity, rates)
    }
}

/// Unified D3D12 VRS Tier 2 Engine combining contrast, motion, and foveated rate sources.
#[derive(Debug, Clone)]
pub struct UnifiedVrsEngine {
    /// Contrast and motion generator.
    pub contrast_motion_gen: ContrastAdaptiveVrsGenerator,
    /// Foveated peripheral generator.
    pub foveated_gen: FoveatedVrsGenerator,
    /// Multi-source combiner rules.
    pub combiners: MultiSourceCombinerDescriptor,
}

impl UnifiedVrsEngine {
    /// Constructs a unified VRS engine.
    pub const fn new(
        contrast_motion_gen: ContrastAdaptiveVrsGenerator,
        foveated_gen: FoveatedVrsGenerator,
        combiners: MultiSourceCombinerDescriptor,
    ) -> Self {
        Self {
            contrast_motion_gen,
            foveated_gen,
            combiners,
        }
    }

    /// Generates a unified screen-space Shading Rate Image fusing contrast, motion, and foveation.
    pub fn generate_unified_sri(
        &self,
        desc: ShadingRateImageDescriptor,
        luma_grid: &[LumaTile],
        motion_grid: &[MotionVector2D],
    ) -> Result<(ShadingRateImage, Vec<SriTileDescriptor>), VrsError> {
        let tile_size = desc.granularity.pixel_size();
        let tiles_x = (desc.width_pixels + tile_size - 1) / tile_size;
        let tiles_y = (desc.height_pixels + tile_size - 1) / tile_size;
        let total_tiles = (tiles_x as usize) * (tiles_y as usize);

        if luma_grid.len() != total_tiles || motion_grid.len() != total_tiles {
            return Err(VrsError::TileBufferMismatch);
        }

        let mut descriptors = Vec::with_capacity(total_tiles);
        let mut final_rates = Vec::with_capacity(total_tiles);

        for y in 0..tiles_y {
            for x in 0..tiles_x {
                let idx = (y as usize) * (tiles_x as usize) + (x as usize);

                // 1. Evaluate contrast & motion
                let cm_desc = self.contrast_motion_gen.evaluate_tile(x, y, &luma_grid[idx], &motion_grid[idx]);

                // 2. Evaluate foveated periphery
                let (fov_rate, fov_dist) = self.foveated_gen.evaluate_tile(x, y, desc.granularity);

                // 3. Fuse image rates:
                // If the tile is an edge, protect it with Min combiner (so 1x1 wins unless peripheral is forced)
                let fused_image_rate = if cm_desc.is_edge_protected {
                    combine_shading_rates(cm_desc.rate, fov_rate, ShadingRateCombiner::Min)
                } else {
                    combine_shading_rates(cm_desc.rate, fov_rate, ShadingRateCombiner::Max)
                };

                // Clamp to descriptor bounds
                let clamped_rate = combine_shading_rates(fused_image_rate, desc.min_rate, ShadingRateCombiner::Max);
                let clamped_rate = combine_shading_rates(clamped_rate, desc.max_rate, ShadingRateCombiner::Min);

                final_rates.push(clamped_rate);
                descriptors.push(SriTileDescriptor {
                    tile_x: x,
                    tile_y: y,
                    rate: clamped_rate,
                    contrast_metric: cm_desc.contrast_metric,
                    motion_magnitude: cm_desc.motion_magnitude,
                    foveal_distance: fov_dist,
                    is_edge_protected: cm_desc.is_edge_protected,
                });
            }
        }

        let sri = ShadingRateImage::from_rates(desc.width_pixels, desc.height_pixels, desc.granularity, final_rates)?;
        Ok((sri, descriptors))
    }
}



