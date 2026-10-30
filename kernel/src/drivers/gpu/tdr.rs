//! GPU Timeout Detection and Recovery (TDR) subsystem.

use alloc::collections::BTreeMap;
use alloc::vec::Vec;
use crate::drivers::pci::flr::{MockPciConfigSpace, PciLinkState, PcieResetError};

// Constants and Timing Thresholds.

/// Default GPU execution timeout before declaring engine hang (2,000 ms / 2.0s).
pub const GPU_TDR_DEFAULT_TIMEOUT_MS: u64 = 2000;

/// Maximum consecutive TDR resets allowed within sampling window before disabling acceleration.
pub const GPU_TDR_MAX_CONSECUTIVE_RESETS: u32 = 5;

/// Rolling window for rapid recovery failure thresholding (60,000 ms / 60s).
pub const GPU_TDR_WINDOW_MS: u64 = 60_000;

// GPU Engine & Fence Progression Data Structures.

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

// Non-Destructive Context Recovery & Resource Reclamation.

/// State of a userland or compositor graphics context.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContextState {
    Active,
    GuiltyReset,   // Culprit behind GPU hang; evicted and faulted
    InnocentReset, // Collateral impact during reset; restored seamlessly
    Terminated,
}

/// Graphics hardware allocation tracked by the kernel.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GpuResourceAllocation {
    pub resource_id: u32,
    pub context_id: u32,
    pub size_bytes: usize,
    pub is_shared_or_scanout: bool,
    pub is_orphan: bool,
}

/// Ring buffer descriptor for submission commands.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GpuRingBuffer {
    pub engine: GpuEngineType,
    pub ring_size_bytes: usize,
    pub head_ptr: u32,
    pub tail_ptr: u32,
    pub sequence_id: u64,
    pub is_quiesced: bool,
}

impl GpuRingBuffer {
    pub fn new(engine: GpuEngineType, ring_size_bytes: usize) -> Self {
        Self {
            engine,
            ring_size_bytes,
            head_ptr: 0,
            tail_ptr: 0,
            sequence_id: 1,
            is_quiesced: false,
        }
    }

    /// Re-initializes ring pointers, resets head/tail registers, and increments sequence ID.
    pub fn reinitialize(&mut self) {
        self.head_ptr = 0;
        self.tail_ptr = 0;
        self.sequence_id += 1;
        self.is_quiesced = false;
    }
}

/// TDR Recovery Action and Result Outcome.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TdrRecoveryReport {
    pub hung_engine: GpuEngineType,
    pub guilty_context_id: u32,
    pub innocent_contexts_preserved: Vec<u32>,
    pub orphan_resources_reclaimed: usize,
    pub reclaimed_bytes: usize,
    pub rings_reinitialized: usize,
    pub link_restored: PciLinkState,
}

/// Central GPU Timeout Detection and Recovery (TDR) Subsystem Manager.
pub struct GpuTdrManager {
    pub watchdogs: BTreeMap<GpuEngineType, GpuEngineWatchdog>,
    pub contexts: BTreeMap<u32, ContextState>,
    pub resources: BTreeMap<u32, GpuResourceAllocation>,
    pub ring_buffers: BTreeMap<GpuEngineType, GpuRingBuffer>,
    pub total_tdr_events: u64,
    pub acceleration_enabled: bool,
}

impl GpuTdrManager {
    pub fn new(default_timeout_ms: u64, current_time_ms: u64) -> Self {
        let mut watchdogs = BTreeMap::new();
        let mut ring_buffers = BTreeMap::new();

        let engines = [
            GpuEngineType::Render3D,
            GpuEngineType::Compute,
            GpuEngineType::BlitCopy,
            GpuEngineType::VideoDecode,
            GpuEngineType::VideoEncode,
        ];

        for &eng in &engines {
            watchdogs.insert(eng, GpuEngineWatchdog::new(eng, default_timeout_ms, current_time_ms));
            ring_buffers.insert(eng, GpuRingBuffer::new(eng, 64 * 1024)); // 64 KiB rings
        }

        Self {
            watchdogs,
            contexts: BTreeMap::new(),
            resources: BTreeMap::new(),
            ring_buffers,
            total_tdr_events: 0,
            acceleration_enabled: true,
        }
    }

    /// Registers a graphics client context.
    pub fn register_context(&mut self, context_id: u32) {
        self.contexts.insert(context_id, ContextState::Active);
    }

    /// Allocates and tracks a GPU resource bound to a context.
    pub fn track_resource(
        &mut self,
        resource_id: u32,
        context_id: u32,
        size_bytes: usize,
        is_shared_or_scanout: bool,
    ) {
        self.resources.insert(
            resource_id,
            GpuResourceAllocation {
                resource_id,
                context_id,
                size_bytes,
                is_shared_or_scanout,
                is_orphan: false,
            },
        );
    }

    /// Executes complete TDR recovery sequence following a detected hang.
    pub fn recover_from_hang(
        &mut self,
        hung_fence: EngineFence,
        pci_space: &mut MockPciConfigSpace,
        current_time_ms: u64,
    ) -> Result<TdrRecoveryReport, PcieResetError> {
        self.total_tdr_events += 1;

        // Step 1: Execute hardware FLR reset
        let link_restored = pci_space.simulate_flr_execution()?;

        // Step 2: Context triage (guilty vs innocent)
        let guilty_id = hung_fence.context_id;
        let mut innocent_preserved = Vec::new();

        for (&ctx_id, state) in self.contexts.iter_mut() {
            if ctx_id == guilty_id {
                *state = ContextState::GuiltyReset;
            } else if *state == ContextState::Active {
                *state = ContextState::InnocentReset; // Marked innocent, preserved
                innocent_preserved.push(ctx_id);
            }
        }

        // Step 3: Reclaim orphan resources owned by guilty context
        let mut reclaimed_count = 0;
        let mut reclaimed_bytes = 0;

        let mut orphan_keys = Vec::new();
        for (&res_id, res) in self.resources.iter_mut() {
            if res.context_id == guilty_id {
                // If it's scanout or shared, do not destroy display continuity; otherwise mark orphan and reclaim
                if !res.is_shared_or_scanout {
                    res.is_orphan = true;
                    reclaimed_count += 1;
                    reclaimed_bytes += res.size_bytes;
                    orphan_keys.push(res_id);
                }
            }
        }

        // Remove private orphan resources
        for k in orphan_keys {
            self.resources.remove(&k);
        }

        // Step 4: Re-initialize ring buffers
        let mut rings_count = 0;
        for ring in self.ring_buffers.values_mut() {
            ring.reinitialize();
            rings_count += 1;
        }

        // Step 5: Reset watchdog on hung engine
        if let Some(wd) = self.watchdogs.get_mut(&hung_fence.engine) {
            wd.reset_watchdog(current_time_ms);
            if wd.consecutive_hang_count >= GPU_TDR_MAX_CONSECUTIVE_RESETS {
                self.acceleration_enabled = false;
            }
        }

        // Restore innocent contexts back to Active
        for ctx_id in &innocent_preserved {
            if let Some(state) = self.contexts.get_mut(ctx_id) {
                *state = ContextState::Active;
            }
        }

        Ok(TdrRecoveryReport {
            hung_engine: hung_fence.engine,
            guilty_context_id: guilty_id,
            innocent_contexts_preserved: innocent_preserved,
            orphan_resources_reclaimed: reclaimed_count,
            reclaimed_bytes,
            rings_reinitialized: rings_count,
            link_restored,
        })
    }
}

// Unit Tests / Verification for TDR Recovery & Context Isolation.

/// Unit test verifying non-destructive context recovery and ring buffer re-initialization.
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

pub fn verify_tdr_context_recovery_and_reclamation() -> bool {
    let mut tdr = GpuTdrManager::new(2000, 1000);
    let mut pci = MockPciConfigSpace::new_mock_pcie_gpu(0x10DE, 0x1F02, true);

    // Register contexts: Context 1 = Compositor (innocent), Context 2 = Buggy App (guilty)
    tdr.register_context(1);
    tdr.register_context(2);

    // Track resources
    // Context 1: Scanout buffer 1920x1080 (8 MB), private shader buffer (1 MB)
    tdr.track_resource(10, 1, 8 * 1024 * 1024, true);
    tdr.track_resource(11, 1, 1024 * 1024, false);

    // Context 2: Private texture (4 MB), private vertex buffer (2 MB), shared IPC buffer (1 MB)
    tdr.track_resource(20, 2, 4 * 1024 * 1024, false);
    tdr.track_resource(21, 2, 2 * 1024 * 1024, false);
    tdr.track_resource(22, 2, 1024 * 1024, true); // shared surface

    // Simulate hang on Render3D caused by Context 2
    let hung_fence = EngineFence::new(555, GpuEngineType::Render3D, 2, 2000);
    if let Some(wd) = tdr.watchdogs.get_mut(&GpuEngineType::Render3D) {
        wd.submit_fence(hung_fence);
        let timed_out = wd.poll_watchdog(4500).expect("Watchdog should detect timeout");
        assert_eq!(timed_out.fence_id, 555);
    }

    // Execute TDR recovery
    let report = match tdr.recover_from_hang(hung_fence, &mut pci, 4500) {
        Ok(r) => r,
        Err(_) => return false,
    };

    // Verify recovery report
    if report.guilty_context_id != 2 {
        return false;
    }
    if report.innocent_contexts_preserved != [1] {
        return false;
    }
    // Context 2 had 2 private resources (20, 21) => 6 MB reclaimed
    if report.orphan_resources_reclaimed != 2 || report.reclaimed_bytes != 6 * 1024 * 1024 {
        return false;
    }
    if report.rings_reinitialized != 5 {
        return false;
    }

    // Context 1 must still be Active and have all its resources intact
    if tdr.contexts.get(&1) != Some(&ContextState::Active) {
        return false;
    }
    if !tdr.resources.contains_key(&10) || !tdr.resources.contains_key(&11) {
        return false;
    }

    // Context 2 must be GuiltyReset; its private resources (20, 21) deleted, shared resource (22) preserved
    if tdr.contexts.get(&2) != Some(&ContextState::GuiltyReset) {
        return false;
    }
    if tdr.resources.contains_key(&20) || tdr.resources.contains_key(&21) {
        return false;
    }
    if !tdr.resources.contains_key(&22) {
        return false;
    }

    // Ring buffer check: Render3D ring head/tail must be 0, sequence incremented
    let render_ring = tdr.ring_buffers.get(&GpuEngineType::Render3D).unwrap();
    if render_ring.head_ptr != 0 || render_ring.tail_ptr != 0 || render_ring.sequence_id != 2 {
        return false;
    }

    true
}

/// Comprehensive verification suite for PCIe FLR hardware reset and GPU TDR recovery.
pub fn test_pcie_flr_and_gpu_tdr_subsystem() -> bool {
    // 1. Verify PCIe Function-Level Reset (FLR) capabilities and hardware state restoration
    if !crate::drivers::pci::flr::verify_pcie_flr_logic() {
        return false;
    }

    // 2. Verify GPU engine watchdog execution fence progression and hang detection
    if !verify_gpu_watchdog_progression() {
        return false;
    }

    // 3. Verify non-destructive context recovery, orphan resource reclamation, and ring buffer re-initialization
    if !verify_tdr_context_recovery_and_reclamation() {
        return false;
    }

    // 4. Emit deterministic boot sentinel
    crate::serial_write_str("[KNT-DRV-TDR-PASS] PCIe FLR hardware reset and GPU TDR recovery verified\r\n");

    true
}
