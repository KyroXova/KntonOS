pub mod context;
pub mod thread;
pub mod process;
pub mod scheduler;
pub mod signals;
pub mod checkpoint;
pub mod resilience;

pub use checkpoint::*;
pub use resilience::*;

pub use context::{switch_context, Context};
pub use thread::{Priority, ThreadControlBlock, ThreadState};
pub use process::{FileDescriptor, ProcessControlBlock, ProcessState};
pub use scheduler::{
    block_current_thread, current_thread_id, init, spawn, timer_tick, wake_thread, yield_now,
    SCHEDULER,
};
pub use signals::{
    signal_restorer_trampoline, test_signal_definitions, test_signal_delivery_engine,
    test_signals_subsystem, SigAction, SigContext, SigSet, SignalDeliveryEngine,
    SignalDispositions, SignalError, SignalManager, SIGNAL_MANAGER,
    NSIG, SA_NOCLDSTOP, SA_NOCLDWAIT, SA_NODEFER, SA_ONSTACK, SA_RESETHAND, SA_RESTART,
    SA_RESTORER, SA_SIGINFO, SIGABRT, SIGALRM, SIGBUS, SIGCHLD, SIGCONT, SIGFPE, SIGHUP,
    SIGILL, SIGINT, SIGIO, SIGKILL, SIGPIPE, SIGPROF, SIGPWR, SIGQUIT, SIGSEGV, SIGSTOP,
    SIGSYS, SIGTERM, SIGTRAP, SIGTSTP, SIGTTIN, SIGTTOU, SIGURG, SIGUSR1, SIGUSR2, SIGVTALRM,
    SIGWINCH, SIGXCPU, SIGXFSZ, SIG_BLOCK, SIG_DFL, SIG_ERR, SIG_IGN, SIG_SETMASK,
    SIG_UNBLOCK,
};

use crate::serial_write_str;
use core::sync::atomic::{AtomicBool, Ordering};

static THREAD_A_EXECUTED: AtomicBool = AtomicBool::new(false);
static THREAD_B_EXECUTED: AtomicBool = AtomicBool::new(false);

fn worker_thread_a() {
    serial_write_str("[KNT-PROC] Worker Thread A started, yielding to Thread B...\r\n");
    THREAD_A_EXECUTED.store(true, Ordering::SeqCst);
    yield_now();
    serial_write_str("[KNT-PROC] Worker Thread A resumed, completing execution.\r\n");
}

fn worker_thread_b() {
    serial_write_str("[KNT-PROC] Worker Thread B started, yielding back to Thread A...\r\n");
    THREAD_B_EXECUTED.store(true, Ordering::SeqCst);
    yield_now();
    serial_write_str("[KNT-PROC] Worker Thread B resumed, completing execution.\r\n");
}

/// Verification test ensuring cooperative multi-threaded context switching operates cleanly.
pub fn test_process_subsystem() -> bool {
    serial_write_str("[KNT-PROC] Running process & scheduler subsystem verification test...\r\n");

    let tid_a = spawn("worker_a", Priority::Normal, worker_thread_a);
    let tid_b = spawn("worker_b", Priority::Normal, worker_thread_b);

    if tid_a == 0 || tid_b == 0 {
        serial_write_str("[KNT-PROC] Error: Failed to spawn test threads\r\n");
        return false;
    }

    // Yield control to let Worker A and Worker B execute
    yield_now();
    yield_now();
    yield_now();

    let passed = THREAD_A_EXECUTED.load(Ordering::SeqCst)
        && THREAD_B_EXECUTED.load(Ordering::SeqCst)
        && test_signal_definitions()
        && test_signal_delivery_engine();
    if passed {
        serial_write_str("[KNT-PROC] Process & thread scheduler test: PASSED\r\n");
    } else {
        serial_write_str("[KNT-PROC] Process & thread scheduler test: FAILED\r\n");
    }

    passed
}

/// Verification self-test suite for state-externalized dual-ring partitions & sub-15ms crash recovery.
pub fn test_resilience_subsystem() -> bool {
    serial_write_str("[KNT-PROC-RESIL] Testing resilient service partitions and crash recovery...\r\n");

    // 1. Test Externalized State Partition & Double-Buffered Checkpointing
    {
        let mut part = StatePartition::new("knton.shm.compositor");
        if part.header.name_as_str() != "knton.shm.compositor" {
            serial_write_str("[KNT-PANIC] Partition name mismatch!\r\n");
            return false;
        }

        // Commit state 1: Window layout configuration
        let state1 = b"DESKTOP_SURFACE: width=1280,height=800,windows=4,focused_id=2";
        if !part.commit_checkpoint(state1) {
            serial_write_str("[KNT-PANIC] Partition initial commit failed!\r\n");
            return false;
        }

        if part.header.active_bank != 1 || part.active_data() != state1 {
            serial_write_str("[KNT-PANIC] Bank 1 active data mismatch!\r\n");
            return false;
        }
        if !part.validate_active_bank() {
            serial_write_str("[KNT-PANIC] Active bank checksum validation failed!\r\n");
            return false;
        }

        // Commit state 2: Dynamic window movement
        let state2 = b"DESKTOP_SURFACE: width=1280,height=800,windows=5,focused_id=5";
        if !part.commit_checkpoint(state2) {
            serial_write_str("[KNT-PANIC] Partition second commit failed!\r\n");
            return false;
        }

        // Double-buffer toggle: active bank must switch back to Bank A (0)
        if part.header.active_bank != 0 || part.active_data() != state2 {
            serial_write_str("[KNT-PANIC] Bank 0 toggle active data mismatch!\r\n");
            return false;
        }
        if !part.validate_active_bank() {
            serial_write_str("[KNT-PANIC] Bank 0 validation failed!\r\n");
            return false;
        }

        // Verify write-ahead journal recorded transaction
        let latest_journal = match part.journal.recover_latest_committed() {
            Some(j) => j,
            None => {
                serial_write_str("[KNT-PANIC] Write-ahead journal recovery failed!\r\n");
                return false;
            }
        };
        if latest_journal.payload_size as usize != state2.len()
            || &latest_journal.payload[..state2.len()] != state2
        {
            serial_write_str("[KNT-PANIC] Latest journal entry content mismatch!\r\n");
            return false;
        }
    }

    // 2. Test In-Place Crash Interception & Thread/PCB Recycling (Zero-Teardown)
    {
        fn dummy_resurrected_worker() {}

        let mut pcb = ProcessControlBlock::new("knton.shm.audiomixer", 0x0020_0000, None);
        let mut thread = ThreadControlBlock::new_kernel_thread(
            pcb.pid,
            "audiomixer_worker",
            Priority::High,
            dummy_resurrected_worker,
        );

        let initial_cr3 = pcb.page_table_cr3;
        let initial_fd_count = pcb.file_descriptors.len();

        let mut interceptor = FastCrashInterceptor::new();
        let service = ResilientService::new(
            "knton.shm.audiomixer",
            pcb.pid,
            thread.tid,
            dummy_resurrected_worker,
            ResilientPolicy::default(),
        );
        interceptor.register_service(service);

        // Simulate fatal segmentation fault (SIGSEGV) at address 0xDEAD_BEEF
        let intercepted = interceptor.intercept_crash(
            &mut pcb,
            &mut thread,
            SIGSEGV,
            0xDEAD_BEEF,
            0x0040_1234,
            1_000_000_000,
            42,
        );

        if !intercepted {
            serial_write_str("[KNT-PANIC] Crash interception failed for resilient service!\r\n");
            return false;
        }

        // CRITICAL INVARIANT: Page tables (CR3) and file descriptor table must remain intact!
        if pcb.page_table_cr3 != initial_cr3 || pcb.file_descriptors.len() != initial_fd_count {
            serial_write_str("[KNT-PANIC] Kernel mappings or FDs were torn down during crash!\r\n");
            return false;
        }

        // Thread state must be recycled back to Ready
        if thread.state != ThreadState::Ready || thread.pending_signals != 0 {
            serial_write_str("[KNT-PANIC] Thread was not recycled into Ready state!\r\n");
            return false;
        }

        // Verify telemetry recorded
        let svc = interceptor.find_service_by_pid_mut(pcb.pid).unwrap();
        if svc.total_crashes_intercepted.load(core::sync::atomic::Ordering::Relaxed) != 1 {
            serial_write_str("[KNT-PANIC] Crash interception count not incremented!\r\n");
            return false;
        }
        let crash_ev = svc.last_crash_event.unwrap();
        if crash_ev.signal != SIGSEGV || crash_ev.fault_addr != 0xDEAD_BEEF {
            serial_write_str("[KNT-PANIC] Crash event telemetry corrupted!\r\n");
            return false;
        }
    }

    // 3. Test Deterministic Sub-15ms Resurrection & State Re-Attachment Engine
    {
        fn compositor_entry() {}

        let mut supervisor = ResilienceSupervisor::new();
        let mut part = StatePartition::new("knton.shm.compositor");
        let initial_state = b"SURFACE_ACTIVE: id=1,fb_paddr=0x80000000,pitch=5120,ready=1";
        part.commit_checkpoint(initial_state);

        let mut pcb = ProcessControlBlock::new("knton.shm.compositor", 0x0010_0000, None);
        let mut thread = ThreadControlBlock::new_kernel_thread(
            pcb.pid,
            "comp_worker",
            Priority::Realtime,
            compositor_entry,
        );

        let service = ResilientService::new(
            "knton.shm.compositor",
            pcb.pid,
            thread.tid,
            compositor_entry,
            ResilientPolicy::default(),
        );

        supervisor.register_service_partition(service, part);

        // Execute full crash -> intercept -> state validate -> resurrect pipeline
        let crash_time_ns = 5_000_000_000u64;
        let res = match supervisor.handle_crash_and_resurrect(
            &mut pcb,
            &mut thread,
            SIGSEGV,
            0x0,
            0x0040_2000,
            crash_time_ns,
        ) {
            Ok(r) => r,
            Err(_e) => {
                serial_write_str("[KNT-PANIC] Full resilience resurrection cycle failed!\r\n");
                return false;
            }
        };

        if !res.state_restored || !res.sub_15ms_guarantee_met {
            serial_write_str("[KNT-PANIC] Sub-15ms SLA or state restoration guarantee failed!\r\n");
            return false;
        }

        if res.latency_nanos >= MAX_RESURRECTION_LATENCY_NS {
            serial_write_str("[KNT-PANIC] Resurrection latency exceeded 15ms ceiling!\r\n");
            return false;
        }

        // Validate re-attached state matches committed checkpoint
        let recovered_part = supervisor.partitions.find("knton.shm.compositor").unwrap();
        if recovered_part.active_data() != initial_state {
            serial_write_str("[KNT-PANIC] Re-attached partition data mismatch!\r\n");
            return false;
        }

        // Test torn-write corruption auto-recovery
        let mut torn_part = StatePartition::new("knton.shm.sensor");
        torn_part.commit_checkpoint(b"SENSOR_CALIBRATION_OK_V1");
        // Corrupt active bank bytes
        torn_part.bank_b[0] = !torn_part.bank_b[0]; // Invalidate checksum

        let mut sensor_service = ResilientService::new(
            "knton.shm.sensor",
            100,
            101,
            compositor_entry,
            ResilientPolicy::default(),
        );
        let torn_res = ResurrectionEngine::resurrect(&mut sensor_service, &mut torn_part, 6_000_000_000);
        if torn_res.is_err() || !torn_res.unwrap().state_restored {
            serial_write_str("[KNT-PANIC] Torn-write recovery from journal failed!\r\n");
            return false;
        }
    }

    serial_write_str("[KNT-PROC-RESIL-PASS] State-externalized dual-ring partitions and sub-15ms crash recovery verified\r\n");
    true
}
