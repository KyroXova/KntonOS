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


// -----------------------------------------------------------------------------
// Conservative Rasterization Boundary Expander & Inner Coverage Engine
// -----------------------------------------------------------------------------

/// 2D Triangle representation in screen space.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Triangle2D {
    pub v0: [f32; 2],
    pub v1: [f32; 2],
    pub v2: [f32; 2],
}

impl Triangle2D {
    pub fn new(v0: [f32; 2], v1: [f32; 2], v2: [f32; 2]) -> Result<Self, DepthBoundsError> {
        for v in [v0, v1, v2] {
            if !v[0].is_finite() || !v[1].is_finite() {
                return Err(DepthBoundsError::InvalidPrimitive);
            }
        }
        Ok(Self { v0, v1, v2 })
    }

    /// Computes the signed 2D area (cross product). Positive for CCW, negative for CW.
    #[inline(always)]
    pub fn signed_area_x2(&self) -> f32 {
        (self.v1[0] - self.v0[0]) * (self.v2[1] - self.v0[1])
            - (self.v1[1] - self.v0[1]) * (self.v2[0] - self.v0[0])
    }

    /// Computes axis-aligned bounding box enclosing the triangle.
    pub fn bounding_box(&self) -> (f32, f32, f32, f32) {
        let min_x = self.v0[0].min(self.v1[0]).min(self.v2[0]);
        let max_x = self.v0[0].max(self.v1[0]).max(self.v2[0]);
        let min_y = self.v0[1].min(self.v1[1]).min(self.v2[1]);
        let max_y = self.v0[1].max(self.v1[1]).max(self.v2[1]);
        (min_x, min_y, max_x, max_y)
    }

    /// Evaluates if a single point (px, py) is inside the triangle.
    pub fn contains_point(&self, p: [f32; 2]) -> bool {
        let area = self.signed_area_x2();
        if area.abs() < 1e-7 {
            return false;
        }
        let sign = area > 0.0;

        let e0 = (p[0] - self.v0[0]) * (self.v1[1] - self.v0[1]) - (p[1] - self.v0[1]) * (self.v1[0] - self.v0[0]);
        let e1 = (p[0] - self.v1[0]) * (self.v2[1] - self.v1[1]) - (p[1] - self.v1[1]) * (self.v2[0] - self.v1[0]);
        let e2 = (p[0] - self.v2[0]) * (self.v0[1] - self.v2[1]) - (p[1] - self.v2[1]) * (self.v0[0] - self.v2[0]);

        if sign {
            e0 <= 0.0 && e1 <= 0.0 && e2 <= 0.0
        } else {
            e0 >= 0.0 && e1 >= 0.0 && e2 >= 0.0
        }
    }

    /// Evaluates conservative coverage and `SV_InnerCoverage` against a pixel box [x, x+1] x [y, y+1].
    pub fn evaluate_pixel_coverage(
        &self,
        pixel_x: i32,
        pixel_y: i32,
        desc: &ConservativeRasterDesc,
    ) -> PixelCoverage {
        let aabb = PixelAabb::from_pixel_coords(pixel_x, pixel_y);
        let center = aabb.center();

        let center_covered = self.contains_point(center);

        if !desc.enabled {
            return PixelCoverage {
                is_covered: center_covered,
                inner_coverage: false,
                sample_mask: if center_covered { 1 } else { 0 },
            };
        }

        // Conservative Rasterization:
        // Test 1: Check if center is covered or any corner is covered
        let corners = aabb.corners();
        let mut corners_inside = 0;
        for c in corners {
            if self.contains_point(c) {
                corners_inside += 1;
            }
        }

        // Test 2: If all 4 corners are inside, pixel is fully covered (SV_InnerCoverage)
        let inner_coverage = desc.supports_inner_coverage() && (corners_inside == 4);

        // Test 3: Separating Axis Theorem (SAT) between pixel AABB and triangle
        // Pixel intersects triangle if corners are inside, or triangle vertices are inside pixel box,
        // or triangle edges intersect pixel box edges.
        let mut is_covered = center_covered || (corners_inside > 0);

        if !is_covered {
            // Check if any triangle vertex is inside the pixel box
            for v in [self.v0, self.v1, self.v2] {
                if v[0] >= aabb.min_x && v[0] <= aabb.max_x && v[1] >= aabb.min_y && v[1] <= aabb.max_y {
                    is_covered = true;
                    break;
                }
            }
        }

        // Edge-box intersection test
        if !is_covered {
            let edges = [
                (self.v0, self.v1),
                (self.v1, self.v2),
                (self.v2, self.v0),
            ];
            for (p1, p2) in edges {
                if line_segment_intersects_aabb(p1, p2, &aabb) {
                    is_covered = true;
                    break;
                }
            }
        }

        // Compute 4-sample MSAA sub-pixel sample mask if Tier 3
        let sample_mask = if desc.tier == ConservativeRasterTier::Tier3 {
            let sub_samples = [
                [aabb.min_x + 0.375, aabb.min_y + 0.125],
                [aabb.min_x + 0.875, aabb.min_y + 0.375],
                [aabb.min_x + 0.125, aabb.min_y + 0.625],
                [aabb.min_x + 0.625, aabb.min_y + 0.875],
            ];
            let mut mask = 0u32;
            for (idx, sp) in sub_samples.iter().enumerate() {
                if self.contains_point(*sp) {
                    mask |= 1 << idx;
                }
            }
            if is_covered && mask == 0 {
                mask = 1; // Conservative rasterization guarantees at least 1 sample active
            }
            mask
        } else if is_covered {
            1
        } else {
            0
        };

        PixelCoverage {
            is_covered,
            inner_coverage,
            sample_mask,
        }
    }
}

/// Helper testing if a 2D line segment intersects a pixel AABB.
fn line_segment_intersects_aabb(p1: [f32; 2], p2: [f32; 2], aabb: &PixelAabb) -> bool {
    let min_x = p1[0].min(p2[0]);
    let max_x = p1[0].max(p2[0]);
    let min_y = p1[1].min(p2[1]);
    let max_y = p1[1].max(p2[1]);

    if max_x < aabb.min_x || min_x > aabb.max_x || max_y < aabb.min_y || min_y > aabb.max_y {
        return false;
    }

    // Line equation: A*x + B*y + C = 0
    let a = p2[1] - p1[1];
    let b = p1[0] - p2[0];
    let c = p2[0] * p1[1] - p1[0] * p2[1];

    let corners = aabb.corners();
    let signs = [
        a * corners[0][0] + b * corners[0][1] + c > 0.0,
        a * corners[1][0] + b * corners[1][1] + c > 0.0,
        a * corners[2][0] + b * corners[2][1] + c > 0.0,
        a * corners[3][0] + b * corners[3][1] + c > 0.0,
    ];

    // If all corners are on the same side of the line, no intersection
    let all_pos = signs[0] && signs[1] && signs[2] && signs[3];
    let all_neg = !signs[0] && !signs[1] && !signs[2] && !signs[3];
    !all_pos && !all_neg
}

#[inline(always)]
fn f32_floor(v: f32) -> i32 {
    let i = v as i32;
    if v < (i as f32) { i - 1 } else { i }
}

#[inline(always)]
fn f32_ceil(v: f32) -> i32 {
    let i = v as i32;
    if v > (i as f32) { i + 1 } else { i }
}

/// Consolidated Conservative Rasterization & Depth Bounds Execution Engine.
#[derive(Debug, Clone, Default)]
pub struct ConservativeRasterEngine {
    pub desc: ConservativeRasterDesc,
    pub depth_evaluator: DepthBoundsEvaluator,
    pub total_pixels_rasterized_standard: u64,
    pub total_pixels_rasterized_conservative: u64,
    pub conservative_expansion_pixels: u64,
    pub inner_coverage_pixels: u64,
}

impl ConservativeRasterEngine {
    pub fn new(tier: ConservativeRasterTier, depth_bounds: DepthBoundsState) -> Self {
        Self {
            desc: ConservativeRasterDesc::new(tier),
            depth_evaluator: DepthBoundsEvaluator::new(depth_bounds, DepthComparisonFunc::LessEqual, true),
            total_pixels_rasterized_standard: 0,
            total_pixels_rasterized_conservative: 0,
            conservative_expansion_pixels: 0,
            inner_coverage_pixels: 0,
        }
    }

    /// Rasterizes a triangle with conservative boundary expansion and early-Z depth bounds testing.
    pub fn rasterize_triangle(
        &mut self,
        triangle: &Triangle2D,
        frag_z: f32,
        depth_buffer: &mut DepthBuffer,
    ) -> Result<Vec<(u32, u32, PixelCoverage)>, DepthBoundsError> {
        let (min_x, min_y, max_x, max_y) = triangle.bounding_box();

        let start_x = f32_floor(min_x).max(0) as u32;
        let end_x = (f32_ceil(max_x) + 1).min(depth_buffer.width as i32).max(0) as u32;
        let start_y = f32_floor(min_y).max(0) as u32;
        let end_y = (f32_ceil(max_y) + 1).min(depth_buffer.height as i32).max(0) as u32;

        let mut output = Vec::new();

        for y in start_y..end_y {
            for x in start_x..end_x {
                let cov = triangle.evaluate_pixel_coverage(x as i32, y as i32, &self.desc);
                if cov.is_covered {
                    self.total_pixels_rasterized_conservative += 1;

                    let std_cov = triangle.contains_point([x as f32 + 0.5, y as f32 + 0.5]);
                    if std_cov {
                        self.total_pixels_rasterized_standard += 1;
                    } else {
                        self.conservative_expansion_pixels += 1;
                    }

                    if cov.inner_coverage {
                        self.inner_coverage_pixels += 1;
                    }

                    // Apply Early-Z and Depth Bounds Testing
                    if self.depth_evaluator.evaluate_fragment(depth_buffer, x, y, frag_z) {
                        output.push((x, y, cov));
                    }
                }
            }
        }

        Ok(output)
    }
}
