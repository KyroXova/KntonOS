pub mod gdt;
pub mod tss;
pub mod idt;
pub mod syscall;
pub mod acpi;
pub mod apic;
pub mod timer;
pub mod smp;
pub mod power;

pub use smp::{test_smp_and_timer_subsystem, SMP};
pub use power::{
    inw, outw, parse_dsdt_sleep_state, test_fadt_parser_and_power_structures,
    test_sleep_and_reset_engine, FadtInfo, GenericAddressStructure, PowerError, PowerManager,
    PowerState, SleepTypeConfig, DSDT_SIGNATURE, FADT_SIGNATURE, GAS_ADDR_SPACE_PCI_CFG,
    GAS_ADDR_SPACE_SYS_IO, GAS_ADDR_SPACE_SYS_MEM, PM1_BM_RLD, PM1_GBL_RLS, PM1_SCI_EN,
    PM1_SLP_EN, PM1_SLP_TYP_MASK, PORT_CF9_RESET, PORT_KBD_RESET, POWER_MANAGER,
};

use crate::serial_write_str;

pub fn init() {
    serial_write_str("[KNT-ARCH] Initializing x86-64 CPU infrastructure...\r\n");

    // Ensure interrupts are disabled and legacy 8259 PIC is masked
    unsafe {
        core::arch::asm!("cli", options(nomem, nostack, preserves_flags));
        crate::outb(0x21, 0xFF);
        crate::outb(0xA1, 0xFF);
    }

    // 1. Initialize GDT & TSS
    gdt::init();
    serial_write_str("  * GDT & TSS loaded with kernel/user segments and IST[0] stack\r\n");

    // 2. Initialize IDT with 32 exception handlers and 224 interrupt gates
    idt::init();
    serial_write_str("  * IDT loaded with 32 CPU exception vectors and 224 interrupt gates\r\n");

    // 3. Initialize Syscall MSRs
    syscall::init();
    serial_write_str("  * Fast system call MSRs (STAR, LSTAR, FMASK, EFER.SCE) configured\r\n");

    serial_write_str("[KNT-ARCH] CPU infrastructure initialization complete\r\n");
}

pub fn init_smp() {
    smp::SMP.lock().init();
}
