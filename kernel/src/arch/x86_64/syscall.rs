#![allow(dead_code)]

use crate::serial_write_str;

const IA32_EFER: u32 = 0xC0000080;
const IA32_STAR: u32 = 0xC0000081;
const IA32_LSTAR: u32 = 0xC0000082;
const IA32_FMASK: u32 = 0xC0000084;

const EFER_SCE: u64 = 1 << 0; // System Call Extensions enable
const RFLAGS_IF: u64 = 1 << 9; // Interrupt flag

#[inline(always)]
pub unsafe fn rdmsr(msr: u32) -> u64 {
    let (low, high): (u32, u32);
    core::arch::asm!(
        "rdmsr",
        in("ecx") msr,
        out("eax") low,
        out("edx") high,
        options(nomem, nostack, preserves_flags)
    );
    ((high as u64) << 32) | (low as u64)
}

#[inline(always)]
pub unsafe fn wrmsr(msr: u32, val: u64) {
    let low = (val & 0xFFFFFFFF) as u32;
    let high = (val >> 32) as u32;
    core::arch::asm!(
        "wrmsr",
        in("ecx") msr,
        in("eax") low,
        in("edx") high,
        options(nomem, nostack, preserves_flags)
    );
}

#[no_mangle]
pub extern "C" fn syscall_dispatch(num: u64) -> u64 {
    serial_write_str("[KNT-SYSCALL] Fast syscall executed via MSR LSTAR (ID: ");
    if num == 0 {
        serial_write_str("0: SYS_TEST)\r\n");
    } else {
        serial_write_str("UNKNOWN)\r\n");
    }
    0 // Return success
}

#[unsafe(naked)]
pub unsafe extern "C" fn syscall_entry() {
    core::arch::naked_asm!(
        // On syscall: RCX holds user RIP, R11 holds user RFLAGS
        "push rcx",
        "push r11",
        "push rbp",
        "push rbx",
        "push r12",
        "push r13",
        "push r14",
        "push r15",
        "mov rcx, rax", // pass syscall number as first argument
        "call {dispatch}",
        "pop r15",
        "pop r14",
        "pop r13",
        "pop r12",
        "pop rbx",
        "pop rbp",
        "pop r11",
        "pop rcx",
        "sysretq",
        dispatch = sym syscall_dispatch,
    );
}

pub fn init() {
    unsafe {
        // 1. Enable SCE in IA32_EFER
        let efer = rdmsr(IA32_EFER);
        wrmsr(IA32_EFER, efer | EFER_SCE);

        // 2. Configure IA32_STAR: kernel CS/SS (bits 32..47), user sysret CS/SS (bits 48..63)
        let star: u64 = ((0x10_u64) << 48) | ((0x08_u64) << 32);
        wrmsr(IA32_STAR, star);

        // 3. Configure IA32_LSTAR with entry point
        wrmsr(IA32_LSTAR, syscall_entry as *const () as usize as u64);

        // 4. Configure IA32_FMASK to clear Interrupt Flag on syscall entry
        wrmsr(IA32_FMASK, RFLAGS_IF);
    }
}
