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

// -----------------------------------------------------------------------------
// Compressed Displacement Payload & Decompression Engine
// -----------------------------------------------------------------------------

/// Compressed displacement binary buffer storing micro-vertex displacements.
#[derive(Debug, Clone)]
pub struct DisplacementPayload {
    /// Raw byte payload containing encoded displacement values.
    pub data: Vec<u8>,
    /// Format used to decode the displacement bytes.
    pub format: MicroMeshFormat,
}

impl DisplacementPayload {
    /// Constructs a payload from uncompressed 3D vectors.
    pub fn from_vec3_slice(vectors: &[Vec3]) -> Self {
        let mut data = Vec::with_capacity(vectors.len() * 12);
        for v in vectors {
            data.extend_from_slice(&v.x.to_le_bytes());
            data.extend_from_slice(&v.y.to_le_bytes());
            data.extend_from_slice(&v.z.to_le_bytes());
        }
        Self {
            data,
            format: MicroMeshFormat::UncompressedVec3,
        }
    }

    /// Packs a 3D unit/normalized displacement vector into a 32-bit 11-11-10 word.
    ///
    /// Components in `[-1.0, 1.0]` are mapped into 11-bit X (0..2047), 11-bit Y (0..2047), and 10-bit Z (0..1023).
    pub fn pack_11_11_10(v: &Vec3) -> u32 {
        let clamp_x = if v.x < -1.0 { -1.0 } else if v.x > 1.0 { 1.0 } else { v.x };
        let clamp_y = if v.y < -1.0 { -1.0 } else if v.y > 1.0 { 1.0 } else { v.y };
        let clamp_z = if v.z < -1.0 { -1.0 } else if v.z > 1.0 { 1.0 } else { v.z };

        let ux = ((clamp_x + 1.0) * 0.5 * 2047.0 + 0.5) as u32;
        let uy = ((clamp_y + 1.0) * 0.5 * 2047.0 + 0.5) as u32;
        let uz = ((clamp_z + 1.0) * 0.5 * 1023.0 + 0.5) as u32;

        (ux & 0x7FF) | ((uy & 0x7FF) << 11) | ((uz & 0x3FF) << 22)
    }

    /// Constructs a payload from an array of packed 32-bit 11-11-10 vectors.
    pub fn from_packed_11_11_10(packed_words: &[u32]) -> Self {
        let mut data = Vec::with_capacity(packed_words.len() * 4);
        for &word in packed_words {
            data.extend_from_slice(&word.to_le_bytes());
        }
        Self {
            data,
            format: MicroMeshFormat::Packed11_11_10,
        }
    }

    /// Constructs a payload from 8-bit quantized scalar displacement values.
    pub fn from_quantized_u8(scalars: &[u8]) -> Self {
        Self {
            data: scalars.to_vec(),
            format: MicroMeshFormat::QuantizedScalar8,
        }
    }

    /// Constructs a payload from 16-bit quantized scalar displacement values.
    pub fn from_quantized_u16(scalars: &[u16]) -> Self {
        let mut data = Vec::with_capacity(scalars.len() * 2);
        for &s in scalars {
            data.extend_from_slice(&s.to_le_bytes());
        }
        Self {
            data,
            format: MicroMeshFormat::QuantizedScalar16,
        }
    }

    /// Decompresses the displacement vector for the micro-vertex at `index`.
    pub fn decompress_at(
        &self,
        index: usize,
        interpolated_normal: &Vec3,
        scale: f32,
        min_displacement: f32,
        max_displacement: f32,
    ) -> Vec3 {
        match self.format {
            MicroMeshFormat::UncompressedVec3 => {
                let offset = index * 12;
                if offset + 12 <= self.data.len() {
                    let mut bx = [0u8; 4];
                    let mut by = [0u8; 4];
                    let mut bz = [0u8; 4];
                    bx.copy_from_slice(&self.data[offset..offset + 4]);
                    by.copy_from_slice(&self.data[offset + 4..offset + 8]);
                    bz.copy_from_slice(&self.data[offset + 8..offset + 12]);
                    let x = f32::from_le_bytes(bx) * scale;
                    let y = f32::from_le_bytes(by) * scale;
                    let z = f32::from_le_bytes(bz) * scale;
                    Vec3::new(x, y, z)
                } else {
                    Vec3::zero()
                }
            }
            MicroMeshFormat::Packed11_11_10 => {
                let offset = index * 4;
                if offset + 4 <= self.data.len() {
                    let mut b = [0u8; 4];
                    b.copy_from_slice(&self.data[offset..offset + 4]);
                    let word = u32::from_le_bytes(b);

                    let raw_x = word & 0x7FF;
                    let raw_y = (word >> 11) & 0x7FF;
                    let raw_z = (word >> 22) & 0x3FF;

                    let fx = ((raw_x as f32) / 2047.0) * 2.0 - 1.0;
                    let fy = ((raw_y as f32) / 2047.0) * 2.0 - 1.0;
                    let fz = ((raw_z as f32) / 1023.0) * 2.0 - 1.0;

                    Vec3::new(fx * scale, fy * scale, fz * scale)
                } else {
                    Vec3::zero()
                }
            }
            MicroMeshFormat::QuantizedScalar8 => {
                if index < self.data.len() {
                    let raw = self.data[index];
                    let norm = (raw as f32) / 255.0;
                    let height = min_displacement + norm * (max_displacement - min_displacement);
                    interpolated_normal.scale(height * scale)
                } else {
                    Vec3::zero()
                }
            }
            MicroMeshFormat::QuantizedScalar16 => {
                let offset = index * 2;
                if offset + 2 <= self.data.len() {
                    let mut b = [0u8; 2];
                    b.copy_from_slice(&self.data[offset..offset + 2]);
                    let raw = u16::from_le_bytes(b);
                    let norm = (raw as f32) / 65535.0;
                    let height = min_displacement + norm * (max_displacement - min_displacement);
                    interpolated_normal.scale(height * scale)
                } else {
                    Vec3::zero()
                }
            }
        }
    }
}

// -----------------------------------------------------------------------------
// Displaced Micro-Mesh Container & Surface Normal Synthesis
// -----------------------------------------------------------------------------

/// Concrete displaced micro-mesh geometry container holding subdivided micro-vertices
/// and micro-triangles.
#[derive(Debug, Clone)]
pub struct DisplacedMicroMesh {
    /// Asset configuration descriptor.
    pub desc: MicroMeshDescriptor,
    /// Base triangle domain.
    pub base_triangle: BaseTriangle,
    /// Generated micro-vertices with final displaced positions.
    pub vertices: Vec<MicroVertex>,
    /// Sub-triangle index buffer.
    pub triangles: Vec<MicroTriangle>,
}

impl DisplacedMicroMesh {
    /// Generates fully displaced micro-vertices and micro-triangles from a base triangle
    /// and a compressed displacement payload.
    pub fn generate(
        desc: MicroMeshDescriptor,
        base_triangle: BaseTriangle,
        payload: &DisplacementPayload,
    ) -> Self {
        let (bary_coords, triangles) =
            BarycentricSubdivisionTable::generate_subdivision(desc.subdivision_level);

        let mut vertices = Vec::with_capacity(bary_coords.len());

        for (idx, bary) in bary_coords.iter().enumerate() {
            let base_pos = base_triangle.evaluate_surface_point(bary);
            let interp_normal = base_triangle.evaluate_interpolated_normal(bary);
            let displacement = payload.decompress_at(
                idx,
                &interp_normal,
                desc.displacement_scale,
                desc.min_displacement,
                desc.max_displacement,
            );
            let final_pos = base_pos.add(&displacement);

            vertices.push(MicroVertex {
                barycentric: *bary,
                base_position: base_pos,
                normal: interp_normal,
                displacement,
                final_position: final_pos,
            });
        }

        let mut mesh = Self {
            desc,
            base_triangle,
            vertices,
            triangles,
        };

        mesh.synthesize_surface_normals();
        mesh
    }

    /// Synthesizes high-frequency surface normals by accumulating displaced micro-triangle
    /// facet cross-products onto micro-vertices.
    pub fn synthesize_surface_normals(&mut self) {
        if self.vertices.is_empty() || self.triangles.is_empty() {
            return;
        }

        let mut accumulated_normals = alloc::vec![Vec3::zero(); self.vertices.len()];

        // Accumulate facet normals for each micro-triangle
        for tri in self.triangles.iter() {
            let i0 = tri.v0 as usize;
            let i1 = tri.v1 as usize;
            let i2 = tri.v2 as usize;

            if i0 < self.vertices.len() && i1 < self.vertices.len() && i2 < self.vertices.len() {
                let p0 = self.vertices[i0].final_position;
                let p1 = self.vertices[i1].final_position;
                let p2 = self.vertices[i2].final_position;

                let e1 = p1.sub(&p0);
                let e2 = p2.sub(&p0);
                let facet_normal = e1.cross(&e2);

                accumulated_normals[i0] = accumulated_normals[i0].add(&facet_normal);
                accumulated_normals[i1] = accumulated_normals[i1].add(&facet_normal);
                accumulated_normals[i2] = accumulated_normals[i2].add(&facet_normal);
            }
        }

        // Normalize and assign synthesized normals
        for (i, v) in self.vertices.iter_mut().enumerate() {
            let synth = accumulated_normals[i].normalize();
            if synth.length_sq() > 1e-4 {
                v.normal = synth;
            }
        }
    }

    /// Total number of generated micro-vertices.
    #[inline(always)]
    pub fn vertex_count(&self) -> usize {
        self.vertices.len()
    }

    /// Total number of generated micro-triangles.
    #[inline(always)]
    pub fn triangle_count(&self) -> usize {
        self.triangles.len()
    }

    /// Resolves linear vertex indices along one of the three base triangle boundary edges:
    /// - Edge 0: $V_0 \to V_1$ ($v = 0$, row $i = 0$, $j = 0..N$)
    /// - Edge 1: $V_1 \to V_2$ ($w = 0$, $i + j = N$, $i = 0..N$, $j = N - i$)
    /// - Edge 2: $V_0 \to V_2$ ($u = 0$, col $j = 0$, $i = 0..N$)
    pub fn get_edge_vertex_indices(&self, edge_idx: usize) -> Vec<usize> {
        let n = self.desc.subdivision_level.segments_per_edge();
        let mut indices = Vec::with_capacity((n + 1) as usize);

        match edge_idx {
            0 => {
                // Edge 0: row i = 0, col j from 0 to n
                for j in 0..=n {
                    indices.push(BarycentricSubdivisionTable::linear_vertex_index(n, 0, j));
                }
            }
            1 => {
                // Edge 1: from V1 (i=0, j=n) to V2 (i=n, j=0)
                for i in 0..=n {
                    let j = n - i;
                    indices.push(BarycentricSubdivisionTable::linear_vertex_index(n, i, j));
                }
            }
            2 => {
                // Edge 2: col j = 0, row i from 0 to n (V0 to V2)
                for i in 0..=n {
                    indices.push(BarycentricSubdivisionTable::linear_vertex_index(n, i, 0));
                }
            }
            _ => {}
        }

        indices
    }

    /// Performs crack-free edge stitching against an adjoining neighbor base triangle
    /// evaluated at `neighbor_level`.
    ///
    /// If `neighbor_level < self.desc.subdivision_level`, intermediate micro-vertices along
    /// the shared boundary edge are linearly snapped to the coarser edge segments of the neighbor,
    /// guaranteeing bit-exact geometric coincidence and eliminating cracks/tearing.
    pub fn stitch_edge(&mut self, edge_idx: usize, neighbor_level: SubdivisionLevel) -> usize {
        let self_level = self.desc.subdivision_level.to_u8();
        let neigh_level = neighbor_level.to_u8();

        if neigh_level >= self_level {
            return 0;
        }

        let ratio = 1usize << (self_level - neigh_level);
        let edge_indices = self.get_edge_vertex_indices(edge_idx);
        let count = edge_indices.len();
        if count == 0 {
            return 0;
        }

        let mut stitched_count = 0;

        for k in 0..count {
            if k % ratio != 0 {
                let k0 = (k / ratio) * ratio;
                let k1 = (k0 + ratio).min(count - 1);

                let p0 = self.vertices[edge_indices[k0]].final_position;
                let p1 = self.vertices[edge_indices[k1]].final_position;

                let alpha = ((k - k0) as f32) / ((k1 - k0) as f32);
                let interp_pos = p0.scale(1.0 - alpha).add(&p1.scale(alpha));

                self.vertices[edge_indices[k]].final_position = interp_pos;
                stitched_count += 1;
            }
        }

        if stitched_count > 0 {
            self.synthesize_surface_normals();
        }

        stitched_count
    }
}

// -----------------------------------------------------------------------------
// Continuous Sub-Triangle LOD Governor & Distance Evaluator
// -----------------------------------------------------------------------------

/// Continuous LOD Governor that determines the optimal micro-mesh subdivision level
/// based on camera distance and screen-space geometric fidelity thresholds.
#[derive(Debug, Clone)]
pub struct MicroMeshLodGovernor {
    /// Distance thresholds `[d0, d1, d2, d3, d4]` for transitions:
    /// - `dist < d0`: Level 5 (1024 micro-triangles)
    /// - `dist < d1`: Level 4 (256 micro-triangles)
    /// - `dist < d2`: Level 3 (64 micro-triangles)
    /// - `dist < d3`: Level 2 (16 micro-triangles)
    /// - `dist < d4`: Level 1 (4 micro-triangles)
    /// - `dist >= d4`: Level 0 (1 base triangle)
    pub distance_thresholds: [f32; 5],
}

impl MicroMeshLodGovernor {
    /// Creates a new LOD governor with explicit distance thresholds.
    pub const fn new(distance_thresholds: [f32; 5]) -> Self {
        Self { distance_thresholds }
    }

    /// Evaluates the discrete subdivision level and continuous morph parameter `t in [0.0, 1.0]`.
    pub fn evaluate_lod(&self, camera_pos: &Vec3, target_pos: &Vec3) -> (SubdivisionLevel, f32) {
        let dist = camera_pos.sub(target_pos).length();

        if dist < self.distance_thresholds[0] {
            let t = dist / self.distance_thresholds[0];
            (SubdivisionLevel::Level5, t)
        } else if dist < self.distance_thresholds[1] {
            let t = (dist - self.distance_thresholds[0])
                / (self.distance_thresholds[1] - self.distance_thresholds[0]);
            (SubdivisionLevel::Level4, t)
        } else if dist < self.distance_thresholds[2] {
            let t = (dist - self.distance_thresholds[1])
                / (self.distance_thresholds[2] - self.distance_thresholds[1]);
            (SubdivisionLevel::Level3, t)
        } else if dist < self.distance_thresholds[3] {
            let t = (dist - self.distance_thresholds[2])
                / (self.distance_thresholds[3] - self.distance_thresholds[2]);
            (SubdivisionLevel::Level2, t)
        } else if dist < self.distance_thresholds[4] {
            let t = (dist - self.distance_thresholds[3])
                / (self.distance_thresholds[4] - self.distance_thresholds[3]);
            (SubdivisionLevel::Level1, t)
        } else {
            (SubdivisionLevel::Level0, 0.0)
        }
    }
}

// -----------------------------------------------------------------------------
// 3D Axis-Aligned Bounding Box (AABB) & Micro-Mesh Bounding Hierarchy
// -----------------------------------------------------------------------------

/// 3D Axis-Aligned Bounding Box for spatial culling and BVH acceleration.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Aabb3D {
    /// Minimum corner of the bounding box.
    pub min: Vec3,
    /// Maximum corner of the bounding box.
    pub max: Vec3,
}

impl Aabb3D {
    /// Creates an empty/inverted AABB ready for point expansion.
    pub const fn empty() -> Self {
        Self {
            min: Vec3::new(f32::MAX, f32::MAX, f32::MAX),
            max: Vec3::new(-f32::MAX, -f32::MAX, -f32::MAX),
        }
    }

    /// Creates an AABB enclosing a single initial point.
    pub const fn from_point(p: &Vec3) -> Self {
        Self { min: *p, max: *p }
    }

    /// Expands the AABB to enclose the point `p`.
    pub fn expand_by_point(&mut self, p: &Vec3) {
        if p.x < self.min.x { self.min.x = p.x; }
        if p.y < self.min.y { self.min.y = p.y; }
        if p.z < self.min.z { self.min.z = p.z; }

        if p.x > self.max.x { self.max.x = p.x; }
        if p.y > self.max.y { self.max.y = p.y; }
        if p.z > self.max.z { self.max.z = p.z; }
    }

    /// Expands all dimensions by a scalar margin.
    pub fn expand_by_margin(&mut self, margin: f32) {
        self.min.x -= margin;
        self.min.y -= margin;
        self.min.z -= margin;

        self.max.x += margin;
        self.max.y += margin;
        self.max.z += margin;
    }

    /// Center position of the bounding box.
    pub fn center(&self) -> Vec3 {
        self.min.add(&self.max).scale(0.5)
    }

    /// Extents (half-dimensions) of the bounding box.
    pub fn extents(&self) -> Vec3 {
        self.max.sub(&self.min).scale(0.5)
    }

    /// Tests for intersection between this AABB and another AABB.
    pub fn intersects(&self, other: &Self) -> bool {
        self.min.x <= other.max.x && self.max.x >= other.min.x
            && self.min.y <= other.max.y && self.max.y >= other.min.y
            && self.min.z <= other.max.z && self.max.z >= other.min.z
    }
}

/// Tight Bounding Volume and Culling Hierarchy for a Displaced Micro-Mesh.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MicroMeshBoundingHierarchy {
    /// Exact tight AABB computed directly from all displaced micro-vertices.
    pub tight_aabb: Aabb3D,
    /// Conservative AABB computed from base triangle bounds expanded by maximum displacement.
    pub conservative_aabb: Aabb3D,
    /// Average surface normal across all micro-vertices.
    pub average_normal: Vec3,
    /// Minimum dot product between any micro-triangle facet normal and the average normal.
    pub normal_cone_cutoff: f32,
}

impl MicroMeshBoundingHierarchy {
    /// Constructs the bounding hierarchy by analyzing displaced vertices and normals.
    pub fn build(mesh: &DisplacedMicroMesh) -> Self {
        let mut tight = Aabb3D::empty();
        let mut normal_sum = Vec3::zero();

        for v in mesh.vertices.iter() {
            tight.expand_by_point(&v.final_position);
            normal_sum = normal_sum.add(&v.normal);
        }

        let avg_normal = normal_sum.normalize();

        // Compute conservative AABB from base triangle positions
        let mut cons = Aabb3D::empty();
        for pos in mesh.base_triangle.positions.iter() {
            cons.expand_by_point(pos);
        }
        let max_disp = mesh.desc.max_displacement.abs().max(mesh.desc.min_displacement.abs())
            * mesh.desc.displacement_scale;
        cons.expand_by_margin(max_disp);

        // Find minimum normal cone cutoff
        let mut min_dot = 1.0f32;
        for v in mesh.vertices.iter() {
            let dot = v.normal.dot(&avg_normal);
            if dot < min_dot {
                min_dot = dot;
            }
        }

        Self {
            tight_aabb: tight,
            conservative_aabb: cons,
            average_normal: avg_normal,
            normal_cone_cutoff: min_dot,
        }
    }

    /// Evaluates whether the entire micro-mesh is backfacing relative to the camera position.
    pub fn is_backfacing(&self, camera_pos: &Vec3) -> bool {
        let center = self.tight_aabb.center();
        let view_vec = camera_pos.sub(&center).normalize();

        // Dot product with average normal
        let dot = view_vec.dot(&self.average_normal);

        // If the view vector points away from the surface beyond the normal spread cone, it is backfacing
        dot < 0.0 && dot.abs() > (1.0 - self.normal_cone_cutoff).max(0.1)
    }
}


