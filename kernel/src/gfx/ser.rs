//! Direct3D 12 Shader Execution Reordering (SER) & Raytracing Hit-Sort Accelerator Engine.
//! Solves ray tracing SIMD lane divergence and texture cache thrashing by reordering

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

// Spatial Bit-Key Hashing (3D Morton Codes)

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

// Reorder Table Layout

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


// Radix Hit-Sort Accelerator Engine

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

// Threadgroup / SIMD Divergence Estimator

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


// ReorderThread Execution Pipeline & HitObject Dispatch Loop

/// Coherence profiler recording before-and-after SIMD efficiency gains.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct SerCoherenceProfiler {
    pub total_batches_reordered: u64,
    pub total_rays_reordered: u64,
    pub total_waves_dispatched: u64,
    pub pre_reorder_lane_utilization: f32,
    pub post_reorder_lane_utilization: f32,
    pub pre_reorder_spatial_coherence: f32,
    pub post_reorder_spatial_coherence: f32,
    pub lane_utilization_gain_factor: f32,
}

impl SerCoherenceProfiler {
    /// Records a reordering execution cycle and updates cumulative profiling statistics.
    pub fn record_cycle(&mut self, pre: DivergenceMetrics, post: DivergenceMetrics) {
        self.total_batches_reordered += 1;
        self.total_rays_reordered += pre.total_rays as u64;
        self.total_waves_dispatched += post.wave_count as u64;

        self.pre_reorder_lane_utilization = pre.lane_utilization;
        self.post_reorder_lane_utilization = post.lane_utilization;
        self.pre_reorder_spatial_coherence = pre.spatial_coherence;
        self.post_reorder_spatial_coherence = post.spatial_coherence;

        if pre.lane_utilization > 0.0 {
            self.lane_utilization_gain_factor = post.lane_utilization / pre.lane_utilization;
        } else {
            self.lane_utilization_gain_factor = 1.0;
        }
    }
}

/// A coherent wave of rays dispatched to hardware SIMD execution units.
#[derive(Debug, Clone, PartialEq)]
pub struct DispatchWave {
    pub wave_id: u32,
    pub shader_table_index: u32,
    pub active_lanes_mask: u64,
    pub ray_count: u32,
    pub rays: alloc::vec::Vec<HitObject>,
}

/// Unified Shader Execution Reordering (SER) Engine & Dispatch Accelerator.
#[derive(Debug, Clone)]
pub struct SerEngine {
    pub reorder_table: ReorderTable,
    pub sorter: RadixHitSorter,
    pub profiler: SerCoherenceProfiler,
    pub bbox_min: [f32; 3],
    pub bbox_max: [f32; 3],
    pub next_wave_id: u32,
}

impl Default for SerEngine {
    fn default() -> Self {
        Self::new(256, 32, [-100.0; 3], [100.0; 3])
    }
}

impl SerEngine {
    /// Creates a new SER Engine with specified capacity, wave size, and scene bounding box.
    pub fn new(capacity: usize, wave_size: u32, bbox_min: [f32; 3], bbox_max: [f32; 3]) -> Self {
        Self {
            reorder_table: ReorderTable::new(capacity, wave_size),
            sorter: RadixHitSorter::new(),
            profiler: SerCoherenceProfiler::default(),
            bbox_min,
            bbox_max,
            next_wave_id: 0,
        }
    }

    /// Emulates the HLSL `ReorderThread()` intrinsic:
    /// Intercepts a ray hit record, hashes spatial coordinates, and buffers the HitObject.
    pub fn reorder_thread(&mut self, hit_record: HitRecord) -> Result<(), SerError> {
        let hit_object = HitObject::new(hit_record, self.bbox_min, self.bbox_max)?;
        self.reorder_table.push(hit_object)
    }

    /// Sorts buffered HitObjects with the hardware radix binning engine and dispatches
    /// coherent SIMD execution waves.
    pub fn dispatch_reordered_rays(&mut self) -> Result<alloc::vec::Vec<DispatchWave>, SerError> {
        if self.reorder_table.is_empty() {
            return Err(SerError::EmptyTable);
        }

        let wave_size = self.reorder_table.wave_size;

        // 1. Analyze pre-reordering divergence
        let pre_metrics = DivergenceEstimator::analyze(&self.reorder_table.entries, wave_size);

        // 2. Execute hardware-assisted Radix Hit-Sort
        self.sorter.sort(&mut self.reorder_table.entries);

        // 3. Analyze post-reordering divergence
        let post_metrics = DivergenceEstimator::analyze(&self.reorder_table.entries, wave_size);

        // 4. Update coherence profiler
        self.profiler.record_cycle(pre_metrics, post_metrics);

        // 5. Package sorted rays into coherent SIMD dispatch waves
        let mut dispatch_waves = alloc::vec::Vec::new();
        let w_size = wave_size as usize;

        for chunk in self.reorder_table.entries.chunks(w_size) {
            let wave_id = self.next_wave_id;
            self.next_wave_id += 1;

            // Generate active lanes bitmask
            let mut mask: u64 = 0;
            for i in 0..chunk.len().min(64) {
                mask |= 1u64 << i;
            }

            let primary_shader = if !chunk.is_empty() {
                chunk[0].reorder_key.shader_index()
            } else {
                0
            };

            dispatch_waves.push(DispatchWave {
                wave_id,
                shader_table_index: primary_shader,
                active_lanes_mask: mask,
                ray_count: chunk.len() as u32,
                rays: chunk.to_vec(),
            });
        }

        // 6. Reset reorder table for next batch
        self.reorder_table.clear();

        Ok(dispatch_waves)
    }
}


// Verification Self-Test Suite

/// Comprehensive verification self-test for D3D12 Shader Execution Reordering (SER).
/// Emits sentinel [KNT-GPU-SER-PASS] upon successful verification.
pub fn test_ser_subsystem() -> bool {
    let bbox_min = [-10.0, -10.0, -10.0];
    let bbox_max = [10.0, 10.0, 10.0];

    // 1. Verify Morton 3D code spatial bit-key hashing
    let p_min = morton_code_3d(bbox_min, bbox_min, bbox_max);
    let p_max = morton_code_3d(bbox_max, bbox_min, bbox_max);
    if p_min != 0 || p_max != 0x3FFF_FFFF {
        return false;
    }

    let p_mid1 = morton_code_3d([0.0, 0.0, 0.0], bbox_min, bbox_max);
    let p_mid2 = morton_code_3d([0.0, 0.0, 0.0], bbox_min, bbox_max);
    if p_mid1 != p_mid2 || p_mid1 == 0 {
        return false;
    }

    // 2. Verify ReorderKey packing and unpacking
    let key = ReorderKey::new(42, 0x12345);
    if key.shader_index() != 42 || key.morton_code() != 0x12345 {
        return false;
    }

    // Primary sort on shader_index must take precedence
    let key_a = ReorderKey::new(1, 0xFFFFFF);
    let key_b = ReorderKey::new(2, 0x000001);
    if key_a >= key_b {
        return false;
    }

    // 3. Verify HitRecord validation
    let mut invalid_hit = HitRecord::default();
    invalid_hit.origin[0] = f32::NAN;
    if invalid_hit.validate().is_ok() {
        return false;
    }

    // 4. Verify Radix Hit-Sort Accelerator and SIMD Coherence Gains
    // Allocate SER Engine for 128 rays with wave size = 32
    let mut engine = SerEngine::new(256, 32, bbox_min, bbox_max);

    // Interleave 128 rays across 4 materials: (0, 1, 2, 3, 0, 1, 2, 3...)
    // This creates worst-case SIMD divergence where every 32-lane wave executes 4 different shaders (25% lane utilization)
    for i in 0..128 {
        let shader_id = (i % 4) as u32;
        let pos = [
            -8.0 + (i as f32 * 0.12),
            -8.0 + ((i % 8) as f32 * 2.0),
            0.0,
        ];
        let hit = HitRecord {
            ray_id: i as u32,
            origin: pos,
            direction: [0.0, 0.0, 1.0],
            t_hit: 1.0,
            instance_id: 1,
            geometry_index: 0,
            primitive_id: i as u32,
            barycentrics: [0.33, 0.33],
            shader_table_index: shader_id,
            hit_kind: HitKind::TriangleFrontFace,
        };
        if engine.reorder_thread(hit).is_err() {
            return false;
        }
    }

    if engine.reorder_table.len() != 128 {
        return false;
    }

    // Dispatch reordered rays through the hardware radix binning engine
    let waves = match engine.dispatch_reordered_rays() {
        Ok(w) => w,
        Err(_) => return false,
    };

    // 128 rays / 32 rays per wave = 4 waves
    if waves.len() != 4 {
        return false;
    }

    // After reordering, each wave should have 100% shader coherence (32 rays of a single material)
    for (i, wave) in waves.iter().enumerate() {
        if wave.ray_count != 32 {
            return false;
        }
        if wave.shader_table_index != i as u32 {
            return false;
        }
        for ray in &wave.rays {
            if ray.hit_record.shader_table_index != i as u32 {
                return false;
            }
        }
    }

    // Verify profiler measured the jump from 25% to 100% lane utilization (4x gain)
    let profiler = engine.profiler;
    if profiler.total_batches_reordered != 1 || profiler.total_rays_reordered != 128 {
        return false;
    }
    if (profiler.pre_reorder_lane_utilization - 0.25).abs() > 0.01 {
        return false;
    }
    if (profiler.post_reorder_lane_utilization - 1.0).abs() > 0.01 {
        return false;
    }
    if profiler.lane_utilization_gain_factor < 3.9 {
        return false;
    }

    // 5. Verify error on empty reorder table dispatch
    if engine.dispatch_reordered_rays().is_ok() {
        return false;
    }

    // 6. Emit deterministic boot sentinel
    crate::serial_write_str("[KNT-GPU-SER-PASS] Direct3D 12 shader execution reordering and raytracing hit-sort engine verified\r\n");

    true
}
