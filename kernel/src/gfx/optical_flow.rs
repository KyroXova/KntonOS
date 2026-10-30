//! Direct3D 12 Hardware Optical Flow Vector Accelerator & Frame Interpolation Motion Engine.
//!
//! Provides hardware-accelerated optical flow vector estimation, hierarchical block matching
//! with sub-pixel 1/4th-pel accuracy, cost surface evaluation, dynamic disocclusion mask detection,
//! and bidirectional temporal motion warping for driver-level frame generation (DLSS 3 / FSR 3 style).

use alloc::vec::Vec;

/// Scaling factor for 16.16 fixed-point arithmetic (65536 = 1.0).
pub const FIXED_POINT_ONE: i32 = 65536;

/// Scaling factor for quarter-pel (1/4th pixel) in 16.16 fixed-point (16384 = 0.25).
pub const QUARTER_PEL_FIXED: i32 = 16384;

/// Errors returned by the optical flow and motion estimation engine.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OpticalFlowError {
    /// Specified frame or block dimensions are invalid or zero.
    InvalidDimensions,
    /// Vector field or frame buffer is too small for the specified grid.
    BufferTooSmall,
    /// Coordinates or search offsets exceed buffer boundaries.
    OutOfBounds,
    /// Hardware accelerator queue failure or invalid parameter set.
    HardwareFault,
}

/// Precision mode for optical flow motion vector calculation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OpticalFlowPrecision {
    /// Full integer pixel precision.
    FullPel,
    /// Half-pixel (1/2 pel) sub-pixel precision.
    HalfPel,
    /// Quarter-pixel (1/4th pel) fine sub-pixel precision.
    QuarterPel,
}

/// Temporal direction of optical flow vector estimation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OpticalFlowDirection {
    /// Forward motion vectors from Frame N to Frame N+1.
    Forward,
    /// Backward motion vectors from Frame N+1 to Frame N.
    Backward,
}

/// 16.16 fixed-point 2D motion vector supporting sub-pixel 1/4th-pel precision.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct MotionVector {
    /// Horizontal displacement in 16.16 fixed-point format.
    pub x: i32,
    /// Vertical displacement in 16.16 fixed-point format.
    pub y: i32,
}

impl MotionVector {
    /// Creates a motion vector from raw 16.16 fixed-point coordinates.
    pub const fn new(x_fixed: i32, y_fixed: i32) -> Self {
        Self { x: x_fixed, y: y_fixed }
    }

    /// Creates a zero motion vector.
    pub const fn zero() -> Self {
        Self { x: 0, y: 0 }
    }

    /// Creates a motion vector from integer pixel displacements.
    pub const fn from_integer_pel(px: i32, py: i32) -> Self {
        Self {
            x: px * FIXED_POINT_ONE,
            y: py * FIXED_POINT_ONE,
        }
    }

    /// Creates a motion vector from quarter-pel units (1 unit = 0.25 pixels).
    pub const fn from_quarter_pel(qx: i16, qy: i16) -> Self {
        Self {
            x: (qx as i32) * QUARTER_PEL_FIXED,
            y: (qy as i32) * QUARTER_PEL_FIXED,
        }
    }

    /// Creates a motion vector from floating-point pixel displacements.
    pub fn from_f32(px: f32, py: f32) -> Self {
        Self {
            x: (px * (FIXED_POINT_ONE as f32)) as i32,
            y: (py * (FIXED_POINT_ONE as f32)) as i32,
        }
    }

    /// Converts the vector to quarter-pel integer units.
    pub const fn to_quarter_pel(&self) -> (i16, i16) {
        let qx = (self.x + (QUARTER_PEL_FIXED / 2)) / QUARTER_PEL_FIXED;
        let qy = (self.y + (QUARTER_PEL_FIXED / 2)) / QUARTER_PEL_FIXED;
        (qx as i16, qy as i16)
    }

    /// Converts the vector to integer pixels (truncated).
    pub const fn to_integer_pel(&self) -> (i32, i32) {
        (self.x / FIXED_POINT_ONE, self.y / FIXED_POINT_ONE)
    }

    /// Converts the vector to floating-point pixels.
    pub fn to_f32(&self) -> (f32, f32) {
        (
            (self.x as f32) / (FIXED_POINT_ONE as f32),
            (self.y as f32) / (FIXED_POINT_ONE as f32),
        )
    }

    /// Computes the squared magnitude of displacement in 16.16 fixed-point.
    pub fn magnitude_squared(&self) -> i64 {
        let x64 = self.x as i64;
        let y64 = self.y as i64;
        (x64 * x64 + y64 * y64) >> 16
    }

    /// Returns true if the motion vector is exactly zero.
    pub const fn is_zero(&self) -> bool {
        self.x == 0 && self.y == 0
    }

    /// Adds another motion vector.
    pub const fn add(&self, other: &Self) -> Self {
        Self {
            x: self.x + other.x,
            y: self.y + other.y,
        }
    }

    /// Subtracts another motion vector.
    pub const fn sub(&self, other: &Self) -> Self {
        Self {
            x: self.x - other.x,
            y: self.y - other.y,
        }
    }

    /// Scales the vector by a 16.16 fixed-point scalar factor.
    pub fn scale_fixed(&self, factor_16_16: i32) -> Self {
        let x64 = (self.x as i64 * factor_16_16 as i64) / (FIXED_POINT_ONE as i64);
        let y64 = (self.y as i64 * factor_16_16 as i64) / (FIXED_POINT_ONE as i64);
        Self {
            x: x64 as i32,
            y: y64 as i32,
        }
    }
}

/// 2D regular grid of optical flow motion vectors.
#[derive(Debug, Clone, PartialEq)]
pub struct OpticalFlowVectorField {
    pub frame_width: u32,
    pub frame_height: u32,
    pub block_size: u32,
    pub grid_width: u32,
    pub grid_height: u32,
    pub vectors: Vec<MotionVector>,
}

impl OpticalFlowVectorField {
    /// Creates a new zero-initialized vector field for the given frame and block dimensions.
    pub fn new(frame_width: u32, frame_height: u32, block_size: u32) -> Self {
        let b = block_size.max(1);
        let gw = (frame_width + b - 1) / b;
        let gh = (frame_height + b - 1) / b;
        let total = (gw * gh) as usize;
        Self {
            frame_width,
            frame_height,
            block_size: b,
            grid_width: gw,
            grid_height: gh,
            vectors: alloc::vec![MotionVector::zero(); total],
        }
    }

    /// Returns the motion vector at grid coordinate (gx, gy).
    #[inline(always)]
    pub fn get_vector(&self, gx: u32, gy: u32) -> MotionVector {
        if gx < self.grid_width && gy < self.grid_height {
            let idx = (gy * self.grid_width + gx) as usize;
            self.vectors[idx]
        } else {
            MotionVector::zero()
        }
    }

    /// Sets the motion vector at grid coordinate (gx, gy).
    #[inline(always)]
    pub fn set_vector(&mut self, gx: u32, gy: u32, mv: MotionVector) {
        if gx < self.grid_width && gy < self.grid_height {
            let idx = (gy * self.grid_width + gx) as usize;
            self.vectors[idx] = mv;
        }
    }

    /// Samples a motion vector at an arbitrary pixel coordinate (px, py) with bilinear interpolation.
    pub fn sample_at_pixel(&self, px: f32, py: f32) -> MotionVector {
        if self.grid_width == 0 || self.grid_height == 0 || self.block_size == 0 {
            return MotionVector::zero();
        }

        let b = self.block_size as f32;
        let gx = (px / b).max(0.0);
        let gy = (py / b).max(0.0);

        let gx0 = (gx as u32).min(self.grid_width.saturating_sub(1));
        let gy0 = (gy as u32).min(self.grid_height.saturating_sub(1));
        let gx1 = (gx0 + 1).min(self.grid_width.saturating_sub(1));
        let gy1 = (gy0 + 1).min(self.grid_height.saturating_sub(1));

        let fx = gx - (gx0 as f32);
        let fy = gy - (gy0 as f32);

        let v00 = self.get_vector(gx0, gy0);
        let v10 = self.get_vector(gx1, gy0);
        let v01 = self.get_vector(gx0, gy1);
        let v11 = self.get_vector(gx1, gy1);

        let fx_fixed = (fx * (FIXED_POINT_ONE as f32)) as i32;
        let fy_fixed = (fy * (FIXED_POINT_ONE as f32)) as i32;

        let inv_fx = FIXED_POINT_ONE - fx_fixed;
        let inv_fy = FIXED_POINT_ONE - fy_fixed;

        let w00 = ((inv_fx as i64 * inv_fy as i64) >> 16) as i32;
        let w10 = ((fx_fixed as i64 * inv_fy as i64) >> 16) as i32;
        let w01 = ((inv_fx as i64 * fy_fixed as i64) >> 16) as i32;
        let w11 = ((fx_fixed as i64 * fy_fixed as i64) >> 16) as i32;

        let interp_x = ((v00.x as i64 * w00 as i64)
            + (v10.x as i64 * w10 as i64)
            + (v01.x as i64 * w01 as i64)
            + (v11.x as i64 * w11 as i64))
            >> 16;
        let interp_y = ((v00.y as i64 * w00 as i64)
            + (v10.y as i64 * w10 as i64)
            + (v01.y as i64 * w01 as i64)
            + (v11.y as i64 * w11 as i64))
            >> 16;

        MotionVector::new(interp_x as i32, interp_y as i32)
    }
}

/// D3D12 hardware optical flow cost surface grid for confidence evaluation and fallback.
#[derive(Debug, Clone, PartialEq)]
pub struct CostSurfaceGrid {
    pub grid_width: u32,
    pub grid_height: u32,
    pub search_radius: u32,
    pub window_dim: u32,
    pub costs: Vec<u32>,
}

impl CostSurfaceGrid {
    /// Creates a new cost surface grid with the given dimensions and search radius.
    pub fn new(grid_width: u32, grid_height: u32, search_radius: u32) -> Self {
        let win = search_radius * 2 + 1;
        let total = (grid_width * grid_height * win * win) as usize;
        Self {
            grid_width,
            grid_height,
            search_radius,
            window_dim: win,
            costs: alloc::vec![u32::MAX; total],
        }
    }

    #[inline(always)]
    fn offset_to_index(&self, gx: u32, gy: u32, dx: i32, dy: i32) -> Option<usize> {
        let r = self.search_radius as i32;
        if gx >= self.grid_width || gy >= self.grid_height || dx < -r || dx > r || dy < -r || dy > r {
            return None;
        }
        let wx = (dx + r) as u32;
        let wy = (dy + r) as u32;
        let block_idx = (gy * self.grid_width + gx) as usize;
        let win_stride = (self.window_dim * self.window_dim) as usize;
        let local_idx = (wy * self.window_dim + wx) as usize;
        Some(block_idx * win_stride + local_idx)
    }

    /// Sets the matching cost (SAD) for a candidate displacement (dx, dy) at block (gx, gy).
    pub fn set_cost(&mut self, gx: u32, gy: u32, dx: i32, dy: i32, cost: u32) {
        if let Some(idx) = self.offset_to_index(gx, gy, dx, dy) {
            self.costs[idx] = cost;
        }
    }

    /// Gets the matching cost (SAD) for a candidate displacement (dx, dy) at block (gx, gy).
    pub fn get_cost(&self, gx: u32, gy: u32, dx: i32, dy: i32) -> u32 {
        if let Some(idx) = self.offset_to_index(gx, gy, dx, dy) {
            self.costs[idx]
        } else {
            u32::MAX
        }
    }

    /// Finds the candidate displacement with minimal matching cost for block (gx, gy).
    pub fn find_min_cost(&self, gx: u32, gy: u32) -> (i32, i32, u32) {
        let r = self.search_radius as i32;
        let mut min_dx = 0;
        let mut min_dy = 0;
        let mut min_cost = u32::MAX;

        for dy in -r..=r {
            for dx in -r..=r {
                let cost = self.get_cost(gx, gy, dx, dy);
                if cost < min_cost {
                    min_cost = cost;
                    min_dx = dx;
                    min_dy = dy;
                }
            }
        }

        (min_dx, min_dy, min_cost)
    }
}

/// Hardware Optical Flow Accelerator Configuration Descriptor.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OpticalFlowDescriptor {
    /// Frame width in pixels.
    pub frame_width: u32,
    /// Frame height in pixels.
    pub frame_height: u32,
    /// Block size (e.g., 8x8 or 16x16 pixels).
    pub block_size: u32,
    /// Sub-pixel precision mode.
    pub precision: OpticalFlowPrecision,
    /// Maximum integer search radius in pixels.
    pub search_radius: u32,
    /// Whether cost surface buffers are generated.
    pub enable_cost_surface: bool,
    /// Whether prior motion hints are consumed.
    pub enable_hint_vectors: bool,
}

impl Default for OpticalFlowDescriptor {
    fn default() -> Self {
        Self {
            frame_width: 1920,
            frame_height: 1080,
            block_size: 8,
            precision: OpticalFlowPrecision::QuarterPel,
            search_radius: 16,
            enable_cost_surface: true,
            enable_hint_vectors: false,
        }
    }
}

impl OpticalFlowDescriptor {
    /// Validates the optical flow descriptor parameters.
    pub fn validate(&self) -> Result<(), OpticalFlowError> {
        if self.frame_width == 0 || self.frame_height == 0 || self.block_size == 0 {
            return Err(OpticalFlowError::InvalidDimensions);
        }
        if self.search_radius == 0 || self.search_radius > 64 {
            return Err(OpticalFlowError::OutOfBounds);
        }
        Ok(())
    }
}
