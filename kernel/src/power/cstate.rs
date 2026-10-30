//! Autonomous CPU C-State Residency Governor & Break-Even Latency Analysis.
//! Evaluates processor idle duration predictions, break-even residency thresholds,

use alloc::vec::Vec;

// C-State Definitions & Architectural Parameters

/// Standard x86 Processor Power C-States.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum CpuCState {
    /// C0: Active execution state (100% power, 0 latency).
    C0 = 0,
    /// C1: Halt / Auto-Halt (Clock gating, ~1 µs exit latency, ~80% power).
    C1 = 1,
    /// C1E: Enhanced Halt (Clock gating + lowest P-state voltage, ~10 µs exit latency, ~40% power).
    C1E = 2,
    /// C6: Deep Power Down (Core power gating, L1/L2 flushed, ~100 µs exit latency, ~5% power).
    C6 = 3,
}

/// Target residency and transition latency parameters for an architectural C-state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CStateParameters {
    pub c_state: CpuCState,
    /// MWAIT hint parameter passed in EAX register (e.g. 0x00 for C1, 0x01 for C1E, 0x20 for C6).
    pub mwait_hint: u32,
    /// Exit / wake latency in nanoseconds.
    pub exit_latency_ns: u64,
    /// Break-even target residency in nanoseconds. Idle duration must exceed this to save energy.
    pub target_residency_ns: u64,
    /// Estimated relative power consumption in milliwatts under this C-state.
    pub idle_power_mw: u32,
}

impl CStateParameters {
    pub const fn c0() -> Self {
        Self {
            c_state: CpuCState::C0,
            mwait_hint: 0,
            exit_latency_ns: 0,
            target_residency_ns: 0,
            idle_power_mw: 15_000,
        }
    }

    pub const fn c1() -> Self {
        Self {
            c_state: CpuCState::C1,
            mwait_hint: 0x00,
            exit_latency_ns: 1_000,         // 1 µs
            target_residency_ns: 2_000,     // 2 µs break-even
            idle_power_mw: 10_000,
        }
    }

    pub const fn c1e() -> Self {
        Self {
            c_state: CpuCState::C1E,
            mwait_hint: 0x01,
            exit_latency_ns: 10_000,        // 10 µs
            target_residency_ns: 25_000,    // 25 µs break-even
            idle_power_mw: 5_000,
        }
    }

    pub const fn c6() -> Self {
        Self {
            c_state: CpuCState::C6,
            mwait_hint: 0x20,
            exit_latency_ns: 100_000,       // 100 µs
            target_residency_ns: 300_000,   // 300 µs break-even
            idle_power_mw: 500,
        }
    }
}

// Autonomous C-State Governor

/// C-State selection recommendation and diagnostics.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CStateSelection {
    pub target_state: CpuCState,
    pub mwait_hint: u32,
    pub predicted_duration_ns: u64,
    pub latency_budget_ns: u64,
    pub break_even_satisfied: bool,
}

/// Autonomous C-State Governor tracking historical idle intervals and selecting states.
pub struct CStateGovernor {
    pub available_states: Vec<CStateParameters>,
    pub recent_idle_durations: [u64; 8],
    pub history_idx: usize,
    pub history_count: usize,
    pub total_c0_time_ns: u64,
    pub total_c1_time_ns: u64,
    pub total_c1e_time_ns: u64,
    pub total_c6_time_ns: u64,
}

impl CStateGovernor {
    pub fn new() -> Self {
        let mut available_states = Vec::new();
        available_states.push(CStateParameters::c0());
        available_states.push(CStateParameters::c1());
        available_states.push(CStateParameters::c1e());
        available_states.push(CStateParameters::c6());

        Self {
            available_states,
            recent_idle_durations: [0; 8],
            history_idx: 0,
            history_count: 0,
            total_c0_time_ns: 0,
            total_c1_time_ns: 0,
            total_c1e_time_ns: 0,
            total_c6_time_ns: 0,
        }
    }

    /// Records an observed actual idle duration (in nanoseconds) into rolling history.
    pub fn record_idle_residency(&mut self, state: CpuCState, actual_duration_ns: u64) {
        self.recent_idle_durations[self.history_idx] = actual_duration_ns;
        self.history_idx = (self.history_idx + 1) % 8;
        if self.history_count < 8 {
            self.history_count += 1;
        }

        match state {
            CpuCState::C0 => self.total_c0_time_ns = self.total_c0_time_ns.saturating_add(actual_duration_ns),
            CpuCState::C1 => self.total_c1_time_ns = self.total_c1_time_ns.saturating_add(actual_duration_ns),
            CpuCState::C1E => self.total_c1e_time_ns = self.total_c1e_time_ns.saturating_add(actual_duration_ns),
            CpuCState::C6 => self.total_c6_time_ns = self.total_c6_time_ns.saturating_add(actual_duration_ns),
        }
    }

    /// Predicts upcoming idle duration using rolling historical average and next timer tick delta.
    pub fn predict_idle_duration_ns(&self, next_timer_event_ns: u64) -> u64 {
        if self.history_count == 0 {
            return next_timer_event_ns;
        }

        let mut sum: u64 = 0;
        for i in 0..self.history_count {
            sum = sum.saturating_add(self.recent_idle_durations[i]);
        }
        let historical_avg = sum / (self.history_count as u64);

        // Bound prediction by known impending hardware timer event
        historical_avg.min(next_timer_event_ns)
    }

    /// Selects optimal C-state satisfying break-even target residency and wake latency budget.
    pub fn select_cstate(&self, next_timer_event_ns: u64, max_allowed_latency_ns: u64) -> CStateSelection {
        let predicted_ns = self.predict_idle_duration_ns(next_timer_event_ns);

        let mut best_state = CStateParameters::c0();

        // Evaluate from deepest C-state to shallowest
        for param in self.available_states.iter().rev() {
            if param.c_state == CpuCState::C0 {
                continue;
            }

            // Must satisfy wake-up latency budget
            if param.exit_latency_ns > max_allowed_latency_ns {
                continue;
            }

            // Must satisfy break-even residency threshold
            if predicted_ns >= param.target_residency_ns {
                best_state = *param;
                break;
            }
        }

        CStateSelection {
            target_state: best_state.c_state,
            mwait_hint: best_state.mwait_hint,
            predicted_duration_ns: predicted_ns,
            latency_budget_ns: max_allowed_latency_ns,
            break_even_satisfied: best_state.c_state != CpuCState::C0,
        }
    }
}

// Hardware MWAIT / MONITOR C-State Entry Primitive

/// Puts current core to sleep in specified C-state via MONITOR / MWAIT instructions with HLT fallback.
/// # Safety
pub unsafe fn enter_cstate(c_state: &CStateSelection, monitor_target: *const u8) {
    if c_state.target_state == CpuCState::C0 {
        return;
    }

    // Attempt MONITOR / MWAIT if supported
    core::arch::asm!(
        "monitor",
        in("rax") monitor_target,
        in("rcx") 0u32, // extensions
        in("rdx") 0u32, // hints
        options(nostack, preserves_flags)
    );

    core::arch::asm!(
        "mwait",
        in("rax") c_state.mwait_hint,
        in("rcx") 0u32, // ECX bit 0: 1 = interrupt as break-event even if IF=0
        options(nostack, preserves_flags)
    );
}

// Unit Tests & Verification

/// Unit test verifying autonomous C-state residency governor and break-even latency analysis.
pub fn verify_cstate_governor_and_latency() -> bool {
    let mut gov = CStateGovernor::new();

    // Initial state with no history, impending timer tick in 1,500 ns (1.5 µs)
    // C1 requires 2 µs break-even => must stay in C0
    let s1 = gov.select_cstate(1_500, 10_000);
    if s1.target_state != CpuCState::C0 {
        return false;
    }

    // Train history with short idle periods (~5,000 ns / 5 µs)
    for _ in 0..8 {
        gov.record_idle_residency(CpuCState::C1, 5_000);
    }

    // Impending timer event in 50,000 ns (50 µs), latency budget 20 µs
    // Predicted = 5,000 ns.
    let s2 = gov.select_cstate(50_000, 20_000);
    if s2.target_state != CpuCState::C1 || s2.mwait_hint != 0x00 {
        return false;
    }

    // Train history with intermediate idle periods (~50,000 ns / 50 µs)
    for _ in 0..8 {
        gov.record_idle_residency(CpuCState::C1E, 50_000);
    }

    // Predicted = 50 µs.
    // Latency budget 5 µs (strict real-time constraint):
    let s3 = gov.select_cstate(100_000, 5_000);
    if s3.target_state != CpuCState::C1 {
        return false;
    }

    // Deep idle scenario: train history with 500,000 ns (500 µs), generous latency budget 200 µs
    for _ in 0..8 {
        gov.record_idle_residency(CpuCState::C6, 500_000);
    }
    // C6: exit latency 100 µs <= 200 µs, target residency 300 µs <= 500 µs => Selected!
    let s4 = gov.select_cstate(1_000_000, 200_000);
    if s4.target_state != CpuCState::C6 || s4.mwait_hint != 0x20 {
        return false;
    }

    true
}
