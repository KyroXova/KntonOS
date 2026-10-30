//! x86-64 Hardware Debug Registers (DR0-DR7) Watchpoint Control & Status Engine.
//!
//! Provides configuration for CPU execution breakpoints and data read/write watchpoints
//! across DR0..DR3 registers, with DR7 enable/length/condition controls and DR6 status parsing.

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
