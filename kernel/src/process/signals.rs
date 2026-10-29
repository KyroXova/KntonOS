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
