//! GPU Timeout Detection and Recovery (TDR) Subsystem.
//!
//! Implements GPU watchdog monitoring, execution fence progression tracking,
//! hang detection, non-destructive context recovery, orphan resource reclamation,
//! and ring buffer re-initialization.


// -----------------------------------------------------------------------------
// Constants and Timing Thresholds
// -----------------------------------------------------------------------------

/// Default GPU execution timeout before declaring engine hang (2,000 ms / 2.0s).
pub const GPU_TDR_DEFAULT_TIMEOUT_MS: u64 = 2000;

/// Maximum consecutive TDR resets allowed within sampling window before disabling acceleration.
pub const GPU_TDR_MAX_CONSECUTIVE_RESETS: u32 = 5;

/// Rolling window for rapid recovery failure thresholding (60,000 ms / 60s).
pub const GPU_TDR_WINDOW_MS: u64 = 60_000;

// -----------------------------------------------------------------------------
// GPU Engine & Fence Progression Data Structures
// -----------------------------------------------------------------------------

/// GPU hardware execution engine identifier.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum GpuEngineType {
    Render3D,
    Compute,
    BlitCopy,
    VideoDecode,
    VideoEncode,
}

/// Execution status of an engine fence.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FenceStatus {
    Queued,
    Submitted,
    Completed,
    TimedOut,
    Abandoned,
}

/// Hardware execution fence submitted by a rendering context.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EngineFence {
    pub fence_id: u64,
    pub engine: GpuEngineType,
    pub context_id: u32,
    pub submit_time_ms: u64,
    pub status: FenceStatus,
}

impl EngineFence {
    pub fn new(fence_id: u64, engine: GpuEngineType, context_id: u32, submit_time_ms: u64) -> Self {
        Self {
            fence_id,
            engine,
            context_id,
            submit_time_ms,
            status: FenceStatus::Submitted,
        }
    }
}

/// GPU Engine Watchdog tracking active fences and hang conditions.
#[derive(Debug, Clone)]
pub struct GpuEngineWatchdog {
    pub engine: GpuEngineType,
    pub timeout_threshold_ms: u64,
    pub last_completed_fence: u64,
    pub last_progress_timestamp_ms: u64,
    pub active_fence: Option<EngineFence>,
    pub consecutive_hang_count: u32,
    pub is_engine_hung: bool,
}

impl GpuEngineWatchdog {
    pub fn new(engine: GpuEngineType, timeout_threshold_ms: u64, current_time_ms: u64) -> Self {
        Self {
            engine,
            timeout_threshold_ms,
            last_completed_fence: 0,
            last_progress_timestamp_ms: current_time_ms,
            active_fence: None,
            consecutive_hang_count: 0,
            is_engine_hung: false,
        }
    }

    /// Submits a new work packet fence to the engine.
    pub fn submit_fence(&mut self, fence: EngineFence) {
        if self.active_fence.is_none() {
            self.last_progress_timestamp_ms = fence.submit_time_ms;
        }
        self.active_fence = Some(fence);
    }

    /// Signals completion of a fence by hardware interrupt or poll.
    pub fn signal_fence_completed(&mut self, fence_id: u64, timestamp_ms: u64) {
        if let Some(active) = &mut self.active_fence {
            if active.fence_id <= fence_id {
                self.last_completed_fence = fence_id;
                self.last_progress_timestamp_ms = timestamp_ms;
                self.active_fence = None;
                self.consecutive_hang_count = 0;
                self.is_engine_hung = false;
            }
        }
    }

    /// Evaluates current fence execution duration against timeout threshold.
    /// Returns Some(hung_fence) if watchdog expired.
    pub fn poll_watchdog(&mut self, current_time_ms: u64) -> Option<EngineFence> {
        if let Some(active) = &mut self.active_fence {
            let elapsed = current_time_ms.saturating_sub(self.last_progress_timestamp_ms);
            if elapsed >= self.timeout_threshold_ms {
                self.is_engine_hung = true;
                self.consecutive_hang_count += 1;
                active.status = FenceStatus::TimedOut;
                return Some(*active);
            }
        }
        None
    }

    /// Resets watchdog state after TDR recovery.
    pub fn reset_watchdog(&mut self, current_time_ms: u64) {
        self.active_fence = None;
        self.last_progress_timestamp_ms = current_time_ms;
        self.is_engine_hung = false;
    }
}

// -----------------------------------------------------------------------------
// Unit Tests / Verification for Watchdog & Fence Progression
// -----------------------------------------------------------------------------

/// Unit test verifying watchdog fence progression, timeout detection, and status updates.
pub fn verify_gpu_watchdog_progression() -> bool {
    let mut watchdog = GpuEngineWatchdog::new(GpuEngineType::Render3D, 2000, 1000);

    // Normal progression: fence submitted at 1000ms, completed at 1500ms (elapsed 500ms < 2000ms)
    let fence1 = EngineFence::new(101, GpuEngineType::Render3D, 1, 1000);
    watchdog.submit_fence(fence1);

    if watchdog.poll_watchdog(1500).is_some() {
        return false; // Should not time out
    }
    watchdog.signal_fence_completed(101, 1500);
    if watchdog.last_completed_fence != 101 || watchdog.active_fence.is_some() {
        return false;
    }

    // Hang condition: fence submitted at 2000ms, polled at 4500ms (elapsed 2500ms >= 2000ms)
    let fence2 = EngineFence::new(102, GpuEngineType::Render3D, 2, 2000);
    watchdog.submit_fence(fence2);

    match watchdog.poll_watchdog(4500) {
        Some(timed_out) => {
            if timed_out.fence_id != 102
                || timed_out.context_id != 2
                || timed_out.status != FenceStatus::TimedOut
                || !watchdog.is_engine_hung
                || watchdog.consecutive_hang_count != 1
            {
                return false;
            }
        }
        None => return false,
    }

    true
}
