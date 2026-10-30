//! Direct3D / Vulkan-Style Command Buffer Recording & Hardware Queue Submission Engine.
//!
//! Provides low-overhead command packet encoding, linear memory command buffer recording,
//! multi-queue hardware submission (Graphics, Compute, Copy), monotonic GPU fences, and watchdog timers.

pub mod cmdbuf;
pub mod compute;
pub mod gpu_fence;
pub mod gpu_queue;
pub mod texture_stream;
pub mod ring_alloc;
pub mod cbv_pool;
pub mod dxr_as;
pub mod dispatch_rays;

pub use cmdbuf::*;
pub use compute::*;
pub use gpu_fence::*;
pub use gpu_queue::*;
pub use texture_stream::*;
pub use ring_alloc::*;
pub use cbv_pool::*;
pub use dxr_as::*;
pub use dispatch_rays::*;

/// Comprehensive verification self-test suite for Direct3D / Vulkan-Style Command Buffer
/// Recording and Hardware Queue Submission Engine.
///
/// Emits sentinel [KNT-GPU-CMDBUF-PASS] upon successful validation.
pub fn test_gpu_command_buffer_and_queue_subsystem() -> bool {
    // 1. Verify 64-bit compact command packet encoding, linear recording, and stream decoding
    if !verify_command_packet_encoding_and_recorder() {
        return false;
    }

    // 2. Verify D3D12/Vulkan pipeline barriers (Transition, Aliasing, UAV) and descriptor heap bindings
    if !verify_barriers_and_descriptor_bindings() {
        return false;
    }

    // 3. Verify multi-queue hardware submission (Graphics, Compute, Copy), monotonic fences, and watchdog hang detection
    if !verify_hardware_queue_and_fence_engine() {
        return false;
    }

    // 4. Emit deterministic boot sentinel
    crate::serial_write_str("[KNT-GPU-CMDBUF-PASS] Direct3D/Vulkan command buffer recorder and hardware queue submission engine verified\r\n");

    true
}


/// Comprehensive verification self-test suite for Direct3D 12 Dynamic Constant Buffer Pool
/// and Ring Allocator Engine.
///
/// Emits sentinel [KNT-GPU-CBV-PASS] upon successful validation.
pub fn test_constant_buffer_pool_subsystem() -> bool {
    crate::serial_write_str("[KNT-GFX] Testing Direct3D 12 dynamic constant buffer pool & ring allocator...\r\n");

    // 1. Verify 256-byte aligned constant buffer ring suballocation and wrap-around
    if !verify_ring_suballocation_and_alignment() {
        crate::serial_write_str("[KNT-PANIC] Constant buffer 256-byte ring suballocation verification failed!\r\n");
        return false;
    }

    // 2. Verify multi-frame ring chunk retirement synchronized with timeline fence progression
    if !verify_frame_retirement_and_fence_sync() {
        crate::serial_write_str("[KNT-PANIC] Frame retirement and timeline fence synchronization failed!\r\n");
        return false;
    }

    // 3. Verify dynamic CBV heap descriptor binder and per-draw constant update stream
    if !verify_dynamic_cbv_pool_and_binding() {
        crate::serial_write_str("[KNT-PANIC] Dynamic CBV descriptor pool and root binding failed!\r\n");
        return false;
    }

    // 4. Emit deterministic boot sentinel
    crate::serial_write_str("[KNT-GPU-CBV-PASS] Direct3D 12 constant buffer pool and ring allocator engine verified\r\n");

    true
}


/// Comprehensive verification self-test suite for Direct3D 12 Raytracing Acceleration Structure (BVH)
/// Memory Manager & DispatchRays Engine.
///
/// Emits sentinel [KNT-GPU-DXR-PASS] upon successful validation.
pub fn test_dxr_acceleration_structure_subsystem() -> bool {
    crate::serial_write_str("[KNT-GFX] Testing Direct3D 12 raytracing acceleration structures & DispatchRays...\r\n");

    // 1. Verify BLAS geometry descriptors & 64-byte alignment
    let mut blas = match BottomLevelAsDesc::new(1, 0x0000_7000_0000_1000) {
        Ok(b) => b,
        Err(_) => return false,
    };
    let tri_desc = RaytracingGeometryTrianglesDesc::new(
        0x0000_7000_0000_2000,
        1500, // 1500 vertices
        12,   // 12 bytes stride
        0x0000_7000_0000_3000,
        3000, // 3000 indices = 1000 triangles
    );
    if tri_desc.primitive_count() != 1000 {
        return false;
    }
    blas.add_geometry(RaytracingGeometryDesc::triangles(RaytracingGeometryFlags::OPAQUE, tri_desc));
    if blas.total_primitive_count() != 1000 {
        return false;
    }
    let uncompacted_size = blas.estimated_result_size_bytes();
    if (uncompacted_size % D3D12_RAYTRACING_ACCELERATION_STRUCTURE_BYTE_ALIGNMENT) != 0 {
        return false;
    }

    // Unaligned address must fail
    if BottomLevelAsDesc::new(2, 0x0000_7000_0000_1020).is_ok() {
        return false;
    }

    // 2. Verify DXR 64-byte instance descriptors and TLAS hierarchy
    let mut tlas = match TopLevelAsDesc::new(100, 0x0000_7000_0000_5000) {
        Ok(t) => t,
        Err(_) => return false,
    };
    let inst0 = D3D12_RAYTRACING_INSTANCE_DESC::identity(0, 0, blas.gpu_virtual_address);
    let inst1 = D3D12_RAYTRACING_INSTANCE_DESC::new(
        [
            2.0, 0.0, 0.0, 10.0,
            0.0, 2.0, 0.0, 0.0,
            0.0, 0.0, 2.0, 5.0,
        ],
        1,
        0xFF,
        1,
        0,
        blas.gpu_virtual_address,
    );
    tlas.add_instance(inst0);
    tlas.add_instance(inst1);
    if tlas.instance_count() != 2 || inst1.instance_id() != 1 || inst1.hit_group_index() != 1 {
        return false;
    }

    // 3. Verify 256-byte aligned scratch buffer allocator & BVH compaction
    let mut as_mgr = match RaytracingAccelerationStructureManager::new(0x0000_8000_0000_0000, 1024 * 1024) {
        Ok(m) => m,
        Err(_) => return false,
    };
    as_mgr.register_blas(blas);
    as_mgr.register_tlas(tlas);

    // Build BLAS scratch allocation
    let (_, scratch_va) = match as_mgr.build_blas(1) {
        Ok(res) => res,
        Err(_) => return false,
    };
    if (scratch_va % D3D12_RAYTRACING_ACCELERATION_STRUCTURE_SCRATCH_BYTE_ALIGNMENT) != 0 {
        return false;
    }

    // Compact BLAS (compact from uncompacted size down to ~55%)
    let compacted_size = (uncompacted_size * 55) / 100;
    let compacted_gpu_va = 0x0000_7000_0000_6000;
    let record = match as_mgr.compact_blas(1, compacted_size, compacted_gpu_va) {
        Ok(r) => r,
        Err(_) => return false,
    };
    if record.vram_saved_bytes() == 0 || record.savings_percentage() == 0 {
        return false;
    }
    // Verify TLAS instances were updated to the compacted BLAS address
    let updated_tlas = as_mgr.tlas_table.iter().find(|t| t.tlas_id == 100).unwrap();
    if updated_tlas.instances[0].acceleration_structure != compacted_gpu_va
        || updated_tlas.instances[1].acceleration_structure != compacted_gpu_va
    {
        return false;
    }

    // 4. Verify Shader Binding Table (SBT) generator & DispatchRays engine
    let mut sbt_gen = ShaderBindingTableGenerator::new();
    let raygen_id = [1u8; 32];
    let miss_id = [2u8; 32];
    let hit_id = [3u8; 32];
    let callable_id = [4u8; 32];

    sbt_gen.set_raygen(raygen_id, &[0xAA; 16]);
    sbt_gen.add_miss(miss_id, &[0xBB; 8]);
    sbt_gen.add_hit_group(hit_id, &[0xCC; 24]);
    sbt_gen.add_callable(callable_id, &[]);

    let dispatch_desc = match sbt_gen.build_descriptor(0x0000_9000_0000_0000, 1920, 1080) {
        Ok(d) => d,
        Err(_) => return false,
    };
    if dispatch_desc.total_rays() != 1920 * 1080 {
        return false;
    }

    let mut ray_engine = DispatchRaysEngine::new();
    let dispatched = match ray_engine.dispatch_rays(&dispatch_desc, updated_tlas.gpu_virtual_address) {
        Ok(r) => r,
        Err(_) => return false,
    };
    if dispatched != 1920 * 1080 || ray_engine.bvh_to_raytrace_barriers_flushed != 1 {
        return false;
    }

    // 5. Emit deterministic boot sentinel
    crate::serial_write_str("[KNT-GPU-DXR-PASS] Direct3D 12 raytracing acceleration structure manager and DispatchRays engine verified\r\n");

    true
}
