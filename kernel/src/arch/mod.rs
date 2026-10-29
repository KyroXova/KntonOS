pub mod x86_64;

pub fn init() {
    #[cfg(target_arch = "x86_64")]
    x86_64::init();
}

pub fn init_smp() {
    #[cfg(target_arch = "x86_64")]
    x86_64::init_smp();
}

pub fn test_smp_and_timer_subsystem() -> bool {
    #[cfg(target_arch = "x86_64")]
    {
        x86_64::test_smp_and_timer_subsystem()
    }
    #[cfg(not(target_arch = "x86_64"))]
    true
}
