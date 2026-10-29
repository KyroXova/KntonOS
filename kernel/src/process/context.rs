//! Architecture-specific context definition and assembly context switch routine.

#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct Context {
    pub rflags: u64,
    pub r15: u64,
    pub r14: u64,
    pub r13: u64,
    pub r12: u64,
    pub rbx: u64,
    pub rbp: u64,
    pub rip: u64,
}

impl Context {
    pub const fn empty() -> Self {
        Self {
            rflags: 0x202, // IF (Interrupt Flag) enabled
            r15: 0,
            r14: 0,
            r13: 0,
            r12: 0,
            rbx: 0,
            rbp: 0,
            rip: 0,
        }
    }
}

/// Low-level assembly context switch between two execution contexts.
///
/// Saves callee-saved registers and RFLAGS of `prev` onto its stack,
/// saves `rsp` to `*prev_rsp`, loads `next_rsp` into `rsp`,
/// and restores the new thread's registers and RFLAGS before returning.
#[unsafe(naked)]
pub unsafe extern "C" fn switch_context(prev_rsp: *mut u64, next_rsp: u64) {
    core::arch::naked_asm!(
        // Save callee-saved registers onto current thread stack
        "push rbp",
        "push rbx",
        "push r12",
        "push r13",
        "push r14",
        "push r15",
        "pushfq",
        // Save current stack pointer to prev_rsp (rdi in SysV, rcx in Win64)
        "mov [rdi], rsp",
        // Load next stack pointer (rsi in SysV, rdx in Win64)
        "mov rsp, rsi",
        // Restore next thread callee-saved registers
        "popfq",
        "pop r15",
        "pop r14",
        "pop r13",
        "pop r12",
        "pop rbx",
        "pop rbp",
        "ret",
    );
}
