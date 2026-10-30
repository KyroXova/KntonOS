//! Direct3D 12 Raytracing DispatchRays Engine & Shader Binding Table (SBT) Generator.
//! Implements DXR 1.1 / Vulkan KHR ray dispatching infrastructure:

#![allow(dead_code)]

extern crate alloc;
use alloc::vec::Vec;

/// Required byte size of a DXR shader identifier.
pub const D3D12_SHADER_IDENTIFIER_SIZE_IN_BYTES: usize = 32;

/// Required byte alignment for GPU virtual addresses of raytracing shader tables.
pub const D3D12_RAYTRACING_SHADER_TABLE_BYTE_ALIGNMENT: u64 = 64;

/// Required byte alignment for individual shader records within a shader table.
pub const D3D12_RAYTRACING_SHADER_RECORD_BYTE_ALIGNMENT: u64 = 32;

// Shader Record & Address Range Descriptors

/// Single shader record containing a 32-byte shader identifier and optional local root arguments.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShaderRecord {
    pub shader_identifier: [u8; D3D12_SHADER_IDENTIFIER_SIZE_IN_BYTES],
    pub local_root_arguments: Vec<u8>,
}

impl ShaderRecord {
    pub fn new(shader_identifier: [u8; 32], local_args: &[u8]) -> Self {
        Self {
            shader_identifier,
            local_root_arguments: local_args.to_vec(),
        }
    }

    /// Total unpadded byte size of this shader record.
    #[inline(always)]
    pub fn raw_size_bytes(&self) -> usize {
        D3D12_SHADER_IDENTIFIER_SIZE_IN_BYTES + self.local_root_arguments.len()
    }

    /// Size aligned to 32-byte or 64-byte shader record alignment rules.
    #[inline(always)]
    pub fn aligned_size_bytes(&self, alignment: usize) -> usize {
        let align = alignment.max(32);
        (self.raw_size_bytes() + align - 1) & !(align - 1)
    }
}

/// GPU virtual address range descriptor for RayGeneration shader table.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct D3D12_GPU_VIRTUAL_ADDRESS_RANGE {
    pub start_address: u64,
    pub size_in_bytes: u64,
}

impl D3D12_GPU_VIRTUAL_ADDRESS_RANGE {
    pub const fn new(start_address: u64, size_in_bytes: u64) -> Self {
        Self {
            start_address,
            size_in_bytes,
        }
    }

    #[inline(always)]
    pub fn is_valid(&self) -> bool {
        self.size_in_bytes == 0
            || ((self.start_address % D3D12_RAYTRACING_SHADER_TABLE_BYTE_ALIGNMENT) == 0
                && self.size_in_bytes >= (D3D12_SHADER_IDENTIFIER_SIZE_IN_BYTES as u64))
    }
}

/// GPU virtual address range and stride descriptor for Miss, HitGroup, and Callable tables.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct D3D12_GPU_VIRTUAL_ADDRESS_RANGE_AND_STRIDE {
    pub start_address: u64,
    pub size_in_bytes: u64,
    pub stride_in_bytes: u64,
}

impl D3D12_GPU_VIRTUAL_ADDRESS_RANGE_AND_STRIDE {
    pub const fn new(start_address: u64, size_in_bytes: u64, stride_in_bytes: u64) -> Self {
        Self {
            start_address,
            size_in_bytes,
            stride_in_bytes,
        }
    }

    #[inline(always)]
    pub fn is_valid(&self) -> bool {
        if self.size_in_bytes == 0 {
            true
        } else {
            (self.start_address % D3D12_RAYTRACING_SHADER_TABLE_BYTE_ALIGNMENT) == 0
                && (self.stride_in_bytes % D3D12_RAYTRACING_SHADER_RECORD_BYTE_ALIGNMENT) == 0
                && self.stride_in_bytes >= (D3D12_SHADER_IDENTIFIER_SIZE_IN_BYTES as u64)
        }
    }
}

/// Complete raytracing dispatch description (D3D12_DISPATCH_RAYS_DESC).
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct D3D12_DISPATCH_RAYS_DESC {
    pub ray_generation_shader_record: D3D12_GPU_VIRTUAL_ADDRESS_RANGE,
    pub miss_shader_table: D3D12_GPU_VIRTUAL_ADDRESS_RANGE_AND_STRIDE,
    pub hit_group_table: D3D12_GPU_VIRTUAL_ADDRESS_RANGE_AND_STRIDE,
    pub callable_shader_table: D3D12_GPU_VIRTUAL_ADDRESS_RANGE_AND_STRIDE,
    pub width: u32,
    pub height: u32,
    pub depth: u32,
}

impl D3D12_DISPATCH_RAYS_DESC {
    pub const fn new_2d(width: u32, height: u32) -> Self {
        Self {
            ray_generation_shader_record: D3D12_GPU_VIRTUAL_ADDRESS_RANGE {
                start_address: 0,
                size_in_bytes: 0,
            },
            miss_shader_table: D3D12_GPU_VIRTUAL_ADDRESS_RANGE_AND_STRIDE {
                start_address: 0,
                size_in_bytes: 0,
                stride_in_bytes: 0,
            },
            hit_group_table: D3D12_GPU_VIRTUAL_ADDRESS_RANGE_AND_STRIDE {
                start_address: 0,
                size_in_bytes: 0,
                stride_in_bytes: 0,
            },
            callable_shader_table: D3D12_GPU_VIRTUAL_ADDRESS_RANGE_AND_STRIDE {
                start_address: 0,
                size_in_bytes: 0,
                stride_in_bytes: 0,
            },
            width,
            height,
            depth: 1,
        }
    }

    #[inline(always)]
    pub const fn total_rays(&self) -> u64 {
        (self.width as u64) * (self.height as u64) * (self.depth as u64)
    }

    /// Validates all table alignments and dimensions according to the D3D12 spec.
    pub fn validate(&self) -> Result<(), &'static str> {
        if self.width == 0 || self.height == 0 || self.depth == 0 {
            return Err("DispatchRays dimensions must be strictly positive");
        }
        if !self.ray_generation_shader_record.is_valid() {
            return Err("Invalid RayGeneration shader record address or size");
        }
        if !self.miss_shader_table.is_valid() {
            return Err("Invalid Miss shader table address or stride");
        }
        if !self.hit_group_table.is_valid() {
            return Err("Invalid HitGroup shader table address or stride");
        }
        if !self.callable_shader_table.is_valid() {
            return Err("Invalid Callable shader table address or stride");
        }
        Ok(())
    }
}

// Shader Binding Table (SBT) Generator

/// Generator packing shader records into a contiguous, 64-byte aligned Shader Binding Table.
#[derive(Debug, Default)]
pub struct ShaderBindingTableGenerator {
    pub raygen_record: Option<ShaderRecord>,
    pub miss_records: Vec<ShaderRecord>,
    pub hit_group_records: Vec<ShaderRecord>,
    pub callable_records: Vec<ShaderRecord>,
}

impl ShaderBindingTableGenerator {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn set_raygen(&mut self, shader_id: [u8; 32], local_args: &[u8]) {
        self.raygen_record = Some(ShaderRecord::new(shader_id, local_args));
    }

    pub fn add_miss(&mut self, shader_id: [u8; 32], local_args: &[u8]) {
        self.miss_records.push(ShaderRecord::new(shader_id, local_args));
    }

    pub fn add_hit_group(&mut self, shader_id: [u8; 32], local_args: &[u8]) {
        self.hit_group_records.push(ShaderRecord::new(shader_id, local_args));
    }

    pub fn add_callable(&mut self, shader_id: [u8; 32], local_args: &[u8]) {
        self.callable_records.push(ShaderRecord::new(shader_id, local_args));
    }

    /// Formats the complete Shader Binding Table and populates D3D12_DISPATCH_RAYS_DESC.
    pub fn build_descriptor(
        &self,
        base_gpu_va: u64,
        width: u32,
        height: u32,
    ) -> Result<D3D12_DISPATCH_RAYS_DESC, &'static str> {
        if (base_gpu_va % D3D12_RAYTRACING_SHADER_TABLE_BYTE_ALIGNMENT) != 0 {
            return Err("SBT base GPU virtual address must be 64-byte aligned");
        }
        let raygen = self.raygen_record.as_ref().ok_or("RayGeneration record is mandatory")?;

        let mut current_va = base_gpu_va;

        // 1. RayGen Section (exactly one record)
        let raygen_size = (raygen.aligned_size_bytes(64) as u64).max(64);
        let raygen_range = D3D12_GPU_VIRTUAL_ADDRESS_RANGE::new(current_va, raygen_size);
        current_va += (raygen_size + 63) & !63;

        // 2. Miss Section
        let miss_stride = if self.miss_records.is_empty() {
            0
        } else {
            let max_raw = self.miss_records.iter().map(|r| r.raw_size_bytes()).max().unwrap_or(32);
            ((max_raw + 63) & !63) as u64
        };
        let miss_size = miss_stride * (self.miss_records.len() as u64);
        let miss_range = if miss_size > 0 {
            let range = D3D12_GPU_VIRTUAL_ADDRESS_RANGE_AND_STRIDE::new(current_va, miss_size, miss_stride);
            current_va += (miss_size + 63) & !63;
            range
        } else {
            D3D12_GPU_VIRTUAL_ADDRESS_RANGE_AND_STRIDE::default()
        };

        // 3. HitGroup Section
        let hit_stride = if self.hit_group_records.is_empty() {
            0
        } else {
            let max_raw = self.hit_group_records.iter().map(|r| r.raw_size_bytes()).max().unwrap_or(32);
            ((max_raw + 63) & !63) as u64
        };
        let hit_size = hit_stride * (self.hit_group_records.len() as u64);
        let hit_range = if hit_size > 0 {
            let range = D3D12_GPU_VIRTUAL_ADDRESS_RANGE_AND_STRIDE::new(current_va, hit_size, hit_stride);
            current_va += (hit_size + 63) & !63;
            range
        } else {
            D3D12_GPU_VIRTUAL_ADDRESS_RANGE_AND_STRIDE::default()
        };

        // 4. Callable Section
        let callable_stride = if self.callable_records.is_empty() {
            0
        } else {
            let max_raw = self.callable_records.iter().map(|r| r.raw_size_bytes()).max().unwrap_or(32);
            ((max_raw + 63) & !63) as u64
        };
        let callable_size = callable_stride * (self.callable_records.len() as u64);
        let callable_range = if callable_size > 0 {
            let range = D3D12_GPU_VIRTUAL_ADDRESS_RANGE_AND_STRIDE::new(current_va, callable_size, callable_stride);
            range
        } else {
            D3D12_GPU_VIRTUAL_ADDRESS_RANGE_AND_STRIDE::default()
        };

        let mut desc = D3D12_DISPATCH_RAYS_DESC::new_2d(width, height);
        desc.ray_generation_shader_record = raygen_range;
        desc.miss_shader_table = miss_range;
        desc.hit_group_table = hit_range;
        desc.callable_shader_table = callable_range;
        desc.validate()?;

        Ok(desc)
    }
}

// DispatchRays Engine & Pipeline Barrier Synchronization

/// Ray dispatch execution engine with pipeline barrier synchronization.
#[derive(Debug, Default)]
pub struct DispatchRaysEngine {
    pub total_dispatched_rays: u64,
    pub bvh_to_raytrace_barriers_flushed: u32,
}

impl DispatchRaysEngine {
    pub fn new() -> Self {
        Self::default()
    }

    /// Enforces resource barrier transition from BVH build (UAV write) to Raytracing shader read.
    pub fn insert_bvh_execution_barrier(&mut self, tlas_gpu_va: u64) -> Result<(), &'static str> {
        if (tlas_gpu_va % D3D12_RAYTRACING_SHADER_TABLE_BYTE_ALIGNMENT) != 0 {
            return Err("TLAS address must be aligned to insert execution barrier");
        }
        // Barrier: PipelineStageFlags::COMPUTE_SHADER -> PipelineStageFlags::COMPUTE_SHADER / RAYTRACING
        // State: ResourceState::UNORDERED_ACCESS -> ResourceState::NON_PIXEL_SHADER_RESOURCE
        self.bvh_to_raytrace_barriers_flushed += 1;
        Ok(())
    }

    /// Dispatches a raytracing workload after barrier validation.
    pub fn dispatch_rays(
        &mut self,
        desc: &D3D12_DISPATCH_RAYS_DESC,
        tlas_gpu_va: u64,
    ) -> Result<u64, &'static str> {
        desc.validate()?;
        self.insert_bvh_execution_barrier(tlas_gpu_va)?;

        let ray_count = desc.total_rays();
        self.total_dispatched_rays += ray_count;

        Ok(ray_count)
    }
}
