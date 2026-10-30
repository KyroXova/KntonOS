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

pub use cmdbuf::*;
pub use compute::*;
pub use gpu_fence::*;
pub use gpu_queue::*;
pub use texture_stream::*;
pub use ring_alloc::*;

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
