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
