//! x86-64 hardware debug registers (DR0-DR7) watchpoint control and status engine.

/// Condition triggering a hardware breakpoint/watchpoint in DR7 (bits 16-31).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum WatchpointCondition {
    /// 00b: Break on instruction execution only.
    InstructionExecution = 0b00,
    /// 01b: Break on data writes only.
    DataWrite = 0b01,
    /// 10b: Break on I/O port read or write (if CR4.DE is set).
    IoReadWrite = 0b10,
    /// 11b: Break on data reads or writes (excluding instruction fetches).
    DataReadWrite = 0b11,
}

/// Size/alignment of monitored memory in DR7 (bits 18-19, 22-23, 26-27, 30-31).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum WatchpointLength {
    /// 00b: 1-byte length.
    Byte1 = 0b00,
    /// 01b: 2-byte length (must be 2-byte aligned).
    Byte2 = 0b01,
    /// 10b: 8-byte length in 64-bit mode (must be 8-byte aligned).
    Byte8 = 0b10,
    /// 11b: 4-byte length (must be 4-byte aligned).
    Byte4 = 0b11,
}

/// Hardware debug register index (0 to 3).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum DebugRegisterIndex {
    Dr0 = 0,
    Dr1 = 1,
    Dr2 = 2,
    Dr3 = 3,
}

/// Status flags parsed from DR6 (Debug Status Register).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct DebugStatusRegister {
    /// Bit 0: DR0 breakpoint condition was detected.
    pub dr0_triggered: bool,
    /// Bit 1: DR1 breakpoint condition was detected.
    pub dr1_triggered: bool,
    /// Bit 2: DR2 breakpoint condition was detected.
    pub dr2_triggered: bool,
    /// Bit 3: DR3 breakpoint condition was detected.
    pub dr3_triggered: bool,
    /// Bit 13: Debug register access detected (BD flag, when DR7.GD is 1).
    pub debug_register_access: bool,
    /// Bit 14: Single-step execution trap (BS flag, triggered by EFLAGS.TF).
    pub single_step: bool,
    /// Bit 15: Task switch trap (BT flag).
    pub task_switch: bool,
    /// Raw 64-bit DR6 value.
    pub raw: u64,
}

impl DebugStatusRegister {
    pub const fn from_raw(raw: u64) -> Self {
        Self {
            dr0_triggered: (raw & (1 << 0)) != 0,
            dr1_triggered: (raw & (1 << 1)) != 0,
            dr2_triggered: (raw & (1 << 2)) != 0,
            dr3_triggered: (raw & (1 << 3)) != 0,
            debug_register_access: (raw & (1 << 13)) != 0,
            single_step: (raw & (1 << 14)) != 0,
            task_switch: (raw & (1 << 15)) != 0,
            raw,
        }
    }

    #[inline(always)]
    pub fn is_watchpoint_hit(&self, index: DebugRegisterIndex) -> bool {
        match index {
            DebugRegisterIndex::Dr0 => self.dr0_triggered,
            DebugRegisterIndex::Dr1 => self.dr1_triggered,
            DebugRegisterIndex::Dr2 => self.dr2_triggered,
            DebugRegisterIndex::Dr3 => self.dr3_triggered,
        }
    }
}

/// Control register DR7 (Debug Control Register) builder and parser.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DebugControlRegister {
    pub raw: u64,
}

impl DebugControlRegister {
    pub const fn new() -> Self {
        // Bit 10 is reserved and must be written as 1
        Self { raw: 1 << 10 }
    }

    pub const fn from_raw(raw: u64) -> Self {
        Self { raw }
    }

    /// Enables or disables a debug register slot (local enable L0..L3, global enable G0..G3).
    pub fn set_enable(&mut self, index: DebugRegisterIndex, local: bool, global: bool) {
        let bit_local = (index as u8) * 2;
        let bit_global = bit_local + 1;

        if local {
            self.raw |= 1 << bit_local;
        } else {
            self.raw &= !(1 << bit_local);
        }

        if global {
            self.raw |= 1 << bit_global;
        } else {
            self.raw &= !(1 << bit_global);
        }
    }

    /// Configures watchpoint condition and length for slot (DR0..DR3).
    pub fn configure_slot(
        &mut self,
        index: DebugRegisterIndex,
        condition: WatchpointCondition,
        length: WatchpointLength,
    ) {
        let shift = 16 + (index as u8) * 4;
        let cond_bits = (condition as u64) & 0x03;
        let len_bits = (length as u64) & 0x03;
        let field = cond_bits | (len_bits << 2);

        // Clear existing 4-bit field and insert new configuration
        self.raw &= !(0x0F << shift);
        self.raw |= field << shift;
    }

    /// Enables General Detect (GD flag, bit 13) to intercept MOVs to/from DR registers.
    pub fn set_general_detect(&mut self, enable: bool) {
        if enable {
            self.raw |= 1 << 13;
        } else {
            self.raw &= !(1 << 13);
        }
    }
}

/// Software controller managing hardware watchpoint registers DR0-DR7.
pub struct HardwareWatchpointController {
    dr: [usize; 4],
    dr7: DebugControlRegister,
}

impl HardwareWatchpointController {
    pub const fn new() -> Self {
        Self {
            dr: [0; 4],
            dr7: DebugControlRegister::new(),
        }
    }

    /// Sets watchpoint address and attributes in slot 0..3.
    pub fn set_watchpoint(
        &mut self,
        index: DebugRegisterIndex,
        address: usize,
        condition: WatchpointCondition,
        length: WatchpointLength,
    ) {
        self.dr[index as usize] = address;
        self.dr7.configure_slot(index, condition, length);
        self.dr7.set_enable(index, true, false);
    }

    /// Clears watchpoint from slot 0..3.
    pub fn clear_watchpoint(&mut self, index: DebugRegisterIndex) {
        self.dr[index as usize] = 0;
        self.dr7.set_enable(index, false, false);
    }

    #[inline(always)]
    pub fn get_address(&self, index: DebugRegisterIndex) -> usize {
        self.dr[index as usize]
    }

    #[inline(always)]
    pub fn get_dr7(&self) -> DebugControlRegister {
        self.dr7
    }

    /// Simulates hardware write to DR registers and syncs to CPU if running in privileged mode.
    pub fn apply_to_cpu(&self) {
        unsafe {
            let dr0 = self.dr[0];
            let dr1 = self.dr[1];
            let dr2 = self.dr[2];
            let dr3 = self.dr[3];
            let dr7 = self.dr7.raw;

            core::arch::asm!(
                "mov dr0, {0}",
                "mov dr1, {1}",
                "mov dr2, {2}",
                "mov dr3, {3}",
                "mov dr7, {4}",
                in(reg) dr0,
                in(reg) dr1,
                in(reg) dr2,
                in(reg) dr3,
                in(reg) dr7,
                options(nostack, preserves_flags)
            );
        }
    }

    /// Reads active status from CPU DR6 register.
    pub fn read_status_from_cpu() -> DebugStatusRegister {
        let raw: u64;
        unsafe {
            core::arch::asm!(
                "mov {0}, dr6",
                out(reg) raw,
                options(nostack, preserves_flags)
            );
        }
        DebugStatusRegister::from_raw(raw)
    }

    /// Clears CPU DR6 register.
    pub fn clear_status_on_cpu() {
        unsafe {
            let zero: u64 = 0xFFFF_0FF0; // Reserved bits high per Intel SDM
            core::arch::asm!(
                "mov dr6, {0}",
                in(reg) zero,
                options(nostack, preserves_flags)
            );
        }
    }
}

/// Unit test verifying hardware debug register DR0-DR7 configuration, bitfield manipulation, and status parsing.
pub fn test_hardware_watchpoint_registers() -> bool {
    let mut ctrl = HardwareWatchpointController::new();

    // Configure DR0 for 4-byte MMIO register write watchpoint at 0xFEE00300 (LAPIC ICR)
    let addr_icr = 0xFEE0_0300;
    ctrl.set_watchpoint(
        DebugRegisterIndex::Dr0,
        addr_icr,
        WatchpointCondition::DataWrite,
        WatchpointLength::Byte4,
    );

    // Configure DR1 for 8-byte MMIO read/write watchpoint at 0xFED00010 (HPET Main Counter)
    let addr_hpet = 0xFED0_0010;
    ctrl.set_watchpoint(
        DebugRegisterIndex::Dr1,
        addr_hpet,
        WatchpointCondition::DataReadWrite,
        WatchpointLength::Byte8,
    );

    if ctrl.get_address(DebugRegisterIndex::Dr0) != addr_icr {
        return false;
    }
    if ctrl.get_address(DebugRegisterIndex::Dr1) != addr_hpet {
        return false;
    }

    let dr7 = ctrl.get_dr7();
    // Bit 0 (L0) must be 1, Bit 2 (L1) must be 1
    if (dr7.raw & 0x01) == 0 || (dr7.raw & 0x04) == 0 {
        return false;
    }

    // Verify DR0 condition/length bits (bits 16-19): cond=01b (DataWrite), len=11b (Byte4) -> 1101b = 0xD
    let slot0_cfg = (dr7.raw >> 16) & 0x0F;
    if slot0_cfg != 0x0D {
        return false;
    }

    // Verify DR1 condition/length bits (bits 20-23): cond=11b (DataReadWrite), len=10b (Byte8) -> 1011b = 0xB
    let slot1_cfg = (dr7.raw >> 20) & 0x0F;
    if slot1_cfg != 0x0B {
        return false;
    }

    // Verify DR6 status decoding
    let mock_dr6: u64 = 0xFFFF_0FF1; // DR0 hit (bit 0 = 1)
    let status = DebugStatusRegister::from_raw(mock_dr6);
    if !status.is_watchpoint_hit(DebugRegisterIndex::Dr0) {
        return false;
    }
    if status.is_watchpoint_hit(DebugRegisterIndex::Dr1) {
        return false;
    }
    if status.single_step {
        return false;
    }

    // Clear DR0 watchpoint and verify disabled bit
    ctrl.clear_watchpoint(DebugRegisterIndex::Dr0);
    if (ctrl.get_dr7().raw & 0x01) != 0 {
        return false;
    }

    true
}

// Single-step trap execution and register state snapshotting.

/// CPU EFLAGS Trap Flag (TF, bit 8).
pub const EFLAGS_TF: u64 = 1 << 8;

/// Complete 64-bit CPU architectural register snapshot captured during a debug trap or watchpoint.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct CpuRegisterSnapshot {
    pub rax: u64,
    pub rbx: u64,
    pub rcx: u64,
    pub rdx: u64,
    pub rsi: u64,
    pub rdi: u64,
    pub rbp: u64,
    pub rsp: u64,
    pub r8: u64,
    pub r9: u64,
    pub r10: u64,
    pub r11: u64,
    pub r12: u64,
    pub r13: u64,
    pub r14: u64,
    pub r15: u64,
    pub rip: u64,
    pub rflags: u64,
    pub cs: u64,
    pub ss: u64,
    pub cr2: u64,
    pub dr6: u64,
}

impl CpuRegisterSnapshot {
    pub fn empty() -> Self {
        Self::default()
    }

    /// Sets or clears the single-step Trap Flag (TF) in saved RFLAGS.
    #[inline(always)]
    pub fn set_single_step(&mut self, enable: bool) {
        if enable {
            self.rflags |= EFLAGS_TF;
        } else {
            self.rflags &= !EFLAGS_TF;
        }
    }

    #[inline(always)]
    pub fn is_single_step_enabled(&self) -> bool {
        (self.rflags & EFLAGS_TF) != 0
    }
}

/// Diagnostic trap record logging instruction operand, trigger cause, and state snapshot.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrapReason {
    HardwareWatchpoint(DebugRegisterIndex),
    MmioPageFaultRange(u32),
    SingleStepInstruction,
    SoftwareBreakpoint,
}

#[derive(Debug, Clone, Copy)]
pub struct DiagnosticTrapLog {
    pub reason: TrapReason,
    pub snapshot: CpuRegisterSnapshot,
    pub instruction_bytes: [u8; 15],
    pub instruction_len: usize,
    pub timestamp_tsc: u64,
}

impl DiagnosticTrapLog {
    pub fn new(reason: TrapReason, snapshot: CpuRegisterSnapshot, insn: &[u8], tsc: u64) -> Self {
        let mut insn_buf = [0u8; 15];
        let len = core::cmp::min(insn.len(), 15);
        insn_buf[..len].copy_from_slice(&insn[..len]);
        Self {
            reason,
            snapshot,
            instruction_bytes: insn_buf,
            instruction_len: len,
            timestamp_tsc: tsc,
        }
    }
}

/// Single-step debug engine maintaining trace stepping sessions and diagnostic logs.
pub const MAX_TRAP_LOGS: usize = 16;

pub struct SingleStepTrapEngine {
    stepping_active: bool,
    break_on_next_instruction: bool,
    #[allow(dead_code)]
    target_rip: Option<u64>,
    trap_history: [Option<DiagnosticTrapLog>; MAX_TRAP_LOGS],
    history_count: usize,
    step_count: u64,
}

impl SingleStepTrapEngine {
    pub const fn new() -> Self {
        Self {
            stepping_active: false,
            break_on_next_instruction: false,
            target_rip: None,
            trap_history: [None; MAX_TRAP_LOGS],
            history_count: 0,
            step_count: 0,
        }
    }

    /// Arms single-stepping for the thread resuming from `snapshot`.
    pub fn arm_single_step(&mut self, snapshot: &mut CpuRegisterSnapshot) {
        self.stepping_active = true;
        self.break_on_next_instruction = true;
        snapshot.set_single_step(true);
    }

    /// Disarms single-stepping on `snapshot`.
    pub fn disarm_single_step(&mut self, snapshot: &mut CpuRegisterSnapshot) {
        self.stepping_active = false;
        self.break_on_next_instruction = false;
        snapshot.set_single_step(false);
    }

    /// Handles a #DB (Debug Exception, vector 1) single step or watchpoint hit.
    pub fn handle_debug_exception(
        &mut self,
        snapshot: &mut CpuRegisterSnapshot,
        dr6_raw: u64,
        insn: &[u8],
        tsc: u64,
    ) -> TrapReason {
        let status = DebugStatusRegister::from_raw(dr6_raw);
        let reason = if status.single_step {
            self.step_count += 1;
            TrapReason::SingleStepInstruction
        } else if status.dr0_triggered {
            TrapReason::HardwareWatchpoint(DebugRegisterIndex::Dr0)
        } else if status.dr1_triggered {
            TrapReason::HardwareWatchpoint(DebugRegisterIndex::Dr1)
        } else if status.dr2_triggered {
            TrapReason::HardwareWatchpoint(DebugRegisterIndex::Dr2)
        } else if status.dr3_triggered {
            TrapReason::HardwareWatchpoint(DebugRegisterIndex::Dr3)
        } else {
            TrapReason::SoftwareBreakpoint
        };

        let log = DiagnosticTrapLog::new(reason, *snapshot, insn, tsc);
        self.record_log(log);

        reason
    }

    fn record_log(&mut self, log: DiagnosticTrapLog) {
        if self.history_count < MAX_TRAP_LOGS {
            self.trap_history[self.history_count] = Some(log);
            self.history_count += 1;
        } else {
            for i in 1..MAX_TRAP_LOGS {
                self.trap_history[i - 1] = self.trap_history[i];
            }
            self.trap_history[MAX_TRAP_LOGS - 1] = Some(log);
        }
    }

    #[inline(always)]
    pub const fn is_stepping_active(&self) -> bool {
        self.stepping_active
    }

    #[inline(always)]
    pub const fn total_steps(&self) -> u64 {
        self.step_count
    }

    #[inline(always)]
    pub const fn logged_traps_count(&self) -> usize {
        self.history_count
    }

    pub fn last_log(&self) -> Option<DiagnosticTrapLog> {
        if self.history_count > 0 {
            self.trap_history[self.history_count - 1]
        } else {
            None
        }
    }
}

/// Unit test verifying single-step arming, disarming, EFLAGS.TF manipulation, and register snapshotting.
pub fn test_single_step_and_snapshotting() -> bool {
    let mut engine = SingleStepTrapEngine::new();
    let mut snapshot = CpuRegisterSnapshot::empty();

    snapshot.rip = 0xFFFFFFFF_8010_4000;
    snapshot.rsp = 0xFFFF8000_0000_FFF0;
    snapshot.rax = 0x1234_5678_9ABC_DEF0;
    snapshot.rflags = 0x0000_0202; // IF=1, reserved=1

    // 1. Arm single step and verify TF set
    engine.arm_single_step(&mut snapshot);
    if !snapshot.is_single_step_enabled() || !engine.is_stepping_active() {
        return false;
    }
    if (snapshot.rflags & EFLAGS_TF) == 0 {
        return false;
    }

    // 2. Simulate #DB debug exception firing on single-step completion
    // DR6 BS bit is bit 14
    let dr6_single_step = 0xFFFF_0FF0 | (1 << 14);
    let mock_insn = [0x48, 0x89, 0xC3]; // mov rbx, rax
    let tsc = 1_000_000;

    snapshot.rip += 3; // Advanced past instruction
    snapshot.rbx = snapshot.rax;

    let reason = engine.handle_debug_exception(&mut snapshot, dr6_single_step, &mock_insn, tsc);
    if reason != TrapReason::SingleStepInstruction {
        return false;
    }

    if engine.total_steps() != 1 || engine.logged_traps_count() != 1 {
        return false;
    }

    let last_log = match engine.last_log() {
        Some(l) => l,
        None => return false,
    };
    if last_log.snapshot.rbx != snapshot.rax || last_log.snapshot.rip != 0xFFFFFFFF_8010_4003 {
        return false;
    }
    if last_log.instruction_len != 3 || &last_log.instruction_bytes[..3] != &mock_insn {
        return false;
    }

    // 3. Disarm single step
    engine.disarm_single_step(&mut snapshot);
    if snapshot.is_single_step_enabled() || engine.is_stepping_active() {
        return false;
    }

    true
}
