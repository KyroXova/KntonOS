//! Direct3D 12 Dynamic Depth Bounds Testing & Conservative Rasterization Engine.
//!
//! Provides hardware-accelerated early-Z cull optimization and gap-free conservative
//! rasterization for deferred shading light volumes, voxelization, and ray-traced shadows:
//! - Dynamic Depth Bounds Testing (`OMSetDepthBounds` / DBT) within $[z_{min}, z_{max}]$
//! - Conservative Rasterization (Tiers 1, 2, 3) with sub-pixel uncertainty dilation
//! - Sub-pixel sample coverage masks & `SV_InnerCoverage` detection
//! - Early-Z rejection pipeline & rasterizer stats tracking

#![allow(dead_code)]

extern crate alloc;
#[allow(unused_imports)]
use alloc::vec::Vec;

/// Direct3D 12 Conservative Rasterization Hardware Tiers (`D3D12_CONSERVATIVE_RASTERIZATION_TIER`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Default)]
#[repr(u8)]
pub enum ConservativeRasterTier {
    #[default]
    NotSupported = 0,
    /// Tier 1: Uncertainty region <= 1/2 pixel. Post-snap degenerates may be culled.
    Tier1 = 1,
    /// Tier 2: Uncertainty region <= 1/256 pixel. Post-snap degenerates preserved. SV_InnerCoverage supported.
    Tier2 = 2,
    /// Tier 3: Fully conservative with MSAA sample-mask and depth clipping integration.
    Tier3 = 3,
}

/// Errors returned by the Depth Bounds & Conservative Rasterization subsystem.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DepthBoundsError {
    /// Invalid depth bounds: min > max or outside [0.0, 1.0].
    InvalidDepthRange,
    /// Viewport or render target dimensions are invalid (e.g. zero width/height).
    InvalidViewportBounds,
    /// Requested conservative rasterization tier is unsupported or invalid.
    UnsupportedTier,
    /// Invalid triangle or polygon coordinates (e.g. non-finite float).
    InvalidPrimitive,
}

/// Dynamic Depth Bounds Testing state (`OMSetDepthBounds`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DepthBoundsState {
    pub enabled: bool,
    pub min_depth: f32,
    pub max_depth: f32,
}

impl Default for DepthBoundsState {
    fn default() -> Self {
        Self {
            enabled: false,
            min_depth: 0.0,
            max_depth: 1.0,
        }
    }
}

impl DepthBoundsState {
    /// Creates a new enabled depth bounds test state with range [min_depth, max_depth].
    pub fn new(min_depth: f32, max_depth: f32) -> Result<Self, DepthBoundsError> {
        let state = Self {
            enabled: true,
            min_depth,
            max_depth,
        };
        state.validate()?;
        Ok(state)
    }

    /// Creates a disabled depth bounds state spanning full [0.0, 1.0] range.
    pub const fn disabled() -> Self {
        Self {
            enabled: false,
            min_depth: 0.0,
            max_depth: 1.0,
        }
    }

    /// Validates that bounds lie within [0.0, 1.0] and min <= max.
    pub fn validate(&self) -> Result<(), DepthBoundsError> {
        if !self.min_depth.is_finite()
            || !self.max_depth.is_finite()
            || self.min_depth < 0.0
            || self.max_depth > 1.0
            || self.min_depth > self.max_depth
        {
            return Err(DepthBoundsError::InvalidDepthRange);
        }
        Ok(())
    }

    /// Evaluates whether a given depth value $z$ falls within $[z_{min}, z_{max}]$.
    #[inline(always)]
    pub fn test(&self, z: f32) -> bool {
        if !self.enabled {
            return true;
        }
        z >= self.min_depth && z <= self.max_depth
    }
}

/// Conservative rasterization configuration descriptor.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ConservativeRasterDesc {
    pub enabled: bool,
    pub tier: ConservativeRasterTier,
    pub subpixel_snapping_bits: u32,
}

impl Default for ConservativeRasterDesc {
    fn default() -> Self {
        Self {
            enabled: false,
            tier: ConservativeRasterTier::NotSupported,
            subpixel_snapping_bits: 8,
        }
    }
}

impl ConservativeRasterDesc {
    pub fn new(tier: ConservativeRasterTier) -> Self {
        let (enabled, bits) = match tier {
            ConservativeRasterTier::NotSupported => (false, 0),
            ConservativeRasterTier::Tier1 => (true, 4), // 1/16 sub-pixel
            ConservativeRasterTier::Tier2 => (true, 8), // 1/256 sub-pixel
            ConservativeRasterTier::Tier3 => (true, 8), // 1/256 sub-pixel
        };
        Self {
            enabled,
            tier,
            subpixel_snapping_bits: bits,
        }
    }

    /// Returns true if this configuration supports `SV_InnerCoverage`.
    #[inline(always)]
    pub fn supports_inner_coverage(&self) -> bool {
        self.enabled && self.tier >= ConservativeRasterTier::Tier2
    }
}

/// Coverage evaluation result for an individual pixel.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct PixelCoverage {
    /// True if any part of the pixel area is covered (or center sample in standard raster).
    pub is_covered: bool,
    /// `SV_InnerCoverage`: True if the primitive completely covers the entire pixel box.
    pub inner_coverage: bool,
    /// Bitmask of covered MSAA sample locations (up to 16 samples).
    pub sample_mask: u32,
}

/// 2D axis-aligned bounding box of a pixel in screen coordinates $[x, x+1] \times [y, y+1]$.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PixelAabb {
    pub min_x: f32,
    pub min_y: f32,
    pub max_x: f32,
    pub max_y: f32,
}

impl PixelAabb {
    #[inline(always)]
    pub fn from_pixel_coords(x: i32, y: i32) -> Self {
        Self {
            min_x: x as f32,
            min_y: y as f32,
            max_x: (x + 1) as f32,
            max_y: (y + 1) as f32,
        }
    }

    /// Center coordinate of the pixel.
    #[inline(always)]
    pub fn center(&self) -> [f32; 2] {
        [self.min_x + 0.5, self.min_y + 0.5]
    }

    /// 4 corner vertices of the pixel box:
    /// (min_x, min_y), (max_x, min_y), (max_x, max_y), (min_x, max_y).
    #[inline(always)]
    pub fn corners(&self) -> [[f32; 2]; 4] {
        [
            [self.min_x, self.min_y],
            [self.max_x, self.min_y],
            [self.max_x, self.max_y],
            [self.min_x, self.max_y],
        ]
    }
}


// -----------------------------------------------------------------------------
// Depth Buffer & Early-Z Rejection Pipeline
// -----------------------------------------------------------------------------

/// Depth test comparison function (`D3D12_COMPARISON_FUNC`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum DepthComparisonFunc {
    Never = 0,
    #[default]
    Less = 1,
    Equal = 2,
    LessEqual = 3,
    Greater = 4,
    NotEqual = 5,
    GreaterEqual = 6,
    Always = 7,
}

impl DepthComparisonFunc {
    #[inline(always)]
    pub fn compare(&self, frag_z: f32, buf_z: f32) -> bool {
        match self {
            Self::Never => false,
            Self::Less => frag_z < buf_z,
            Self::Equal => (frag_z - buf_z).abs() < 1e-6,
            Self::LessEqual => frag_z <= buf_z,
            Self::Greater => frag_z > buf_z,
            Self::NotEqual => (frag_z - buf_z).abs() >= 1e-6,
            Self::GreaterEqual => frag_z >= buf_z,
            Self::Always => true,
        }
    }
}

/// 2D floating-point depth buffer supporting fast clear and early-Z depth tests.
#[derive(Debug, Clone)]
pub struct DepthBuffer {
    pub width: u32,
    pub height: u32,
    pub data: Vec<f32>,
}

impl DepthBuffer {
    pub fn new(width: u32, height: u32, clear_depth: f32) -> Result<Self, DepthBoundsError> {
        if width == 0 || height == 0 {
            return Err(DepthBoundsError::InvalidViewportBounds);
        }
        let total = (width as usize) * (height as usize);
        let data = alloc::vec![clear_depth; total];
        Ok(Self { width, height, data })
    }

    #[inline(always)]
    pub fn get_depth(&self, x: u32, y: u32) -> Option<f32> {
        if x < self.width && y < self.height {
            let idx = (y * self.width + x) as usize;
            Some(self.data[idx])
        } else {
            None
        }
    }

    #[inline(always)]
    pub fn set_depth(&mut self, x: u32, y: u32, z: f32) -> bool {
        if x < self.width && y < self.height {
            let idx = (y * self.width + x) as usize;
            self.data[idx] = z;
            true
        } else {
            false
        }
    }

    pub fn clear(&mut self, clear_depth: f32) {
        for z in self.data.iter_mut() {
            *z = clear_depth;
        }
    }
}

/// Statistics tracking early-Z and Depth Bounds Testing efficiency.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct EarlyZStats {
    pub total_fragments_tested: u64,
    pub culled_by_depth_bounds: u64,
    pub culled_by_depth_func: u64,
    pub fragments_passed: u64,
}

impl EarlyZStats {
    /// Computes the fraction of fragments successfully eliminated by depth bounds testing.
    #[inline]
    pub fn depth_bounds_cull_rate(&self) -> f32 {
        if self.total_fragments_tested > 0 {
            self.culled_by_depth_bounds as f32 / self.total_fragments_tested as f32
        } else {
            0.0
        }
    }
}

/// Dynamic Depth Bounds Testing & Early-Z Rejection Pipeline.
#[derive(Debug, Clone, Default)]
pub struct DepthBoundsEvaluator {
    pub bounds_state: DepthBoundsState,
    pub depth_func: DepthComparisonFunc,
    pub depth_write_enable: bool,
    pub stats: EarlyZStats,
}

impl DepthBoundsEvaluator {
    pub fn new(bounds_state: DepthBoundsState, depth_func: DepthComparisonFunc, depth_write_enable: bool) -> Self {
        Self {
            bounds_state,
            depth_func,
            depth_write_enable,
            stats: EarlyZStats::default(),
        }
    }

    /// Direct3D 12 API: `OMSetDepthBounds(min_depth, max_depth)`
    pub fn om_set_depth_bounds(&mut self, min_depth: f32, max_depth: f32) -> Result<(), DepthBoundsError> {
        self.bounds_state = DepthBoundsState::new(min_depth, max_depth)?;
        Ok(())
    }

    /// Disables Depth Bounds Testing.
    pub fn disable_depth_bounds(&mut self) {
        self.bounds_state = DepthBoundsState::disabled();
    }

    /// Evaluates a candidate fragment against Depth Bounds Testing and Early-Z.
    ///
    /// Pipeline Flow:
    /// 1. Reads current buffer depth $z_{buf}$.
    /// 2. Tests $z_{buf}$ against $[z_{min}, z_{max}]$. If outside, culled immediately without shading!
    /// 3. Tests $z_{frag}$ against $z_{buf}$ via `depth_func`. If failed, culled.
    /// 4. If passed and `depth_write_enable`, writes $z_{frag}$ into buffer.
    pub fn evaluate_fragment(
        &mut self,
        depth_buffer: &mut DepthBuffer,
        x: u32,
        y: u32,
        frag_z: f32,
    ) -> bool {
        self.stats.total_fragments_tested += 1;

        let buf_z = match depth_buffer.get_depth(x, y) {
            Some(z) => z,
            None => return false,
        };

        // 1. Depth Bounds Testing (DBT) on current buffer depth
        if !self.bounds_state.test(buf_z) {
            self.stats.culled_by_depth_bounds += 1;
            return false;
        }

        // 2. Standard Depth Test
        if !self.depth_func.compare(frag_z, buf_z) {
            self.stats.culled_by_depth_func += 1;
            return false;
        }

        // 3. Depth Write
        if self.depth_write_enable {
            depth_buffer.set_depth(x, y, frag_z);
        }

        self.stats.fragments_passed += 1;
        true
    }
}
