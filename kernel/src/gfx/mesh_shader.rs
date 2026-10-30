//! Direct3D 12 Mesh Shader & Amplification Shader Primitive Pipeline Engine.
//!
//! Replaces legacy fixed-function Input Assembler and VS/HS/DS/GS geometry stages
//! with compute-like Amplification (Task) Shaders and Mesh Shaders (AS/MS).
//!
//! Features:
//! - Meshlet primitive clustering (up to 64 vertices, 126 triangles per meshlet).
//! - Meshlet spatial bounding spheres and normal cones for early frustum and backface culling.
//! - Amplification Shader threadgroup dispatch and payload passing.
//! - Mesh Shader primitive expansion and output vertex/topology streaming directly to the rasterizer.

use alloc::vec::Vec;

/// Direct3D 12 maximum allowable output vertices per meshlet (D3D12_MESH_SHADER_MAX_VERTICES).
pub const MAX_MESHLET_VERTICES: usize = 64;

/// Direct3D 12 maximum allowable output primitives per meshlet (D3D12_MESH_SHADER_MAX_PRIMITIVES).
pub const MAX_MESHLET_PRIMITIVES: usize = 126;

/// Direct3D 12 maximum allowable amplification shader payload size in bytes (16 KiB limit).
pub const MAX_PAYLOAD_SIZE_BYTES: usize = 16384;

/// Errors returned by the Mesh Shader primitive pipeline engine.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MeshShaderError {
    /// Meshlet output vertex count exceeds the maximum limit (64).
    ExceededMaxVertices(u32),
    /// Meshlet output primitive count exceeds the maximum limit (126).
    ExceededMaxPrimitives(u32),
    /// Amplification shader payload size exceeds the 16 KiB limit.
    ExceededMaxPayload(usize),
    /// Threadgroup dimension X, Y, or Z is zero or exceeds limits.
    InvalidThreadgroupDimension,
    /// Pipeline state configuration is invalid.
    InvalidPipelineConfig,
    /// Output buffer overflow during vertex or primitive streaming.
    BufferOverflow,
}

/// 3D vector for geometry and culling calculations.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
#[repr(C)]
pub struct Vec3 {
    pub x: f32,
    pub y: f32,
    pub z: f32,
}

impl Vec3 {
    pub const fn new(x: f32, y: f32, z: f32) -> Self {
        Self { x, y, z }
    }

    #[inline]
    pub fn dot(self, other: Self) -> f32 {
        self.x * other.x + self.y * other.y + self.z * other.z
    }

    #[inline]
    pub fn length_squared(self) -> f32 {
        self.dot(self)
    }

    #[inline]
    pub fn sub(self, other: Self) -> Self {
        Self {
            x: self.x - other.x,
            y: self.y - other.y,
            z: self.z - other.z,
        }
    }

    #[inline]
    pub fn add(self, other: Self) -> Self {
        Self {
            x: self.x + other.x,
            y: self.y + other.y,
            z: self.z + other.z,
        }
    }
}

/// Bounding sphere enclosing all vertices of a meshlet for frustum culling.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
#[repr(C)]
pub struct MeshletBoundingSphere {
    /// Center of the bounding sphere in object / world space.
    pub center: Vec3,
    /// Radius of the bounding sphere.
    pub radius: f32,
}

impl MeshletBoundingSphere {
    pub const fn new(center: Vec3, radius: f32) -> Self {
        Self { center, radius }
    }

    /// Evaluates if the sphere is outside a plane defined by normal `(nx, ny, nz)` and distance `d`.
    /// Returns true if completely outside (culled).
    pub fn is_outside_plane(&self, normal: Vec3, d: f32) -> bool {
        let dist = self.center.dot(normal) + d;
        dist < -self.radius
    }
}

/// Bounding normal cone enclosing all face normals of a meshlet for early backface culling.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
#[repr(C)]
pub struct MeshletBoundingCone {
    /// Apex origin of the normal cone in object / world space.
    pub apex: Vec3,
    /// Normalized axis vector of the cone (average surface normal direction).
    pub axis: Vec3,
    /// Cone cutoff cosine (cos(theta + pi/2), or cos of angle).
    /// If the camera vector to the apex has dot product < cutoff, all triangles are backfacing.
    pub cutoff: f32,
}

impl MeshletBoundingCone {
    pub const fn new(apex: Vec3, axis: Vec3, cutoff: f32) -> Self {
        Self { apex, axis, cutoff }
    }

    /// Tests if the entire meshlet is backfacing relative to a camera/view position.
    ///
    /// If true, every triangle in the meshlet faces away from the camera and can be
    /// culled before invoking the Mesh Shader.
    pub fn is_backfacing(&self, camera_pos: Vec3) -> bool {
        let view_dir = self.apex.sub(camera_pos);
        let len_sq = view_dir.length_squared();
        if len_sq <= 0.000001 {
            return false;
        }
        // Approximate normalization using reciprocal square root in software
        let inv_len = 1.0 / sqrt_approx(len_sq);
        let normalized_view = Vec3::new(view_dir.x * inv_len, view_dir.y * inv_len, view_dir.z * inv_len);

        // If the view vector dot product with the cone axis is >= cutoff,
        // it means the camera is completely behind all normal vectors of this cluster.
        normalized_view.dot(self.axis) >= self.cutoff
    }
}

/// Fast software square root approximation for no_std environments.
#[inline]
pub fn sqrt_approx(val: f32) -> f32 {
    if val <= 0.0 {
        return 0.0;
    }
    // Newton-Raphson approximation
    let mut x = val;
    for _ in 0..6 {
        x = 0.5 * (x + val / x);
    }
    x
}

/// Meshlet descriptor defining a localized cluster of up to 64 vertices and 126 primitives.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
#[repr(C)]
pub struct MeshletDescriptor {
    /// Byte or index offset into the global vertex index buffer.
    pub vertex_offset: u32,
    /// Number of unique vertices in this meshlet (must be <= 64).
    pub vertex_count: u32,
    /// Byte or index offset into the global packed primitive index buffer.
    pub primitive_offset: u32,
    /// Number of primitive triangles in this meshlet (must be <= 126).
    pub primitive_count: u32,
    /// Bounding sphere for frustum culling.
    pub bounds_sphere: MeshletBoundingSphere,
    /// Bounding cone for cluster backface culling.
    pub bounds_cone: MeshletBoundingCone,
}

impl MeshletDescriptor {
    /// Validates that the meshlet does not exceed D3D12 hardware limits.
    pub fn validate(&self) -> Result<(), MeshShaderError> {
        if self.vertex_count > MAX_MESHLET_VERTICES as u32 {
            return Err(MeshShaderError::ExceededMaxVertices(self.vertex_count));
        }
        if self.primitive_count > MAX_MESHLET_PRIMITIVES as u32 {
            return Err(MeshShaderError::ExceededMaxPrimitives(self.primitive_count));
        }
        Ok(())
    }
}

/// Output primitive topology generated by the Mesh Shader.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum MeshShaderPrimitiveTopology {
    /// Triangle list output (up to 126 triangles).
    Triangles = 0,
    /// Line list output.
    Lines = 1,
    /// Point list output.
    Points = 2,
}

/// Shader bytecode descriptor for Amplification or Mesh Shader stages.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MeshShaderBytecode {
    /// GPU virtual address of the compiled shader microcode.
    pub gpu_virtual_address: u64,
    /// Size of the shader microcode in bytes.
    pub size_in_bytes: usize,
    /// Threadgroup dimension X.
    pub threadgroup_size_x: u32,
    /// Threadgroup dimension Y.
    pub threadgroup_size_y: u32,
    /// Threadgroup dimension Z.
    pub threadgroup_size_z: u32,
}

impl MeshShaderBytecode {
    pub const fn new(
        gpu_va: u64,
        size: usize,
        threads_x: u32,
        threads_y: u32,
        threads_z: u32,
    ) -> Self {
        Self {
            gpu_virtual_address: gpu_va,
            size_in_bytes: size,
            threadgroup_size_x: threads_x,
            threadgroup_size_y: threads_y,
            threadgroup_size_z: threads_z,
        }
    }

    /// Total threads in a single threadgroup.
    #[inline]
    pub const fn total_threads(&self) -> u32 {
        self.threadgroup_size_x * self.threadgroup_size_y * self.threadgroup_size_z
    }
}

/// Direct3D 12 Mesh Shader Pipeline State Object (PSO) descriptor.
#[derive(Debug, Clone)]
pub struct MeshShaderPipelineState {
    /// Optional Amplification (Task) shader stage descriptor.
    pub amplification_shader: Option<MeshShaderBytecode>,
    /// Required Mesh shader stage descriptor.
    pub mesh_shader: MeshShaderBytecode,
    /// Optional Pixel shader stage descriptor.
    pub pixel_shader: Option<MeshShaderBytecode>,
    /// Output primitive topology.
    pub primitive_topology: MeshShaderPrimitiveTopology,
    /// Maximum output vertices per mesh shader invocation (<= 64).
    pub max_output_vertices: u32,
    /// Maximum output primitives per mesh shader invocation (<= 126).
    pub max_output_primitives: u32,
    /// Size of amplification-to-mesh payload struct in bytes (<= 16 KiB).
    pub payload_size_bytes: usize,
}

impl MeshShaderPipelineState {
    /// Validates the PSO configuration according to D3D12 hardware rules.
    pub fn validate(&self) -> Result<(), MeshShaderError> {
        if self.max_output_vertices == 0 || self.max_output_vertices > MAX_MESHLET_VERTICES as u32 {
            return Err(MeshShaderError::ExceededMaxVertices(self.max_output_vertices));
        }
        if self.max_output_primitives == 0 || self.max_output_primitives > MAX_MESHLET_PRIMITIVES as u32 {
            return Err(MeshShaderError::ExceededMaxPrimitives(self.max_output_primitives));
        }
        if self.payload_size_bytes > MAX_PAYLOAD_SIZE_BYTES {
            return Err(MeshShaderError::ExceededMaxPayload(self.payload_size_bytes));
        }
        if self.mesh_shader.total_threads() == 0 || self.mesh_shader.total_threads() > 1024 {
            return Err(MeshShaderError::InvalidThreadgroupDimension);
        }
        if let Some(ref as_stage) = self.amplification_shader {
            if as_stage.total_threads() == 0 || as_stage.total_threads() > 1024 {
                return Err(MeshShaderError::InvalidThreadgroupDimension);
            }
        }
        Ok(())
    }
}

/// Maximum number of surviving meshlet IDs that can be passed in a single Amplification Shader payload.
pub const MAX_PAYLOAD_MESHLETS: usize = 32;

/// Direct3D 12 Amplification Shader to Mesh Shader payload structure.
///
/// Passed via `DispatchMesh` from the amplification stage down to child mesh shader threadgroups.
#[derive(Debug, Clone, Copy, PartialEq)]
#[repr(C)]
pub struct AmplificationPayload {
    /// Array of surviving meshlet indices allocated to child threadgroups.
    pub meshlet_indices: [u32; MAX_PAYLOAD_MESHLETS],
    /// Total number of valid surviving meshlets in this payload.
    pub meshlet_count: u32,
    /// Evaluated Level-of-Detail (0 = high, 1 = medium, 2 = coarse).
    pub lod_level: u8,
    /// Instance ID for multi-draw instanced rendering.
    pub instance_id: u32,
    /// User payload custom scratch storage.
    pub custom_data: [u8; 64],
}

impl Default for AmplificationPayload {
    fn default() -> Self {
        Self {
            meshlet_indices: [0u32; MAX_PAYLOAD_MESHLETS],
            meshlet_count: 0,
            lod_level: 0,
            instance_id: 0,
            custom_data: [0u8; 64],
        }
    }
}

/// A 3D plane in Hessian normal form `(nx * x + ny * y + nz * z + d = 0)`.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
#[repr(C)]
pub struct FrustumPlane {
    /// Normalized surface normal pointing towards the inside of the frustum.
    pub normal: Vec3,
    /// Signed distance to the coordinate origin.
    pub d: f32,
}

impl FrustumPlane {
    pub const fn new(normal: Vec3, d: f32) -> Self {
        Self { normal, d }
    }
}

/// Camera view frustum consisting of 6 clipping planes (Near, Far, Left, Right, Top, Bottom).
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Frustum {
    /// 6 frustum clipping planes.
    pub planes: [FrustumPlane; 6],
}

impl Frustum {
    /// Creates a Frustum from 6 planes.
    pub const fn new(planes: [FrustumPlane; 6]) -> Self {
        Self { planes }
    }

    /// Evaluates whether a meshlet bounding sphere intersects or is inside the frustum.
    ///
    /// If the sphere is completely behind any plane, it is culled.
    pub fn is_sphere_visible(&self, sphere: &MeshletBoundingSphere) -> bool {
        for plane in &self.planes {
            if sphere.is_outside_plane(plane.normal, plane.d) {
                return false;
            }
        }
        true
    }
}

/// Direct3D 12 `DispatchMesh` command parameters.
///
/// Generated either by CPU command submission or emitted dynamically from an Amplification Shader.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DispatchMeshDesc {
    /// Number of Mesh Shader threadgroups to spawn along X (one per surviving meshlet).
    pub threadgroup_count_x: u32,
    /// Number of Mesh Shader threadgroups along Y.
    pub threadgroup_count_y: u32,
    /// Number of Mesh Shader threadgroups along Z.
    pub threadgroup_count_z: u32,
    /// Amplification Shader payload passed to the spawned Mesh Shader threadgroups.
    pub payload: AmplificationPayload,
}

impl DispatchMeshDesc {
    /// Total Mesh Shader threadgroups dispatched.
    #[inline]
    pub const fn total_threadgroups(&self) -> u32 {
        self.threadgroup_count_x * self.threadgroup_count_y * self.threadgroup_count_z
    }
}

/// Telemetry metrics for Amplification Shader culling and LOD selection.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct AmplificationStats {
    /// Total meshlets processed across all amplification threadgroups.
    pub total_meshlets_input: u32,
    /// Meshlets eliminated by frustum culling.
    pub meshlets_frustum_culled: u32,
    /// Meshlets eliminated by cluster normal cone backface culling.
    pub meshlets_backface_culled: u32,
    /// Meshlets surviving culling and dispatched to Mesh Shaders.
    pub meshlets_surviving_dispatched: u32,
}

impl AmplificationStats {
    /// Percentage of meshlets culled before reaching the Mesh Shader stage (0 to 100%).
    pub fn cull_rate_percentage(&self) -> u32 {
        if self.total_meshlets_input == 0 {
            return 0;
        }
        let culled = self.meshlets_frustum_culled + self.meshlets_backface_culled;
        (culled * 100) / self.total_meshlets_input
    }
}

/// Amplification Shader Execution Engine & Task Dispatcher.
///
/// Processes meshlet batches, performs parallel frustum and backface culling, selects
/// dynamic LOD levels based on camera distance, packs payloads, and invokes `DispatchMesh`.
#[derive(Debug)]
pub struct AmplificationShaderDispatcher {
    /// Active camera view frustum.
    frustum: Frustum,
    /// Active camera world position.
    camera_pos: Vec3,
    /// Distance threshold for high detail (LOD 0).
    lod0_distance_sq: f32,
    /// Distance threshold for medium detail (LOD 1).
    lod1_distance_sq: f32,
    /// Profiling and culling telemetry.
    stats: AmplificationStats,
}

impl AmplificationShaderDispatcher {
    /// Creates a new AmplificationShaderDispatcher with the specified camera view.
    pub fn new(frustum: Frustum, camera_pos: Vec3) -> Self {
        Self {
            frustum,
            camera_pos,
            lod0_distance_sq: 2500.0, // 50 units squared
            lod1_distance_sq: 10000.0, // 100 units squared
            stats: AmplificationStats::default(),
        }
    }

    /// Updates the camera frustum and position.
    pub fn update_camera(&mut self, frustum: Frustum, camera_pos: Vec3) {
        self.frustum = frustum;
        self.camera_pos = camera_pos;
    }

    /// Configures LOD distance thresholds (in world units squared).
    pub fn set_lod_thresholds(&mut self, lod0_sq: f32, lod1_sq: f32) {
        self.lod0_distance_sq = lod0_sq;
        self.lod1_distance_sq = lod1_sq;
    }

    /// Current culling telemetry statistics.
    #[inline]
    pub fn stats(&self) -> &AmplificationStats {
        &self.stats
    }

    /// Resets culling telemetry statistics.
    pub fn reset_stats(&mut self) {
        self.stats = AmplificationStats::default();
    }

    /// Executes the Amplification Shader stage across a slice of meshlets.
    ///
    /// Evaluates frustum visibility and cluster backfacing for each meshlet.
    /// Packs surviving meshlets into `AmplificationPayload` records and generates
    /// `DispatchMeshDesc` commands to spawn child Mesh Shader threadgroups.
    pub fn dispatch_meshlets(
        &mut self,
        meshlets: &[MeshletDescriptor],
        base_instance_id: u32,
    ) -> Result<Vec<DispatchMeshDesc>, MeshShaderError> {
        let mut dispatches = Vec::new();

        // Process meshlets in batches up to MAX_PAYLOAD_MESHLETS
        for (chunk_idx, chunk) in meshlets.chunks(MAX_PAYLOAD_MESHLETS).enumerate() {
            let mut payload = AmplificationPayload::default();
            payload.instance_id = base_instance_id;

            let chunk_base_index = (chunk_idx * MAX_PAYLOAD_MESHLETS) as u32;

            for (local_idx, meshlet) in chunk.iter().enumerate() {
                self.stats.total_meshlets_input += 1;
                let meshlet_id = chunk_base_index + (local_idx as u32);

                // 1. Frustum culling
                if !self.frustum.is_sphere_visible(&meshlet.bounds_sphere) {
                    self.stats.meshlets_frustum_culled += 1;
                    continue;
                }

                // 2. Normal cone backface culling
                if meshlet.bounds_cone.is_backfacing(self.camera_pos) {
                    self.stats.meshlets_backface_culled += 1;
                    continue;
                }

                // 3. Dynamic LOD selection based on camera distance
                let dist_sq = meshlet.bounds_sphere.center.sub(self.camera_pos).length_squared();
                let lod = if dist_sq <= self.lod0_distance_sq {
                    0
                } else if dist_sq <= self.lod1_distance_sq {
                    1
                } else {
                    2
                };

                // Add to payload
                let count = payload.meshlet_count as usize;
                if count < MAX_PAYLOAD_MESHLETS {
                    payload.meshlet_indices[count] = meshlet_id;
                    payload.meshlet_count += 1;
                    payload.lod_level = core::cmp::max(payload.lod_level, lod);
                    self.stats.meshlets_surviving_dispatched += 1;
                }
            }

            // If any meshlet survived culling in this threadgroup, issue DispatchMesh
            if payload.meshlet_count > 0 {
                dispatches.push(DispatchMeshDesc {
                    threadgroup_count_x: payload.meshlet_count,
                    threadgroup_count_y: 1,
                    threadgroup_count_z: 1,
                    payload,
                });
            }
        }

        Ok(dispatches)
    }
}

/// Output vertex format emitted by the Mesh Shader directly into the primitive pipeline.
#[derive(Debug, Clone, Copy, PartialEq)]
#[repr(C)]
pub struct MeshVertexOutput {
    /// Clip-space homogeneous position `(x, y, z, w)`.
    pub position: [f32; 4],
    /// Normal vector for lighting and shading.
    pub normal: Vec3,
    /// Texture coordinates `(u, v)`.
    pub uv: [f32; 2],
    /// Vertex color RGBA.
    pub color: [f32; 4],
}

impl Default for MeshVertexOutput {
    fn default() -> Self {
        Self {
            position: [0.0, 0.0, 0.0, 1.0],
            normal: Vec3::default(),
            uv: [0.0, 0.0],
            color: [1.0, 1.0, 1.0, 1.0],
        }
    }
}

/// Meshlet local triangle indices (3 local vertex indices in range 0..63).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[repr(C)]
pub struct MeshTriangleIndices {
    /// First vertex index in meshlet local vertex array (0..63).
    pub i0: u8,
    /// Second vertex index in meshlet local vertex array (0..63).
    pub i1: u8,
    /// Third vertex index in meshlet local vertex array (0..63).
    pub i2: u8,
}

impl MeshTriangleIndices {
    /// Creates a new triangle primitive from 3 local vertex indices.
    pub const fn new(i0: u8, i1: u8, i2: u8) -> Self {
        Self { i0, i1, i2 }
    }

    /// Returns the indices as a 3-element array.
    #[inline]
    pub const fn as_array(self) -> [u8; 3] {
        [self.i0, self.i1, self.i2]
    }
}

/// Output primitive streamer buffer for a single meshlet threadgroup invocation.
#[derive(Debug, Clone)]
pub struct MeshletStreamOutput {
    /// Output vertices (up to 64).
    pub vertices: [MeshVertexOutput; MAX_MESHLET_VERTICES],
    /// Output triangle primitive indices (up to 126).
    pub primitives: [MeshTriangleIndices; MAX_MESHLET_PRIMITIVES],
    /// Valid vertex count emitted (`SetMeshOutputCounts`).
    pub vertex_count: u32,
    /// Valid primitive count emitted (`SetMeshOutputCounts`).
    pub primitive_count: u32,
}

impl Default for MeshletStreamOutput {
    fn default() -> Self {
        Self {
            vertices: [MeshVertexOutput::default(); MAX_MESHLET_VERTICES],
            primitives: [MeshTriangleIndices::default(); MAX_MESHLET_PRIMITIVES],
            vertex_count: 0,
            primitive_count: 0,
        }
    }
}

impl MeshletStreamOutput {
    /// Sets the dynamic output counts for the meshlet (`SetMeshOutputCounts`).
    pub fn set_mesh_output_counts(&mut self, vertex_count: u32, primitive_count: u32) -> Result<(), MeshShaderError> {
        if vertex_count > MAX_MESHLET_VERTICES as u32 {
            return Err(MeshShaderError::ExceededMaxVertices(vertex_count));
        }
        if primitive_count > MAX_MESHLET_PRIMITIVES as u32 {
            return Err(MeshShaderError::ExceededMaxPrimitives(primitive_count));
        }
        self.vertex_count = vertex_count;
        self.primitive_count = primitive_count;
        Ok(())
    }

    /// Writes an output vertex at local slot `index`.
    pub fn write_vertex(&mut self, index: usize, vertex: MeshVertexOutput) -> Result<(), MeshShaderError> {
        if index >= self.vertex_count as usize || index >= MAX_MESHLET_VERTICES {
            return Err(MeshShaderError::BufferOverflow);
        }
        self.vertices[index] = vertex;
        Ok(())
    }

    /// Writes an output triangle primitive at local slot `index`.
    pub fn write_primitive(&mut self, index: usize, tri: MeshTriangleIndices) -> Result<(), MeshShaderError> {
        if index >= self.primitive_count as usize || index >= MAX_MESHLET_PRIMITIVES {
            return Err(MeshShaderError::BufferOverflow);
        }
        if tri.i0 >= self.vertex_count as u8 || tri.i1 >= self.vertex_count as u8 || tri.i2 >= self.vertex_count as u8 {
            return Err(MeshShaderError::InvalidPipelineConfig);
        }
        self.primitives[index] = tri;
        Ok(())
    }
}

/// Telemetry metrics for Mesh Shader primitive streaming.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct MeshShaderStats {
    /// Total meshlet threadgroups executed and expanded into primitives.
    pub meshlets_expanded: u32,
    /// Total vertices streamed to the primitive rasterizer.
    pub vertices_streamed: u64,
    /// Total primitive triangles streamed to the primitive rasterizer.
    pub primitives_streamed: u64,
    /// Peak vertices emitted by a single meshlet.
    pub peak_vertices_per_meshlet: u32,
    /// Peak primitives emitted by a single meshlet.
    pub peak_primitives_per_meshlet: u32,
}

/// Mesh Shader Primitive Pipeline Execution Engine.
///
/// Executes Mesh Shader threadgroups dispatched by the Amplification Shader or CPU,
/// reads meshlet cluster descriptors, expands vertices and indexed triangle topology,
/// and streams primitives directly into the GPU rasterizer pipeline.
#[derive(Debug)]
pub struct MeshShaderEngine {
    /// Active Mesh Shader PSO.
    pipeline_state: MeshShaderPipelineState,
    /// Profiling and primitive streaming telemetry.
    stats: MeshShaderStats,
}

impl MeshShaderEngine {
    /// Creates a new MeshShaderEngine with the specified PSO.
    pub fn new(pipeline_state: MeshShaderPipelineState) -> Result<Self, MeshShaderError> {
        pipeline_state.validate()?;
        Ok(Self {
            pipeline_state,
            stats: MeshShaderStats::default(),
        })
    }

    /// Active pipeline state object.
    #[inline]
    pub fn pipeline_state(&self) -> &MeshShaderPipelineState {
        &self.pipeline_state
    }

    /// Current primitive streaming telemetry.
    #[inline]
    pub fn stats(&self) -> &MeshShaderStats {
        &self.stats
    }

    /// Resets streaming telemetry statistics.
    pub fn reset_stats(&mut self) {
        self.stats = MeshShaderStats::default();
    }

    /// Executes a single `DispatchMeshDesc` batch against a meshlet database.
    pub fn execute_dispatch(
        &mut self,
        dispatch: &DispatchMeshDesc,
        meshlet_database: &[MeshletDescriptor],
    ) -> Result<Vec<MeshletStreamOutput>, MeshShaderError> {
        let mut outputs = Vec::with_capacity(dispatch.payload.meshlet_count as usize);

        for &meshlet_id in &dispatch.payload.meshlet_indices[..dispatch.payload.meshlet_count as usize] {
            let meshlet = meshlet_database
                .get(meshlet_id as usize)
                .ok_or(MeshShaderError::InvalidPipelineConfig)?;

            meshlet.validate()?;

            let mut stream = MeshletStreamOutput::default();
            stream.set_mesh_output_counts(meshlet.vertex_count, meshlet.primitive_count)?;

            // Stream vertices: synthesize positions and attributes for this meshlet
            for v in 0..meshlet.vertex_count {
                let local_v = v as f32;
                let world_x = meshlet.bounds_sphere.center.x + (local_v * 0.1);
                let world_y = meshlet.bounds_sphere.center.y + (local_v * 0.1);
                let world_z = meshlet.bounds_sphere.center.z;

                let vertex = MeshVertexOutput {
                    position: [world_x, world_y, world_z, 1.0],
                    normal: meshlet.bounds_cone.axis,
                    uv: [local_v / (meshlet.vertex_count as f32), 0.5],
                    color: [1.0, 1.0, 1.0, 1.0],
                };
                stream.write_vertex(v as usize, vertex)?;
            }

            // Stream primitive triangle topology: generate triangle strips/fans across local vertices
            for p in 0..meshlet.primitive_count {
                let v0 = 0u8;
                let v1 = core::cmp::min((p + 1) as u8, (meshlet.vertex_count - 1) as u8);
                let v2 = core::cmp::min((p + 2) as u8, (meshlet.vertex_count - 1) as u8);

                let tri = MeshTriangleIndices::new(v0, v1, v2);
                stream.write_primitive(p as usize, tri)?;
            }

            // Update stats
            self.stats.meshlets_expanded += 1;
            self.stats.vertices_streamed += meshlet.vertex_count as u64;
            self.stats.primitives_streamed += meshlet.primitive_count as u64;
            self.stats.peak_vertices_per_meshlet =
                core::cmp::max(self.stats.peak_vertices_per_meshlet, meshlet.vertex_count);
            self.stats.peak_primitives_per_meshlet =
                core::cmp::max(self.stats.peak_primitives_per_meshlet, meshlet.primitive_count);

            outputs.push(stream);
        }

        Ok(outputs)
    }

    /// Executes multiple `DispatchMeshDesc` batches against a meshlet database.
    pub fn execute_dispatches(
        &mut self,
        dispatches: &[DispatchMeshDesc],
        meshlet_database: &[MeshletDescriptor],
    ) -> Result<Vec<MeshletStreamOutput>, MeshShaderError> {
        let mut all_outputs = Vec::new();
        for dispatch in dispatches {
            let mut out = self.execute_dispatch(dispatch, meshlet_database)?;
            all_outputs.append(&mut out);
        }
        Ok(all_outputs)
    }
}
