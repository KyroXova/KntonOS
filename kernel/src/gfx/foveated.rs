//! Eye-Tracked and Center-Weighted Foveated Rasterizer.
//!
//! Generates high-density screen-space Shading Rate Images (SRI) dynamically based on
//! eye-tracking gaze vectors or center-weighted focus heuristics.
//!
//! Preserves full 1x1 shading in the central foveal zone, steps down to 2x2 in the
//! parafoveal zone, and drops to 4x4 in the peripheral zone. This drastically reduces
//! pixel shader invocations (typically 40–60% or more), saving substantial memory
//! bandwidth, battery power, and thermal headroom on handheld gaming consoles.

use crate::gfx::vrs::{ShadingRate, ShadingRateImage, TileGranularity, VrsError};

/// Foveal visual zone classification for coarse shading.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FoveatedZone {
    /// Central foveal vision: highest visual acuity, full 1x1 shading.
    Foveal,
    /// Parafoveal vision: medium acuity, 2x2 coarse shading.
    Parafoveal,
    /// Peripheral vision: low acuity, 4x4 maximum coarse shading.
    Peripheral,
}

impl FoveatedZone {
    /// Maps visual zone to corresponding Direct3D 12 shading rate.
    #[inline]
    pub const fn default_shading_rate(self) -> ShadingRate {
        match self {
            Self::Foveal => ShadingRate::Rate1x1,
            Self::Parafoveal => ShadingRate::Rate2x2,
            Self::Peripheral => ShadingRate::Rate4x4,
        }
    }
}

/// Gaze tracking coordinates and smoothing state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GazePoint {
    /// Screen-space X coordinate in pixels.
    pub x: u32,
    /// Screen-space Y coordinate in pixels.
    pub y: u32,
    /// Whether eye tracking is actively locked on the pupil.
    pub is_valid: bool,
}

impl GazePoint {
    /// Creates a valid gaze point at `(x, y)`.
    pub const fn new(x: u32, y: u32) -> Self {
        Self { x, y, is_valid: true }
    }

    /// Creates an invalid / lost-tracking gaze point.
    pub const fn invalid() -> Self {
        Self { x: 0, y: 0, is_valid: false }
    }
}

/// Configuration parameters for the foveated rasterizer.
#[derive(Debug, Clone, Copy)]
pub struct FoveatedConfig {
    /// Viewport width in pixels.
    pub viewport_width: u32,
    /// Viewport height in pixels.
    pub viewport_height: u32,
    /// Tile granularity for the generated Shading Rate Image.
    pub granularity: TileGranularity,
    /// Radius of the central foveal zone (1x1 shading) in pixels.
    pub foveal_radius: u32,
    /// Outer radius of the parafoveal zone (2x2 shading) in pixels.
    pub parafoveal_radius: u32,
    /// Shading rate for the central foveal zone.
    pub foveal_rate: ShadingRate,
    /// Shading rate for the parafoveal zone.
    pub parafoveal_rate: ShadingRate,
    /// Shading rate for the peripheral zone.
    pub peripheral_rate: ShadingRate,
    /// Saccade smoothing weight (0 = instantaneous raw gaze, 1..99 = exponential smoothing).
    pub smoothing_factor_pct: u32,
}

impl Default for FoveatedConfig {
    fn default() -> Self {
        Self {
            viewport_width: 1920,
            viewport_height: 1080,
            granularity: TileGranularity::Tile16x16,
            // Designed for typical handheld displays (~40-60% shader invocation reduction)
            foveal_radius: 280,
            parafoveal_radius: 580,
            foveal_rate: ShadingRate::Rate1x1,
            parafoveal_rate: ShadingRate::Rate2x2,
            peripheral_rate: ShadingRate::Rate4x4,
            smoothing_factor_pct: 35,
        }
    }
}

impl FoveatedConfig {
    /// Creates a handheld gaming preset (e.g. 1280x800, 16x16 tiles, calibrated for 40-60% savings).
    pub fn handheld_preset() -> Self {
        Self {
            viewport_width: 1280,
            viewport_height: 800,
            granularity: TileGranularity::Tile16x16,
            foveal_radius: 200,
            parafoveal_radius: 420,
            foveal_rate: ShadingRate::Rate1x1,
            parafoveal_rate: ShadingRate::Rate2x2,
            peripheral_rate: ShadingRate::Rate4x4,
            smoothing_factor_pct: 30,
        }
    }
}

/// Eye-tracked and center-weighted foveated rasterizer engine.
#[derive(Debug)]
pub struct FoveatedRasterizer {
    /// Active configuration.
    config: FoveatedConfig,
    /// Current smoothed gaze focus point.
    current_gaze: GazePoint,
    /// Raw un-smoothed gaze focus point from sensor.
    raw_gaze: GazePoint,
    /// Number of gaze updates processed.
    gaze_updates_count: u64,
    /// Saccade detection count (rapid gaze position transitions).
    saccade_count: u32,
}

impl FoveatedRasterizer {
    /// Creates a new FoveatedRasterizer with the specified configuration.
    pub fn new(config: FoveatedConfig) -> Self {
        let center_x = config.viewport_width / 2;
        let center_y = config.viewport_height / 2;
        let default_gaze = GazePoint::new(center_x, center_y);

        Self {
            config,
            current_gaze: default_gaze,
            raw_gaze: default_gaze,
            gaze_updates_count: 0,
            saccade_count: 0,
        }
    }

    /// Viewport configuration.
    #[inline]
    pub fn config(&self) -> &FoveatedConfig {
        &self.config
    }

    /// Mutably access configuration.
    #[inline]
    pub fn config_mut(&mut self) -> &mut FoveatedConfig {
        &mut self.config
    }

    /// Returns the current smoothed gaze focal coordinate.
    #[inline]
    pub fn current_gaze(&self) -> (u32, u32) {
        (self.current_gaze.x, self.current_gaze.y)
    }

    /// Returns whether the gaze point is valid.
    #[inline]
    pub fn is_gaze_valid(&self) -> bool {
        self.current_gaze.is_valid
    }

    /// Total saccades detected.
    #[inline]
    pub fn saccade_count(&self) -> u32 {
        self.saccade_count
    }

    /// Resets the gaze position back to the exact viewport center (center-weighted fallback).
    pub fn center_gaze(&mut self) {
        let cx = self.config.viewport_width / 2;
        let cy = self.config.viewport_height / 2;
        self.current_gaze = GazePoint::new(cx, cy);
        self.raw_gaze = self.current_gaze;
    }

    /// Updates eye-tracking gaze focus coordinates with sensor validation and saccade smoothing.
    pub fn update_gaze(&mut self, raw_x: u32, raw_y: u32) {
        self.gaze_updates_count += 1;

        // Clamp to viewport boundaries
        let clamped_x = core::cmp::min(raw_x, self.config.viewport_width.saturating_sub(1));
        let clamped_y = core::cmp::min(raw_y, self.config.viewport_height.saturating_sub(1));

        self.raw_gaze = GazePoint::new(clamped_x, clamped_y);

        // Detect saccades: Euclidean distance delta > 150 pixels in a single sample
        let dx = (clamped_x as i64) - (self.current_gaze.x as i64);
        let dy = (clamped_y as i64) - (self.current_gaze.y as i64);
        let dist_sq = dx * dx + dy * dy;
        let is_saccade = dist_sq > (150 * 150);

        if is_saccade {
            self.saccade_count += 1;
            // Saccade snap: snap gaze quickly to follow fast eye movement without lagging
            self.current_gaze = self.raw_gaze;
        } else {
            // Exponential moving average smoothing: smoothed = alpha * raw + (1 - alpha) * prev
            let alpha = 100 - core::cmp::min(self.config.smoothing_factor_pct, 95);
            let smooth_x = (clamped_x as u64 * alpha as u64 + self.current_gaze.x as u64 * (100 - alpha as u64)) / 100;
            let smooth_y = (clamped_y as u64 * alpha as u64 + self.current_gaze.y as u64 * (100 - alpha as u64)) / 100;
            self.current_gaze = GazePoint::new(smooth_x as u32, smooth_y as u32);
        }
    }

    /// Marks eye tracking as lost, immediately reverting to center-weighted focus.
    pub fn lose_tracking(&mut self) {
        self.center_gaze();
        self.current_gaze.is_valid = false;
    }

    /// Classifies a screen pixel coordinate `(x, y)` into its visual foveated zone.
    pub fn classify_pixel(&self, x: u32, y: u32) -> FoveatedZone {
        let (focal_x, focal_y) = self.current_gaze();
        let dx = (x as i64) - (focal_x as i64);
        let dy = (y as i64) - (focal_y as i64);
        let dist_sq = (dx * dx + dy * dy) as u64;

        let foveal_r = self.config.foveal_radius as u64;
        let parafoveal_r = self.config.parafoveal_radius as u64;

        if dist_sq <= foveal_r * foveal_r {
            FoveatedZone::Foveal
        } else if dist_sq <= parafoveal_r * parafoveal_r {
            FoveatedZone::Parafoveal
        } else {
            FoveatedZone::Peripheral
        }
    }

    /// Generates a screen-space Shading Rate Image (SRI) tile map matching current gaze.
    pub fn generate_shading_rate_image(&self) -> Result<ShadingRateImage, VrsError> {
        let tile_size = self.config.granularity.pixel_size();
        let mut sri = ShadingRateImage::new(
            self.config.viewport_width,
            self.config.viewport_height,
            self.config.granularity,
            self.config.peripheral_rate,
        )?;

        let tiles_x = sri.tiles_x();
        let tiles_y = sri.tiles_y();

        for ty in 0..tiles_y {
            for tx in 0..tiles_x {
                // Sample at tile center
                let center_x = tx * tile_size + (tile_size / 2);
                let center_y = ty * tile_size + (tile_size / 2);

                let zone = self.classify_pixel(center_x, center_y);
                let rate = match zone {
                    FoveatedZone::Foveal => self.config.foveal_rate,
                    FoveatedZone::Parafoveal => self.config.parafoveal_rate,
                    FoveatedZone::Peripheral => self.config.peripheral_rate,
                };
                sri.set_tile_rate(tx, ty, rate);
            }
        }

        Ok(sri)
    }

    /// Simulates a full-frame rasterization pass across the viewport using the generated SRI.
    ///
    /// Returns a tuple: `(total_pixels, shader_invocations, reduction_percentage)`.
    pub fn simulate_frame_rasterization(&self) -> Result<(u64, u64, u32), VrsError> {
        let sri = self.generate_shading_rate_image()?;
        let tile_size = self.config.granularity.pixel_size();

        let mut total_pixels: u64 = 0;
        let mut total_invocations: u64 = 0;

        for ty in 0..sri.tiles_y() {
            for tx in 0..sri.tiles_x() {
                // Determine actual pixel bounds of this tile (handling edge tile clipping)
                let px_start = tx * tile_size;
                let py_start = ty * tile_size;
                let px_end = core::cmp::min(px_start + tile_size, self.config.viewport_width);
                let py_end = core::cmp::min(py_start + tile_size, self.config.viewport_height);

                let actual_tile_pixels = ((px_end - px_start) * (py_end - py_start)) as u64;
                if actual_tile_pixels == 0 {
                    continue;
                }

                let rate = sri.get_tile_rate(tx, ty).unwrap_or(ShadingRate::Rate1x1);
                let coarse_area = rate.pixel_area() as u64;
                let invocations = (actual_tile_pixels + coarse_area - 1) / coarse_area;

                total_pixels += actual_tile_pixels;
                total_invocations += invocations;
            }
        }

        let reduction_pct = if total_pixels > 0 && total_invocations < total_pixels {
            ((total_pixels - total_invocations) * 100 / total_pixels) as u32
        } else {
            0
        };

        Ok((total_pixels, total_invocations, reduction_pct))
    }

    /// Estimates battery power reduction in milliwatts based on shader invocation savings.
    ///
    /// Given a baseline GPU dynamic shader power in mW (e.g. 6000 mW on a 15W handheld APU),
    /// estimates active power savings assuming dynamic shader power scales linearly with invocations.
    pub fn estimate_power_savings_mw(&self, baseline_gpu_power_mw: u32, reduction_pct: u32) -> u32 {
        (baseline_gpu_power_mw as u64 * reduction_pct as u64 / 100) as u32
    }

    /// Estimates memory bandwidth savings in megabytes per second (MB/s).
    pub fn estimate_bandwidth_savings_mbs(&self, baseline_bandwidth_mbs: u32, reduction_pct: u32) -> u32 {
        (baseline_bandwidth_mbs as u64 * reduction_pct as u64 / 100) as u32
    }
}
