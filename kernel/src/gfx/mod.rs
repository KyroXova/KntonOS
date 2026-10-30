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
