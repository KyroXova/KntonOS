//! POSIX Signal Definitions, Signal Masks, and SigAction Structures.
//!
//! Implements POSIX.1-2008 compatible signal numbers, bitmask sets (sigset_t),
//! sigaction disposition tables, and pending signal queues.

pub const SIGHUP: usize = 1;
pub const SIGINT: usize = 2;
pub const SIGQUIT: usize = 3;
pub const SIGILL: usize = 4;
pub const SIGTRAP: usize = 5;
pub const SIGABRT: usize = 6;
pub const SIGBUS: usize = 7;
pub const SIGFPE: usize = 8;
pub const SIGKILL: usize = 9;
pub const SIGUSR1: usize = 10;
pub const SIGSEGV: usize = 11;
pub const SIGUSR2: usize = 12;
pub const SIGPIPE: usize = 13;
pub const SIGALRM: usize = 14;
pub const SIGTERM: usize = 15;
pub const SIGCHLD: usize = 17;
pub const SIGCONT: usize = 18;
pub const SIGSTOP: usize = 19;
pub const SIGTSTP: usize = 20;
pub const SIGTTIN: usize = 21;
pub const SIGTTOU: usize = 22;
pub const SIGURG: usize = 23;
pub const SIGXCPU: usize = 24;
pub const SIGXFSZ: usize = 25;
pub const SIGVTALRM: usize = 26;
pub const SIGPROF: usize = 27;
pub const SIGWINCH: usize = 28;
pub const SIGIO: usize = 29;
pub const SIGPWR: usize = 30;
pub const SIGSYS: usize = 31;

pub const NSIG: usize = 32;

pub const SIG_DFL: u64 = 0;
pub const SIG_IGN: u64 = 1;
pub const SIG_ERR: u64 = !0;

// sa_flags definitions
pub const SA_NOCLDSTOP: u32 = 0x0000_0001;
pub const SA_NOCLDWAIT: u32 = 0x0000_0002;
pub const SA_SIGINFO:   u32 = 0x0000_0004;
pub const SA_RESTORER:  u32 = 0x0400_0000;
pub const SA_ONSTACK:   u32 = 0x0800_0000;
pub const SA_RESTART:   u32 = 0x1000_0000;
pub const SA_NODEFER:   u32 = 0x4000_0000;
pub const SA_RESETHAND: u32 = 0x8000_0000;

// sigprocmask action definitions
pub const SIG_BLOCK: u32 = 0;
pub const SIG_UNBLOCK: u32 = 1;
pub const SIG_SETMASK: u32 = 2;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SignalError {
    InvalidSignal,
    UncatchableSignal,
    ProcessNotFound,
    ThreadNotFound,
    PermissionDenied,
}

/// Bitmask representing a set of signals (sigset_t).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[repr(transparent)]
pub struct SigSet(pub u64);

impl SigSet {
    pub const fn empty() -> Self {
        Self(0)
    }

    pub const fn all() -> Self {
        // All signals 1..31 except SIGKILL and SIGSTOP
        Self(0xFFFF_FFFF & !((1 << SIGKILL) | (1 << SIGSTOP)))
    }

    /// Adds a signal to the set. SIGKILL and SIGSTOP cannot be masked.
    pub fn add(&mut self, sig: usize) -> bool {
        if sig == 0 || sig >= NSIG {
            return false;
        }
        if sig == SIGKILL || sig == SIGSTOP {
            return false;
        }
        self.0 |= 1u64 << sig;
        true
    }

    /// Removes a signal from the set.
    pub fn remove(&mut self, sig: usize) -> bool {
        if sig == 0 || sig >= NSIG {
            return false;
        }
        self.0 &= !(1u64 << sig);
        true
    }

    /// Checks if a signal is contained in the set.
    pub const fn contains(&self, sig: usize) -> bool {
        if sig == 0 || sig >= NSIG {
            return false;
        }
        (self.0 & (1u64 << sig)) != 0
    }

    pub const fn is_empty(&self) -> bool {
        self.0 == 0
    }

    pub const fn union(&self, other: SigSet) -> SigSet {
        SigSet(self.0 | other.0)
    }

    pub const fn intersection(&self, other: SigSet) -> SigSet {
        SigSet(self.0 & other.0)
    }

    pub const fn difference(&self, other: SigSet) -> SigSet {
        SigSet(self.0 & !other.0)
    }
}

/// POSIX sigaction structure describing the action taken upon receipt of a signal.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SigAction {
    pub sa_handler: u64,
    pub sa_mask: u64,
    pub sa_flags: u32,
    pub sa_restorer: u64,
}

impl Default for SigAction {
    fn default() -> Self {
        Self {
            sa_handler: SIG_DFL,
            sa_mask: 0,
            sa_flags: 0,
            sa_restorer: 0,
        }
    }
}

/// Architecture-specific execution context saved on user stack during signal delivery.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct SigContext {
    pub r8: u64,
    pub r9: u64,
    pub r10: u64,
    pub r11: u64,
    pub r12: u64,
    pub r13: u64,
    pub r14: u64,
    pub r15: u64,
    pub rdi: u64,
    pub rsi: u64,
    pub rbp: u64,
    pub rbx: u64,
    pub rdx: u64,
    pub rax: u64,
    pub rcx: u64,
    pub rsp: u64,
    pub rip: u64,
    pub rflags: u64,
    pub cs: u64,
    pub fs: u64,
    pub gs: u64,
    pub ss: u64,
    pub err: u64,
    pub trapno: u64,
    pub oldmask: u64,
    pub cr2: u64,
}

/// Signal table holding registered actions for all POSIX signals in a process.
#[derive(Debug, Clone)]
pub struct SignalDispositions {
    pub actions: [SigAction; NSIG],
}

impl Default for SignalDispositions {
    fn default() -> Self {
        Self {
            actions: [SigAction::default(); NSIG],
        }
    }
}

impl SignalDispositions {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn get(&self, sig: usize) -> Result<SigAction, SignalError> {
        if sig == 0 || sig >= NSIG {
            return Err(SignalError::InvalidSignal);
        }
        Ok(self.actions[sig])
    }

    pub fn set(&mut self, sig: usize, action: SigAction) -> Result<SigAction, SignalError> {
        if sig == 0 || sig >= NSIG {
            return Err(SignalError::InvalidSignal);
        }
        if sig == SIGKILL || sig == SIGSTOP {
            return Err(SignalError::UncatchableSignal);
        }
        let old = self.actions[sig];
        self.actions[sig] = action;
        Ok(old)
    }
}

use super::process::{ProcessControlBlock, ProcessState};
use super::thread::{ThreadControlBlock, ThreadState};

/// Userland trampoline that invokes sys_sigreturn upon handler completion.
#[unsafe(naked)]
pub unsafe extern "C" fn signal_restorer_trampoline() {
    core::arch::naked_asm!(
        "mov rax, 0x0052", // SYS_RT_SIGRETURN
        "syscall",
        "ud2",
    );
}

pub struct SignalDeliveryEngine;

impl SignalDeliveryEngine {
    /// Queues a signal to be delivered to a thread.
    pub fn queue_signal_thread(thread: &mut ThreadControlBlock, sig: usize) -> Result<(), SignalError> {
        if sig == 0 || sig >= NSIG {
            return Err(SignalError::InvalidSignal);
        }
        thread.pending_signals |= 1u64 << sig;
        Ok(())
    }

    /// Queues a signal to be delivered to a process.
    pub fn queue_signal_process(pcb: &mut ProcessControlBlock, sig: usize) -> Result<(), SignalError> {
        if sig == 0 || sig >= NSIG {
            return Err(SignalError::InvalidSignal);
        }
        pcb.pending_signals |= 1u64 << sig;
        Ok(())
    }

    /// Checks and delivers any pending unmasked signal to the current thread.
    /// Returns true if a signal handler was dispatched and user_ctx was redirected.
    pub fn deliver_pending_signals(
        thread: &mut ThreadControlBlock,
        pcb: &mut ProcessControlBlock,
        user_ctx: &mut SigContext,
    ) -> bool {
        // Collect pending signals not blocked by thread signal mask.
        // SIGKILL and SIGSTOP cannot be blocked.
        let unblockable = (1u64 << SIGKILL) | (1u64 << SIGSTOP);
        let effective_mask = thread.signal_mask & !unblockable;
        let combined_pending = (thread.pending_signals | pcb.pending_signals) & !effective_mask;

        if combined_pending == 0 {
            return false;
        }

        // Find lowest pending signal number (1..31)
        let sig = combined_pending.trailing_zeros() as usize;
        if sig == 0 || sig >= NSIG {
            return false;
        }

        // Clear pending bit from both thread and process
        thread.pending_signals &= !(1u64 << sig);
        pcb.pending_signals &= !(1u64 << sig);

        // Fetch disposition
        let act = match pcb.signals.get(sig) {
            Ok(a) => a,
            Err(_) => return false,
        };

        // Handle default actions
        if act.sa_handler == SIG_IGN {
            return false;
        }

        if act.sa_handler == SIG_DFL {
            if sig == SIGCHLD || sig == SIGURG || sig == SIGWINCH {
                return false;
            }
            if sig == SIGSTOP || sig == SIGTSTP || sig == SIGTTIN || sig == SIGTTOU {
                thread.state = ThreadState::Blocked;
                return false;
            }
            // Fatal default signals terminate the process
            crate::serial_write_str("[KNT-SIGNAL] Fatal default signal received (sig: ");
            crate::serial_write_byte(b'0' + (sig as u8 % 10));
            crate::serial_write_str("), terminating process\r\n");
            pcb.state = ProcessState::Terminated;
            thread.state = ThreadState::Terminated;
            return false;
        }

        // Custom signal handler registered:
        // 1. Construct SigContext with full CPU register state
        let mut sigctx = *user_ctx;
        sigctx.oldmask = thread.signal_mask;

        // 2. Compute user stack frame location (reserve 128-byte red zone and align to 16 bytes)
        let base_rsp = if user_ctx.rsp > 256 { user_ctx.rsp - 128 } else { user_ctx.rsp };
        let ctx_size = core::mem::size_of::<SigContext>() as u64;
        let sigctx_vaddr = (base_rsp - ctx_size) & !0x0F;

        // Write to user stack if in user space range
        if sigctx_vaddr >= 0x0000_0000_0001_0000 && sigctx_vaddr < 0x0000_7FFF_FFFF_0000 {
            unsafe {
                let dst = sigctx_vaddr as *mut SigContext;
                *dst = sigctx;
                let ret_slot = (sigctx_vaddr - 8) as *mut u64;
                let restorer = if act.sa_restorer != 0 {
                    act.sa_restorer
                } else {
                    signal_restorer_trampoline as *const () as usize as u64
                };
                *ret_slot = restorer;
            }
        }

        // Save backup in TCB for reliable sigreturn restoration
        thread.saved_sigcontext = Some(sigctx);

        // 3. Update thread signal mask
        thread.signal_mask |= act.sa_mask;
        if (act.sa_flags & SA_NODEFER) == 0 {
            thread.signal_mask |= 1u64 << sig;
        }

        if (act.sa_flags & SA_RESETHAND) != 0 {
            let _ = pcb.signals.set(sig, SigAction::default());
        }

        // 4. Redirect user execution context to handler
        user_ctx.rip = act.sa_handler;
        user_ctx.rsp = sigctx_vaddr.saturating_sub(8);
        user_ctx.rdi = sig as u64;
        user_ctx.rsi = 0;
        user_ctx.rdx = sigctx_vaddr;

        true
    }

    /// Restores the interrupted userland execution context from SigContext (sys_sigreturn).
    pub fn restore_sigcontext(
        thread: &mut ThreadControlBlock,
        user_ctx: &mut SigContext,
        sigctx_ptr: Option<u64>,
    ) -> Result<(), SignalError> {
        let saved = if let Some(ptr) = sigctx_ptr {
            if ptr != 0 && ptr >= 0x0000_0000_0001_0000 && ptr < 0x0000_7FFF_FFFF_0000 {
                unsafe { *(ptr as *const SigContext) }
            } else if let Some(saved) = thread.saved_sigcontext.take() {
                saved
            } else {
                return Err(SignalError::ProcessNotFound);
            }
        } else if let Some(saved) = thread.saved_sigcontext.take() {
            saved
        } else {
            return Err(SignalError::ProcessNotFound);
        };

        // Restore thread signal mask
        thread.signal_mask = saved.oldmask;

        // Restore full execution context
        *user_ctx = saved;
        thread.saved_sigcontext = None;

        Ok(())
    }
}

/// Verification unit test ensuring POSIX signal constants, masks, and unmaskable rules behave correctly.
pub fn test_signal_definitions() -> bool {
    let mut mask = SigSet::empty();
    assert!(mask.is_empty());

    // Adding valid signals
    assert!(mask.add(SIGHUP));
    assert!(mask.add(SIGINT));
    assert!(mask.add(SIGSEGV));
    assert!(mask.add(SIGTERM));
    assert!(mask.add(SIGCHLD));

    assert!(mask.contains(SIGHUP));
    assert!(mask.contains(SIGINT));
    assert!(mask.contains(SIGSEGV));
    assert!(mask.contains(SIGTERM));
    assert!(mask.contains(SIGCHLD));
    assert!(!mask.contains(SIGQUIT));

    // SIGKILL and SIGSTOP cannot be blocked in signal mask
    assert!(!mask.add(SIGKILL));
    assert!(!mask.add(SIGSTOP));
    assert!(!mask.contains(SIGKILL));
    assert!(!mask.contains(SIGSTOP));

    // Removing signal
    assert!(mask.remove(SIGINT));
    assert!(!mask.contains(SIGINT));

    // SigAction configuration & dispositions
    let mut disp = SignalDispositions::new();
    let custom_action = SigAction {
        sa_handler: 0x0040_1000,
        sa_mask: 1 << SIGQUIT,
        sa_flags: SA_RESTART | SA_NODEFER,
        sa_restorer: 0x0040_2000,
    };

    assert!(disp.set(SIGINT, custom_action).is_ok());
    let fetched = disp.get(SIGINT).unwrap();
    assert_eq!(fetched.sa_handler, 0x0040_1000);
    assert_eq!(fetched.sa_flags, SA_RESTART | SA_NODEFER);

    // SIGKILL must not allow custom disposition
    assert_eq!(disp.set(SIGKILL, custom_action), Err(SignalError::UncatchableSignal));
    assert_eq!(disp.set(SIGSTOP, custom_action), Err(SignalError::UncatchableSignal));

    true
}

/// Verification unit test ensuring signal delivery, stack frame construction, and context restoration.
pub fn test_signal_delivery_engine() -> bool {
    let mut pcb = ProcessControlBlock::new("sig_test", 0, None);
    let mut thread = ThreadControlBlock::new_idle_thread(pcb.pid, "sig_thread");

    // Configure SIGINT with custom handler and restorer
    let custom_action = SigAction {
        sa_handler: 0x0040_5000,
        sa_mask: 1 << SIGQUIT,
        sa_flags: 0,
        sa_restorer: 0x0040_9000,
    };
    assert!(pcb.signals.set(SIGINT, custom_action).is_ok());

    // Initial simulated userland context
    let mut user_ctx = SigContext {
        rip: 0x0040_0050,
        rsp: 0x0000_7FFF_FFFF_0000,
        rax: 0xCAFE_BABE,
        rflags: 0x202,
        ..Default::default()
    };

    // 1. Queue SIGINT
    assert!(SignalDeliveryEngine::queue_signal_thread(&mut thread, SIGINT).is_ok());

    // 2. Deliver pending signals
    let delivered = SignalDeliveryEngine::deliver_pending_signals(&mut thread, &mut pcb, &mut user_ctx);
    assert!(delivered);

    // Verify user context was modified to enter handler
    assert_eq!(user_ctx.rip, 0x0040_5000);
    assert_eq!(user_ctx.rdi, SIGINT as u64);
    assert!(user_ctx.rsp < 0x0000_7FFF_FFFF_0000);
    // Verify SIGINT and sa_mask (SIGQUIT) are blocked
    assert!((thread.signal_mask & (1 << SIGINT)) != 0);
    assert!((thread.signal_mask & (1 << SIGQUIT)) != 0);
    // Verify pending signal cleared
    assert_eq!(thread.pending_signals & (1 << SIGINT), 0);

    // 3. Restore context via sigreturn
    assert!(SignalDeliveryEngine::restore_sigcontext(&mut thread, &mut user_ctx, None).is_ok());
    assert_eq!(user_ctx.rip, 0x0040_0050);
    assert_eq!(user_ctx.rax, 0xCAFE_BABE);
    assert_eq!(user_ctx.rsp, 0x0000_7FFF_FFFF_0000);
    assert_eq!(thread.signal_mask, 0);

    // 4. Test signal mask blocking
    // Block SIGUSR1
    thread.signal_mask |= 1 << SIGUSR1;
    let usr1_action = SigAction {
        sa_handler: 0x0040_6000,
        sa_mask: 0,
        sa_flags: 0,
        sa_restorer: 0,
    };
    assert!(pcb.signals.set(SIGUSR1, usr1_action).is_ok());
    assert!(SignalDeliveryEngine::queue_signal_thread(&mut thread, SIGUSR1).is_ok());

    // Should NOT deliver because SIGUSR1 is blocked
    let blocked_delivered = SignalDeliveryEngine::deliver_pending_signals(&mut thread, &mut pcb, &mut user_ctx);
    assert!(!blocked_delivered);
    assert_eq!(user_ctx.rip, 0x0040_0050); // Unchanged
    assert!((thread.pending_signals & (1 << SIGUSR1)) != 0); // Remains pending

    // Unblock SIGUSR1 and verify delivery
    thread.signal_mask &= !(1 << SIGUSR1);
    let unblocked_delivered = SignalDeliveryEngine::deliver_pending_signals(&mut thread, &mut pcb, &mut user_ctx);
    assert!(unblocked_delivered);
    assert_eq!(user_ctx.rip, 0x0040_6000);
    assert_eq!(user_ctx.rdi, SIGUSR1 as u64);

    true
}
