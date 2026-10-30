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


// -----------------------------------------------------------------------------
// Radix Hit-Sort Accelerator Engine
// -----------------------------------------------------------------------------

/// Hardware-assisted 8-bit Radix Sort engine operating on 64-bit composite ReorderKey.
#[derive(Debug, Clone, Default)]
pub struct RadixHitSorter {
    pub total_sort_passes: u64,
    pub total_rays_sorted: u64,
}

impl RadixHitSorter {
    pub fn new() -> Self {
        Self::default()
    }

    /// Sorts a slice of HitObjects in-place by their 64-bit composite ReorderKey
    /// using an 8-pass 256-bucket Least Significant Digit (LSD) Radix Sort.
    pub fn sort(&mut self, items: &mut [HitObject]) {
        let n = items.len();
        if n <= 1 {
            return;
        }

        let mut scratch = alloc::vec::Vec::with_capacity(n);
        scratch.resize(n, HitObject::default());

        let mut source: &mut [HitObject] = items;
        let mut target: &mut [HitObject] = &mut scratch;

        // 8 passes for 64-bit keys (8 bits per pass)
        for pass in 0..8 {
            let shift = pass * 8;
            let mut counts = [0usize; 256];

            // 1. Compute histogram
            for item in source.iter() {
                let bucket = ((item.reorder_key.0 >> shift) & 0xFF) as usize;
                counts[bucket] += 1;
            }

            // 2. Compute prefix sum (starting offsets)
            let mut offsets = [0usize; 256];
            let mut total = 0;
            for i in 0..256 {
                offsets[i] = total;
                total += counts[i];
            }

            // 3. Scatter items into target buffer
            for item in source.iter() {
                let bucket = ((item.reorder_key.0 >> shift) & 0xFF) as usize;
                target[offsets[bucket]] = *item;
                offsets[bucket] += 1;
            }

            // 4. Swap source and target references
            core::mem::swap(&mut source, &mut target);
        }

        // If the final result ended up in scratch (even number of passes = 8, so source is items again),
        // we verify and copy if needed. Since 8 is even, source is items, so items already contains sorted data!
        self.total_sort_passes += 8;
        self.total_rays_sorted += n as u64;
    }
}

// -----------------------------------------------------------------------------
// Threadgroup / SIMD Divergence Estimator
// -----------------------------------------------------------------------------

/// Coherence and SIMD lane efficiency metrics for a ray batch.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct DivergenceMetrics {
    /// Total number of rays evaluated.
    pub total_rays: usize,
    /// Number of simulated hardware waves/warps.
    pub wave_count: usize,
    /// Total distinct shader invocations across all waves.
    pub total_wave_shader_invocations: usize,
    /// Average distinct shaders executed per wave.
    pub avg_shaders_per_wave: f32,
    /// SIMD lane utilization ratio (1.0 = 100% full wave lockstep, 0.25 = 25% due to 4-way divergence).
    pub lane_utilization: f32,
    /// Normalized spatial coherence index [0.0, 1.0].
    pub spatial_coherence: f32,
}

/// Hardware wave divergence estimator analyzing SIMD branch efficiency.
pub struct DivergenceEstimator;

impl DivergenceEstimator {
    /// Computes SIMD lane utilization and spatial coherence for a set of HitObjects
    /// grouped into hardware waves of size `wave_size`.
    pub fn analyze(items: &[HitObject], wave_size: u32) -> DivergenceMetrics {
        if items.is_empty() {
            return DivergenceMetrics::default();
        }

        let w_size = wave_size.max(1) as usize;
        let mut total_shader_invocations = 0;
        let mut total_waves = 0;

        // Track spatial Morton code delta for spatial coherence
        let mut total_morton_delta: u64 = 0;
        let mut delta_comparisons: u64 = 0;

        for chunk in items.chunks(w_size) {
            total_waves += 1;
            // Identify distinct shader targets within this wave
            let mut distinct_shaders = alloc::vec::Vec::with_capacity(chunk.len());
            for item in chunk {
                let shader = item.reorder_key.shader_index();
                if !distinct_shaders.contains(&shader) {
                    distinct_shaders.push(shader);
                }
            }
            total_shader_invocations += distinct_shaders.len();

            // Compute pairwise spatial delta within wave
            for i in 1..chunk.len() {
                let d = (chunk[i].reorder_key.morton_code() as i64 - chunk[i - 1].reorder_key.morton_code() as i64).abs() as u64;
                total_morton_delta += d;
                delta_comparisons += 1;
            }
        }

        let avg_shaders = if total_waves > 0 {
            total_shader_invocations as f32 / total_waves as f32
        } else {
            1.0
        };

        // Lane utilization is reciprocal of average serialized passes per wave
        let lane_utilization = if avg_shaders >= 1.0 {
            (1.0 / avg_shaders).clamp(0.0, 1.0)
        } else {
            1.0
        };

        // Normalized spatial coherence: lower Morton delta = higher spatial clustering
        let avg_delta = if delta_comparisons > 0 {
            total_morton_delta as f32 / delta_comparisons as f32
        } else {
            0.0
        };
        // Normalize against 30-bit Morton range (1 << 30)
        let spatial_coherence = (1.0 - (avg_delta / (1u32 << 26) as f32)).clamp(0.0, 1.0);

        DivergenceMetrics {
            total_rays: items.len(),
            wave_count: total_waves,
            total_wave_shader_invocations: total_shader_invocations,
            avg_shaders_per_wave: avg_shaders,
            lane_utilization,
            spatial_coherence,
        }
    }
}
