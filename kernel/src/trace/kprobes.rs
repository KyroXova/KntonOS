//! Dynamic Kernel Probes (kprobes) Infrastructure & INT3 Breakpoint Handler.
//! Provides dynamic software breakpoint insertion (INT3 0xCC), probe registry,

use core::sync::atomic::{fence, Ordering};

/// INT3 software breakpoint opcode in x86-64.
pub const INT3_OPCODE: u8 = 0xCC;

/// CPU EFLAGS Trap Flag (TF, bit 8).
pub const EFLAGS_TF: u64 = 1 << 8;

/// Maximum number of active kprobes supported in the registry.
pub const MAX_KPROBES: usize = 32;

/// Maximum length of symbol name stored in a kprobe.
pub const MAX_SYMBOL_LEN: usize = 32;

/// Size of the single-step instruction buffer per probe.
pub const SINGLE_STEP_BUF_SIZE: usize = 16;

/// Errors occurring during kprobe registration, arming, or execution.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum KprobeError {
    SlotFull,
    AlreadyRegistered,
    NotFound,
    NotArmed,
    AlreadyArmed,
    InvalidAddress,
    MemoryFault,
}

/// Outcome status from the INT3 breakpoint or debug exception handler.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum KprobeBreakStatus {
    /// Breakpoint or single-step trap was recognized and handled by an active kprobe.
    Handled,
    /// Event was not registered in the kprobe subsystem (forward to kernel debugger/panic).
    NotOurs,
}

/// 64-bit architectural register context snapshot captured during kprobe invocation.
#[repr(C)]
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub struct PtRegs {
    pub r15: u64,
    pub r14: u64,
    pub r13: u64,
    pub r12: u64,
    pub r11: u64,
    pub r10: u64,
    pub r9: u64,
    pub r8: u64,
    pub rbp: u64,
    pub rdi: u64,
    pub rsi: u64,
    pub rdx: u64,
    pub rcx: u64,
    pub rbx: u64,
    pub rax: u64,
    pub rip: u64,
    pub cs: u64,
    pub rflags: u64,
    pub rsp: u64,
    pub ss: u64,
}

impl PtRegs {
    pub const fn empty() -> Self {
        Self {
            r15: 0,
            r14: 0,
            r13: 0,
            r12: 0,
            r11: 0,
            r10: 0,
            r9: 0,
            r8: 0,
            rbp: 0,
            rdi: 0,
            rsi: 0,
            rdx: 0,
            rcx: 0,
            rbx: 0,
            rax: 0,
            rip: 0,
            cs: 0,
            rflags: 0,
            rsp: 0,
            ss: 0,
        }
    }
}

/// Pre-execution callback invoked before the probed instruction is executed.
/// If returns `false`, post-handler and instruction execution are bypassed.
pub type KprobePreHandler = fn(&Kprobe, &mut PtRegs) -> bool;

/// Post-execution callback invoked after the probed instruction has completed.
pub type KprobePostHandler = fn(&Kprobe, &mut PtRegs, flags: u64);

/// Per-core single-step state during out-of-line execution.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub struct KprobeSingleStepState {
    pub active_probe_id: u64,
    pub original_rip: u64,
    pub next_rip: u64,
    pub original_rflags: u64,
    pub in_progress: bool,
}

/// Dynamic kernel instrumentation probe descriptor.
#[derive(Copy, Clone, Debug)]
pub struct Kprobe {
    /// Unique probe identifier.
    pub id: u64,
    /// Memory address in kernel space where the probe is installed.
    pub addr: usize,
    /// Original instruction byte replaced by INT3 (0xCC).
    pub original_byte: u8,
    /// Human-readable symbol name.
    pub symbol: [u8; MAX_SYMBOL_LEN],
    pub symbol_len: usize,
    /// Whether probe is enabled.
    pub enabled: bool,
    /// Whether probe is actively armed with 0xCC in memory.
    pub armed: bool,
    /// Number of times this probe has been triggered.
    pub hit_count: u64,
    /// Optional pre-invocation callback.
    pub pre_handler: Option<KprobePreHandler>,
    /// Optional post-invocation callback.
    pub post_handler: Option<KprobePostHandler>,
    /// Out-of-line single-step buffer holding the original instruction.
    pub single_step_buffer: [u8; SINGLE_STEP_BUF_SIZE],
    pub single_step_len: usize,
}

impl Kprobe {
    pub const fn new(id: u64, addr: usize) -> Self {
        Self {
            id,
            addr,
            original_byte: 0,
            symbol: [0; MAX_SYMBOL_LEN],
            symbol_len: 0,
            enabled: true,
            armed: false,
            hit_count: 0,
            pre_handler: None,
            post_handler: None,
            single_step_buffer: [0x90; SINGLE_STEP_BUF_SIZE],
            single_step_len: 0,
        }
    }

    /// Sets the symbol name for this probe.
    pub fn set_symbol(&mut self, name: &str) {
        let bytes = name.as_bytes();
        let copy_len = bytes.len().min(MAX_SYMBOL_LEN);
        self.symbol[..copy_len].copy_from_slice(&bytes[..copy_len]);
        self.symbol_len = copy_len;
    }

    /// Returns the symbol name as a string slice.
    pub fn symbol_name(&self) -> &str {
        core::str::from_utf8(&self.symbol[..self.symbol_len]).unwrap_or("<unknown>")
    }
}

/// Central registry managing all kernel dynamic probes.
pub struct KprobeRegistry {
    probes: [Option<Kprobe>; MAX_KPROBES],
    next_id: u64,
    count: usize,
    pub step_state: KprobeSingleStepState,
}

impl KprobeRegistry {
    pub const fn new() -> Self {
        Self {
            probes: [None; MAX_KPROBES],
            next_id: 1,
            count: 0,
            step_state: KprobeSingleStepState {
                active_probe_id: 0,
                original_rip: 0,
                next_rip: 0,
                original_rflags: 0,
                in_progress: false,
            },
        }
    }

    #[inline(always)]
    pub fn count(&self) -> usize {
        self.count
    }

    #[inline(always)]
    pub fn capacity(&self) -> usize {
        MAX_KPROBES
    }

    /// Registers a new kprobe at `addr` with a symbolic name.
    pub fn register(&mut self, addr: usize, symbol: &str) -> Result<u64, KprobeError> {
        self.register_with_handlers(addr, symbol, None, None)
    }

    /// Registers a new kprobe with pre and post handlers.
    pub fn register_with_handlers(
        &mut self,
        addr: usize,
        symbol: &str,
        pre_handler: Option<KprobePreHandler>,
        post_handler: Option<KprobePostHandler>,
    ) -> Result<u64, KprobeError> {
        if addr == 0 {
            return Err(KprobeError::InvalidAddress);
        }
        // Check for existing probe at same address
        if self.find_by_addr(addr).is_some() {
            return Err(KprobeError::AlreadyRegistered);
        }

        for slot in self.probes.iter_mut() {
            if slot.is_none() {
                let id = self.next_id;
                self.next_id += 1;
                let mut kp = Kprobe::new(id, addr);
                kp.set_symbol(symbol);
                kp.pre_handler = pre_handler;
                kp.post_handler = post_handler;
                *slot = Some(kp);
                self.count += 1;
                return Ok(id);
            }
        }

        Err(KprobeError::SlotFull)
    }

    /// Unregisters and frees a kprobe slot. If armed, probe must be disarmed first or will error.
    pub fn unregister(&mut self, id: u64) -> Result<(), KprobeError> {
        for slot in self.probes.iter_mut() {
            if let Some(kp) = slot {
                if kp.id == id {
                    if kp.armed {
                        return Err(KprobeError::AlreadyArmed);
                    }
                    *slot = None;
                    self.count -= 1;
                    return Ok(());
                }
            }
        }
        Err(KprobeError::NotFound)
    }

    /// Arms the probe by saving the original byte and writing INT3 (0xCC) into code memory.
    pub fn arm_probe(&mut self, id: u64, code_mem: &mut [u8]) -> Result<(), KprobeError> {
        let probe = self.find_by_id_mut(id).ok_or(KprobeError::NotFound)?;
        if probe.armed {
            return Err(KprobeError::AlreadyArmed);
        }
        if code_mem.is_empty() {
            return Err(KprobeError::MemoryFault);
        }

        probe.original_byte = code_mem[0];
        // Populate single step buffer with instruction bytes (up to 16 bytes)
        let copy_len = code_mem.len().min(SINGLE_STEP_BUF_SIZE);
        probe.single_step_buffer[..copy_len].copy_from_slice(&code_mem[..copy_len]);
        probe.single_step_len = copy_len;

        // Atomically replace first byte with INT3 (0xCC)
        code_mem[0] = INT3_OPCODE;
        fence(Ordering::SeqCst);
        probe.armed = true;
        Ok(())
    }

    /// Disarms the probe by restoring the original instruction byte.
    pub fn disarm_probe(&mut self, id: u64, code_mem: &mut [u8]) -> Result<(), KprobeError> {
        let probe = self.find_by_id_mut(id).ok_or(KprobeError::NotFound)?;
        if !probe.armed {
            return Err(KprobeError::NotArmed);
        }
        if code_mem.is_empty() {
            return Err(KprobeError::MemoryFault);
        }

        code_mem[0] = probe.original_byte;
        fence(Ordering::SeqCst);
        probe.armed = false;
        Ok(())
    }

    pub fn find_by_addr(&self, addr: usize) -> Option<&Kprobe> {
        for slot in self.probes.iter() {
            if let Some(kp) = slot {
                if kp.addr == addr {
                    return Some(kp);
                }
            }
        }
        None
    }

    pub fn find_by_addr_mut(&mut self, addr: usize) -> Option<&mut Kprobe> {
        for slot in self.probes.iter_mut() {
            if let Some(kp) = slot {
                if kp.addr == addr {
                    return Some(kp);
                }
            }
        }
        None
    }

    pub fn find_by_id(&self, id: u64) -> Option<&Kprobe> {
        for slot in self.probes.iter() {
            if let Some(kp) = slot {
                if kp.id == id {
                    return Some(kp);
                }
            }
        }
        None
    }

    pub fn find_by_id_mut(&mut self, id: u64) -> Option<&mut Kprobe> {
        for slot in self.probes.iter_mut() {
            if let Some(kp) = slot {
                if kp.id == id {
                    return Some(kp);
                }
            }
        }
        None
    }

    /// Handles an INT3 (#BP) breakpoint exception.
    /// In x86-64, CPU pushes `saved_rip = probe_addr + 1`.
    pub fn handle_int3(&mut self, regs: &mut PtRegs, insn_len: usize) -> KprobeBreakStatus {
        if regs.rip == 0 {
            return KprobeBreakStatus::NotOurs;
        }

        let fault_addr = (regs.rip as usize).saturating_sub(1);
        let (id, pre_fn) = match self.find_by_addr(fault_addr) {
            Some(probe) if probe.enabled && probe.armed => (probe.id, probe.pre_handler),
            _ => return KprobeBreakStatus::NotOurs,
        };

        // Increment hit count
        if let Some(p) = self.find_by_id_mut(id) {
            p.hit_count = p.hit_count.saturating_add(1);
        }

        // 1. Invoke pre-handler callback with register snapshot
        let should_continue = if let Some(pre) = pre_fn {
            let p_ref = self.find_by_id(id).unwrap();
            pre(p_ref, regs)
        } else {
            true
        };

        if !should_continue {
            // Pre-handler requested skip: advance RIP past instruction
            regs.rip = (fault_addr + insn_len) as u64;
            return KprobeBreakStatus::Handled;
        }

        // 2. Prepare single-step execution off-trampoline
        let orig_rflags = regs.rflags;
        regs.rflags |= EFLAGS_TF; // Set Trap Flag

        let probe = self.find_by_id(id).unwrap();
        let ss_buf_addr = probe.single_step_buffer.as_ptr() as usize as u64;
        regs.rip = ss_buf_addr;

        self.step_state = KprobeSingleStepState {
            active_probe_id: id,
            original_rip: fault_addr as u64,
            next_rip: (fault_addr + insn_len) as u64,
            original_rflags: orig_rflags,
            in_progress: true,
        };

        KprobeBreakStatus::Handled
    }

    /// Handles a Single-Step (#DB) debug exception following instruction execution.
    /// Clears Trap Flag, invokes post-handler, and resumes original execution path.
    pub fn handle_debug_trap(&mut self, regs: &mut PtRegs) -> KprobeBreakStatus {
        if !self.step_state.in_progress {
            return KprobeBreakStatus::NotOurs;
        }

        let id = self.step_state.active_probe_id;
        let next_rip = self.step_state.next_rip;
        let orig_rflags = self.step_state.original_rflags;

        // 1. Clear Trap Flag from RFLAGS
        regs.rflags &= !EFLAGS_TF;

        // 2. Invoke post-handler callback
        let post_fn = self.find_by_id(id).and_then(|p| p.post_handler);
        if let Some(post) = post_fn {
            let p_ref = self.find_by_id(id).unwrap();
            post(p_ref, regs, orig_rflags);
        }

        // 3. Restore RIP to the next instruction in original function body
        regs.rip = next_rip;
        self.step_state.in_progress = false;

        KprobeBreakStatus::Handled
    }
}

/// Unit test verifying INT3 software breakpoint handling and dynamic kprobe registry.
pub fn test_int3_and_kprobe_registry() -> bool {
    let mut registry = KprobeRegistry::new();
    let mut fake_code = [0x55, 0x48, 0x89, 0xE5, 0xC3]; // push rbp; mov rbp, rsp; ret
    let probe_addr = 0x2000_1000usize;

    // 1. Register kprobe
    let id = match registry.register(probe_addr, "sys_open") {
        Ok(id) => id,
        Err(_) => return false,
    };
    if registry.count() != 1 {
        return false;
    }
    let p = registry.find_by_id(id).expect("probe lookup failed");
    if p.addr != probe_addr || p.symbol_name() != "sys_open" || p.armed {
        return false;
    }

    // 2. Arm probe: replaces fake_code[0] with 0xCC and saves 0x55
    if registry.arm_probe(id, &mut fake_code).is_err() {
        return false;
    }
    if fake_code[0] != INT3_OPCODE {
        return false;
    }
    let p_armed = registry.find_by_id(id).expect("probe lookup 2 failed");
    if !p_armed.armed || p_armed.original_byte != 0x55 {
        return false;
    }

    // 3. Simulate INT3 breakpoint trap: CPU pushes saved_rip = probe_addr + 1
    let mut regs = PtRegs::empty();
    regs.rip = (probe_addr + 1) as u64;
    regs.rax = 0x1234;

    let status = registry.handle_int3(&mut regs, 1);
    if status != KprobeBreakStatus::Handled {
        return false;
    }
    // TF must be set
    if (regs.rflags & EFLAGS_TF) == 0 {
        return false;
    }
    let p_hit = registry.find_by_id(id).expect("probe lookup 3 failed");
    if p_hit.hit_count != 1 {
        return false;
    }

    // Single step trap resolution
    let db_status = registry.handle_debug_trap(&mut regs);
    if db_status != KprobeBreakStatus::Handled {
        return false;
    }
    // RIP must be at next instruction (probe_addr + 1)
    if regs.rip != (probe_addr + 1) as u64 {
        return false;
    }
    // TF must be cleared
    if (regs.rflags & EFLAGS_TF) != 0 {
        return false;
    }

    // 4. Test non-registered address returns NotOurs
    let mut rogue_regs = PtRegs::empty();
    rogue_regs.rip = 0x3000_5001; // Not registered
    if registry.handle_int3(&mut rogue_regs, 1) != KprobeBreakStatus::NotOurs {
        return false;
    }

    // 5. Disarm probe: restores fake_code[0] to 0x55
    if registry.disarm_probe(id, &mut fake_code).is_err() {
        return false;
    }
    if fake_code[0] != 0x55 {
        return false;
    }

    // 6. Unregister probe
    if registry.unregister(id).is_err() {
        return false;
    }
    if registry.count() != 0 {
        return false;
    }

    true
}

static mut TEST_PRE_HIT: bool = false;
static mut TEST_POST_HIT: bool = false;
static mut TEST_CAPTURED_ARG: u64 = 0;

fn sample_pre_handler(_kp: &Kprobe, regs: &mut PtRegs) -> bool {
    unsafe {
        TEST_PRE_HIT = true;
        TEST_CAPTURED_ARG = regs.rdi;
        // Mutate register to verify context modification
        regs.rax = 0x9999;
    }
    true
}

fn sample_post_handler(_kp: &Kprobe, regs: &mut PtRegs, _flags: u64) {
    unsafe {
        TEST_POST_HIT = true;
        // Verify register mutation survived
        if regs.rax == 0x9999 {
            regs.rbx = 0xAAAA;
        }
    }
}

/// Unit test verifying pre/post probe callback invocation and register context capture.
pub fn test_kprobe_callbacks_and_context() -> bool {
    unsafe {
        TEST_PRE_HIT = false;
        TEST_POST_HIT = false;
        TEST_CAPTURED_ARG = 0;
    }

    let mut registry = KprobeRegistry::new();
    let mut code_buf = [0x48, 0x89, 0xF8, 0xC3]; // mov rax, rdi (3 bytes); ret
    let target_addr = 0x4000_2000usize;

    let probe_id = registry
        .register_with_handlers(
            target_addr,
            "sample_fn",
            Some(sample_pre_handler),
            Some(sample_post_handler),
        )
        .expect("registration failed");

    registry.arm_probe(probe_id, &mut code_buf).expect("arm failed");

    // Simulate entry into probed function with RDI argument 0xCAFE_BABE
    let mut regs = PtRegs::empty();
    regs.rip = (target_addr + 1) as u64; // CPU executed INT3 at target_addr
    regs.rdi = 0xCAFE_BABE;
    regs.rax = 0;

    // Handle INT3: triggers pre-handler
    let res = registry.handle_int3(&mut regs, 3);
    if res != KprobeBreakStatus::Handled {
        return false;
    }

    unsafe {
        if !TEST_PRE_HIT || TEST_CAPTURED_ARG != 0xCAFE_BABE {
            return false;
        }
    }

    // Verify pre-handler mutated RAX and set TF
    if regs.rax != 0x9999 || (regs.rflags & EFLAGS_TF) == 0 {
        return false;
    }

    // Simulate CPU executing single instruction and triggering #DB
    let db_res = registry.handle_debug_trap(&mut regs);
    if db_res != KprobeBreakStatus::Handled {
        return false;
    }

    unsafe {
        if !TEST_POST_HIT {
            return false;
        }
    }

    // Verify post-handler verified RAX and mutated RBX
    if regs.rbx != 0xAAAA {
        return false;
    }
    // Verify TF is cleared and RIP points to next instruction (target_addr + 3)
    if (regs.rflags & EFLAGS_TF) != 0 || regs.rip != (target_addr + 3) as u64 {
        return false;
    }

    // Disarm and unregister
    registry.disarm_probe(probe_id, &mut code_buf).expect("disarm failed");
    registry.unregister(probe_id).expect("unregister failed");

    true
}
