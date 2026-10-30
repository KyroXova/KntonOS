//! Milestone 100: Unified Gaming Subsystem Health Matrix & Telemetry Descriptors.
//!
//! Defines the core data structures and metrics representing the health,
//! operational state, microsecond response latencies, and memory footprints
//! across all KntonOS micro-kernel gaming subsystems.

#![allow(dead_code)]

extern crate alloc;

// -----------------------------------------------------------------------------
// Subsystem Identification
// -----------------------------------------------------------------------------

/// Enumeration of all core KntonOS gaming micro-kernel subsystems.
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SubsystemId {
    /// Physical memory, VMM paging, Slab cache, THP, and dynamic defragmenter.
    Memory = 0,
    /// Multi-core SMP scheduling, thread priority, preemption, and IPC.
    Scheduler = 1,
    /// Virtual File System, directory namespace, initramfs, and DevFS.
    Vfs = 2,
    /// NVMe high-throughput controller, submission/completion queues, and DMA buffers.
    Nvme = 3,
    /// Real-time waveform mixer, 3D spatial binaural HRTF DSP, and zero-copy audio client.
    Audio = 4,
    /// GOP Linear framebuffer, direct scanout overlay compositor, and video streamer.
    DisplayScanout = 5,
    /// Low-latency gamepad ring, 6-axis IMU gyro/accel, and touchpad engine.
    InputGamepad = 6,
    /// Active cooling fan curves, hysteresis governor, and thermal trip protection.
    ThermalFan = 7,
    /// Battery fuel gauge, smart SMBus charger, and ACPI CPPC v2 DVFS governor.
    BatteryPower = 8,
    /// Multi-zone RGB lighting controller, aura sync, and hardware LED sequences.
    RgbLighting = 9,
    /// VirtIO-GPU, software/hardware rasterizer, DirectStorage GDeflate, CBV, DXR BVH, VRS.
    GraphicsGpu = 10,
    /// VirtIO-Net, zero-copy packet ring buffers, and low-latency gaming UDP/TCP stack.
    Network = 11,
    /// Hardware video decode engine (H.264/HEVC/AV1), NV12/P010 surfaces, and DPB ring.
    VideoDecode = 12,
}

pub const TOTAL_SUBSYSTEM_COUNT: usize = 13;

impl SubsystemId {
    /// Friendly human-readable identifier name for the subsystem.
    pub const fn name(self) -> &'static str {
        match self {
            SubsystemId::Memory => "Memory & Allocators",
            SubsystemId::Scheduler => "SMP Scheduler & IPC",
            SubsystemId::Vfs => "Virtual File System",
            SubsystemId::Nvme => "NVMe Storage Engine",
            SubsystemId::Audio => "Audio Mixer & DSP",
            SubsystemId::DisplayScanout => "Display & Direct Scanout",
            SubsystemId::InputGamepad => "Gamepad & IMU Input",
            SubsystemId::ThermalFan => "Thermal & Fan Governor",
            SubsystemId::BatteryPower => "Battery & DVFS Power",
            SubsystemId::RgbLighting => "RGB LED Lighting",
            SubsystemId::GraphicsGpu => "GPU & DXR/VRS Graphics",
            SubsystemId::Network => "Network & Packet Ring",
            SubsystemId::VideoDecode => "Hardware Video Decode",
        }
    }

    /// Default critical readiness weighting (higher values indicate foundational subsystems).
    pub const fn readiness_weight(self) -> u32 {
        match self {
            SubsystemId::Memory => 15,
            SubsystemId::Scheduler => 15,
            SubsystemId::Vfs => 10,
            SubsystemId::DisplayScanout => 10,
            SubsystemId::GraphicsGpu => 10,
            SubsystemId::InputGamepad => 8,
            SubsystemId::Audio => 8,
            SubsystemId::Nvme => 6,
            SubsystemId::BatteryPower => 6,
            SubsystemId::VideoDecode => 4,
            SubsystemId::ThermalFan => 3,
            SubsystemId::Network => 3,
            SubsystemId::RgbLighting => 2,
        }
    }
}

// -----------------------------------------------------------------------------
// Operational State
// -----------------------------------------------------------------------------

/// Subsystem operational health status.
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SubsystemState {
    /// Subsystem has not yet completed initialization.
    Uninitialized = 0,
    /// Subsystem is actively executing bootstrap and self-tests.
    Initializing = 1,
    /// Subsystem is fully functional and healthy within standard operational parameters.
    Operational = 2,
    /// Subsystem is functional with minor warnings or degraded fallback mode.
    Degraded = 3,
    /// Subsystem encountered a non-fatal hardware/software fault.
    Faulted = 4,
    /// Subsystem has been shut down or disabled.
    Offline = 5,
}

impl SubsystemState {
    #[inline]
    pub fn is_healthy(self) -> bool {
        matches!(self, SubsystemState::Operational)
    }

    #[inline]
    pub fn is_functional(self) -> bool {
        matches!(self, SubsystemState::Operational | SubsystemState::Degraded)
    }
}

// -----------------------------------------------------------------------------
// Latency Profiling Metrics
// -----------------------------------------------------------------------------

/// Microsecond response and dispatch latency statistics.
#[derive(Debug, Clone, Copy)]
pub struct LatencyMetrics {
    pub min_latency_us: u32,
    pub max_latency_us: u32,
    pub avg_latency_us: u32,
    pub p99_latency_us: u32,
    pub total_samples: u64,
    pub total_accum_us: u64,
}

impl LatencyMetrics {
    pub const fn new() -> Self {
        Self {
            min_latency_us: u32::MAX,
            max_latency_us: 0,
            avg_latency_us: 0,
            p99_latency_us: 0,
            total_samples: 0,
            total_accum_us: 0,
        }
    }

    /// Record a single latency observation in microseconds.
    pub fn record(&mut self, sample_us: u32) {
        if sample_us < self.min_latency_us {
            self.min_latency_us = sample_us;
        }
        if sample_us > self.max_latency_us {
            self.max_latency_us = sample_us;
        }
        self.total_samples += 1;
        self.total_accum_us += sample_us as u64;
        self.avg_latency_us = (self.total_accum_us / self.total_samples) as u32;

        // Exponential moving average approximation for p99
        if self.p99_latency_us == 0 {
            self.p99_latency_us = sample_us;
        } else if sample_us > self.p99_latency_us {
            self.p99_latency_us = (self.p99_latency_us * 3 + sample_us) / 4;
        } else {
            self.p99_latency_us = (self.p99_latency_us * 99 + sample_us) / 100;
        }
    }
}

// -----------------------------------------------------------------------------
// Memory Footprint Audit
// -----------------------------------------------------------------------------

/// Memory utilization audit breakdown for a kernel subsystem.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MemoryFootprint {
    /// Static read-only code and lookup tables in bytes.
    pub static_bytes: usize,
    /// Dynamically allocated heap memory in bytes.
    pub heap_bytes: usize,
    /// Physical contiguous DMA and hardware ring buffer pages in bytes.
    pub dma_bytes: usize,
}

impl MemoryFootprint {
    pub const fn empty() -> Self {
        Self {
            static_bytes: 0,
            heap_bytes: 0,
            dma_bytes: 0,
        }
    }

    pub const fn new(static_bytes: usize, heap_bytes: usize, dma_bytes: usize) -> Self {
        Self {
            static_bytes,
            heap_bytes,
            dma_bytes,
        }
    }

    #[inline]
    pub const fn total_bytes(self) -> usize {
        self.static_bytes + self.heap_bytes + self.dma_bytes
    }
}

// -----------------------------------------------------------------------------
// Subsystem Health Descriptor
// -----------------------------------------------------------------------------

/// Comprehensive health record for an individual gaming micro-kernel subsystem.
#[derive(Debug, Clone, Copy)]
pub struct SubsystemHealthDescriptor {
    pub id: SubsystemId,
    pub state: SubsystemState,
    pub readiness_score: u8, // 0 to 100 percentage
    pub latency: LatencyMetrics,
    pub memory: MemoryFootprint,
    pub error_count: u32,
    pub warning_count: u32,
    pub heartbeat_timestamp_us: u64,
}

impl SubsystemHealthDescriptor {
    pub const fn new(id: SubsystemId) -> Self {
        Self {
            id,
            state: SubsystemState::Uninitialized,
            readiness_score: 0,
            latency: LatencyMetrics::new(),
            memory: MemoryFootprint::empty(),
            error_count: 0,
            warning_count: 0,
            heartbeat_timestamp_us: 0,
        }
    }

    /// Mark subsystem as fully operational with 100% readiness score.
    pub fn mark_operational(&mut self, timestamp_us: u64) {
        self.state = SubsystemState::Operational;
        self.readiness_score = 100;
        self.heartbeat_timestamp_us = timestamp_us;
    }

    /// Mark subsystem as degraded with custom readiness penalty.
    pub fn mark_degraded(&mut self, readiness_score: u8, timestamp_us: u64) {
        self.state = SubsystemState::Degraded;
        self.readiness_score = readiness_score.min(99);
        self.warning_count += 1;
        self.heartbeat_timestamp_us = timestamp_us;
    }

    /// Mark subsystem as faulted.
    pub fn mark_faulted(&mut self, timestamp_us: u64) {
        self.state = SubsystemState::Faulted;
        self.readiness_score = 0;
        self.error_count += 1;
        self.heartbeat_timestamp_us = timestamp_us;
    }

    /// Record a microsecond latency observation and update heartbeat.
    pub fn record_latency(&mut self, latency_us: u32, timestamp_us: u64) {
        self.latency.record(latency_us);
        self.heartbeat_timestamp_us = timestamp_us;
    }
}

// -----------------------------------------------------------------------------
// Unit Self-Tests for Descriptors & Latency
// -----------------------------------------------------------------------------

/// Verify subsystem descriptor creation, status transitions, and latency statistics.
pub fn test_subsystem_descriptors_and_latency() -> bool {
    let mut desc = SubsystemHealthDescriptor::new(SubsystemId::Audio);

    if desc.id != SubsystemId::Audio {
        return false;
    }
    if desc.id.name() != "Audio Mixer & DSP" {
        return false;
    }
    if desc.id.readiness_weight() != 8 {
        return false;
    }
    if desc.state != SubsystemState::Uninitialized {
        return false;
    }
    if desc.readiness_score != 0 {
        return false;
    }

    // Operational transition
    desc.mark_operational(100_000);
    if !desc.state.is_healthy() || desc.readiness_score != 100 {
        return false;
    }
    if desc.heartbeat_timestamp_us != 100_000 {
        return false;
    }

    // Latency observations: 50us, 100us, 150us
    desc.record_latency(50, 100_050);
    desc.record_latency(100, 100_150);
    desc.record_latency(150, 100_300);

    if desc.latency.total_samples != 3 {
        return false;
    }
    if desc.latency.min_latency_us != 50 {
        return false;
    }
    if desc.latency.max_latency_us != 150 {
        return false;
    }
    if desc.latency.avg_latency_us != 100 {
        return false;
    }

    // Memory footprint calculation
    desc.memory = MemoryFootprint::new(4096, 8192, 16384);
    if desc.memory.total_bytes() != 28672 {
        return false;
    }

    // Degraded transition
    desc.mark_degraded(75, 101_000);
    if desc.state != SubsystemState::Degraded || desc.readiness_score != 75 || desc.warning_count != 1 {
        return false;
    }

    true
}
