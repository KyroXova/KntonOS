pub mod gdt;
pub mod tss;
pub mod idt;
pub mod syscall;

use crate::serial_write_str;

pub fn init() {
    serial_write_str("[KNT-ARCH] Initializing x86-64 CPU infrastructure...\r\n");

    // 1. Initialize GDT & TSS
    gdt::init();
    serial_write_str("  * GDT & TSS loaded with kernel/user segments and IST[0] stack\r\n");

    // 2. Initialize IDT with 32 exception handlers
    idt::init();
    serial_write_str("  * IDT loaded with 32 CPU exception vectors\r\n");

    // 3. Initialize Syscall MSRs
    syscall::init();
    serial_write_str("  * Fast system call MSRs (STAR, LSTAR, FMASK, EFER.SCE) configured\r\n");

    serial_write_str("[KNT-ARCH] CPU infrastructure initialization complete\r\n");
}
