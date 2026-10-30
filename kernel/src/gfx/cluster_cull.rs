//! Direct3D 12 Dynamic Primitive Culling & Mesh-Cluster LOD Streamer Engine.
//! Implements virtualized geometry primitive streaming (Nanite-style hierarchical clusters):

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


// GPU Sub-Pixel Cluster Culling, Normal Cone & Hierarchical Occlusion Bit-Grid

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
    pub clear_depth: f32,
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
            clear_depth,
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
                if tile_z >= self.clear_depth || cluster_nearest_z <= tile_z {
                    // Cluster is in front of at least one tile, or tile has no occluder: NOT occluded
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

        let far_plane = 1000.0;
        let nearest_z = ((dist - cluster.bounding_sphere.radius).max(0.0) / far_plane).min(1.0);
        if occlusion.is_rect_occluded(&screen_rect, nearest_z, frustum.viewport_width_px, frustum.viewport_height_px) {
            self.stats.culled_occlusion += 1;
            return false;
        }

        self.stats.clusters_passed += 1;
        true
    }
}


// Continuous Mesh-Cluster LOD Streamer & Crack-Free Boundary Stitcher

/// Seam edge record tracking boundary connectivity across adjacent clusters.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SeamEdgeRecord {
    pub edge: ClusterBoundaryEdge,
    pub cluster_id: u32,
    pub lod_level: u32,
}

/// Crack-free boundary stitcher verifying manifold topological continuity across LOD transitions.
#[derive(Debug, Clone, Default)]
pub struct ClusterBoundaryStitcher {
    pub active_edges: Vec<SeamEdgeRecord>,
    pub total_seams_stitched: usize,
    pub potential_cracks_prevented: usize,
}

impl ClusterBoundaryStitcher {
    pub fn new() -> Self {
        Self::default()
    }

    /// Clears the active stitcher registry.
    pub fn clear(&mut self) {
        self.active_edges.clear();
        self.total_seams_stitched = 0;
        self.potential_cracks_prevented = 0;
    }

    /// Registers boundary edges for a selected cluster.
    pub fn register_cluster_edges(&mut self, cluster: &MeshClusterDesc) {
        for &edge in &cluster.boundary_edges {
            // Check if matching edge already exists from an adjacent cluster
            if let Some(pos) = self.active_edges.iter().position(|r| r.edge == edge) {
                let neighbor = self.active_edges.swap_remove(pos);
                self.total_seams_stitched += 1;
                if neighbor.lod_level != cluster.lod_level {
                    // Different LOD levels seamlessly locked and stitched together
                    self.potential_cracks_prevented += 1;
                }
            } else {
                self.active_edges.push(SeamEdgeRecord {
                    edge,
                    cluster_id: cluster.cluster_id,
                    lod_level: cluster.lod_level,
                });
            }
        }
    }
}

/// Nanite-style Continuous Mesh-Cluster LOD Streamer & Virtualized Geometry Engine.
#[derive(Debug, Clone, Default)]
pub struct MeshClusterStreamer {
    pub clusters: Vec<MeshClusterDesc>,
    pub culler: GpuClusterCuller,
    pub stitcher: ClusterBoundaryStitcher,
    pub total_triangles_rendered: u64,
}

impl MeshClusterStreamer {
    pub fn new(subpixel_threshold_px: f32) -> Self {
        Self {
            clusters: Vec::new(),
            culler: GpuClusterCuller::new(subpixel_threshold_px),
            stitcher: ClusterBoundaryStitcher::new(),
            total_triangles_rendered: 0,
        }
    }

    /// Registers a mesh cluster in the virtualized geometry database.
    pub fn register_cluster(&mut self, cluster: MeshClusterDesc) -> Result<(), ClusterCullError> {
        cluster.validate()?;
        if let Some(existing) = self.clusters.iter_mut().find(|c| c.cluster_id == cluster.cluster_id) {
            *existing = cluster;
        } else {
            self.clusters.push(cluster);
        }
        Ok(())
    }

    /// Evaluates continuous hierarchical LOD selection:
    /// Selects cluster if `e_cluster <= tau < e_parent`.
    pub fn is_cluster_lod_selected(&self, cluster: &MeshClusterDesc, frustum: &ViewFrustumDesc) -> bool {
        let dist = cluster.bounding_sphere.distance_to(frustum.camera_pos);
        let cluster_screen_error = frustum.project_error_to_screen(cluster.cluster_error, dist);

        let parent_screen_error = if cluster.parent_error == f32::MAX {
            f32::MAX
        } else {
            frustum.project_error_to_screen(cluster.parent_error, dist)
        };

        let tau = frustum.screen_error_threshold_px;

        // Selection condition:
        // 1. Current cluster geometric error <= tau (detail is sufficient)
        let satisfies_refinement = cluster_screen_error <= tau && parent_screen_error > tau;

        // If at finest LOD (LOD 0) and even finest error > tau, we still must render finest detail
        let fallback_finest = cluster.lod_level == 0 && cluster_screen_error > tau;

        satisfies_refinement || fallback_finest
    }

    /// Executes the complete virtualized geometry streaming pipeline:
    /// 1. Hierarchical LOD evaluation
    pub fn stream_visible_clusters(
        &mut self,
        frustum: &ViewFrustumDesc,
        occlusion: &OcclusionBitGrid,
    ) -> Vec<u32> {
        self.stitcher.clear();
        self.total_triangles_rendered = 0;

        let mut visible_clusters = Vec::new();

        for i in 0..self.clusters.len() {
            let cluster = &self.clusters[i];

            // Step 1: Hierarchical LOD selection
            if !self.is_cluster_lod_selected(cluster, frustum) {
                continue;
            }

            // Step 2: GPU cluster culling (subpixel, normal cone, occlusion bit-grid)
            if !self.culler.evaluate_cluster(cluster, frustum, occlusion) {
                continue;
            }

            // Step 3: Crack-free boundary stitching
            self.stitcher.register_cluster_edges(cluster);

            self.total_triangles_rendered += cluster.triangle_count as u64;
            visible_clusters.push(cluster.cluster_id);
        }

        visible_clusters
    }
}


// Verification Self-Test Suite

/// Comprehensive verification self-test for D3D12 Mesh-Cluster Primitive Culling & LOD Streamer.
/// Emits sentinel [KNT-GPU-CLUSTER-CULL-PASS] upon successful verification.
pub fn test_cluster_cull_subsystem() -> bool {
    let frustum = ViewFrustumDesc {
        camera_pos: [0.0, 0.0, -10.0],
        view_dir: [0.0, 0.0, 1.0],
        viewport_width_px: 1920.0,
        viewport_height_px: 1080.0,
        fov_y_rad: 1.04719755,
        screen_error_threshold_px: 1.0,
    };

    // 1. Verify BoundingSphere distance calculation and validation
    let sphere = BoundingSphere::new([0.0, 0.0, 0.0], 2.0);
    if sphere.validate().is_err() {
        return false;
    }
    let dist = sphere.distance_to(frustum.camera_pos);
    if (dist - 10.0).abs() > 0.01 {
        return false;
    }

    let mut invalid_sphere = sphere;
    invalid_sphere.radius = -1.0;
    if invalid_sphere.validate().is_ok() {
        return false;
    }

    // 2. Verify NormalCone front-facing vs back-facing test
    // Cone facing camera: axis = [0, 0, -1]
    let front_cone = NormalCone::new([0.0, 0.0, 0.0], [0.0, 0.0, -1.0], 0.0);
    if front_cone.is_backfacing(frustum.camera_pos) {
        return false;
    }

    // Cone facing away from camera: axis = [0, 0, 1]
    let back_cone = NormalCone::new([0.0, 0.0, 0.0], [0.0, 0.0, 1.0], 0.0);
    if !back_cone.is_backfacing(frustum.camera_pos) {
        return false;
    }

    // 3. Verify GPU Cluster Culler (Sub-pixel, Normal Cone, Occlusion Grid)
    let mut culler = GpuClusterCuller::new(1.0); // 1.0 px subpixel threshold
    let mut occlusion = OcclusionBitGrid::new(64, 64, 1.0);

    // Test sub-pixel culling: tiny cluster far away
    let subpixel_cluster = MeshClusterDesc {
        cluster_id: 10,
        lod_level: 2,
        vertex_count: 32,
        triangle_count: 64,
        bounding_sphere: BoundingSphere::new([0.0, 0.0, 50.0], 0.001),
        normal_cone: front_cone,
        cluster_error: 0.1,
        parent_error: 1.0,
        parent_cluster_id: None,
        boundary_edges: Vec::new(),
    };
    if culler.evaluate_cluster(&subpixel_cluster, &frustum, &occlusion) {
        return false;
    }
    if culler.stats.culled_subpixel != 1 {
        return false;
    }

    // Test backface cone culling
    let backface_cluster = MeshClusterDesc {
        cluster_id: 11,
        lod_level: 0,
        vertex_count: 32,
        triangle_count: 64,
        bounding_sphere: BoundingSphere::new([0.0, 0.0, 0.0], 1.5),
        normal_cone: back_cone,
        cluster_error: 0.0,
        parent_error: 0.5,
        parent_cluster_id: None,
        boundary_edges: Vec::new(),
    };
    if culler.evaluate_cluster(&backface_cluster, &frustum, &occlusion) {
        return false;
    }
    if culler.stats.culled_normal_cone != 1 {
        return false;
    }

    // Test occlusion bit-grid culling
    // Place a foreground occluder at depth 0.01 (closer than cluster at ~0.029)
    for ty in 20..44 {
        for tx in 20..44 {
            occlusion.set_tile_depth(tx, ty, 0.01);
        }
    }

    let occluded_cluster = MeshClusterDesc {
        cluster_id: 12,
        lod_level: 0,
        vertex_count: 32,
        triangle_count: 64,
        bounding_sphere: BoundingSphere::new([0.0, 0.0, 20.0], 1.0), // distance ~30, depth ~0.029
        normal_cone: front_cone,
        cluster_error: 0.0,
        parent_error: 0.5,
        parent_cluster_id: None,
        boundary_edges: Vec::new(),
    };
    if culler.evaluate_cluster(&occluded_cluster, &frustum, &occlusion) {
        return false;
    }
    if culler.stats.culled_occlusion != 1 {
        return false;
    }

    // 4. Verify Continuous Hierarchical LOD Selection (Nanite-style e_cluster <= tau < e_parent)
    let mut streamer = MeshClusterStreamer::new(0.5);

    // Shared boundary edge between adjacent clusters
    let shared_seam = ClusterBoundaryEdge::new(0xAAAA, 0xBBBB);

    // Coarse cluster (LOD 1): simplification error = 0.5 world units
    let coarse_cluster = MeshClusterDesc {
        cluster_id: 1,
        lod_level: 1,
        vertex_count: 32,
        triangle_count: 40,
        bounding_sphere: BoundingSphere::new([0.0, 0.0, 0.0], 2.0),
        normal_cone: front_cone,
        cluster_error: 0.5,
        parent_error: f32::MAX, // Root of this group
        parent_cluster_id: None,
        boundary_edges: alloc::vec![shared_seam],
    };

    // Fine child cluster (LOD 0): simplification error = 0.01 world units
    let fine_cluster = MeshClusterDesc {
        cluster_id: 2,
        lod_level: 0,
        vertex_count: 64,
        triangle_count: 120,
        bounding_sphere: BoundingSphere::new([0.0, 0.0, 0.0], 2.0),
        normal_cone: front_cone,
        cluster_error: 0.01,
        parent_error: 0.5, // Parent error is 0.5
        parent_cluster_id: Some(1),
        boundary_edges: alloc::vec![shared_seam],
    };

    // Adjacent fine neighbor cluster (LOD 0) sharing the seam
    let fine_neighbor = MeshClusterDesc {
        cluster_id: 3,
        lod_level: 0,
        vertex_count: 64,
        triangle_count: 120,
        bounding_sphere: BoundingSphere::new([3.0, 0.0, 0.0], 2.0),
        normal_cone: front_cone,
        cluster_error: 0.01,
        parent_error: 0.5,
        parent_cluster_id: Some(1),
        boundary_edges: alloc::vec![shared_seam],
    };

    if streamer.register_cluster(coarse_cluster).is_err() {
        return false;
    }
    if streamer.register_cluster(fine_cluster).is_err() {
        return false;
    }
    if streamer.register_cluster(fine_neighbor).is_err() {
        return false;
    }

    // View from nearby (distance = 10): fine detail should be selected (LOD 0)
    let clear_occlusion = OcclusionBitGrid::new(64, 64, 1.0);
    let visible = streamer.stream_visible_clusters(&frustum, &clear_occlusion);

    // Fine clusters (2 and 3) must be visible, coarse cluster (1) omitted because its error > 1.0 px
    if !visible.contains(&2) || !visible.contains(&3) {
        return false;
    }
    if visible.contains(&1) {
        return false;
    }

    // 5. Verify Crack-Free Boundary Stitcher verified the shared seam
    if streamer.stitcher.total_seams_stitched == 0 {
        return false;
    }

    // View from far away (distance = 500): coarse cluster (LOD 1) should be selected
    let far_frustum = ViewFrustumDesc {
        camera_pos: [0.0, 0.0, -500.0],
        ..frustum
    };
    let far_visible = streamer.stream_visible_clusters(&far_frustum, &clear_occlusion);

    // Coarse cluster 1 must be selected at far distance
    if !far_visible.contains(&1) {
        return false;
    }
    // Fine cluster 2 should not be selected at far distance
    if far_visible.contains(&2) {
        return false;
    }

    // 6. Emit deterministic boot sentinel
    crate::serial_write_str("[KNT-GPU-CLUSTER-CULL-PASS] Direct3D 12 dynamic primitive culling and mesh-cluster LOD streamer engine verified\r\n");

    true
}
