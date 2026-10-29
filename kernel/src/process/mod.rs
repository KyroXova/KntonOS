pub mod context;
pub mod thread;
pub mod process;
pub mod scheduler;

pub use context::{switch_context, Context};
pub use thread::{Priority, ThreadControlBlock, ThreadState};
pub use process::{FileDescriptor, ProcessControlBlock, ProcessState};
pub use scheduler::{init, spawn, yield_now, timer_tick, SCHEDULER};

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

    let passed = THREAD_A_EXECUTED.load(Ordering::SeqCst) && THREAD_B_EXECUTED.load(Ordering::SeqCst);
    if passed {
        serial_write_str("[KNT-PROC] Process & thread scheduler test: PASSED\r\n");
    } else {
        serial_write_str("[KNT-PROC] Process & thread scheduler test: FAILED\r\n");
    }

    passed
}
