//! Direct3D 12 Raytracing Acceleration Structure (BVH) Memory Manager.
//!
//! Implements DXR 1.1 / Vulkan KHR Raytracing Bounding Volume Hierarchy (BVH) structures:
//! - Bottom-Level Acceleration Structure (BLAS) for triangle meshes and procedural AABBs
//! - 64-byte D3D12_RAYTRACING_INSTANCE_DESC formatting for Top-Level Acceleration Structure (TLAS)
//! - 64-byte BVH memory alignment (D3D12_RAYTRACING_ACCELERATION_STRUCTURE_BYTE_ALIGNMENT)
//! - Pre-build and post-build memory size estimation and descriptor layout

#![allow(dead_code)]

extern crate alloc;
use alloc::vec::Vec;

/// Required byte alignment for GPU virtual addresses of raytracing acceleration structures.
pub const D3D12_RAYTRACING_ACCELERATION_STRUCTURE_BYTE_ALIGNMENT: u64 = 64;

/// Required byte alignment for instance descriptors in GPU memory.
pub const D3D12_RAYTRACING_INSTANCE_DESCS_BYTE_ALIGNMENT: u64 = 16;

// -----------------------------------------------------------------------------
// Raytracing Geometry Descriptors (BLAS Input)
// -----------------------------------------------------------------------------

/// Geometry type within a Bottom-Level Acceleration Structure.
#[repr(u32)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RaytracingGeometryType {
    Triangles = 0,
    Aabbs = 1,
}

/// Geometry flags controlling ray intersection behavior.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RaytracingGeometryFlags(pub u32);

impl RaytracingGeometryFlags {
    pub const NONE: Self = Self(0);
    /// Geometry contains no transparent/any-hit triangles (fast traversal).
    pub const OPAQUE: Self = Self(1 << 0);
    /// Prevents duplicate any-hit shader invocations for this geometry.
    pub const NO_DUPLICATE_ANYHIT_INVOCATION: Self = Self(1 << 1);

    #[inline(always)]
    pub const fn bits(&self) -> u32 {
        self.0
    }
}

/// Triangle mesh geometry description.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RaytracingGeometryTrianglesDesc {
    pub transform3x4_gpu_va: u64,
    pub index_format: u32,
    pub vertex_format: u32,
    pub index_count: u32,
    pub vertex_count: u32,
    pub index_buffer_gpu_va: u64,
    pub vertex_buffer_gpu_va: u64,
    pub vertex_stride_bytes: u64,
}

impl RaytracingGeometryTrianglesDesc {
    pub const fn new(
        vertex_buffer_gpu_va: u64,
        vertex_count: u32,
        vertex_stride_bytes: u64,
        index_buffer_gpu_va: u64,
        index_count: u32,
    ) -> Self {
        Self {
            transform3x4_gpu_va: 0,
            index_format: 42, // DXGI_FORMAT_R32_UINT
            vertex_format: 6, // DXGI_FORMAT_R32G32B32_FLOAT
            index_count,
            vertex_count,
            index_buffer_gpu_va,
            vertex_buffer_gpu_va,
            vertex_stride_bytes,
        }
    }

    #[inline(always)]
    pub const fn primitive_count(&self) -> u32 {
        if self.index_count > 0 {
            self.index_count / 3
        } else {
            self.vertex_count / 3
        }
    }
}

/// Generic raytracing geometry container.
#[derive(Debug, Clone, PartialEq)]
pub struct RaytracingGeometryDesc {
    pub geometry_type: RaytracingGeometryType,
    pub flags: RaytracingGeometryFlags,
    pub triangles: RaytracingGeometryTrianglesDesc,
}

impl RaytracingGeometryDesc {
    pub const fn triangles(flags: RaytracingGeometryFlags, triangles: RaytracingGeometryTrianglesDesc) -> Self {
        Self {
            geometry_type: RaytracingGeometryType::Triangles,
            flags,
            triangles,
        }
    }
}

/// Bottom-Level Acceleration Structure (BLAS) descriptor.
#[derive(Debug, Clone, PartialEq)]
pub struct BottomLevelAsDesc {
    pub blas_id: u32,
    pub geometries: Vec<RaytracingGeometryDesc>,
    pub gpu_virtual_address: u64,
}

impl BottomLevelAsDesc {
    pub fn new(blas_id: u32, gpu_virtual_address: u64) -> Result<Self, &'static str> {
        if (gpu_virtual_address % D3D12_RAYTRACING_ACCELERATION_STRUCTURE_BYTE_ALIGNMENT) != 0 {
            return Err("BLAS GPU virtual address must be 64-byte aligned");
        }
        Ok(Self {
            blas_id,
            geometries: Vec::new(),
            gpu_virtual_address,
        })
    }

    pub fn add_geometry(&mut self, geom: RaytracingGeometryDesc) {
        self.geometries.push(geom);
    }

    /// Total geometric primitives contained across all geometries in this BLAS.
    pub fn total_primitive_count(&self) -> u32 {
        self.geometries.iter().map(|g| g.triangles.primitive_count()).sum()
    }

    /// Estimates uncompacted BVH storage requirements in bytes.
    pub fn estimated_result_size_bytes(&self) -> u64 {
        let prims = self.total_primitive_count() as u64;
        // ~64 bytes header + ~96 bytes per triangle primitive for BVH node tree
        let raw = 128 + prims * 96;
        (raw + 63) & !63 // 64-byte align
    }

    /// Estimates scratch buffer requirements needed during GPU BVH construction.
    pub fn estimated_scratch_size_bytes(&self) -> u64 {
        let prims = self.total_primitive_count() as u64;
        let raw = 256 + prims * 64;
        (raw + 255) & !255 // 256-byte scratch align
    }
}

// -----------------------------------------------------------------------------
// Top-Level Acceleration Structure (TLAS) & Instance Descriptors
// -----------------------------------------------------------------------------

/// Exact 64-byte hardware instance descriptor (D3D12_RAYTRACING_INSTANCE_DESC / VkAccelerationStructureInstanceKHR).
#[repr(C, align(16))]
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct D3D12_RAYTRACING_INSTANCE_DESC {
    /// 3x4 affine transformation matrix in row-major order.
    pub transform: [f32; 12],
    /// Bits [23:0]: InstanceID, Bits [31:24]: InstanceMask.
    pub instance_id_and_mask: u32,
    /// Bits [23:0]: InstanceContributionToHitGroupIndex, Bits [31:24]: Flags.
    pub instance_contribution_and_flags: u32,
    /// GPU virtual address of the referenced Bottom-Level Acceleration Structure (BLAS).
    pub acceleration_structure: u64,
}

impl D3D12_RAYTRACING_INSTANCE_DESC {
    pub const fn identity(instance_id: u32, hit_group_index: u32, blas_gpu_va: u64) -> Self {
        Self {
            transform: [
                1.0, 0.0, 0.0, 0.0,
                0.0, 1.0, 0.0, 0.0,
                0.0, 0.0, 1.0, 0.0,
            ],
            instance_id_and_mask: (instance_id & 0x00FF_FFFF) | (0xFF << 24),
            instance_contribution_and_flags: hit_group_index & 0x00FF_FFFF,
            acceleration_structure: blas_gpu_va,
        }
    }

    pub const fn new(
        transform: [f32; 12],
        instance_id: u32,
        mask: u8,
        hit_group_index: u32,
        flags: u8,
        blas_gpu_va: u64,
    ) -> Self {
        Self {
            transform,
            instance_id_and_mask: (instance_id & 0x00FF_FFFF) | ((mask as u32) << 24),
            instance_contribution_and_flags: (hit_group_index & 0x00FF_FFFF) | ((flags as u32) << 24),
            acceleration_structure: blas_gpu_va,
        }
    }

    #[inline(always)]
    pub const fn instance_id(&self) -> u32 {
        self.instance_id_and_mask & 0x00FF_FFFF
    }

    #[inline(always)]
    pub const fn instance_mask(&self) -> u8 {
        ((self.instance_id_and_mask >> 24) & 0xFF) as u8
    }

    #[inline(always)]
    pub const fn hit_group_index(&self) -> u32 {
        self.instance_contribution_and_flags & 0x00FF_FFFF
    }
}

/// Top-Level Acceleration Structure (TLAS) descriptor.
#[derive(Debug, Clone, PartialEq)]
pub struct TopLevelAsDesc {
    pub tlas_id: u32,
    pub instances: Vec<D3D12_RAYTRACING_INSTANCE_DESC>,
    pub gpu_virtual_address: u64,
}

impl TopLevelAsDesc {
    pub fn new(tlas_id: u32, gpu_virtual_address: u64) -> Result<Self, &'static str> {
        if (gpu_virtual_address % D3D12_RAYTRACING_ACCELERATION_STRUCTURE_BYTE_ALIGNMENT) != 0 {
            return Err("TLAS GPU virtual address must be 64-byte aligned");
        }
        Ok(Self {
            tlas_id,
            instances: Vec::new(),
            gpu_virtual_address,
        })
    }

    pub fn add_instance(&mut self, instance: D3D12_RAYTRACING_INSTANCE_DESC) {
        self.instances.push(instance);
    }

    #[inline(always)]
    pub fn instance_count(&self) -> u32 {
        self.instances.len() as u32
    }

    /// Estimates TLAS storage requirements in bytes.
    pub fn estimated_result_size_bytes(&self) -> u64 {
        let count = self.instance_count() as u64;
        let raw = 256 + count * 128;
        (raw + 63) & !63 // 64-byte align
    }

    /// Estimates scratch buffer requirements for TLAS building.
    pub fn estimated_scratch_size_bytes(&self) -> u64 {
        let count = self.instance_count() as u64;
        let raw = 512 + count * 64;
        (raw + 255) & !255 // 256-byte scratch align
    }
}
