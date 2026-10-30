//! Direct3D 12 Shader Execution Reordering (SER) & Raytracing Hit-Sort Accelerator Engine.
//!
//! Solves ray tracing SIMD lane divergence and texture cache thrashing by reordering
//! secondary ray hit invocations into shader-coherent and spatially contiguous batches.
//!
//! Core Architecture:
//! - Hit record descriptors: Ray hit attributes (instance, geometry, primitive, barycentrics)
//! - Spatial bit-key hashing (3D Morton codes / Z-order curve)
//! - Composite reorder keys (material shader target primary, spatial Morton secondary)
//! - Hardware-assisted hit-sort radix binning engine
//! - ReorderThread / HitObject dispatch loop and SIMD coherence profiler

#![allow(dead_code)]

extern crate alloc;
use alloc::vec::Vec;

/// Classification of ray-primitive intersection outcome.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[repr(u8)]
pub enum HitKind {
    #[default]
    Miss = 0,
    TriangleFrontFace = 1,
    TriangleBackFace = 2,
    ProceduralPrimitive = 3,
}

/// Errors returned by the Shader Execution Reordering subsystem.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SerError {
    /// Reorder table capacity exceeded.
    TableFull,
    /// Invalid ray attributes (e.g. NaN or non-finite coordinates).
    InvalidRay,
    /// Operation requested on an empty reorder table.
    EmptyTable,
    /// Target shader table index is out of bounds.
    InvalidShaderIndex(u32),
    /// Invalid radix bin configuration.
    InvalidRadixConfig,
}

/// Detailed descriptor for a ray-surface intersection hit record.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct HitRecord {
    pub ray_id: u32,
    pub origin: [f32; 3],
    pub direction: [f32; 3],
    pub t_hit: f32,
    pub instance_id: u32,
    pub geometry_index: u32,
    pub primitive_id: u32,
    pub barycentrics: [f32; 2],
    pub shader_table_index: u32,
    pub hit_kind: HitKind,
}

impl Default for HitRecord {
    fn default() -> Self {
        Self {
            ray_id: 0,
            origin: [0.0; 3],
            direction: [0.0, 0.0, 1.0],
            t_hit: f32::MAX,
            instance_id: 0,
            geometry_index: 0,
            primitive_id: 0,
            barycentrics: [0.0, 0.0],
            shader_table_index: 0,
            hit_kind: HitKind::Miss,
        }
    }
}

impl HitRecord {
    /// Computes world-space hit coordinate: P = origin + t_hit * direction.
    #[inline]
    pub fn world_position(&self) -> [f32; 3] {
        if self.hit_kind == HitKind::Miss || self.t_hit == f32::MAX {
            return self.origin;
        }
        [
            self.origin[0] + self.t_hit * self.direction[0],
            self.origin[1] + self.t_hit * self.direction[1],
            self.origin[2] + self.t_hit * self.direction[2],
        ]
    }

    /// Validates ray attributes for finite float representation.
    pub fn validate(&self) -> Result<(), SerError> {
        let coords = [
            self.origin[0], self.origin[1], self.origin[2],
            self.direction[0], self.direction[1], self.direction[2],
            self.t_hit, self.barycentrics[0], self.barycentrics[1],
        ];
        for c in coords {
            if !c.is_finite() {
                return Err(SerError::InvalidRay);
            }
        }
        Ok(())
    }
}

// -----------------------------------------------------------------------------
// Spatial Bit-Key Hashing (3D Morton Codes)
// -----------------------------------------------------------------------------

/// Expands a 10-bit integer by inserting 2 zero bits between each bit,
/// mapping bits 0..9 into bits 0, 3, 6, 9, 12, 15, 18, 21, 24, 27.
#[inline(always)]
pub fn morton_expand_10bit(mut v: u32) -> u32 {
    v = (v | (v << 16)) & 0x030000FF;
    v = (v | (v << 8))  & 0x0300F00F;
    v = (v | (v << 4))  & 0x030C30C3;
    v = (v | (v << 2))  & 0x09249249;
    v
}

/// Encodes 3D coordinates into a 30-bit Morton code (Z-order curve)
/// within the specified bounding box domain `[bbox_min, bbox_max]`.
pub fn morton_code_3d(point: [f32; 3], bbox_min: [f32; 3], bbox_max: [f32; 3]) -> u32 {
    let mut q = [0u32; 3];
    for i in 0..3 {
        let span = bbox_max[i] - bbox_min[i];
        let norm = if span > 1e-6 {
            ((point[i] - bbox_min[i]) / span).clamp(0.0, 1.0)
        } else {
            0.0
        };
        q[i] = (norm * 1023.0) as u32;
    }

    (morton_expand_10bit(q[0])) |
    (morton_expand_10bit(q[1]) << 1) |
    (morton_expand_10bit(q[2]) << 2)
}

/// 64-bit composite reorder key:
/// - High 32 bits: Shader table index (material target) for SIMD warp divergence elimination
/// - Low 32 bits: 3D Morton code for spatial cache locality
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Default, Hash)]
pub struct ReorderKey(pub u64);

impl ReorderKey {
    #[inline(always)]
    pub const fn new(shader_table_index: u32, morton_code: u32) -> Self {
        Self(((shader_table_index as u64) << 32) | (morton_code as u64))
    }

    #[inline(always)]
    pub const fn shader_index(&self) -> u32 {
        (self.0 >> 32) as u32
    }

    #[inline(always)]
    pub const fn morton_code(&self) -> u32 {
        self.0 as u32
    }
}

/// HLSL D3D12 `HitObject` abstraction representing an intercepted ray intersection.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct HitObject {
    pub hit_record: HitRecord,
    pub reorder_key: ReorderKey,
}

impl HitObject {
    /// Creates a new HitObject and computes its composite reorder key against a scene bounding box.
    pub fn new(hit_record: HitRecord, bbox_min: [f32; 3], bbox_max: [f32; 3]) -> Result<Self, SerError> {
        hit_record.validate()?;
        let pos = hit_record.world_position();
        let morton = morton_code_3d(pos, bbox_min, bbox_max);
        let reorder_key = ReorderKey::new(hit_record.shader_table_index, morton);
        Ok(Self {
            hit_record,
            reorder_key,
        })
    }

    #[inline(always)]
    pub fn is_hit(&self) -> bool {
        self.hit_record.hit_kind != HitKind::Miss
    }

    #[inline(always)]
    pub fn is_miss(&self) -> bool {
        self.hit_record.hit_kind == HitKind::Miss
    }
}

// -----------------------------------------------------------------------------
// Reorder Table Layout
// -----------------------------------------------------------------------------

/// Reorder table buffering intercepted ray hit records prior to sorting and dispatch.
#[derive(Debug, Clone)]
pub struct ReorderTable {
    pub capacity: usize,
    pub wave_size: u32,
    pub entries: Vec<HitObject>,
}

impl ReorderTable {
    pub fn new(capacity: usize, wave_size: u32) -> Self {
        Self {
            capacity,
            wave_size: wave_size.max(1),
            entries: Vec::with_capacity(capacity),
        }
    }

    /// Appends a HitObject to the reorder table.
    pub fn push(&mut self, hit: HitObject) -> Result<(), SerError> {
        if self.entries.len() >= self.capacity {
            return Err(SerError::TableFull);
        }
        self.entries.push(hit);
        Ok(())
    }

    #[inline(always)]
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    #[inline(always)]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    #[inline(always)]
    pub fn clear(&mut self) {
        self.entries.clear();
    }
}
