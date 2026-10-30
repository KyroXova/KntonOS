pub mod gdt;
pub mod tss;
pub mod idt;
pub mod syscall;
pub mod acpi;
pub mod apic;
pub mod timer;
pub mod smp;
pub mod power;
pub mod hpet;

pub use hpet::{
    calibrate_tsc, calibrate_tsc_with_samples, monotonic_sleep_ns, monotonic_time_ms,
    monotonic_time_ns, monotonic_time_us, nanoseconds_to_ticks, read_tsc, test_hpet_subsystem,
    ticks_to_nanoseconds, verify_hpet_counter_and_timers, verify_hpet_monotonic_and_tsc,
    verify_hpet_structures, AcpiHpetTable, HpetDevice, HpetInfo, HpetRegisters, HpetTableParser,
    HpetTimerConfig, HpetTimerMode, CALIBRATED_TSC_HZ, DEFAULT_HPET_BASE_ADDRESS, HPET_DEVICE,
    HPET_GEN_CONF_ENABLE, HPET_GEN_CONF_LEG_RT, HPET_REG_GCAP_ID, HPET_REG_GEN_CONF,
    HPET_REG_GINTR_STA, HPET_REG_MAIN_CNT, HPET_REG_TIM0_COMP, HPET_REG_TIM0_CONF,
    HPET_REG_TIM0_FSB, HPET_SIGNATURE, HPET_SIGNATURE_U32, HPET_TIMER_STRIDE, HPET_TN_32MODE,
    HPET_TN_INT_ENABLE, HPET_TN_INT_TYPE, HPET_TN_PERIODIC_CAP, HPET_TN_SIZE_CAP,
    HPET_TN_TYPE_PERIODIC, HPET_TN_VAL_SET,
};

pub use smp::{test_smp_and_timer_subsystem, SMP};
pub use power::{
    cpu_idle, cpu_relax, get_cpu_throttling, inw, outw, parse_dsdt_sleep_state,
    set_cpu_throttling, test_cpu_throttling, test_fadt_parser_and_power_structures,
    test_power_subsystem, test_sleep_and_reset_engine, CpuThrottleState, FadtInfo,
    GenericAddressStructure, PowerError, PowerManager, PowerState, SleepTypeConfig,
    CPU_THROTTLE, DSDT_SIGNATURE, FADT_SIGNATURE, GAS_ADDR_SPACE_PCI_CFG,
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

pub fn init_power_management() {
    power::init();
}
