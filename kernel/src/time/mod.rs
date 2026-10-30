//! Kernel High-Resolution Timers & Hierarchical Timing Wheel Subsystem.

pub mod hrtimer;
pub mod wheel;

pub use hrtimer::{
    FramePaceResult, FrameRatePreset, GameFramePacer, HrTimeNs, HrTimer, HrTimerCallback,
    HrTimerMode, HrTimerState, PerCpuHrTimerQueue, RbTreeTimerQueue, NS_PER_MS, NS_PER_SEC,
    NS_PER_US,
};
pub use wheel::{
    HardwareTimerMode, HierarchicalTimingWheel, TscDeadlineProgrammer, WheelCallback,
    WheelTimer, LEVEL0_SPAN, LEVEL1_SPAN, WHEEL_LEVEL0_SIZE, WHEEL_UPPER_SIZE,
};

/// Comprehensive verification suite for high-resolution timers and hierarchical timing wheel.
pub fn test_hrtimer_subsystem() -> bool {
    crate::serial_write_str("Testing High-Resolution Timers Subsystem (TASK-KNT-088)...\r\n");

    // 1. Verify Red-Black tree ordered timer queue
    if !hrtimer::test_hrtimer_rb_tree_queue() {
        crate::serial_write_str("  [FAIL] test_hrtimer_rb_tree_queue failed\r\n");
        return false;
    }
    crate::serial_write_str("  * Ordered Red-Black tree nanosecond timer queue and periodic reload verified\r\n");

    // 2. Verify Hierarchical Timing Wheel (L0-L4) cascade insertion and dispatch
    if !wheel::test_timing_wheel_cascade_and_dispatch() {
        crate::serial_write_str("  [FAIL] test_timing_wheel_cascade_and_dispatch failed\r\n");
        return false;
    }
    crate::serial_write_str("  * Hierarchical timing wheel (5 levels) O(1) cascade and expiry dispatch verified\r\n");

    // 3. Verify Dynamic TSC-Deadline Programmer
    if !wheel::test_tsc_deadline_programmer() {
        crate::serial_write_str("  [FAIL] test_tsc_deadline_programmer failed\r\n");
        return false;
    }
    crate::serial_write_str("  * Dynamic TSC-Deadline and Local APIC one-shot programmer verified\r\n");

    // 4. Verify Game Frame Pacing Governor across 60Hz-240Hz
    if !hrtimer::test_tsc_deadline_and_frame_pacer() {
        crate::serial_write_str("  [FAIL] test_tsc_deadline_and_frame_pacer failed\r\n");
        return false;
    }
    crate::serial_write_str("  * Ultra-low jitter game frame pacing synchronization (60Hz-240Hz) verified\r\n");

    // 5. End-to-end integration: link timer queue with hardware programmer
    let mut cpu0_timers = PerCpuHrTimerQueue::new(0);
    let mut prog = TscDeadlineProgrammer::new(
        HardwareTimerMode::TscDeadline,
        3_200_000_000, // 3.2 GHz
        100_000_000,
    );

    // Arm micro-kernel timer 500us ahead
    cpu0_timers.arm(HrTimer::new_oneshot(101, 500 * NS_PER_US, 0));
    let earliest = match cpu0_timers.earliest_deadline() {
        Some(e) => e,
        None => return false,
    };
    let delta = match prog.program_deadline(earliest, 0) {
        Ok(d) => d,
        Err(_) => return false,
    };
    if delta != 500 * NS_PER_US {
        return false;
    }
    let cycles = prog.delta_ns_to_tsc_cycles(delta);
    if cycles != (500 * NS_PER_US as u128 * 3_200_000_000 / 1_000_000_000) as u64 {
        return false;
    }

    crate::serial_write_str("  * End-to-end timer queue to hardware TSC-Deadline programmer linkage verified\r\n");
    crate::serial_write_str("[KNT-TIMER-HRTIMER-PASS] High-frequency kernel hrtimers and red-black tree timing wheel operational\r\n");

    true
}
