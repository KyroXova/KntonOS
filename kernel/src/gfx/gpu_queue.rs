//! Kernel Hardware Queue Submission Manager & GPU Fence Engine.
//!
//! Implements modern low-level multi-queue submission (Direct3D 12 / Vulkan model):
//! - Independent hardware queues: Graphics (Universal), Compute, and DMA Copy
//! - Monotonic 64-bit GPU fences for cross-queue synchronization and CPU polling
//! - Hardware queue command validation and submission rings
//! - GPU Hang Watchdog Timer with timeout detection and recovery state handling

extern crate alloc;
use alloc::vec::Vec;
use crate::gfx::cmdbuf::{CommandStreamReader, GpuCommandBuffer, GpuCommandOpcode};

// -----------------------------------------------------------------------------
// Hardware Queue Types & Submission Status
// -----------------------------------------------------------------------------

/// Hardware execution queue types.
#[repr(u32)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GpuQueueType {
    /// Universal queue supporting Graphics, Compute, and Copy commands.
    Graphics = 0,
    /// Compute queue supporting compute dispatches, barriers, and memory transfers.
    Compute = 1,
    /// High-speed asynchronous DMA engine supporting copy commands only.
    Copy = 2,
}

/// Execution status of a queued command buffer batch.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SubmissionStatus {
    Pending,
    Executing,
    Completed,
    TimedOut,
}

// -----------------------------------------------------------------------------
// Monotonic 64-bit GPU Fence
// -----------------------------------------------------------------------------

/// Monotonic 64-bit hardware synchronization fence.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GpuFence {
    pub fence_id: u32,
    pub signaled_value: u64,
    pub completed_value: u64,
}

impl GpuFence {
    pub const fn new(fence_id: u32) -> Self {
        Self {
            fence_id,
            signaled_value: 0,
            completed_value: 0,
        }
    }

    /// Advances the signaled value monotonically.
    pub fn signal(&mut self, value: u64) -> Result<(), &'static str> {
        if value <= self.signaled_value {
            return Err("Fence signal value must be strictly monotonic");
        }
        self.signaled_value = value;
        Ok(())
    }

    /// Marks execution complete up to the specified value.
    pub fn complete(&mut self, value: u64) {
        if value > self.completed_value {
            self.completed_value = value.min(self.signaled_value);
        }
    }

    /// Checks if the hardware has completed work up to `value`.
    #[inline(always)]
    pub fn is_completed(&self, value: u64) -> bool {
        self.completed_value >= value
    }

    #[inline(always)]
    pub fn completed_value(&self) -> u64 {
        self.completed_value
    }

    #[inline(always)]
    pub fn signaled_value(&self) -> u64 {
        self.signaled_value
    }
}

// -----------------------------------------------------------------------------
// Hardware Queue Submissions
// -----------------------------------------------------------------------------

/// A single submission batch submitted to a hardware queue.
#[derive(Debug, Clone)]
pub struct QueueSubmission {
    pub submission_id: u64,
    pub command_buffers: Vec<GpuCommandBuffer>,
    pub wait_fence: Option<(u32, u64)>, // (fence_id, value)
    pub signal_fence: Option<(u32, u64)>, // (fence_id, value)
    pub submitted_timestamp_ms: u64,
    pub status: SubmissionStatus,
}

/// An individual hardware execution queue with an in-flight submission ring.
#[derive(Debug)]
pub struct HardwareQueue {
    pub queue_type: GpuQueueType,
    pub queue_id: u32,
    pub max_in_flight: usize,
    pub submissions: Vec<QueueSubmission>,
    pub next_submission_id: u64,
}

impl HardwareQueue {
    pub fn new(queue_type: GpuQueueType, queue_id: u32, max_in_flight: usize) -> Self {
        Self {
            queue_type,
            queue_id,
            max_in_flight,
            submissions: Vec::with_capacity(max_in_flight),
            next_submission_id: 1,
        }
    }

    /// Validates whether all command buffers in the batch are supported on this queue type.
    pub fn validate_command_buffers(&self, cmdbufs: &[GpuCommandBuffer]) -> Result<(), &'static str> {
        for cmdbuf in cmdbufs {
            let reader = CommandStreamReader::new(cmdbuf);
            for packet in reader {
                let opcode = match packet.header.opcode() {
                    Some(op) => op,
                    None => return Err("Invalid or unsupported GPU command opcode in stream"),
                };

                match self.queue_type {
                    GpuQueueType::Graphics => {
                        // Universal queue supports all opcodes
                    }
                    GpuQueueType::Compute => {
                        // Compute queue cannot execute graphics drawing or rasterizer states
                        if opcode == GpuCommandOpcode::DrawIndexed
                            || opcode == GpuCommandOpcode::SetRenderTargets
                            || opcode == GpuCommandOpcode::SetViewports
                            || opcode == GpuCommandOpcode::SetScissorRects
                        {
                            return Err("Graphics drawing commands are not permitted on Compute queue");
                        }
                    }
                    GpuQueueType::Copy => {
                        // Copy queue only supports DMA transfers and resource barriers
                        if opcode != GpuCommandOpcode::CopyBufferRegion
                            && opcode != GpuCommandOpcode::ResourceBarrier
                            && opcode != GpuCommandOpcode::Nop
                        {
                            return Err("Non-transfer commands are not permitted on DMA Copy queue");
                        }
                    }
                }
            }
        }
        Ok(())
    }

    /// Enqueues a batch of command buffers for execution.
    pub fn submit(
        &mut self,
        cmdbufs: Vec<GpuCommandBuffer>,
        wait_fence: Option<(u32, u64)>,
        signal_fence: Option<(u32, u64)>,
        timestamp_ms: u64,
    ) -> Result<u64, &'static str> {
        if self.submissions.len() >= self.max_in_flight {
            return Err("Hardware submission queue ring is full");
        }

        self.validate_command_buffers(&cmdbufs)?;

        let sub_id = self.next_submission_id;
        self.next_submission_id += 1;

        let submission = QueueSubmission {
            submission_id: sub_id,
            command_buffers: cmdbufs,
            wait_fence,
            signal_fence,
            submitted_timestamp_ms: timestamp_ms,
            status: SubmissionStatus::Pending,
        };

        self.submissions.push(submission);
        Ok(sub_id)
    }
}

// -----------------------------------------------------------------------------
// GPU Watchdog & Hang Detection
// -----------------------------------------------------------------------------

/// Hardware queue watchdog monitor to detect and recover from GPU hangs.
#[derive(Debug, Clone, Copy)]
pub struct GpuWatchdog {
    pub timeout_threshold_ms: u64,
    pub hang_detected: bool,
    pub hung_submission_id: Option<u64>,
}

impl GpuWatchdog {
    pub const fn new(timeout_threshold_ms: u64) -> Self {
        Self {
            timeout_threshold_ms,
            hang_detected: false,
            hung_submission_id: None,
        }
    }

    /// Checks whether an in-flight submission has exceeded the timeout threshold.
    pub fn check(&mut self, current_time_ms: u64, active_sub: Option<&QueueSubmission>) -> bool {
        if let Some(sub) = active_sub {
            if sub.status == SubmissionStatus::Executing || sub.status == SubmissionStatus::Pending {
                if current_time_ms >= sub.submitted_timestamp_ms {
                    let elapsed = current_time_ms - sub.submitted_timestamp_ms;
                    if elapsed > self.timeout_threshold_ms {
                        self.hang_detected = true;
                        self.hung_submission_id = Some(sub.submission_id);
                        return false; // Watchdog tripped
                    }
                }
            }
        }
        true
    }

    /// Resets watchdog state after recovery.
    pub fn reset_recovery(&mut self) {
        self.hang_detected = false;
        self.hung_submission_id = None;
    }
}

// -----------------------------------------------------------------------------
// Kernel Hardware Queue Submission Engine
// -----------------------------------------------------------------------------

/// Central GPU submission engine managing queues, fences, and the watchdog.
#[derive(Debug)]
pub struct KernelGpuSubmissionEngine {
    pub graphics_queue: HardwareQueue,
    pub compute_queue: HardwareQueue,
    pub copy_queue: HardwareQueue,
    pub fences: [GpuFence; 8],
    pub watchdog: GpuWatchdog,
}

impl KernelGpuSubmissionEngine {
    pub fn new() -> Self {
        Self {
            graphics_queue: HardwareQueue::new(GpuQueueType::Graphics, 0, 32),
            compute_queue: HardwareQueue::new(GpuQueueType::Compute, 1, 32),
            copy_queue: HardwareQueue::new(GpuQueueType::Copy, 2, 32),
            fences: [
                GpuFence::new(0),
                GpuFence::new(1),
                GpuFence::new(2),
                GpuFence::new(3),
                GpuFence::new(4),
                GpuFence::new(5),
                GpuFence::new(6),
                GpuFence::new(7),
            ],
            watchdog: GpuWatchdog::new(2000), // 2000ms GPU hang timeout
        }
    }

    /// Submits command buffers to the specified queue with wait and signal fence semantics.
    pub fn submit(
        &mut self,
        queue_type: GpuQueueType,
        cmdbufs: Vec<GpuCommandBuffer>,
        wait_fence: Option<(u32, u64)>,
        signal_fence: Option<(u32, u64)>,
        timestamp_ms: u64,
    ) -> Result<u64, &'static str> {
        // If a signal fence is specified, update fence signaled value
        if let Some((fid, val)) = signal_fence {
            let idx = fid as usize;
            if idx >= self.fences.len() {
                return Err("Fence ID out of bounds");
            }
            self.fences[idx].signal(val)?;
        }

        let queue = match queue_type {
            GpuQueueType::Graphics => &mut self.graphics_queue,
            GpuQueueType::Compute => &mut self.compute_queue,
            GpuQueueType::Copy => &mut self.copy_queue,
        };

        queue.submit(cmdbufs, wait_fence, signal_fence, timestamp_ms)
    }

    /// Simulates hardware dispatch and completion ticks across all queues.
    pub fn progress_queues(&mut self, current_time_ms: u64) -> bool {
        // Iterate over queues and complete eligible submissions
        let queues = [
            &mut self.copy_queue,
            &mut self.compute_queue,
            &mut self.graphics_queue,
        ];

        for queue in queues {
            let mut i = 0;
            while i < queue.submissions.len() {
                let sub = &mut queue.submissions[i];

                // Check watchdog against hung submission
                if !self.watchdog.check(current_time_ms, Some(sub)) {
                    sub.status = SubmissionStatus::TimedOut;
                    return false;
                }

                // Check if wait fence is satisfied
                let can_execute = if let Some((w_fid, w_val)) = sub.wait_fence {
                    let idx = w_fid as usize;
                    if idx < self.fences.len() {
                        self.fences[idx].is_completed(w_val)
                    } else {
                        false
                    }
                } else {
                    true
                };

                if can_execute {
                    sub.status = SubmissionStatus::Executing;

                    // If work has completed, retire and advance signal fence
                    if let Some((s_fid, s_val)) = sub.signal_fence {
                        let idx = s_fid as usize;
                        if idx < self.fences.len() {
                            self.fences[idx].complete(s_val);
                        }
                    }
                    sub.status = SubmissionStatus::Completed;
                    queue.submissions.remove(i);
                    continue;
                }

                i += 1;
            }
        }

        true
    }

    /// Recovers the submission engine from a watchdog timeout.
    pub fn recover_from_hang(&mut self) {
        self.graphics_queue.submissions.clear();
        self.compute_queue.submissions.clear();
        self.copy_queue.submissions.clear();
        self.watchdog.reset_recovery();
    }
}

// -----------------------------------------------------------------------------
// Unit Tests & Verification
// -----------------------------------------------------------------------------

/// Unit test verifying multi-queue hardware submission, fence ordering, and watchdog hang detection.
pub fn verify_hardware_queue_and_fence_engine() -> bool {
    use crate::gfx::cmdbuf::{CommandBufferRecorder, CopyBufferRegionPayload, DrawIndexedPayload};

    let mut engine = KernelGpuSubmissionEngine::new();

    // 1. Record Copy command buffer for Queue 2 (Copy)
    let mut copy_rec = CommandBufferRecorder::new(1024, 2);
    let copy_payload = CopyBufferRegionPayload::new(0x2000, 0x1000, 4096, 0, 0);
    if copy_rec.record_copy_buffer_region(&copy_payload).is_err() {
        return false;
    }
    let copy_cmdbuf = match copy_rec.close() {
        Ok(b) => b,
        Err(_) => return false,
    };

    // 2. Submit to Copy queue: signals Fence 0 @ value 1
    let mut copy_vec = Vec::new();
    copy_vec.push(copy_cmdbuf);
    let sub_copy = match engine.submit(
        GpuQueueType::Copy,
        copy_vec,
        None,
        Some((0, 1)),
        100, // t = 100ms
    ) {
        Ok(id) => id,
        Err(_) => return false,
    };
    if sub_copy != 1 {
        return false;
    }

    // 3. Record Compute command buffer: UAV barrier
    let mut compute_rec = CommandBufferRecorder::new(1024, 1);
    if compute_rec.record_uav_barrier(0x2000).is_err() {
        return false;
    }
    let compute_cmdbuf = match compute_rec.close() {
        Ok(b) => b,
        Err(_) => return false,
    };

    // 4. Submit to Compute queue: waits on Fence 0 @ 1, signals Fence 1 @ 1
    let mut compute_vec = Vec::new();
    compute_vec.push(compute_cmdbuf);
    let sub_comp = match engine.submit(
        GpuQueueType::Compute,
        compute_vec,
        Some((0, 1)), // Wait on copy completion
        Some((1, 1)), // Signal compute completion
        110,
    ) {
        Ok(id) => id,
        Err(_) => return false,
    };
    if sub_comp != 1 {
        return false;
    }

    // 5. Record Graphics command buffer: DrawIndexed
    let mut gfx_rec = CommandBufferRecorder::new(1024, 0);
    let draw_payload = DrawIndexedPayload::new(36, 1, 0, 0, 0);
    if gfx_rec.record_draw_indexed(&draw_payload).is_err() {
        return false;
    }
    let gfx_cmdbuf = match gfx_rec.close() {
        Ok(b) => b,
        Err(_) => return false,
    };

    // 6. Submit to Graphics queue: waits on Fence 1 @ 1, signals Fence 2 @ 1
    let mut gfx_vec = Vec::new();
    gfx_vec.push(gfx_cmdbuf);
    let sub_gfx = match engine.submit(
        GpuQueueType::Graphics,
        gfx_vec,
        Some((1, 1)), // Wait on compute completion
        Some((2, 1)), // Signal graphics render completion
        120,
    ) {
        Ok(id) => id,
        Err(_) => return false,
    };
    if sub_gfx != 1 {
        return false;
    }

    // 7. Verify queue type validation prevents illegal commands (DrawIndexed on Copy queue)
    let mut bad_copy_rec = CommandBufferRecorder::new(512, 2);
    let _ = bad_copy_rec.record_draw_indexed(&draw_payload);
    let bad_cmdbuf = bad_copy_rec.close().unwrap();
    let mut bad_vec = Vec::new();
    bad_vec.push(bad_cmdbuf);
    if engine.submit(GpuQueueType::Copy, bad_vec, None, None, 130).is_ok() {
        return false; // Should fail validation!
    }

    // 8. Progress queues through timeline:
    // At t = 150ms: Copy queue executes and completes, signaling Fence 0 @ 1.
    // Compute queue now sees Fence 0 @ 1 completed, executes and signals Fence 1 @ 1.
    // Graphics queue sees Fence 1 @ 1 completed, executes and signals Fence 2 @ 1.
    if !engine.progress_queues(150) {
        return false;
    }

    if !engine.fences[0].is_completed(1) {
        return false;
    }
    if !engine.fences[1].is_completed(1) {
        return false;
    }
    if !engine.fences[2].is_completed(1) {
        return false;
    }

    // All queues should now be retired
    if !engine.copy_queue.submissions.is_empty()
        || !engine.compute_queue.submissions.is_empty()
        || !engine.graphics_queue.submissions.is_empty()
    {
        return false;
    }

    // 9. Verify GPU Hang Watchdog Timer detection
    // Submit a long-running batch that waits on an unsatisfied fence
    let mut stall_rec = CommandBufferRecorder::new(512, 0);
    let _ = stall_rec.record_viewport(crate::gfx::cmdbuf::Viewport::new(0.0, 0.0, 800.0, 600.0, 0.0, 1.0));
    let stall_cmdbuf = stall_rec.close().unwrap();
    let mut stall_vec = Vec::new();
    stall_vec.push(stall_cmdbuf);

    let stall_id = engine
        .submit(
            GpuQueueType::Graphics,
            stall_vec,
            Some((7, 999)), // Unsat fence
            None,
            1000,
        )
        .unwrap();

    // Advance time beyond 2000ms timeout threshold (1000 + 2500 = 3500ms)
    let ok = engine.progress_queues(3500);
    if ok {
        return false; // Watchdog should have tripped!
    }
    if !engine.watchdog.hang_detected {
        return false;
    }
    if engine.watchdog.hung_submission_id != Some(stall_id) {
        return false;
    }

    // 10. Verify watchdog recovery
    engine.recover_from_hang();
    if engine.watchdog.hang_detected {
        return false;
    }
    if !engine.graphics_queue.submissions.is_empty() {
        return false;
    }

    true
}
