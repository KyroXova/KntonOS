//! Kernel-Direct Hardware Video Decode & Presentation Streamer Subsystem.
//!
//! Provides hardware-accelerated video decoding (H.264 / HEVC / AV1),
//! tiled NV12/P010 decoded surface management, reference frame buffering,
//! and zero-copy direct scanout presentation pipelines.

#![allow(dead_code)]

pub mod decode;
pub mod nv12;

/// Verification self-test for hardware video decode engine and picture parameter parsing.
pub fn test_video_decode_subsystem() -> bool {
    if !decode::test_video_decode_queue_and_parser() {
        return false;
    }
    if !nv12::test_nv12_p010_allocator_and_dpb() {
        return false;
    }
    true
}
