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
pub mod vrs;
pub mod foveated;
pub mod mesh_shader;
pub mod work_graphs;

pub use cmdbuf::*;
pub use compute::*;
pub use gpu_fence::*;
pub use gpu_queue::*;
pub use texture_stream::*;
pub use ring_alloc::*;
pub use cbv_pool::*;
pub use dxr_as::*;
pub use dispatch_rays::*;
pub use vrs::*;
pub use foveated::*;
pub use mesh_shader::*;
pub use work_graphs::*;

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

/// Comprehensive verification self-test suite for Direct3D 12 Variable Rate Shading (VRS Tier 2)
/// and Eye-Tracked Foveated Rasterizer Engine.
///
/// Emits sentinel [KNT-GPU-VRS-PASS] upon successful validation.
pub fn test_vrs_foveated_subsystem() -> bool {
    crate::serial_write_str("[KNT-GFX] Testing Direct3D 12 Variable Rate Shading (Tier 2) & Foveated Rasterizer...\r\n");

    // 1. Verify discrete shading rates, dimensions, and D3D12 byte conversions
    let rates = [
        (ShadingRate::Rate1x1, 1, 1, 1, 0x0),
        (ShadingRate::Rate1x2, 1, 2, 2, 0x1),
        (ShadingRate::Rate2x1, 2, 1, 2, 0x4),
        (ShadingRate::Rate2x2, 2, 2, 4, 0x5),
        (ShadingRate::Rate2x4, 2, 4, 8, 0x6),
        (ShadingRate::Rate4x2, 4, 2, 8, 0x9),
        (ShadingRate::Rate4x4, 4, 4, 16, 0xa),
    ];
    for (rate, w, h, area, d3d_val) in rates {
        if rate.width() != w || rate.height() != h || rate.pixel_area() != area {
            return false;
        }
        if rate.to_d3d12() != d3d_val {
            return false;
        }
        if ShadingRate::from_d3d12(d3d_val) != Some(rate) {
            return false;
        }
        if ShadingRate::from_dimensions(w, h) != Some(rate) {
            return false;
        }
    }

    // 2. Verify coarse pixel shading rate combiners (Passthrough, Override, Min, Max, Sum)
    if combine_shading_rates(ShadingRate::Rate1x1, ShadingRate::Rate4x4, ShadingRateCombiner::Passthrough) != ShadingRate::Rate1x1 {
        return false;
    }
    if combine_shading_rates(ShadingRate::Rate1x1, ShadingRate::Rate4x4, ShadingRateCombiner::Override) != ShadingRate::Rate4x4 {
        return false;
    }
    if combine_shading_rates(ShadingRate::Rate2x4, ShadingRate::Rate4x2, ShadingRateCombiner::Min) != ShadingRate::Rate2x2 {
        return false;
    }
    if combine_shading_rates(ShadingRate::Rate2x1, ShadingRate::Rate1x2, ShadingRateCombiner::Max) != ShadingRate::Rate2x2 {
        return false;
    }
    if combine_shading_rates(ShadingRate::Rate2x1, ShadingRate::Rate1x2, ShadingRateCombiner::Sum) != ShadingRate::Rate2x2 {
        return false;
    }
    if combine_shading_rates(ShadingRate::Rate2x2, ShadingRate::Rate2x2, ShadingRateCombiner::Sum) != ShadingRate::Rate4x4 {
        return false;
    }

    // 3. Verify screen-space Shading Rate Image (SRI) tile map generation and sampling (8x8, 16x16, 32x32)
    for gran in [TileGranularity::Tile8x8, TileGranularity::Tile16x16, TileGranularity::Tile32x32] {
        let mut sri = match ShadingRateImage::new(1280, 800, gran, ShadingRate::Rate1x1) {
            Ok(img) => img,
            Err(_) => return false,
        };
        let expected_tx = (1280 + gran.pixel_size() - 1) / gran.pixel_size();
        let expected_ty = (800 + gran.pixel_size() - 1) / gran.pixel_size();
        if sri.tiles_x() != expected_tx || sri.tiles_y() != expected_ty {
            return false;
        }
        if !sri.set_tile_rate(2, 3, ShadingRate::Rate4x4) {
            return false;
        }
        if sri.get_tile_rate(2, 3) != Some(ShadingRate::Rate4x4) {
            return false;
        }
        let sample_x = 2 * gran.pixel_size() + 2;
        let sample_y = 3 * gran.pixel_size() + 2;
        if sri.sample_pixel(sample_x, sample_y) != ShadingRate::Rate4x4 {
            return false;
        }
    }

    // 4. Verify VariableRateShadingManager pipeline and multi-source rate resolution
    let mut vrs_mgr = VariableRateShadingManager::new(VrsTier::Tier2, TileGranularity::Tile16x16);
    vrs_mgr.set_shading_rate(
        ShadingRate::Rate2x1,
        [ShadingRateCombiner::Override, ShadingRateCombiner::Min],
    );
    let mut sri = match ShadingRateImage::new(1920, 1080, TileGranularity::Tile16x16, ShadingRate::Rate2x4) {
        Ok(img) => img,
        Err(_) => return false,
    };
    sri.set_tile_rate(0, 0, ShadingRate::Rate4x2);
    if vrs_mgr.bind_shading_rate_image(sri).is_err() {
        return false;
    }
    let resolved = vrs_mgr.resolve_shading_rate(8, 8, Some(ShadingRate::Rate2x4));
    if resolved != ShadingRate::Rate2x2 {
        return false;
    }
    vrs_mgr.record_rasterization(ShadingRate::Rate2x2);
    vrs_mgr.record_rasterization(ShadingRate::Rate4x4);
    if vrs_mgr.metrics().total_pixels_rasterized != 4 + 16 {
        return false;
    }
    if vrs_mgr.metrics().effective_shader_invocations != 2 {
        return false;
    }
    if vrs_mgr.metrics().invocation_reduction_percentage() != 90 {
        return false;
    }

    // 5. Verify Eye-Tracked Foveated Rasterizer & Handheld Power Governor
    let config = FoveatedConfig::handheld_preset();
    let mut foveated = FoveatedRasterizer::new(config);
    if foveated.current_gaze() != (640, 400) {
        return false;
    }

    if foveated.classify_pixel(640, 400) != FoveatedZone::Foveal {
        return false;
    }
    if foveated.classify_pixel(640 + 400, 400) != FoveatedZone::Parafoveal {
        return false;
    }
    if foveated.classify_pixel(640 + 630, 400) != FoveatedZone::Peripheral {
        return false;
    }

    foveated.update_gaze(850, 450);
    if foveated.saccade_count() != 1 {
        return false;
    }
    foveated.center_gaze();
    if foveated.current_gaze() != (640, 400) {
        return false;
    }

    let (tot_pix, _invoc, red_pct) = match foveated.simulate_frame_rasterization() {
        Ok(res) => res,
        Err(_) => return false,
    };
    if tot_pix != 1280 * 800 {
        return false;
    }
    if red_pct < 40 || red_pct > 60 {
        return false;
    }

    let power_saved_mw = foveated.estimate_power_savings_mw(5000, red_pct);
    let bw_saved_mbs = foveated.estimate_bandwidth_savings_mbs(25000, red_pct);
    if power_saved_mw == 0 || bw_saved_mbs == 0 {
        return false;
    }

    // 6. Emit deterministic boot sentinel
    crate::serial_write_str("[KNT-GPU-VRS-PASS] Direct3D 12 Variable Rate Shading Tier 2 and eye-tracked foveated rasterizer verified\r\n");

    true
}

/// Comprehensive verification self-test suite for Direct3D 12 Mesh Shader and
/// Amplification Shader Primitive Pipeline Engine.
///
/// Emits sentinel [KNT-GPU-MESH-PASS] upon successful validation.
pub fn test_mesh_shader_subsystem() -> bool {
    crate::serial_write_str("[KNT-GFX] Testing Direct3D 12 Mesh Shader & Amplification Shader Primitive Pipeline...\r\n");

    // 1. Verify Meshlet descriptor validation and hardware limits (<= 64 vertices, <= 126 primitives)
    let valid_meshlet = MeshletDescriptor {
        vertex_offset: 0,
        vertex_count: 64,
        primitive_offset: 0,
        primitive_count: 126,
        bounds_sphere: MeshletBoundingSphere::new(Vec3::new(0.0, 0.0, 20.0), 5.0),
        bounds_cone: MeshletBoundingCone::new(Vec3::new(0.0, 0.0, 20.0), Vec3::new(0.0, 0.0, -1.0), 0.0),
    };
    if valid_meshlet.validate().is_err() {
        return false;
    }

    let invalid_verts = MeshletDescriptor {
        vertex_count: 65,
        ..valid_meshlet
    };
    if invalid_verts.validate() != Err(MeshShaderError::ExceededMaxVertices(65)) {
        return false;
    }

    let invalid_prims = MeshletDescriptor {
        primitive_count: 127,
        ..valid_meshlet
    };
    if invalid_prims.validate() != Err(MeshShaderError::ExceededMaxPrimitives(127)) {
        return false;
    }

    // 2. Verify Pipeline State Object (PSO) descriptor validation
    let as_bytecode = MeshShaderBytecode::new(0x0000_1000_0000, 2048, 32, 1, 1);
    let ms_bytecode = MeshShaderBytecode::new(0x0000_2000_0000, 4096, 64, 1, 1);
    let pso = MeshShaderPipelineState {
        amplification_shader: Some(as_bytecode),
        mesh_shader: ms_bytecode,
        pixel_shader: None,
        primitive_topology: MeshShaderPrimitiveTopology::Triangles,
        max_output_vertices: 64,
        max_output_primitives: 126,
        payload_size_bytes: 1024,
    };
    if pso.validate().is_err() {
        return false;
    }

    // 3. Verify Amplification Shader frustum culling, backface culling, and payload dispatch
    let frustum = Frustum::new([
        FrustumPlane::new(Vec3::new(0.0, 0.0, 1.0), -1.0),    // Near (z >= 1)
        FrustumPlane::new(Vec3::new(0.0, 0.0, -1.0), 100.0),  // Far (z <= 100)
        FrustumPlane::new(Vec3::new(1.0, 0.0, 0.0), 50.0),    // Left (x >= -50)
        FrustumPlane::new(Vec3::new(-1.0, 0.0, 0.0), 50.0),   // Right (x <= 50)
        FrustumPlane::new(Vec3::new(0.0, -1.0, 0.0), 50.0),   // Top (y <= 50)
        FrustumPlane::new(Vec3::new(0.0, 1.0, 0.0), 50.0),    // Bottom (y >= -50)
    ]);
    let camera_pos = Vec3::new(0.0, 0.0, 0.0);
    let mut as_dispatcher = AmplificationShaderDispatcher::new(frustum, camera_pos);

    let meshlet0 = MeshletDescriptor {
        vertex_offset: 0,
        vertex_count: 32,
        primitive_offset: 0,
        primitive_count: 48,
        bounds_sphere: MeshletBoundingSphere::new(Vec3::new(0.0, 0.0, 20.0), 2.0),
        bounds_cone: MeshletBoundingCone::new(Vec3::new(0.0, 0.0, 20.0), Vec3::new(0.0, 0.0, -1.0), 0.0),
    };
    let meshlet1 = MeshletDescriptor {
        vertex_offset: 32,
        vertex_count: 32,
        primitive_offset: 48,
        primitive_count: 48,
        bounds_sphere: MeshletBoundingSphere::new(Vec3::new(0.0, 0.0, 20.0), 2.0),
        bounds_cone: MeshletBoundingCone::new(Vec3::new(0.0, 0.0, 20.0), Vec3::new(0.0, 0.0, 1.0), 0.0),
    };
    let meshlet2 = MeshletDescriptor {
        vertex_offset: 64,
        vertex_count: 32,
        primitive_offset: 96,
        primitive_count: 48,
        bounds_sphere: MeshletBoundingSphere::new(Vec3::new(0.0, 0.0, 150.0), 2.0),
        bounds_cone: MeshletBoundingCone::new(Vec3::new(0.0, 0.0, 150.0), Vec3::new(0.0, 0.0, -1.0), 0.0),
    };
    let meshlet3 = MeshletDescriptor {
        vertex_offset: 96,
        vertex_count: 32,
        primitive_offset: 144,
        primitive_count: 48,
        bounds_sphere: MeshletBoundingSphere::new(Vec3::new(10.0, 0.0, 80.0), 2.0),
        bounds_cone: MeshletBoundingCone::new(Vec3::new(10.0, 0.0, 80.0), Vec3::new(0.0, 0.0, -1.0), 0.0),
    };

    let meshlet_db = [meshlet0, meshlet1, meshlet2, meshlet3];
    let dispatches = match as_dispatcher.dispatch_meshlets(&meshlet_db, 1) {
        Ok(d) => d,
        Err(_) => return false,
    };

    if dispatches.len() != 1 {
        return false;
    }
    let dispatch = &dispatches[0];
    if dispatch.threadgroup_count_x != 2 || dispatch.payload.meshlet_count != 2 {
        return false;
    }
    if dispatch.payload.meshlet_indices[0] != 0 || dispatch.payload.meshlet_indices[1] != 3 {
        return false;
    }

    let stats = as_dispatcher.stats();
    if stats.total_meshlets_input != 4
        || stats.meshlets_frustum_culled != 1
        || stats.meshlets_backface_culled != 1
        || stats.meshlets_surviving_dispatched != 2
        || stats.cull_rate_percentage() != 50
    {
        return false;
    }

    // 4. Verify Mesh Shader primitive expansion and output topology streamer
    let mut ms_engine = match MeshShaderEngine::new(pso) {
        Ok(eng) => eng,
        Err(_) => return false,
    };
    let stream_outputs = match ms_engine.execute_dispatches(&dispatches, &meshlet_db) {
        Ok(out) => out,
        Err(_) => return false,
    };

    if stream_outputs.len() != 2 {
        return false;
    }
    for out in &stream_outputs {
        if out.vertex_count != 32 || out.primitive_count != 48 {
            return false;
        }
        let v0 = &out.vertices[0];
        if v0.position[3] != 1.0 || v0.color[0] != 1.0 {
            return false;
        }
        for p in 0..out.primitive_count as usize {
            let tri = &out.primitives[p];
            if tri.i0 >= 32 || tri.i1 >= 32 || tri.i2 >= 32 {
                return false;
            }
        }
    }

    let ms_stats = ms_engine.stats();
    if ms_stats.meshlets_expanded != 2
        || ms_stats.vertices_streamed != 64
        || ms_stats.primitives_streamed != 96
        || ms_stats.peak_vertices_per_meshlet != 32
        || ms_stats.peak_primitives_per_meshlet != 48
    {
        return false;
    }

    // 5. Emit deterministic boot sentinel
    crate::serial_write_str("[KNT-GPU-MESH-PASS] Direct3D 12 mesh shader and amplification shader pipeline engine verified\r\n");

    true
}
