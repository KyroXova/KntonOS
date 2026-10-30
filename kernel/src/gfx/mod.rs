//! Direct3D / Vulkan-Style Command Buffer Recording & Hardware Queue Submission Engine.
//!
//! Provides low-overhead command packet encoding, linear memory command buffer recording,
//! multi-queue hardware submission (Graphics, Compute, Copy), monotonic GPU fences, and watchdog timers.

pub mod cmdbuf;
pub mod gpu_queue;

pub use cmdbuf::*;
pub use gpu_queue::*;
