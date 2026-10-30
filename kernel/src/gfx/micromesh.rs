//! Direct3D 12 Dynamic Micro-Mesh Displacement & Sub-Triangle Geometry LOD Compressor.
//!
//! Implements hardware-accelerated displaced micro-mesh geometry pipeline:
//! - Barycentric sub-triangle recursive subdivision (levels 0 to 5, generating up to 1024 micro-triangles).
//! - Compressed displacement encodings (UncompressedVec3, quantized 11-11-10 UNORM vectors, QuantizedScalar8/16).
//! - Continuous distance-based sub-triangle LOD evaluation and crack-free edge stitching.
//! - Tight Axis-Aligned Bounding Box (AABB) hierarchy generation for early view frustum and backface culling.

extern crate alloc;

use alloc::vec::Vec;

/// Standard square root approximation for micro-kernel geometry routines.
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

/// 3D Vector for spatial coordinates, surface normals, and displacement offsets.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Vec3 {
    pub x: f32,
    pub y: f32,
    pub z: f32,
}

impl Vec3 {
    /// Creates a new 3D vector.
    pub const fn new(x: f32, y: f32, z: f32) -> Self {
        Self { x, y, z }
    }

    /// Zero vector `(0, 0, 0)`.
    pub const fn zero() -> Self {
        Self { x: 0.0, y: 0.0, z: 0.0 }
    }

    /// Vector addition: `self + other`.
    #[inline(always)]
    pub fn add(&self, other: &Self) -> Self {
        Self::new(self.x + other.x, self.y + other.y, self.z + other.z)
    }

    /// Vector subtraction: `self - other`.
    #[inline(always)]
    pub fn sub(&self, other: &Self) -> Self {
        Self::new(self.x - other.x, self.y - other.y, self.z - other.z)
    }

    /// Scalar multiplication: `self * s`.
    #[inline(always)]
    pub fn scale(&self, s: f32) -> Self {
        Self::new(self.x * s, self.y * s, self.z * s)
    }

    /// Vector dot product.
    #[inline(always)]
    pub fn dot(&self, other: &Self) -> f32 {
        self.x * other.x + self.y * other.y + self.z * other.z
    }

    /// Vector cross product: `self x other`.
    #[inline(always)]
    pub fn cross(&self, other: &Self) -> Self {
        Self::new(
            self.y * other.z - self.z * other.y,
            self.z * other.x - self.x * other.z,
            self.x * other.y - self.y * other.x,
        )
    }

    /// Squared Euclidean length of the vector.
    #[inline(always)]
    pub fn length_sq(&self) -> f32 {
        self.dot(self)
    }

    /// Euclidean length of the vector.
    #[inline(always)]
    pub fn length(&self) -> f32 {
        f32_sqrt(self.length_sq())
    }

    /// Returns a normalized unit vector, or `Vec3::zero()` if length is negligible.
    pub fn normalize(&self) -> Self {
        let len = self.length();
        if len > 1e-6 {
            self.scale(1.0 / len)
        } else {
            Self::zero()
        }
    }
}

/// Barycentric Coordinates `(u, v, w)` across a triangle surface where `u + v + w = 1.0`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BarycentricCoord {
    /// Coordinate weight along vertex 1.
    pub u: f32,
    /// Coordinate weight along vertex 2.
    pub v: f32,
    /// Coordinate weight along vertex 0 (`w = 1.0 - u - v`).
    pub w: f32,
}

impl BarycentricCoord {
    /// Constructs barycentric coordinates from `(u, v)`, calculating `w = 1.0 - u - v`.
    pub fn new(u: f32, v: f32) -> Self {
        let w = 1.0 - u - v;
        Self { u, v, w }
    }

    /// Constructs barycentric coordinates explicitly with all 3 weights.
    pub const fn from_uvw(u: f32, v: f32, w: f32) -> Self {
        Self { u, v, w }
    }

    /// Validates that weights are non-negative and sum to 1.0 within numerical epsilon.
    pub fn is_valid(&self) -> bool {
        let sum = self.u + self.v + self.w;
        let diff = if sum >= 1.0 { sum - 1.0 } else { 1.0 - sum };
        diff < 1e-4 && self.u >= -1e-5 && self.v >= -1e-5 && self.w >= -1e-5
    }
}

/// Recursive Base Triangle Subdivision Level (Level 0 to 5).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum SubdivisionLevel {
    /// Level 0: 1 triangle, 3 vertices (no subdivision).
    Level0 = 0,
    /// Level 1: 4 micro-triangles, 6 micro-vertices (2 segments per edge).
    Level1 = 1,
    /// Level 2: 16 micro-triangles, 15 micro-vertices (4 segments per edge).
    Level2 = 2,
    /// Level 3: 64 micro-triangles, 45 micro-vertices (8 segments per edge).
    Level3 = 3,
    /// Level 4: 256 micro-triangles, 153 micro-vertices (16 segments per edge).
    Level4 = 4,
    /// Level 5: 1024 micro-triangles, 561 micro-vertices (32 segments per edge).
    Level5 = 5,
}

impl SubdivisionLevel {
    /// Returns the integer level index `0..5`.
    #[inline(always)]
    pub const fn to_u8(&self) -> u8 {
        *self as u8
    }

    /// Constructs a subdivision level from integer `0..5`, clamping to `Level5`.
    pub const fn from_u8(level: u8) -> Self {
        match level {
            0 => Self::Level0,
            1 => Self::Level1,
            2 => Self::Level2,
            3 => Self::Level3,
            4 => Self::Level4,
            _ => Self::Level5,
        }
    }

    /// Number of segments along each triangle edge ($N = 2^{\text{level}}$).
    #[inline(always)]
    pub const fn segments_per_edge(&self) -> u32 {
        1 << self.to_u8()
    }

    /// Total number of micro-vertices produced by this subdivision level.
    /// $V = \frac{(N + 1)(N + 2)}{2}$.
    #[inline(always)]
    pub const fn num_micro_vertices(&self) -> usize {
        let n = self.segments_per_edge() as usize;
        ((n + 1) * (n + 2)) / 2
    }

    /// Total number of micro-triangles produced by this subdivision level ($N^2 = 4^{\text{level}}$).
    #[inline(always)]
    pub const fn num_micro_triangles(&self) -> usize {
        let n = self.segments_per_edge() as usize;
        n * n
    }
}

/// Encoding format for micro-mesh displacement vectors and scalar heights.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MicroMeshFormat {
    /// Uncompressed 32-bit floating point 3D displacement vector (12 bytes/vertex).
    UncompressedVec3,
    /// Packed 32-bit vector with 11-bit X, 11-bit Y, and 10-bit Z normalized components (4 bytes/vertex).
    Packed11_11_10,
    /// Quantized 8-bit scalar height displacement along surface normal (1 byte/vertex).
    QuantizedScalar8,
    /// Quantized 16-bit scalar height displacement along surface normal (2 bytes/vertex).
    QuantizedScalar16,
}

impl MicroMeshFormat {
    /// Returns the byte size per displaced vertex for this format.
    #[inline(always)]
    pub const fn bytes_per_vertex(&self) -> usize {
        match self {
            Self::UncompressedVec3 => 12,
            Self::Packed11_11_10 => 4,
            Self::QuantizedScalar16 => 2,
            Self::QuantizedScalar8 => 1,
        }
    }
}

/// Base geometry triangle that serves as the root domain for micro-mesh displacement.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BaseTriangle {
    /// 3D positions of the base triangle vertices `[V0, V1, V2]`.
    pub positions: [Vec3; 3],
    /// Vertex surface normals `[N0, N1, N2]`.
    pub normals: [Vec3; 3],
    /// Texture / UV coordinates `[UV0, UV1, UV2]`.
    pub uvs: [[f32; 2]; 3],
}

impl BaseTriangle {
    /// Creates a base triangle from positions, normals, and UVs.
    pub const fn new(positions: [Vec3; 3], normals: [Vec3; 3], uvs: [[f32; 2]; 3]) -> Self {
        Self { positions, normals, uvs }
    }

    /// Calculates the flat face normal of the base triangle via cross product $(V1 - V0) \times (V2 - V0)$.
    pub fn face_normal(&self) -> Vec3 {
        let e1 = self.positions[1].sub(&self.positions[0]);
        let e2 = self.positions[2].sub(&self.positions[0]);
        e1.cross(&e2).normalize()
    }

    /// Computes base surface position $P_0(u, v, w) = w \cdot V0 + u \cdot V1 + v \cdot V2$.
    pub fn evaluate_surface_point(&self, bary: &BarycentricCoord) -> Vec3 {
        let p0 = self.positions[0].scale(bary.w);
        let p1 = self.positions[1].scale(bary.u);
        let p2 = self.positions[2].scale(bary.v);
        p0.add(&p1).add(&p2)
    }

    /// Computes linearly interpolated surface normal $N(u, v, w) = \text{norm}(w \cdot N0 + u \cdot N1 + v \cdot N2)$.
    pub fn evaluate_interpolated_normal(&self, bary: &BarycentricCoord) -> Vec3 {
        let n0 = self.normals[0].scale(bary.w);
        let n1 = self.normals[1].scale(bary.u);
        let n2 = self.normals[2].scale(bary.v);
        n0.add(&n1).add(&n2).normalize()
    }

    /// Computes interpolated texture coordinates `[U, V]`.
    pub fn evaluate_uv(&self, bary: &BarycentricCoord) -> [f32; 2] {
        let u = bary.w * self.uvs[0][0] + bary.u * self.uvs[1][0] + bary.v * self.uvs[2][0];
        let v = bary.w * self.uvs[0][1] + bary.u * self.uvs[1][1] + bary.v * self.uvs[2][1];
        [u, v]
    }
}

/// An individual micro-vertex generated by recursive triangle subdivision.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MicroVertex {
    /// Barycentric coordinates within the base triangle domain.
    pub barycentric: BarycentricCoord,
    /// Base triangle domain surface position before displacement.
    pub base_position: Vec3,
    /// Surface normal vector at this micro-vertex.
    pub normal: Vec3,
    /// 3D displacement vector applied to this micro-vertex.
    pub displacement: Vec3,
    /// Final displaced world/object position: `base_position + displacement`.
    pub final_position: Vec3,
}

/// 3-index connectivity descriptor defining an individual micro-triangle.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MicroTriangle {
    /// Index of micro-vertex 0 in the vertex array.
    pub v0: u32,
    /// Index of micro-vertex 1 in the vertex array.
    pub v1: u32,
    /// Index of micro-vertex 2 in the vertex array.
    pub v2: u32,
}

impl MicroTriangle {
    /// Creates a micro-triangle with specified vertex indices.
    pub const fn new(v0: u32, v1: u32, v2: u32) -> Self {
        Self { v0, v1, v2 }
    }
}

/// Configuration descriptor for a Displaced Micro-Mesh asset.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MicroMeshDescriptor {
    /// Unique identifier for the base mesh triangle.
    pub triangle_id: u32,
    /// Tessellation / subdivision level (Level 0 to 5).
    pub subdivision_level: SubdivisionLevel,
    /// Displacement payload encoding format.
    pub format: MicroMeshFormat,
    /// Global displacement magnitude scaling factor.
    pub displacement_scale: f32,
    /// Minimum scalar displacement height bound.
    pub min_displacement: f32,
    /// Maximum scalar displacement height bound.
    pub max_displacement: f32,
}

/// Precomputed and dynamically generated barycentric subdivision tables.
pub struct BarycentricSubdivisionTable;

impl BarycentricSubdivisionTable {
    /// Resolves grid coordinate `(row i, col j)` to its linear micro-vertex index.
    ///
    /// Row $i \in [0, N]$ and col $j \in [0, N - i]$.
    /// Row offset = $i \cdot (N + 1) - \frac{i(i - 1)}{2}$.
    #[inline(always)]
    pub fn linear_vertex_index(n: u32, i: u32, j: u32) -> usize {
        let row_offset = (i as usize) * (n as usize + 1) - ((i as usize) * (i.saturating_sub(1) as usize)) / 2;
        row_offset + (j as usize)
    }

    /// Generates full regular barycentric coordinate grids and micro-triangle index topologies
    /// for the given subdivision level.
    pub fn generate_subdivision(
        level: SubdivisionLevel,
    ) -> (Vec<BarycentricCoord>, Vec<MicroTriangle>) {
        let n = level.segments_per_edge();
        let num_verts = level.num_micro_vertices();
        let num_tris = level.num_micro_triangles();

        let mut coords = Vec::with_capacity(num_verts);
        let mut triangles = Vec::with_capacity(num_tris);

        let inv_n = 1.0 / (n as f32);

        // 1. Generate micro-vertex barycentric coordinates row-by-row
        for i in 0..=n {
            for j in 0..=(n - i) {
                let u = (j as f32) * inv_n;
                let v = (i as f32) * inv_n;
                coords.push(BarycentricCoord::new(u, v));
            }
        }

        // 2. Generate micro-triangle index topology
        for i in 0..n {
            for j in 0..(n - i) {
                let v_ij = Self::linear_vertex_index(n, i, j) as u32;
                let v_next_col = Self::linear_vertex_index(n, i, j + 1) as u32;
                let v_next_row = Self::linear_vertex_index(n, i + 1, j) as u32;

                // Upright micro-triangle
                triangles.push(MicroTriangle::new(v_ij, v_next_col, v_next_row));

                // Inverted micro-triangle (exists when j + 1 < n - i)
                if j + 1 < n - i {
                    let v_diag = Self::linear_vertex_index(n, i + 1, j + 1) as u32;
                    triangles.push(MicroTriangle::new(v_next_col, v_diag, v_next_row));
                }
            }
        }

        (coords, triangles)
    }
}
