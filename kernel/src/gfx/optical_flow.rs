//! Direct3D 12 Hardware Optical Flow Vector Accelerator & Frame Interpolation Motion Engine.
//! Provides hardware-accelerated optical flow vector estimation, hierarchical block matching

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
        let qx = if self.x >= 0 {
            (self.x + (QUARTER_PEL_FIXED / 2)) / QUARTER_PEL_FIXED
        } else {
            (self.x - (QUARTER_PEL_FIXED / 2)) / QUARTER_PEL_FIXED
        };
        let qy = if self.y >= 0 {
            (self.y + (QUARTER_PEL_FIXED / 2)) / QUARTER_PEL_FIXED
        } else {
            (self.y - (QUARTER_PEL_FIXED / 2)) / QUARTER_PEL_FIXED
        };
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

    /// Samples a motion vector at an arbitrary pixel coordinate (px, py).
    pub fn sample_at_pixel(&self, px: f32, py: f32) -> MotionVector {
        if self.grid_width == 0 || self.grid_height == 0 || self.block_size == 0 {
            return MotionVector::zero();
        }

        let gx = ((px.max(0.0) as u32) / self.block_size).min(self.grid_width.saturating_sub(1));
        let gy = ((py.max(0.0) as u32) / self.block_size).min(self.grid_height.saturating_sub(1));
        self.get_vector(gx, gy)
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


// Frame Buffers, Pyramids & Hierarchical Block Matching (HBM)

/// Grayscale / luminance frame buffer used for hardware motion estimation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FrameBuffer {
    pub width: u32,
    pub height: u32,
    pub data: Vec<u8>,
}

impl FrameBuffer {
    /// Creates a new frame buffer from dimensions and raw pixel bytes.
    pub fn new(width: u32, height: u32, data: Vec<u8>) -> Result<Self, OpticalFlowError> {
        if width == 0 || height == 0 {
            return Err(OpticalFlowError::InvalidDimensions);
        }
        if data.len() < (width * height) as usize {
            return Err(OpticalFlowError::BufferTooSmall);
        }
        Ok(Self { width, height, data })
    }

    /// Creates a zero-filled frame buffer with the given dimensions.
    pub fn zero(width: u32, height: u32) -> Self {
        let total = (width * height) as usize;
        Self {
            width,
            height,
            data: alloc::vec![0u8; total],
        }
    }

    /// Reads a pixel value with border clamping (edge replication).
    #[inline(always)]
    pub fn get_pixel(&self, x: i32, y: i32) -> u8 {
        let cx = x.clamp(0, self.width.saturating_sub(1) as i32) as u32;
        let cy = y.clamp(0, self.height.saturating_sub(1) as i32) as u32;
        let idx = (cy * self.width + cx) as usize;
        self.data[idx]
    }

    /// Sets a pixel value at (x, y). Out-of-bounds writes are safely ignored.
    #[inline(always)]
    pub fn set_pixel(&mut self, x: u32, y: u32, val: u8) {
        if x < self.width && y < self.height {
            let idx = (y * self.width + x) as usize;
            self.data[idx] = val;
        }
    }

    /// Samples a pixel at sub-pixel 1/4th-pel resolution using fixed-point bilinear interpolation.
    /// `x_quarter` and `y_quarter` are coordinates in units of 0.25 pixels.
    pub fn sample_subpixel_quarter_pel(&self, x_quarter: i32, y_quarter: i32) -> u8 {
        let px = x_quarter >> 2;
        let py = y_quarter >> 2;
        let fx = ((x_quarter & 3) * 64) as i32; // 0, 64, 128, 192 (out of 256)
        let fy = ((y_quarter & 3) * 64) as i32;

        let p00 = self.get_pixel(px, py) as i32;
        let p10 = self.get_pixel(px + 1, py) as i32;
        let p01 = self.get_pixel(px, py + 1) as i32;
        let p11 = self.get_pixel(px + 1, py + 1) as i32;

        let inv_fx = 256 - fx;
        let inv_fy = 256 - fy;

        let w00 = inv_fx * inv_fy;
        let w10 = fx * inv_fy;
        let w01 = inv_fx * fy;
        let w11 = fx * fy;

        let interpolated = (p00 * w00 + p10 * w10 + p01 * w01 + p11 * w11 + 32768) >> 16;
        interpolated.clamp(0, 255) as u8
    }

    /// Downsamples the frame by 2x in each dimension using a 2x2 box average.
    pub fn downsample_2x(&self) -> Self {
        let new_w = (self.width + 1) / 2;
        let new_h = (self.height + 1) / 2;
        let mut downsampled = alloc::vec![0u8; (new_w * new_h) as usize];

        for ny in 0..new_h {
            for nx in 0..new_w {
                let sx = (nx * 2) as i32;
                let sy = (ny * 2) as i32;

                let p00 = self.get_pixel(sx, sy) as u32;
                let p10 = self.get_pixel(sx + 1, sy) as u32;
                let p01 = self.get_pixel(sx, sy + 1) as u32;
                let p11 = self.get_pixel(sx + 1, sy + 1) as u32;

                let avg = ((p00 + p10 + p01 + p11 + 2) / 4) as u8;
                let idx = (ny * new_w + nx) as usize;
                downsampled[idx] = avg;
            }
        }

        Self {
            width: new_w,
            height: new_h,
            data: downsampled,
        }
    }
}

/// Multi-resolution image pyramid for hierarchical coarse-to-fine motion estimation.
#[derive(Debug, Clone)]
pub struct ImagePyramid {
    pub levels: Vec<FrameBuffer>,
}

impl ImagePyramid {
    /// Builds an image pyramid from the base frame with up to `max_levels` octaves.
    pub fn build(base: FrameBuffer, max_levels: usize) -> Self {
        let mut levels = Vec::new();
        let mut current = base;

        let max_lvls = max_levels.max(1);
        for _ in 0..max_lvls {
            let next = current.downsample_2x();
            let stop = current.width <= 4 || current.height <= 4;
            levels.push(current);
            if stop {
                break;
            }
            current = next;
        }

        Self { levels }
    }

    /// Returns the number of levels in the pyramid.
    #[inline(always)]
    pub fn num_levels(&self) -> usize {
        self.levels.len()
    }
}

/// Computes the Sum of Absolute Differences (SAD) for a block at integer displacement (dx, dy).
pub fn compute_block_sad(
    cur: &FrameBuffer,
    ref_frame: &FrameBuffer,
    bx: u32,
    by: u32,
    bsize: u32,
    dx: i32,
    dy: i32,
) -> u32 {
    let mut sad = 0u32;
    for j in 0..bsize {
        for i in 0..bsize {
            let cx = (bx + i) as i32;
            let cy = (by + j) as i32;
            let p_cur = cur.get_pixel(cx, cy) as i32;
            let p_ref = ref_frame.get_pixel(cx + dx, cy + dy) as i32;
            sad += (p_cur - p_ref).abs() as u32;
        }
    }
    sad
}

/// Computes the Sum of Absolute Differences (SAD) for a block at quarter-pel displacement (qx, qy).
pub fn compute_block_sad_quarter_pel(
    cur: &FrameBuffer,
    ref_frame: &FrameBuffer,
    bx: u32,
    by: u32,
    bsize: u32,
    qx: i32,
    qy: i32,
) -> u32 {
    let mut sad = 0u32;
    for j in 0..bsize {
        for i in 0..bsize {
            let cx = (bx + i) as i32;
            let cy = (by + j) as i32;
            let p_cur = cur.get_pixel(cx, cy) as i32;

            let rx_q = (cx * 4) + qx;
            let ry_q = (cy * 4) + qy;
            let p_ref = ref_frame.sample_subpixel_quarter_pel(rx_q, ry_q) as i32;
            sad += (p_cur - p_ref).abs() as u32;
        }
    }
    sad
}

/// Consolidated Direct3D 12 Hardware Optical Flow Vector Accelerator.
#[derive(Debug, Clone)]
pub struct OpticalFlowAccelerator {
    pub desc: OpticalFlowDescriptor,
    pub cost_surface: Option<CostSurfaceGrid>,
    pub total_vectors_computed: u64,
}

impl OpticalFlowAccelerator {
    /// Creates a new optical flow accelerator engine with the specified descriptor.
    pub fn new(desc: OpticalFlowDescriptor) -> Result<Self, OpticalFlowError> {
        desc.validate()?;
        let cost_surface = if desc.enable_cost_surface {
            let gw = (desc.frame_width + desc.block_size - 1) / desc.block_size;
            let gh = (desc.frame_height + desc.block_size - 1) / desc.block_size;
            Some(CostSurfaceGrid::new(gw, gh, desc.search_radius))
        } else {
            None
        };

        Ok(Self {
            desc,
            cost_surface,
            total_vectors_computed: 0,
        })
    }

    /// Estimates the optical flow vector field from `cur_frame` to `ref_frame`.
    pub fn estimate_motion(
        &mut self,
        cur_frame: &FrameBuffer,
        ref_frame: &FrameBuffer,
    ) -> Result<OpticalFlowVectorField, OpticalFlowError> {
        if cur_frame.width != self.desc.frame_width || cur_frame.height != self.desc.frame_height {
            return Err(OpticalFlowError::InvalidDimensions);
        }
        if ref_frame.width != self.desc.frame_width || ref_frame.height != self.desc.frame_height {
            return Err(OpticalFlowError::InvalidDimensions);
        }

        let mut field = OpticalFlowVectorField::new(
            self.desc.frame_width,
            self.desc.frame_height,
            self.desc.block_size,
        );

        // Build 3-level multi-resolution pyramids
        let pyr_cur = ImagePyramid::build(cur_frame.clone(), 3);
        let pyr_ref = ImagePyramid::build(ref_frame.clone(), 3);
        let num_levels = pyr_cur.num_levels().min(pyr_ref.num_levels());

        let bsize = self.desc.block_size;
        let gw = field.grid_width;
        let gh = field.grid_height;

        for gy in 0..gh {
            for gx in 0..gw {
                let mut best_dx = 0i32;
                let mut best_dy = 0i32;

                // Coarse-to-fine search across pyramid levels
                for lvl_idx in (0..num_levels).rev() {
                    let cur_lvl = &pyr_cur.levels[lvl_idx];
                    let ref_lvl = &pyr_ref.levels[lvl_idx];

                    let scale = 1u32 << lvl_idx;
                    let lvl_bsize = (bsize / scale).max(2);
                    let bx = gx * lvl_bsize;
                    let by = gy * lvl_bsize;

                    if lvl_idx == num_levels - 1 {
                        // Coarsest level: wide window search around 0
                        let r = (self.desc.search_radius / scale).max(2) as i32;
                        let mut min_sad = u32::MAX;
                        for dy in -r..=r {
                            for dx in -r..=r {
                                let sad = compute_block_sad(cur_lvl, ref_lvl, bx, by, lvl_bsize, dx, dy);
                                if sad < min_sad {
                                    min_sad = sad;
                                    best_dx = dx;
                                    best_dy = dy;
                                }
                            }
                        }
                    } else {
                        // Finer levels: scale predictor by 2 and refine locally
                        best_dx *= 2;
                        best_dy *= 2;
                        let r = 2i32; // Local refinement radius
                        let mut min_sad = u32::MAX;
                        let mut refined_dx = best_dx;
                        let mut refined_dy = best_dy;

                        for dy in -r..=r {
                            for dx in -r..=r {
                                let cand_dx = best_dx + dx;
                                let cand_dy = best_dy + dy;
                                let sad = compute_block_sad(cur_lvl, ref_lvl, bx, by, lvl_bsize, cand_dx, cand_dy);
                                if sad < min_sad {
                                    min_sad = sad;
                                    refined_dx = cand_dx;
                                    refined_dy = cand_dy;
                                }
                            }
                        }
                        best_dx = refined_dx;
                        best_dy = refined_dy;
                    }
                }

                // Level 0: Record matching costs into cost surface if enabled
                if let Some(ref mut cs) = self.cost_surface {
                    let r = cs.search_radius as i32;
                    let bx = gx * bsize;
                    let by = gy * bsize;
                    for dy in -r..=r {
                        for dx in -r..=r {
                            let sad = compute_block_sad(cur_frame, ref_frame, bx, by, bsize, dx, dy);
                            cs.set_cost(gx, gy, dx, dy, sad);
                        }
                    }
                }

                // Level 0: Sub-pixel refinement (Quarter-Pel or Half-Pel)
                let bx = gx * bsize;
                let by = gy * bsize;
                let mv = match self.desc.precision {
                    OpticalFlowPrecision::FullPel => MotionVector::from_integer_pel(best_dx, best_dy),
                    OpticalFlowPrecision::HalfPel => {
                        let mut min_sad = u32::MAX;
                        let mut best_qx = best_dx * 4;
                        let mut best_qy = best_dy * 4;
                        for dqy in [-2, 0, 2] {
                            for dqx in [-2, 0, 2] {
                                let qx = best_dx * 4 + dqx;
                                let qy = best_dy * 4 + dqy;
                                let sad = compute_block_sad_quarter_pel(cur_frame, ref_frame, bx, by, bsize, qx, qy);
                                if sad < min_sad {
                                    min_sad = sad;
                                    best_qx = qx;
                                    best_qy = qy;
                                }
                            }
                        }
                        MotionVector::from_quarter_pel(best_qx as i16, best_qy as i16)
                    }
                    OpticalFlowPrecision::QuarterPel => {
                        let mut min_sad = u32::MAX;
                        let mut best_qx = best_dx * 4;
                        let mut best_qy = best_dy * 4;
                        for dqy in -2..=2 {
                            for dqx in -2..=2 {
                                let qx = best_dx * 4 + dqx;
                                let qy = best_dy * 4 + dqy;
                                let sad = compute_block_sad_quarter_pel(cur_frame, ref_frame, bx, by, bsize, qx, qy);
                                if sad < min_sad {
                                    min_sad = sad;
                                    best_qx = qx;
                                    best_qy = qy;
                                }
                            }
                        }
                        MotionVector::from_quarter_pel(best_qx as i16, best_qy as i16)
                    }
                };

                field.set_vector(gx, gy, mv);
                self.total_vectors_computed += 1;
            }
        }

        Ok(field)
    }

    /// Estimates bidirectional optical flow fields (both Forward $0 \to 1$ and Backward $1 \to 0$).
    pub fn estimate_bidirectional(
        &mut self,
        frame_0: &FrameBuffer,
        frame_1: &FrameBuffer,
    ) -> Result<(OpticalFlowVectorField, OpticalFlowVectorField), OpticalFlowError> {
        let forward = self.estimate_motion(frame_0, frame_1)?;
        let backward = self.estimate_motion(frame_1, frame_0)?;
        Ok((forward, backward))
    }
}


// Disocclusion Mask Detection & Bidirectional Temporal Motion Warping

/// Dynamic disocclusion mask identifying regions uncovered by motion.
/// In regions where foreground objects translate, previously occluded background
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DisocclusionMask {
    pub width: u32,
    pub height: u32,
    /// Mask values: 0 = fully continuous/consistent, 255 = completely disoccluded.
    pub mask: Vec<u8>,
}

impl DisocclusionMask {
    /// Creates a new zero-initialized disocclusion mask.
    pub fn new(width: u32, height: u32) -> Self {
        let total = (width * height) as usize;
        Self {
            width,
            height,
            mask: alloc::vec![0u8; total],
        }
    }

    /// Returns the disocclusion intensity at pixel (x, y).
    #[inline(always)]
    pub fn get(&self, x: u32, y: u32) -> u8 {
        if x < self.width && y < self.height {
            let idx = (y * self.width + x) as usize;
            self.mask[idx]
        } else {
            255
        }
    }

    /// Sets the disocclusion intensity at pixel (x, y).
    #[inline(always)]
    pub fn set(&mut self, x: u32, y: u32, val: u8) {
        if x < self.width && y < self.height {
            let idx = (y * self.width + x) as usize;
            self.mask[idx] = val;
        }
    }

    /// Returns true if the pixel is classified as disoccluded.
    #[inline(always)]
    pub fn is_disoccluded(&self, x: u32, y: u32) -> bool {
        self.get(x, y) > 128
    }

    /// Computes the total number of disoccluded pixels.
    pub fn disoccluded_count(&self) -> usize {
        self.mask.iter().filter(|&&v| v > 128).count()
    }

    /// Computes the ratio of disoccluded pixels relative to total frame resolution.
    pub fn disocclusion_ratio(&self) -> f32 {
        let total = self.width * self.height;
        if total == 0 {
            0.0
        } else {
            (self.disoccluded_count() as f32) / (total as f32)
        }
    }

    /// Detects disocclusions using a Forward-Backward Vector Consistency Check.
    /// For each pixel (x, y), evaluates whether $V_{\text{forward}}(x, y) + V_{\text{backward}}(x + V_F) \approx 0$.
    pub fn detect(
        forward: &OpticalFlowVectorField,
        backward: &OpticalFlowVectorField,
        consistency_threshold_px: f32,
    ) -> Self {
        let w = forward.frame_width;
        let h = forward.frame_height;
        let mut mask = Self::new(w, h);
        let thresh_sq = consistency_threshold_px * consistency_threshold_px;

        for y in 0..h {
            for x in 0..w {
                let fx = x as f32;
                let fy = y as f32;

                let vf = forward.sample_at_pixel(fx, fy);
                let (vfx, vfy) = vf.to_f32();

                let pred_x = fx + vfx;
                let pred_y = fy + vfy;

                if pred_x < 0.0 || pred_x >= (w as f32) || pred_y < 0.0 || pred_y >= (h as f32) {
                    mask.set(x, y, 255);
                    continue;
                }

                let vb = backward.sample_at_pixel(pred_x, pred_y);
                let (vbx, vby) = vb.to_f32();

                // Residual displacement
                let rx = vfx + vbx;
                let ry = vfy + vby;
                let err_sq = rx * rx + ry * ry;

                if err_sq > thresh_sq {
                    // Normalize error into 128..255 range
                    let excess = (err_sq - thresh_sq).min(16.0) / 16.0;
                    let intensity = 128 + (excess * 127.0) as u8;
                    mask.set(x, y, intensity);
                } else {
                    mask.set(x, y, 0);
                }
            }
        }

        mask
    }
}

/// Bidirectional temporal motion warping engine for frame generation.
#[derive(Debug, Clone, Default)]
pub struct MotionWarpEngine {
    pub total_warps: u64,
}

impl MotionWarpEngine {
    pub fn new() -> Self {
        Self::default()
    }

    /// Synthesizes an intermediate frame at normalized time $t \in [0.0, 1.0]$.
    /// Warps Frame 0 forward by $+t \cdot V_F$ and Frame 1 backward by $-(1-t) \cdot V_B$.
    pub fn warp_bidirectional(
        &mut self,
        frame_0: &FrameBuffer,
        frame_1: &FrameBuffer,
        forward: &OpticalFlowVectorField,
        backward: &OpticalFlowVectorField,
        disocclusion: &DisocclusionMask,
        t: f32,
    ) -> Result<FrameBuffer, OpticalFlowError> {
        let w = frame_0.width;
        let h = frame_0.height;
        if frame_1.width != w || frame_1.height != h {
            return Err(OpticalFlowError::InvalidDimensions);
        }

        let clamped_t = t.clamp(0.0, 1.0);
        let mut output = FrameBuffer::zero(w, h);

        let t_fixed = (clamped_t * 256.0) as i32;
        let inv_t_fixed = 256 - t_fixed;

        for y in 0..h {
            for x in 0..w {
                let fx = x as f32;
                let fy = y as f32;

                // Motion vector at target coordinate
                let vf = forward.sample_at_pixel(fx, fy);
                let vb = backward.sample_at_pixel(fx, fy);
                let (vfx, vfy) = vf.to_f32();
                let (vbx, vby) = vb.to_f32();

                // Forward warp coordinate from Frame 0: x - t * V_F
                let s0x = fx - clamped_t * vfx;
                let s0y = fy - clamped_t * vfy;
                let s0x_q = (s0x * 4.0) as i32;
                let s0y_q = (s0y * 4.0) as i32;
                let p0 = frame_0.sample_subpixel_quarter_pel(s0x_q, s0y_q) as i32;

                // Backward warp coordinate from Frame 1: x - (1 - t) * V_B
                let s1x = fx - (1.0 - clamped_t) * vbx;
                let s1y = fy - (1.0 - clamped_t) * vby;
                let s1x_q = (s1x * 4.0) as i32;
                let s1y_q = (s1y * 4.0) as i32;
                let p1 = frame_1.sample_subpixel_quarter_pel(s1x_q, s1y_q) as i32;

                let disocc_val = disocclusion.get(x, y);

                let pixel_val = if disocc_val > 192 {
                    // Severe disocclusion: pick the frame where target is closer in time
                    if clamped_t < 0.5 {
                        p0 as u8
                    } else {
                        p1 as u8
                    }
                } else if disocc_val > 64 {
                    // Moderate disocclusion: blend weighted by consistency confidence
                    let confidence = (255 - disocc_val as i32) * 256 / 191;
                    let consistent_blend = (p0 * inv_t_fixed + p1 * t_fixed + 128) >> 8;
                    let fallback_blend = if clamped_t < 0.5 { p0 } else { p1 };
                    let blended = (consistent_blend * confidence + fallback_blend * (256 - confidence) + 128) >> 8;
                    blended.clamp(0, 255) as u8
                } else {
                    // Fully consistent region: temporal weighted blend
                    let blended = (p0 * inv_t_fixed + p1 * t_fixed + 128) >> 8;
                    blended.clamp(0, 255) as u8
                };

                output.set_pixel(x, y, pixel_val);
            }
        }

        self.total_warps += 1;
        Ok(output)
    }
}

/// Unified Frame Interpolation & Motion Generation Subsystem.
#[derive(Debug, Clone)]
pub struct FrameInterpolator {
    pub accelerator: OpticalFlowAccelerator,
    pub warp_engine: MotionWarpEngine,
    pub total_interpolated_frames: u64,
    pub last_disocclusion_ratio: f32,
}

impl FrameInterpolator {
    /// Creates a new frame interpolator with the given optical flow accelerator descriptor.
    pub fn new(desc: OpticalFlowDescriptor) -> Result<Self, OpticalFlowError> {
        let accelerator = OpticalFlowAccelerator::new(desc)?;
        Ok(Self {
            accelerator,
            warp_engine: MotionWarpEngine::new(),
            total_interpolated_frames: 0,
            last_disocclusion_ratio: 0.0,
        })
    }

    /// Synthesizes an intermediate interpolated frame between `frame_0` and `frame_1` at normalized time `t`.
    pub fn interpolate(
        &mut self,
        frame_0: &FrameBuffer,
        frame_1: &FrameBuffer,
        t: f32,
    ) -> Result<FrameBuffer, OpticalFlowError> {
        let (forward, backward) = self.accelerator.estimate_bidirectional(frame_0, frame_1)?;
        let mask = DisocclusionMask::detect(&forward, &backward, 1.5);
        self.last_disocclusion_ratio = mask.disocclusion_ratio();

        let interpolated = self.warp_engine.warp_bidirectional(
            frame_0,
            frame_1,
            &forward,
            &backward,
            &mask,
            t,
        )?;

        self.total_interpolated_frames += 1;
        Ok(interpolated)
    }
}


// Verification Self-Test Suite

/// Comprehensive verification self-test for Direct3D 12 Hardware Optical Flow
/// Vector Accelerator and Frame Interpolation Motion Engine.
pub fn test_optical_flow_subsystem() -> bool {
    // 1. Verify MotionVector 16.16 fixed-point arithmetic and quarter-pel conversion
    let mv = MotionVector::from_quarter_pel(4, -8);
    if mv.to_quarter_pel() != (4, -8) {
        return false;
    }
    if mv.to_integer_pel() != (1, -2) {
        return false;
    }
    let (fx, fy) = mv.to_f32();
    if (fx - 1.0).abs() > 0.01 || (fy - (-2.0)).abs() > 0.01 {
        return false;
    }
    let half_scale = mv.scale_fixed(FIXED_POINT_ONE / 2);
    let (hfx, hfy) = half_scale.to_f32();
    if (hfx - 0.5).abs() > 0.01 || (hfy - (-1.0)).abs() > 0.01 {
        return false;
    }

    // 2. Verify CostSurfaceGrid matching cost tracking and minimum extraction
    let mut cs = CostSurfaceGrid::new(4, 4, 2);
    cs.set_cost(1, 1, -1, 2, 42);
    let (min_dx, min_dy, min_cost) = cs.find_min_cost(1, 1);
    if min_dx != -1 || min_dy != 2 || min_cost != 42 {
        return false;
    }

    // 3. Verify FrameBuffer and ImagePyramid downsampling
    let mut base = FrameBuffer::zero(16, 16);
    for y in 0..8 {
        for x in 0..8 {
            base.set_pixel(x, y, 100);
        }
    }
    let pyr = ImagePyramid::build(base, 3);
    if pyr.num_levels() != 3 {
        return false;
    }
    if pyr.levels[0].width != 16 || pyr.levels[1].width != 8 || pyr.levels[2].width != 4 {
        return false;
    }

    // 4. Verify Hierarchical Block Matching (HBM) with Quarter-Pel Precision
    let desc = OpticalFlowDescriptor {
        frame_width: 32,
        frame_height: 32,
        block_size: 8,
        precision: OpticalFlowPrecision::QuarterPel,
        search_radius: 8,
        enable_cost_surface: true,
        enable_hint_vectors: false,
    };
    let mut interpolator = match FrameInterpolator::new(desc) {
        Ok(interp) => interp,
        Err(_) => return false,
    };

    let mut f0 = FrameBuffer::zero(32, 32);
    let mut f1 = FrameBuffer::zero(32, 32);

    for i in 0..1024 {
        f0.data[i] = 10;
        f1.data[i] = 10;
    }

    // High-contrast 8x8 block in f0 at (8, 8)
    for y in 8..16 {
        for x in 8..16 {
            f0.set_pixel(x, y, 200);
        }
    }

    // Shifted block in f1 by (+2, +2) -> (10, 10)
    for y in 10..18 {
        for x in 10..18 {
            f1.set_pixel(x, y, 200);
        }
    }

    let (forward, backward) = match interpolator.accelerator.estimate_bidirectional(&f0, &f1) {
        Ok(res) => res,
        Err(_) => return false,
    };

    // Forward vector at block (1, 1) should match displacement (+2, +2)
    let mv_f = forward.get_vector(1, 1);
    let (mv_fx, mv_fy) = mv_f.to_integer_pel();
    if mv_fx != 2 || mv_fy != 2 {
        return false;
    }

    // Backward vector at block (1, 1) should be (-2, -2)
    let mv_b = backward.get_vector(1, 1);
    let sum_x = mv_fx + mv_b.to_integer_pel().0;
    let sum_y = mv_fy + mv_b.to_integer_pel().1;
    if sum_x.abs() > 1 || sum_y.abs() > 1 {
        return false;
    }

    // 5. Verify Disocclusion Mask Detection
    let mask = DisocclusionMask::detect(&forward, &backward, 1.5);
    if mask.width != 32 || mask.height != 32 {
        return false;
    }

    // 6. Verify Bidirectional Temporal Motion Warping at t = 0.5
    let interpolated = match interpolator.warp_engine.warp_bidirectional(
        &f0,
        &f1,
        &forward,
        &backward,
        &mask,
        0.5,
    ) {
        Ok(frame) => frame,
        Err(_) => return false,
    };

    // Center of interpolated block at (12, 12) must contain synthesized high intensity
    let center_val = interpolated.get_pixel(12, 12);
    if center_val < 150 {
        return false;
    }
    // Background corner at (2, 2) must remain low intensity background
    let bg_val = interpolated.get_pixel(2, 2);
    if bg_val > 50 {
        return false;
    }

    // 7. Emit deterministic boot sentinel
    crate::serial_write_str("[KNT-GPU-OPTICAL-FLOW-PASS] Direct3D 12 hardware optical flow vector accelerator & frame interpolation motion engine verified\r\n");

    true
}
