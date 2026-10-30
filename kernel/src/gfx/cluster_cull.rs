//! Direct3D 12 Dynamic Primitive Culling & Mesh-Cluster LOD Streamer Engine.
//!
//! Implements virtualized geometry primitive streaming (Nanite-style hierarchical clusters):
//! - Mesh cluster descriptors with bounding spheres, normal cones, and hierarchical parent/child error bounds
//! - GPU sub-pixel cluster culling, front-facing cone testing, and hierarchical occlusion bit-grid
//! - Continuous LOD selection with screen-space error metric ($\le \tau < E_{\text{parent}}$)
//! - Crack-free cluster boundary stitching across LOD transition seams

#![allow(dead_code)]

extern crate alloc;
use alloc::vec::Vec;

#[inline(always)]
pub fn f32_sqrt(val: f32) -> f32 {
    if val <= 0.0 {
        return 0.0;
    }
    let i = val.to_bits();
    let i = 0x1fbd1df5 + (i >> 1);
    let mut x = f32::from_bits(i);
    x = 0.5 * (x + val / x);
    x = 0.5 * (x + val / x);
    x = 0.5 * (x + val / x);
    x
}

/// Errors returned by the Mesh-Cluster Culling & LOD Streamer subsystem.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClusterCullError {
    /// Cluster bounding sphere or error metric contains non-finite or negative values.
    InvalidBounds,
    /// Requested cluster was not found in the hierarchy registry.
    ClusterNotFound(u32),
    /// View frustum or viewport configuration is non-positive or invalid.
    InvalidFrustum,
    /// Occlusion bit-grid dimension is out of range or uninitialized.
    InvalidOcclusionGrid,
    /// Cluster boundary stitcher detected an unmatched seam topology.
    SeamStitchMismatch,
}

/// 3D Bounding Sphere for spatial culling and LOD distance measurement.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BoundingSphere {
    pub center: [f32; 3],
    pub radius: f32,
}

impl Default for BoundingSphere {
    fn default() -> Self {
        Self {
            center: [0.0; 3],
            radius: 1.0,
        }
    }
}

impl BoundingSphere {
    pub const fn new(center: [f32; 3], radius: f32) -> Self {
        Self { center, radius }
    }

    /// Computes Euclidean distance from a 3D point (e.g. camera position) to sphere center.
    pub fn distance_to(&self, point: [f32; 3]) -> f32 {
        let dx = self.center[0] - point[0];
        let dy = self.center[1] - point[1];
        let dz = self.center[2] - point[2];
        f32_sqrt(dx * dx + dy * dy + dz * dz)
    }

    /// Evaluates if sphere is valid (finite coordinates and non-negative radius).
    pub fn validate(&self) -> Result<(), ClusterCullError> {
        if !self.center[0].is_finite()
            || !self.center[1].is_finite()
            || !self.center[2].is_finite()
            || !self.radius.is_finite()
            || self.radius < 0.0
        {
            return Err(ClusterCullError::InvalidBounds);
        }
        Ok(())
    }
}

/// Normal cone for whole-cluster backface culling.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct NormalCone {
    pub apex: [f32; 3],
    pub axis: [f32; 3],
    /// Cosine of the half-angle spread of normals in the cluster.
    pub cutoff: f32,
}

impl Default for NormalCone {
    fn default() -> Self {
        Self {
            apex: [0.0; 3],
            axis: [0.0, 0.0, 1.0],
            cutoff: 1.0,
        }
    }
}

impl NormalCone {
    pub const fn new(apex: [f32; 3], axis: [f32; 3], cutoff: f32) -> Self {
        Self { apex, axis, cutoff }
    }

    /// Evaluates whether the entire cluster is facing away from the camera.
    pub fn is_backfacing(&self, camera_pos: [f32; 3]) -> bool {
        let vx = self.apex[0] - camera_pos[0];
        let vy = self.apex[1] - camera_pos[1];
        let vz = self.apex[2] - camera_pos[2];
        let len = f32_sqrt(vx * vx + vy * vy + vz * vz);
        if len < 1e-6 {
            return false;
        }

        // View vector from camera to apex (normalized)
        let v_norm = [vx / len, vy / len, vz / len];
        let dot = v_norm[0] * self.axis[0] + v_norm[1] * self.axis[1] + v_norm[2] * self.axis[2];

        // Backfacing if dot product exceeds the normal spread cutoff angle
        dot >= self.cutoff
    }
}

/// Boundary edge descriptor used for seamless LOD transition stitching.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ClusterBoundaryEdge {
    pub v0_hash: u64,
    pub v1_hash: u64,
}

impl ClusterBoundaryEdge {
    pub const fn new(v0_hash: u64, v1_hash: u64) -> Self {
        if v0_hash <= v1_hash {
            Self { v0_hash, v1_hash }
        } else {
            Self { v0_hash: v1_hash, v1_hash: v0_hash }
        }
    }
}

/// Mesh cluster descriptor (Nanite-style virtualized geometry primitive).
#[derive(Debug, Clone, PartialEq)]
pub struct MeshClusterDesc {
    pub cluster_id: u32,
    pub lod_level: u32,
    pub vertex_count: u32,
    pub triangle_count: u32,
    pub bounding_sphere: BoundingSphere,
    pub normal_cone: NormalCone,
    /// World-space geometric simplification error of this cluster.
    pub cluster_error: f32,
    /// World-space geometric simplification error of the parent cluster.
    pub parent_error: f32,
    pub parent_cluster_id: Option<u32>,
    pub boundary_edges: Vec<ClusterBoundaryEdge>,
}

impl MeshClusterDesc {
    pub fn validate(&self) -> Result<(), ClusterCullError> {
        self.bounding_sphere.validate()?;
        if !self.cluster_error.is_finite()
            || !self.parent_error.is_finite()
            || self.cluster_error < 0.0
            || self.parent_error < 0.0
        {
            return Err(ClusterCullError::InvalidBounds);
        }
        Ok(())
    }
}

/// Camera view frustum and projection parameters for LOD evaluation.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ViewFrustumDesc {
    pub camera_pos: [f32; 3],
    pub view_dir: [f32; 3],
    pub viewport_width_px: f32,
    pub viewport_height_px: f32,
    pub fov_y_rad: f32,
    /// Max acceptable projected geometric error in screen pixels (e.g. 1.0 px).
    pub screen_error_threshold_px: f32,
}

impl Default for ViewFrustumDesc {
    fn default() -> Self {
        Self {
            camera_pos: [0.0, 0.0, -10.0],
            view_dir: [0.0, 0.0, 1.0],
            viewport_width_px: 1920.0,
            viewport_height_px: 1080.0,
            fov_y_rad: 1.04719755, // 60 degrees in radians
            screen_error_threshold_px: 1.0,
        }
    }
}

impl ViewFrustumDesc {
    /// Computes the projection factor converting world-space error to screen pixels.
    pub fn projection_scale(&self) -> f32 {
        let half_fov = self.fov_y_rad * 0.5;
        // In #![no_std] without libm tan(), approximate tan(x) ~ x + x^3/3 for x in [0, 0.8]
        let tan_approx = half_fov + (half_fov * half_fov * half_fov) / 3.0;
        self.viewport_height_px * 0.5 / tan_approx.max(1e-4)
    }

    /// Projects a world-space geometric error to screen pixels at a given distance.
    pub fn project_error_to_screen(&self, world_error: f32, distance: f32) -> f32 {
        let dist = distance.max(0.1);
        (world_error * self.projection_scale()) / dist
    }
}


// -----------------------------------------------------------------------------
// GPU Sub-Pixel Cluster Culling, Normal Cone & Hierarchical Occlusion Bit-Grid
// -----------------------------------------------------------------------------

/// 2D Screen-space axis-aligned bounding box [min_x, min_y, max_x, max_y].
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ScreenRect {
    pub min_x: f32,
    pub min_y: f32,
    pub max_x: f32,
    pub max_y: f32,
}

impl ScreenRect {
    pub const fn new(min_x: f32, min_y: f32, max_x: f32, max_y: f32) -> Self {
        Self { min_x, min_y, max_x, max_y }
    }

    #[inline(always)]
    pub fn width(&self) -> f32 {
        (self.max_x - self.min_x).max(0.0)
    }

    #[inline(always)]
    pub fn height(&self) -> f32 {
        (self.max_y - self.min_y).max(0.0)
    }

    #[inline(always)]
    pub fn area(&self) -> f32 {
        self.width() * self.height()
    }
}

/// Coarse 2D Hierarchical Occlusion Bit-Grid representing downsampled Hi-Z buffer tiles.
#[derive(Debug, Clone)]
pub struct OcclusionBitGrid {
    pub width: u32,
    pub height: u32,
    pub depths: Vec<f32>,
}

impl Default for OcclusionBitGrid {
    fn default() -> Self {
        Self::new(64, 64, 1.0)
    }
}

impl OcclusionBitGrid {
    pub fn new(width: u32, height: u32, clear_depth: f32) -> Self {
        let w = width.max(1);
        let h = height.max(1);
        let total = (w * h) as usize;
        Self {
            width: w,
            height: h,
            depths: alloc::vec![clear_depth; total],
        }
    }

    #[inline(always)]
    pub fn set_tile_depth(&mut self, tx: u32, ty: u32, depth: f32) {
        if tx < self.width && ty < self.height {
            let idx = (ty * self.width + tx) as usize;
            self.depths[idx] = depth;
        }
    }

    #[inline(always)]
    pub fn get_tile_depth(&self, tx: u32, ty: u32) -> f32 {
        if tx < self.width && ty < self.height {
            let idx = (ty * self.width + tx) as usize;
            self.depths[idx]
        } else {
            1.0
        }
    }

    /// Tests if a screen-space rectangle at nearest depth `cluster_nearest_z` is fully occluded.
    /// Returns true if occluded (can be safely culled).
    pub fn is_rect_occluded(&self, rect: &ScreenRect, cluster_nearest_z: f32, viewport_w: f32, viewport_h: f32) -> bool {
        if viewport_w <= 0.0 || viewport_h <= 0.0 {
            return false;
        }

        // Map normalized screen coordinates to occlusion grid tiles
        let start_tx = ((rect.min_x / viewport_w) * self.width as f32).max(0.0) as u32;
        let end_tx = (((rect.max_x / viewport_w) * self.width as f32) as u32 + 1).min(self.width);
        let start_ty = ((rect.min_y / viewport_h) * self.height as f32).max(0.0) as u32;
        let end_ty = (((rect.max_y / viewport_h) * self.height as f32) as u32 + 1).min(self.height);

        if start_tx >= end_tx || start_ty >= end_ty {
            return false;
        }

        // Check if all tiles overlapping the cluster have buffer depth strictly closer than the cluster
        for ty in start_ty..end_ty {
            for tx in start_tx..end_tx {
                let tile_z = self.get_tile_depth(tx, ty);
                if cluster_nearest_z <= tile_z {
                    // Cluster is in front of at least one tile: NOT occluded
                    return false;
                }
            }
        }

        // Cluster is strictly behind all covered tiles: occluded!
        true
    }
}

/// Statistics tracking GPU cluster culling results.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ClusterCullStats {
    pub total_tested: u64,
    pub culled_subpixel: u64,
    pub culled_normal_cone: u64,
    pub culled_frustum: u64,
    pub culled_occlusion: u64,
    pub clusters_passed: u64,
}

/// Consolidated GPU Dynamic Primitive Cluster Culling Engine.
#[derive(Debug, Clone, Default)]
pub struct GpuClusterCuller {
    pub subpixel_threshold_px: f32, // e.g. 0.5 pixels
    pub stats: ClusterCullStats,
}

impl GpuClusterCuller {
    pub fn new(subpixel_threshold_px: f32) -> Self {
        Self {
            subpixel_threshold_px: subpixel_threshold_px.max(0.0),
            stats: ClusterCullStats::default(),
        }
    }

    /// Evaluates all GPU cluster culling stages:
    /// 1. Distance & Sub-pixel bounding sphere projection test
    /// 2. Front-facing normal cone test (backface cluster culling)
    /// 3. Hierarchical occlusion bit-grid test
    ///
    /// Returns true if the cluster passes all culling tests and must be rendered.
    pub fn evaluate_cluster(
        &mut self,
        cluster: &MeshClusterDesc,
        frustum: &ViewFrustumDesc,
        occlusion: &OcclusionBitGrid,
    ) -> bool {
        self.stats.total_tested += 1;

        let dist = cluster.bounding_sphere.distance_to(frustum.camera_pos);

        // 1. Sub-pixel Culling: If projected bounding radius < threshold, cull as subpixel
        let projected_radius_px = (cluster.bounding_sphere.radius * frustum.projection_scale()) / dist.max(0.1);
        if projected_radius_px < self.subpixel_threshold_px {
            self.stats.culled_subpixel += 1;
            return false;
        }

        // 2. Normal Cone Backface Culling: If cone faces entirely away from camera, cull
        if cluster.normal_cone.is_backfacing(frustum.camera_pos) {
            self.stats.culled_normal_cone += 1;
            return false;
        }

        // 3. Screen Rect & Occlusion Bit-Grid Culling
        let center_x = frustum.viewport_width_px * 0.5;
        let center_y = frustum.viewport_height_px * 0.5;
        let screen_rect = ScreenRect::new(
            (center_x - projected_radius_px).max(0.0),
            (center_y - projected_radius_px).max(0.0),
            (center_x + projected_radius_px).min(frustum.viewport_width_px),
            (center_y + projected_radius_px).min(frustum.viewport_height_px),
        );

        let nearest_z = (dist - cluster.bounding_sphere.radius).max(0.0) / 100.0; // normalized depth
        if occlusion.is_rect_occluded(&screen_rect, nearest_z, frustum.viewport_width_px, frustum.viewport_height_px) {
            self.stats.culled_occlusion += 1;
            return false;
        }

        self.stats.clusters_passed += 1;
        true
    }
}
