//! Direct3D 12 Low-Latency Frame Pacing & Latency Marker Engine.
//!
//! Implements Reflex / Anti-Lag 2 style proactive CPU sleep regulation, fine-grained
//! latency markers, GPU timeline fence synchronization, and end-to-end latency profiling:
//! - Latency Markers: SimulationStart, SimulationEnd, RenderSubmitStart, RenderSubmitEnd,
//!   PresentStart, PresentEnd, DriverSubmit, GpuRenderStart, GpuRenderEnd
//! - Proactive CPU sleep pacing: eliminates GPU queue backlog, locking queue depth to 0..1
//! - GPU timeline fence-aligned PresentWait synchronization
//! - Real-time end-to-end latency metric profiler & moving average estimator

#![allow(dead_code)]

extern crate alloc;
use alloc::vec::Vec;

/// Standard latency marker classification corresponding to NVIDIA Reflex / AMD Anti-Lag 2 APIs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[repr(u8)]
pub enum LatencyMarkerType {
    /// Beginning of input gathering and physics/game simulation.
    SimulationStart = 0,
    /// End of game simulation; state updates completed.
    SimulationEnd = 1,
    /// Beginning of graphics command list recording.
    RenderSubmitStart = 2,
    /// End of graphics command list recording and submission to command queue.
    RenderSubmitEnd = 3,
    /// Beginning of SwapChain::Present call.
    PresentStart = 4,
    /// End of SwapChain::Present call.
    PresentEnd = 5,
    /// OS display driver submitted work to hardware GPU ring.
    DriverSubmit = 6,
    /// GPU hardware execution began for this frame.
    GpuRenderStart = 7,
    /// GPU hardware execution completed for this frame (timeline fence signaled).
    GpuRenderEnd = 8,
}

/// Errors returned by the Frame Pacing & Latency Marker subsystem.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FramePacingError {
    /// Latency marker recorded in an invalid or out-of-order sequence.
    InvalidMarkerSequence { frame_id: u64, marker: LatencyMarkerType },
    /// Frame identifier not found in the active timeline ring.
    FrameNotFound(u64),
    /// Pacing ring buffer capacity exceeded.
    TimelineRingFull,
    /// Target frame rate or pacing interval is non-positive or invalid.
    InvalidTargetFrameRate,
    /// Fence wait synchronization timeout.
    FenceWaitTimeout,
}

/// Life-cycle status of a frame in the pacing pipeline.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum FrameStatus {
    #[default]
    Unallocated = 0,
    Simulating = 1,
    Rendering = 2,
    Presenting = 3,
    GpuInFlight = 4,
    Completed = 5,
}

/// Nanosecond timestamps recorded for a single frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct FrameTimestamps {
    pub frame_id: u64,
    pub status: FrameStatus,
    pub sim_start_ns: u64,
    pub sim_end_ns: u64,
    pub render_submit_start_ns: u64,
    pub render_submit_end_ns: u64,
    pub present_start_ns: u64,
    pub present_end_ns: u64,
    pub driver_submit_ns: u64,
    pub gpu_render_start_ns: u64,
    pub gpu_render_end_ns: u64,
    pub gpu_fence_value: u64,
}

impl FrameTimestamps {
    pub const fn new(frame_id: u64) -> Self {
        Self {
            frame_id,
            status: FrameStatus::Unallocated,
            sim_start_ns: 0,
            sim_end_ns: 0,
            render_submit_start_ns: 0,
            render_submit_end_ns: 0,
            present_start_ns: 0,
            present_end_ns: 0,
            driver_submit_ns: 0,
            gpu_render_start_ns: 0,
            gpu_render_end_ns: 0,
            gpu_fence_value: 0,
        }
    }

    /// Records a marker timestamp and advances frame lifecycle status.
    pub fn record_marker(&mut self, marker: LatencyMarkerType, timestamp_ns: u64) -> Result<(), FramePacingError> {
        match marker {
            LatencyMarkerType::SimulationStart => {
                self.sim_start_ns = timestamp_ns;
                self.status = FrameStatus::Simulating;
            }
            LatencyMarkerType::SimulationEnd => {
                self.sim_end_ns = timestamp_ns;
            }
            LatencyMarkerType::RenderSubmitStart => {
                self.render_submit_start_ns = timestamp_ns;
                self.status = FrameStatus::Rendering;
            }
            LatencyMarkerType::RenderSubmitEnd => {
                self.render_submit_end_ns = timestamp_ns;
            }
            LatencyMarkerType::PresentStart => {
                self.present_start_ns = timestamp_ns;
                self.status = FrameStatus::Presenting;
            }
            LatencyMarkerType::PresentEnd => {
                self.present_end_ns = timestamp_ns;
                self.status = FrameStatus::GpuInFlight;
            }
            LatencyMarkerType::DriverSubmit => {
                self.driver_submit_ns = timestamp_ns;
            }
            LatencyMarkerType::GpuRenderStart => {
                self.gpu_render_start_ns = timestamp_ns;
            }
            LatencyMarkerType::GpuRenderEnd => {
                self.gpu_render_end_ns = timestamp_ns;
                self.status = FrameStatus::Completed;
            }
        }
        Ok(())
    }

    /// Computes the comprehensive duration breakdown for this frame in nanoseconds.
    pub fn breakdown(&self) -> FrameLatencyBreakdown {
        let sim_time_ns = self.sim_end_ns.saturating_sub(self.sim_start_ns);
        let render_time_ns = self.render_submit_end_ns.saturating_sub(self.render_submit_start_ns);
        let present_time_ns = self.present_end_ns.saturating_sub(self.present_start_ns);
        let cpu_total_ns = self.present_end_ns.saturating_sub(self.sim_start_ns);
        let driver_queue_ns = self.gpu_render_start_ns.saturating_sub(self.driver_submit_ns);
        let gpu_render_time_ns = self.gpu_render_end_ns.saturating_sub(self.gpu_render_start_ns);
        let end_to_end_latency_ns = self.gpu_render_end_ns.saturating_sub(self.sim_start_ns);

        FrameLatencyBreakdown {
            frame_id: self.frame_id,
            sim_time_ns,
            render_time_ns,
            present_time_ns,
            cpu_total_ns,
            driver_queue_ns,
            gpu_render_time_ns,
            end_to_end_latency_ns,
        }
    }
}

/// Comprehensive latency breakdown for a single frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct FrameLatencyBreakdown {
    pub frame_id: u64,
    /// Time spent in CPU simulation and input processing.
    pub sim_time_ns: u64,
    /// Time spent recording draw calls and command lists.
    pub render_time_ns: u64,
    /// Time spent in SwapChain::Present.
    pub present_time_ns: u64,
    /// Total CPU pipeline duration (SimStart to PresentEnd).
    pub cpu_total_ns: u64,
    /// Driver queue delay before GPU began execution.
    pub driver_queue_ns: u64,
    /// Actual GPU hardware execution time.
    pub gpu_render_time_ns: u64,
    /// Total end-to-end latency from input simulation start to GPU scanout completion.
    pub end_to_end_latency_ns: u64,
}

/// Circular ring buffer storing recent frame timestamps and latency history.
#[derive(Debug, Clone)]
pub struct FrameTimelineRing {
    pub capacity: usize,
    pub frames: Vec<FrameTimestamps>,
    pub head_index: usize,
    pub total_frames_allocated: u64,
}

impl FrameTimelineRing {
    pub fn new(capacity: usize) -> Self {
        let cap = capacity.max(8);
        let mut frames = Vec::with_capacity(cap);
        for i in 0..cap {
            frames.push(FrameTimestamps::new(i as u64));
        }
        Self {
            capacity: cap,
            frames,
            head_index: 0,
            total_frames_allocated: 0,
        }
    }

    /// Allocates the next frame in the ring, recycling the oldest frame slot.
    pub fn allocate_frame(&mut self) -> u64 {
        let frame_id = self.total_frames_allocated;
        self.total_frames_allocated += 1;

        let slot = (frame_id as usize) % self.capacity;
        self.frames[slot] = FrameTimestamps::new(frame_id);
        self.head_index = slot;
        frame_id
    }

    /// Retrieves a mutable reference to a frame by its frame ID.
    pub fn get_frame_mut(&mut self, frame_id: u64) -> Option<&mut FrameTimestamps> {
        let slot = (frame_id as usize) % self.capacity;
        let frame = &mut self.frames[slot];
        if frame.frame_id == frame_id {
            Some(frame)
        } else {
            None
        }
    }

    /// Retrieves an immutable reference to a frame by its frame ID.
    pub fn get_frame(&self, frame_id: u64) -> Option<&FrameTimestamps> {
        let slot = (frame_id as usize) % self.capacity;
        let frame = &self.frames[slot];
        if frame.frame_id == frame_id {
            Some(frame)
        } else {
            None
        }
    }

    /// Records a marker timestamp for a specific frame.
    pub fn set_marker(
        &mut self,
        frame_id: u64,
        marker: LatencyMarkerType,
        timestamp_ns: u64,
    ) -> Result<(), FramePacingError> {
        let frame = self.get_frame_mut(frame_id).ok_or(FramePacingError::FrameNotFound(frame_id))?;
        frame.record_marker(marker, timestamp_ns)
    }
}
