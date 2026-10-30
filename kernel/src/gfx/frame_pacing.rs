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


// -----------------------------------------------------------------------------
// Dynamic Frame Pacing Regulator & Proactive CPU Sleep Engine
// -----------------------------------------------------------------------------

/// Frame pacing regulation modes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PacingMode {
    /// Pacing disabled; unconstrained CPU frame generation (leads to 2-3 frame queue backlog).
    Unconstrained = 0,
    /// Fixed target frame rate pacing (V-Sync / framerate limiter).
    #[default]
    TargetFramerate = 1,
    /// Low-latency Reflex / Anti-Lag proactive sleep regulation (locks queue depth to 0..1).
    LowLatencyReflex = 2,
    /// Reflex + Boost (forces GPU clocks to maximum and aggressive CPU pacing).
    LowLatencyBoost = 3,
}

/// Dynamic frame pacing regulator maintaining optimal 0..1 GPU queue depth.
#[derive(Debug, Clone)]
pub struct FramePacingRegulator {
    pub mode: PacingMode,
    pub target_fps: f32,
    pub max_queue_depth: u32,
    pub moving_avg_cpu_time_ns: u64,
    pub moving_avg_gpu_time_ns: u64,
    pub total_proactive_sleep_ns: u64,
    pub total_throttled_frames: u64,
    pub total_frames_paced: u64,
    pub alpha_smoothing: f32, // Exponential moving average weight (e.g. 0.15)
}

impl Default for FramePacingRegulator {
    fn default() -> Self {
        Self::new(PacingMode::LowLatencyReflex, 144.0, 1)
    }
}

impl FramePacingRegulator {
    pub fn new(mode: PacingMode, target_fps: f32, max_queue_depth: u32) -> Self {
        Self {
            mode,
            target_fps: if target_fps > 0.0 { target_fps } else { 60.0 },
            max_queue_depth: max_queue_depth.max(1),
            moving_avg_cpu_time_ns: 4_000_000,  // initial default: 4ms
            moving_avg_gpu_time_ns: 6_000_000,  // initial default: 6ms
            total_proactive_sleep_ns: 0,
            total_throttled_frames: 0,
            total_frames_paced: 0,
            alpha_smoothing: 0.20,
        }
    }

    /// Updates exponential moving average of CPU and GPU work durations.
    pub fn update_durations(&mut self, cpu_duration_ns: u64, gpu_duration_ns: u64) {
        if cpu_duration_ns > 0 {
            let old_cpu = self.moving_avg_cpu_time_ns as f32;
            let new_cpu = old_cpu * (1.0 - self.alpha_smoothing) + (cpu_duration_ns as f32) * self.alpha_smoothing;
            self.moving_avg_cpu_time_ns = new_cpu as u64;
        }
        if gpu_duration_ns > 0 {
            let old_gpu = self.moving_avg_gpu_time_ns as f32;
            let new_gpu = old_gpu * (1.0 - self.alpha_smoothing) + (gpu_duration_ns as f32) * self.alpha_smoothing;
            self.moving_avg_gpu_time_ns = new_gpu as u64;
        }
    }

    /// Evaluates whether CPU frame submission must be throttled to prevent render queue backlog.
    #[inline(always)]
    pub fn should_throttle(&self, in_flight_gpu_frames: u32) -> bool {
        match self.mode {
            PacingMode::Unconstrained => false,
            _ => in_flight_gpu_frames >= self.max_queue_depth,
        }
    }

    /// Computes the optimal proactive CPU sleep duration prior to initiating the next frame's simulation.
    ///
    /// Goal: Delay CPU simulation start such that the CPU finishes command recording and submits
    /// precisely when the GPU finishes executing the previous frame. This minimizes the time
    /// input sits in the queue, cutting input-to-display latency drastically.
    pub fn compute_proactive_sleep(
        &mut self,
        current_time_ns: u64,
        last_gpu_finish_estimated_ns: u64,
        in_flight_gpu_frames: u32,
    ) -> u64 {
        self.total_frames_paced += 1;

        if self.mode == PacingMode::Unconstrained {
            return 0;
        }

        let mut sleep_ns = 0u64;

        // 1. If currently at or exceeding max queue depth, must wait for oldest frame to finish
        if self.should_throttle(in_flight_gpu_frames) {
            self.total_throttled_frames += 1;
            if last_gpu_finish_estimated_ns > current_time_ns {
                sleep_ns = last_gpu_finish_estimated_ns - current_time_ns;
            }
        }

        // 2. Reflex proactive sleep: Align CPU completion with GPU availability
        if (self.mode == PacingMode::LowLatencyReflex || self.mode == PacingMode::LowLatencyBoost)
            && last_gpu_finish_estimated_ns > current_time_ns
        {
            let remaining_gpu_time = last_gpu_finish_estimated_ns - current_time_ns;
            let expected_cpu_time = self.moving_avg_cpu_time_ns;

            if remaining_gpu_time > expected_cpu_time {
                // Sleep the delta so CPU finishes right when GPU is free
                let reflex_sleep = remaining_gpu_time - expected_cpu_time;
                sleep_ns = sleep_ns.max(reflex_sleep);
            }
        }

        // 3. Target framerate pacing: Cap frame rate if target_fps is set
        if self.mode == PacingMode::TargetFramerate && self.target_fps > 0.0 {
            let target_interval_ns = (1_000_000_000.0 / self.target_fps) as u64;
            let total_estimated = self.moving_avg_cpu_time_ns.max(self.moving_avg_gpu_time_ns);
            if target_interval_ns > total_estimated {
                let fps_sleep = target_interval_ns - total_estimated;
                sleep_ns = sleep_ns.max(fps_sleep);
            }
        }

        self.total_proactive_sleep_ns += sleep_ns;
        sleep_ns
    }
}


// -----------------------------------------------------------------------------
// GPU Fence-Aligned PresentWait & Latency Metric Profiler
// -----------------------------------------------------------------------------

/// Real-time statistical profiler tracking end-to-end latency and queue metrics.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct LatencyMetricProfiler {
    pub total_samples: u64,
    pub avg_end_to_end_latency_ns: u64,
    pub min_end_to_end_latency_ns: u64,
    pub max_end_to_end_latency_ns: u64,
    pub avg_sim_time_ns: u64,
    pub avg_render_time_ns: u64,
    pub avg_driver_queue_latency_ns: u64,
    pub avg_gpu_render_time_ns: u64,
    pub cumulative_latency_ns: u64,
}

impl LatencyMetricProfiler {
    pub fn new() -> Self {
        Self {
            min_end_to_end_latency_ns: u64::MAX,
            ..Default::default()
        }
    }

    /// Records a completed frame's breakdown into profiling statistics.
    pub fn record_frame(&mut self, breakdown: &FrameLatencyBreakdown) {
        let lat = breakdown.end_to_end_latency_ns;
        self.total_samples += 1;
        self.cumulative_latency_ns += lat;

        if lat < self.min_end_to_end_latency_ns {
            self.min_end_to_end_latency_ns = lat;
        }
        if lat > self.max_end_to_end_latency_ns {
            self.max_end_to_end_latency_ns = lat;
        }

        self.avg_end_to_end_latency_ns = self.cumulative_latency_ns / self.total_samples;

        // Exponential moving average for sub-durations
        let alpha = 0.15f32;
        self.avg_sim_time_ns = ((self.avg_sim_time_ns as f32) * (1.0 - alpha) + (breakdown.sim_time_ns as f32) * alpha) as u64;
        self.avg_render_time_ns = ((self.avg_render_time_ns as f32) * (1.0 - alpha) + (breakdown.render_time_ns as f32) * alpha) as u64;
        self.avg_driver_queue_latency_ns = ((self.avg_driver_queue_latency_ns as f32) * (1.0 - alpha) + (breakdown.driver_queue_ns as f32) * alpha) as u64;
        self.avg_gpu_render_time_ns = ((self.avg_gpu_render_time_ns as f32) * (1.0 - alpha) + (breakdown.gpu_render_time_ns as f32) * alpha) as u64;
    }
}

/// Direct3D 12 Low-Latency Frame Pacing & Latency Marker Engine.
///
/// Unifies latency markers, proactive sleep regulation, GPU fence timeline synchronization,
/// and end-to-end latency profiling.
#[derive(Debug, Clone)]
pub struct LowLatencyPacingEngine {
    pub ring: FrameTimelineRing,
    pub regulator: FramePacingRegulator,
    pub profiler: LatencyMetricProfiler,
    pub last_completed_gpu_fence: u64,
    pub last_completed_frame_id: u64,
    pub in_flight_frames_count: u32,
    pub simulated_clock_ns: u64,
    pub estimated_gpu_idle_ns: u64,
}

impl LowLatencyPacingEngine {
    pub fn new(capacity: usize, mode: PacingMode, target_fps: f32) -> Self {
        Self {
            ring: FrameTimelineRing::new(capacity),
            regulator: FramePacingRegulator::new(mode, target_fps, 1),
            profiler: LatencyMetricProfiler::new(),
            last_completed_gpu_fence: 0,
            last_completed_frame_id: 0,
            in_flight_frames_count: 0,
            simulated_clock_ns: 0,
            estimated_gpu_idle_ns: 0,
        }
    }

    /// Advances simulated timeline clock by delta nanoseconds.
    #[inline(always)]
    pub fn advance_clock(&mut self, delta_ns: u64) {
        self.simulated_clock_ns = self.simulated_clock_ns.saturating_add(delta_ns);
    }

    /// Begins a new frame cycle:
    /// Computes proactive sleep duration, sleeps if necessary, allocates frame ID,
    /// and records `SimulationStart`.
    pub fn begin_frame(&mut self) -> Result<(u64, u64), FramePacingError> {
        let sleep_ns = self.regulator.compute_proactive_sleep(
            self.simulated_clock_ns,
            self.estimated_gpu_idle_ns,
            self.in_flight_frames_count,
        );

        // Advance clock by sleep duration
        self.simulated_clock_ns += sleep_ns;

        let frame_id = self.ring.allocate_frame();
        self.ring.set_marker(frame_id, LatencyMarkerType::SimulationStart, self.simulated_clock_ns)?;

        Ok((frame_id, sleep_ns))
    }

    /// Records simulation completion.
    pub fn mark_simulation_end(&mut self, frame_id: u64) -> Result<(), FramePacingError> {
        self.ring.set_marker(frame_id, LatencyMarkerType::SimulationEnd, self.simulated_clock_ns)
    }

    /// Records render command list recording start.
    pub fn mark_render_submit_start(&mut self, frame_id: u64) -> Result<(), FramePacingError> {
        self.ring.set_marker(frame_id, LatencyMarkerType::RenderSubmitStart, self.simulated_clock_ns)
    }

    /// Records render command list recording end.
    pub fn mark_render_submit_end(&mut self, frame_id: u64) -> Result<(), FramePacingError> {
        self.ring.set_marker(frame_id, LatencyMarkerType::RenderSubmitEnd, self.simulated_clock_ns)
    }

    /// Records OS driver hardware queue submission.
    pub fn mark_driver_submit(&mut self, frame_id: u64) -> Result<(), FramePacingError> {
        self.ring.set_marker(frame_id, LatencyMarkerType::DriverSubmit, self.simulated_clock_ns)
    }

    /// Records SwapChain::Present start.
    pub fn mark_present_start(&mut self, frame_id: u64) -> Result<(), FramePacingError> {
        self.ring.set_marker(frame_id, LatencyMarkerType::PresentStart, self.simulated_clock_ns)
    }

    /// Completes SwapChain::Present, assigns GPU fence, and increments in-flight frame count.
    pub fn mark_present_end(&mut self, frame_id: u64, fence_val: u64) -> Result<(), FramePacingError> {
        self.ring.set_marker(frame_id, LatencyMarkerType::PresentEnd, self.simulated_clock_ns)?;

        if let Some(frame) = self.ring.get_frame_mut(frame_id) {
            frame.gpu_fence_value = fence_val;
        }

        self.in_flight_frames_count += 1;

        // Update estimated GPU completion timeline
        let gpu_est = self.regulator.moving_avg_gpu_time_ns;
        let base_start = self.simulated_clock_ns.max(self.estimated_gpu_idle_ns);
        self.estimated_gpu_idle_ns = base_start + gpu_est;

        Ok(())
    }

    /// Synchronously waits on the GPU timeline fence (PresentWait).
    pub fn wait_for_gpu_fence(&mut self, target_fence: u64) -> Result<u64, FramePacingError> {
        if self.last_completed_gpu_fence >= target_fence {
            return Ok(0);
        }

        let wait_ns = self.estimated_gpu_idle_ns.saturating_sub(self.simulated_clock_ns);
        self.simulated_clock_ns += wait_ns;
        self.last_completed_gpu_fence = target_fence;

        Ok(wait_ns)
    }

    /// Signals GPU frame completion from hardware fence interrupt.
    pub fn signal_gpu_completion(
        &mut self,
        frame_id: u64,
        gpu_start_ns: u64,
        gpu_end_ns: u64,
    ) -> Result<(), FramePacingError> {
        self.ring.set_marker(frame_id, LatencyMarkerType::GpuRenderStart, gpu_start_ns)?;
        self.ring.set_marker(frame_id, LatencyMarkerType::GpuRenderEnd, gpu_end_ns)?;

        if let Some(frame) = self.ring.get_frame(frame_id) {
            let breakdown = frame.breakdown();
            self.regulator.update_durations(breakdown.cpu_total_ns, breakdown.gpu_render_time_ns);
            self.profiler.record_frame(&breakdown);
            self.last_completed_gpu_fence = frame.gpu_fence_value;
        }

        self.last_completed_frame_id = frame_id;
        self.in_flight_frames_count = self.in_flight_frames_count.saturating_sub(1);

        Ok(())
    }
}
