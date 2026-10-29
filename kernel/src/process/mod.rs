pub mod context;
pub mod thread;
pub mod process;
pub mod scheduler;
pub mod signals;

pub use context::{switch_context, Context};
pub use thread::{Priority, ThreadControlBlock, ThreadState};
pub use process::{FileDescriptor, ProcessControlBlock, ProcessState};
pub use scheduler::{
    block_current_thread, current_thread_id, init, spawn, timer_tick, wake_thread, yield_now,
    SCHEDULER,
};
pub use signals::{
    signal_restorer_trampoline, test_signal_definitions, test_signal_delivery_engine,
    SigAction, SigContext, SigSet, SignalDeliveryEngine, SignalDispositions, SignalError,
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
