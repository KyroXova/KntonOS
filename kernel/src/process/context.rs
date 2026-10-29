//! Architecture-specific context definition and assembly context switch routine.

#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct Context {
    pub rflags: u64,
    pub r15: u64,
    pub r14: u64,
    pub r13: u64,
    pub r12: u64,
    pub rsi: u64,
    pub rdi: u64,
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
            rsi: 0,
            rdi: 0,
            rbx: 0,
            rbp: 0,
            rip: 0,
        }
    }
}

/// Low-level assembly context switch between two execution contexts.
/// Uses `extern "win64"` ABI convention (RCX = prev_rsp, RDX = next_rsp).
#[unsafe(naked)]
pub unsafe extern "win64" fn switch_context(prev_rsp: *mut u64, next_rsp: u64) {
    core::arch::naked_asm!(
        // Save callee-saved registers onto current thread stack
        "push rbp",
        "push rbx",
        "push rdi",
        "push rsi",
        "push r12",
        "push r13",
        "push r14",
        "push r15",
        "pushfq",
        // Save current stack pointer to prev_rsp (in RCX for win64)
        "mov [rcx], rsp",
        // Load next stack pointer from next_rsp (in RDX for win64)
        "mov rsp, rdx",
        // Restore next thread callee-saved registers
        "popfq",
        "pop r15",
        "pop r14",
        "pop r13",
        "pop r12",
        "pop rsi",
        "pop rdi",
        "pop rbx",
        "pop rbp",
        "ret",
    );
}
