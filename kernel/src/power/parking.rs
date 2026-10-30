//! Dynamic CPU Core Parking Topology Table & Runqueue Load Evaluation.
//!
//! Tracks logical CPU topology (BSP, APs, packages, NUMA nodes, park status),
//! measures exponential rolling load averages across scheduler runqueues,
//! and determines core park/unpark transitions based on load thresholds.

use alloc::vec::Vec;

// -----------------------------------------------------------------------------
// Core Parking Constants & Thresholds
// -----------------------------------------------------------------------------

/// Minimum unpark threshold percentage (e.g. 75% load on active cores prompts unparking).
pub const DEFAULT_UNPARK_LOAD_PERCENT: u32 = 75;

/// Maximum park threshold percentage (e.g. < 25% load on active cores prompts parking).
pub const DEFAULT_PARK_LOAD_PERCENT: u32 = 25;

/// Minimum unpark duration (hysteresis) to prevent thrashing / rapid flapping (in milliseconds).
pub const MIN_PARK_HYSTERESIS_MS: u64 = 100;

/// Maximum number of logical cores supported in parking table.
pub const MAX_PARKABLE_CORES: usize = 64;

// -----------------------------------------------------------------------------
// Core Topology & Parking State
// -----------------------------------------------------------------------------

/// Operational state of a logical CPU core.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CoreParkState {
    /// Core is active and available for scheduler dispatch.
    Unparked,
    /// Core is parked: placed in low-power sleep (MWAIT/HLT), scheduler excludes it.
    Parked,
    /// Core is transitioning from parked to unparked (waking).
    Waking,
    /// Core is transitioning from unparked to parked (quiescing).
    Parking,
}

/// Logical CPU Core topology and energy information.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LogicalCoreEntry {
    pub cpu_id: u32,
    pub apic_id: u32,
    pub package_id: u32,
    pub core_id: u32,
    pub numa_node: u32,
    pub is_bsp: bool,
    pub is_performance_core: bool,
    pub park_state: CoreParkState,
    pub last_state_change_ms: u64,
    pub current_load_percent: u32,
}

impl LogicalCoreEntry {
    pub fn new(
        cpu_id: u32,
        apic_id: u32,
        package_id: u32,
        core_id: u32,
        numa_node: u32,
        is_bsp: bool,
        is_performance_core: bool,
    ) -> Self {
        Self {
            cpu_id,
            apic_id,
            package_id,
            core_id,
            numa_node,
            is_bsp,
            is_performance_core,
            park_state: CoreParkState::Unparked,
            last_state_change_ms: 0,
            current_load_percent: 0,
        }
    }
}

// -----------------------------------------------------------------------------
// Runqueue Load Evaluator
// -----------------------------------------------------------------------------

/// Rolling Exponential Moving Average (EMA) load evaluator.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RollingLoadMetric {
    /// Current scaled load average (0 to 100).
    pub average_load: u32,
    /// Total threads observed in active runqueues during last sample.
    pub active_threads: u32,
    /// Capacity based on currently unparked cores.
    pub active_cores: u32,
}

impl RollingLoadMetric {
    pub fn new() -> Self {
        Self {
            average_load: 0,
            active_threads: 0,
            active_cores: 1,
        }
    }

    /// Updates the rolling load using an exponential moving average (weight 50/50).
    pub fn update(&mut self, active_threads: u32, active_cores: u32) {
        self.active_threads = active_threads;
        self.active_cores = if active_cores == 0 { 1 } else { active_cores };

        // Instantaneous load % = min(100, (active_threads * 100) / active_cores)
        let instant_load = ((active_threads as u64 * 100) / (self.active_cores as u64)).min(100) as u32;

        // Exponential smoothing: new_avg = (old_avg * 3 + instant_load * 5) / 8
        self.average_load = ((self.average_load as u64 * 3 + instant_load as u64 * 5) / 8) as u32;
    }
}

// -----------------------------------------------------------------------------
// Core Parking Manager
// -----------------------------------------------------------------------------

/// Action recommended by parking evaluation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ParkingDecision {
    NoChange,
    UnparkCore(u32),
    ParkCore(u32),
}

/// Central Dynamic Core Parking Manager.
pub struct CoreParkingManager {
    pub cores: Vec<LogicalCoreEntry>,
    pub unpark_threshold: u32,
    pub park_threshold: u32,
    pub hysteresis_ms: u64,
    pub rolling_load: RollingLoadMetric,
    pub min_unparked_cores: usize,
}

impl CoreParkingManager {
    pub fn new(unpark_threshold: u32, park_threshold: u32, min_unparked_cores: usize) -> Self {
        Self {
            cores: Vec::new(),
            unpark_threshold,
            park_threshold,
            hysteresis_ms: MIN_PARK_HYSTERESIS_MS,
            rolling_load: RollingLoadMetric::new(),
            min_unparked_cores: if min_unparked_cores == 0 { 1 } else { min_unparked_cores },
        }
    }

    /// Registers a logical core in the topology table.
    pub fn add_core(&mut self, core: LogicalCoreEntry) {
        self.cores.push(core);
    }

    /// Returns count of currently unparked cores.
    pub fn unparked_count(&self) -> usize {
        self.cores
            .iter()
            .filter(|c| c.park_state == CoreParkState::Unparked || c.park_state == CoreParkState::Waking)
            .count()
    }

    /// Returns count of currently parked cores.
    pub fn parked_count(&self) -> usize {
        self.cores
            .iter()
            .filter(|c| c.park_state == CoreParkState::Parked || c.park_state == CoreParkState::Parking)
            .count()
    }

    /// Evaluates current workload and decides whether to park or unpark a core.
    pub fn evaluate(&mut self, active_threads: u32, current_time_ms: u64) -> ParkingDecision {
        let unparked = self.unparked_count() as u32;
        self.rolling_load.update(active_threads, unparked);

        let current_load = self.rolling_load.average_load;

        // Condition 1: High load -> unpark a core
        if current_load >= self.unpark_threshold {
            // Find first parked core with elapsed hysteresis
            for core in self.cores.iter_mut() {
                if core.park_state == CoreParkState::Parked {
                    if current_time_ms.saturating_sub(core.last_state_change_ms) >= self.hysteresis_ms {
                        core.park_state = CoreParkState::Unparked;
                        core.last_state_change_ms = current_time_ms;
                        return ParkingDecision::UnparkCore(core.cpu_id);
                    }
                }
            }
        }

        // Condition 2: Low load -> park a core (never park BSP or drop below min_unparked_cores)
        if current_load <= self.park_threshold && self.unparked_count() > self.min_unparked_cores {
            // Pick non-BSP core in reverse order (e.g. higher CPU IDs first)
            for core in self.cores.iter_mut().rev() {
                if !core.is_bsp && core.park_state == CoreParkState::Unparked {
                    if current_time_ms.saturating_sub(core.last_state_change_ms) >= self.hysteresis_ms {
                        core.park_state = CoreParkState::Parked;
                        core.last_state_change_ms = current_time_ms;
                        return ParkingDecision::ParkCore(core.cpu_id);
                    }
                }
            }
        }

        ParkingDecision::NoChange
    }
}

// -----------------------------------------------------------------------------
// Unit Tests & Verification
// -----------------------------------------------------------------------------

/// Verification self-test for core parking topology and load evaluation.
pub fn verify_core_parking_topology_and_load() -> bool {
    let mut mgr = CoreParkingManager::new(DEFAULT_UNPARK_LOAD_PERCENT, DEFAULT_PARK_LOAD_PERCENT, 1);

    // Initialize 4-core topology: Core 0 (BSP), Cores 1..3 (APs)
    mgr.add_core(LogicalCoreEntry::new(0, 0, 0, 0, 0, true, true));
    mgr.add_core(LogicalCoreEntry::new(1, 1, 0, 1, 0, false, true));
    mgr.add_core(LogicalCoreEntry::new(2, 2, 0, 2, 0, false, true));
    mgr.add_core(LogicalCoreEntry::new(3, 3, 0, 3, 0, false, true));

    if mgr.unparked_count() != 4 || mgr.parked_count() != 0 {
        return false;
    }

    // Step 1: Low load evaluation (1 active thread on 4 cores = 25% instantaneous)
    // Run several iterations to allow EMA to decay below park threshold
    let mut decision = ParkingDecision::NoChange;
    for t in 1..=5 {
        decision = mgr.evaluate(1, t * 150);
        if let ParkingDecision::ParkCore(_) = decision {
            break;
        }
    }

    // Must park a non-BSP core (Core 3)
    if decision != ParkingDecision::ParkCore(3) {
        return false;
    }
    if mgr.unparked_count() != 3 || mgr.parked_count() != 1 {
        return false;
    }

    // Step 2: High load surge (12 active threads on 3 active cores = 400% load)
    mgr.evaluate(12, 800);
    let unpark_dec = mgr.evaluate(12, 1000);
    if unpark_dec != ParkingDecision::UnparkCore(3) {
        return false;
    }
    if mgr.unparked_count() != 4 || mgr.parked_count() != 0 {
        return false;
    }

    // Ensure BSP is never parked
    for core in &mgr.cores {
        if core.is_bsp && core.park_state != CoreParkState::Unparked {
            return false;
        }
    }

    true
}
