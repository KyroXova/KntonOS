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
