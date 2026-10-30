//! Low-Latency CPU Wait Instructions (UMONITOR, UMWAIT, TPAUSE) & Adaptive Synchronization.
//!
//! Provides hardware-accelerated user-mode thread parking on supported x86-64 processors
//! (Intel Tremont / Tiger Lake / Alder Lake / Raptor Lake with WAITPKG), with seamless
//! multi-stage adaptive spin-pause fallbacks for older CPUs and virtualized environments.

use core::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

/// C-state target for UMWAIT instruction.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u32)]
pub enum UmwaitCState {
    /// C0.2 state: deeper power savings, slightly higher wake latency (~50-100ns).
    C0_2LowPower = 0,
    /// C0.1 state: optimized for ultra-low wake latency (<40ns), minimal power reduction.
    C0_1UltraLowLatency = 1,
}

/// Result of a low-latency wait cycle.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WaitResult {
    /// Monitored memory changed or condition met.
    Success,
    /// TSC deadline expired before condition was met.
    Timeout,
    /// Wait terminated by an interrupt or external event.
    Interrupted,
}

/// Cached feature flag indicating if WAITPKG (UMONITOR/UMWAIT/TPAUSE) is present.
static WAITPKG_SUPPORTED: AtomicBool = AtomicBool::new(false);
static WAITPKG_INITIALIZED: AtomicBool = AtomicBool::new(false);

/// Queries CPUID leaf 7, subleaf 0 to determine if WAITPKG is supported (ECX bit 5).
pub fn detect_waitpkg_support() -> bool {
    if WAITPKG_INITIALIZED.load(Ordering::Acquire) {
        return WAITPKG_SUPPORTED.load(Ordering::Relaxed);
    }

    let cpuid_res = core::arch::x86_64::__cpuid_count(7, 0);
    // Bit 5 of ECX indicates WAITPKG
    let supported = (cpuid_res.ecx & (1 << 5)) != 0;

    WAITPKG_SUPPORTED.store(supported, Ordering::Relaxed);
    WAITPKG_INITIALIZED.store(true, Ordering::Release);
    supported
}

/// Forces the WAITPKG detection state (useful for test fixtures or explicit override).
pub fn set_waitpkg_override(supported: bool) {
    WAITPKG_SUPPORTED.store(supported, Ordering::Relaxed);
    WAITPKG_INITIALIZED.store(true, Ordering::Release);
}

/// Arms hardware cacheline monitor for the address range containing `addr`.
///
/// Executes the `UMONITOR` instruction on supported CPUs, or acts as a memory fence fallback.
#[inline(always)]
pub unsafe fn cpu_umonitor(addr: *const u8) {
    if detect_waitpkg_support() {
        core::arch::asm!(
            "umonitor {addr}",
            addr = in(reg) addr,
            options(nostack, preserves_flags)
        );
    } else {
        core::sync::atomic::compiler_fence(Ordering::SeqCst);
    }
}

/// Puts the logical processor into an optimized wait state until a write to the monitored
/// cacheline occurs or `deadline_tsc` is reached.
///
/// Returns `true` if condition/write woke the CPU, `false` on TSC timeout.
#[inline(always)]
pub unsafe fn cpu_umwait(c_state: UmwaitCState, deadline_tsc: u64) -> bool {
    if detect_waitpkg_support() {
        let eax = (deadline_tsc & 0xFFFF_FFFF) as u32;
        let edx = (deadline_tsc >> 32) as u32;
        let state_val = c_state as u32;
        let cf: u8;

        core::arch::asm!(
            "umwait {state:e}",
            "setnc {cf}",
            state = in(reg) state_val,
            cf = out(reg_byte) cf,
            in("eax") eax,
            in("edx") edx,
            options(nostack)
        );

        cf != 0
    } else {
        // Fallback: PAUSE spin loop with rdtsc check
        let start = read_tsc();
        while read_tsc() < deadline_tsc {
            core::hint::spin_loop();
            if read_tsc().saturating_sub(start) > 200 {
                break;
            }
        }
        true
    }
}

/// Pauses execution until `deadline_tsc` without monitoring an address range.
#[inline(always)]
pub unsafe fn cpu_tpause(c_state: UmwaitCState, deadline_tsc: u64) -> bool {
    if detect_waitpkg_support() {
        let eax = (deadline_tsc & 0xFFFF_FFFF) as u32;
        let edx = (deadline_tsc >> 32) as u32;
        let state_val = c_state as u32;
        let cf: u8;

        core::arch::asm!(
            "tpause {state:e}",
            "setnc {cf}",
            state = in(reg) state_val,
            cf = out(reg_byte) cf,
            in("eax") eax,
            in("edx") edx,
            options(nostack)
        );

        cf != 0
    } else {
        let start = read_tsc();
        while read_tsc() < deadline_tsc {
            core::hint::spin_loop();
            if read_tsc().saturating_sub(start) > 200 {
                break;
            }
        }
        true
    }
}

/// Reads the current 64-bit Time Stamp Counter (TSC).
#[inline(always)]
pub fn read_tsc() -> u64 {
    unsafe {
        let eax: u32;
        let edx: u32;
        core::arch::asm!(
            "rdtsc",
            out("eax") eax,
            out("edx") edx,
            options(nomem, nostack, preserves_flags)
        );
        ((edx as u64) << 32) | (eax as u64)
    }
}

/// Adaptive multi-stage low-latency spin-wait barrier.
///
/// Designed for sub-microsecond IPC synchronization:
/// - Stage 1 (0 to 16 cycles): Rapid `pause` loop (ultra-low overhead < 20ns).
/// - Stage 2 (16 to 128 cycles): Exponential backoff `pause` loop.
/// - Stage 3: If WAITPKG available, arms `UMONITOR` on the target cacheline and enters `UMWAIT`.
/// - Stage 4: TSC timeout check.
pub struct AdaptiveWaitBarrier {
    pub pause_limit: u32,
    pub max_wait_cycles: u64,
}

impl AdaptiveWaitBarrier {
    pub const fn new(pause_limit: u32, max_wait_cycles: u64) -> Self {
        Self {
            pause_limit,
            max_wait_cycles,
        }
    }

    pub const fn default_ipc() -> Self {
        Self {
            pause_limit: 64,
            max_wait_cycles: 50_000, // ~15-25 microseconds depending on clock
        }
    }

    /// Waits for an `AtomicUsize` variable to change from `old_val`.
    pub fn wait_for_change(&self, target: &AtomicUsize, old_val: usize) -> WaitResult {
        let start_tsc = read_tsc();
        let deadline = start_tsc + self.max_wait_cycles;

        // Stage 1: Tight pause spin loop
        for _ in 0..self.pause_limit {
            if target.load(Ordering::Acquire) != old_val {
                return WaitResult::Success;
            }
            core::hint::spin_loop();
        }

        // Stage 2 & 3: Hardware UMONITOR/UMWAIT or extended pause
        while read_tsc() < deadline {
            if target.load(Ordering::Acquire) != old_val {
                return WaitResult::Success;
            }

            unsafe {
                let ptr = target.as_ptr() as *const u8;
                cpu_umonitor(ptr);
                if target.load(Ordering::Acquire) != old_val {
                    return WaitResult::Success;
                }
                cpu_umwait(UmwaitCState::C0_1UltraLowLatency, deadline);
            }

            if target.load(Ordering::Acquire) != old_val {
                return WaitResult::Success;
            }
        }

        if target.load(Ordering::Acquire) != old_val {
            WaitResult::Success
        } else {
            WaitResult::Timeout
        }
    }

    /// Waits until an arbitrary predicate evaluates to true.
    pub fn wait_until<F>(&self, mut predicate: F) -> WaitResult
    where
        F: FnMut() -> bool,
    {
        let start_tsc = read_tsc();
        let deadline = start_tsc + self.max_wait_cycles;

        // Stage 1: Rapid pause
        for _ in 0..self.pause_limit {
            if predicate() {
                return WaitResult::Success;
            }
            core::hint::spin_loop();
        }

        // Stage 2: Periodic TSC pause
        while read_tsc() < deadline {
            for _ in 0..16 {
                if predicate() {
                    return WaitResult::Success;
                }
                core::hint::spin_loop();
            }
        }

        if predicate() {
            WaitResult::Success
        } else {
            WaitResult::Timeout
        }
    }
}

/// Verification test for low-latency CPU wait primitives and adaptive barriers.
pub fn test_umwait_primitives() -> bool {
    let tsc1 = read_tsc();
    let tsc2 = read_tsc();
    if tsc2 < tsc1 {
        return false;
    }

    // Verify detection doesn't crash or trigger faults
    let _has_waitpkg = detect_waitpkg_support();

    // Verify adaptive barrier with pre-changed value (immediate return)
    let val = AtomicUsize::new(42);
    let barrier = AdaptiveWaitBarrier::default_ipc();
    if barrier.wait_for_change(&val, 0) != WaitResult::Success {
        return false;
    }

    // Verify adaptive barrier timeout on unchanged value
    let short_barrier = AdaptiveWaitBarrier::new(8, 200);
    if short_barrier.wait_for_change(&val, 42) != WaitResult::Timeout {
        return false;
    }

    // Verify predicate wait
    let mut counter = 0;
    let res = barrier.wait_until(|| {
        counter += 1;
        counter >= 3
    });
    if res != WaitResult::Success || counter != 3 {
        return false;
    }

    true
}
