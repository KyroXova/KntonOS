pub mod x86_64;

pub fn init() {
    #[cfg(target_arch = "x86_64")]
    x86_64::init();
}

pub fn init_smp() {
    #[cfg(target_arch = "x86_64")]
    x86_64::init_smp();
}

pub fn init_power_management() {
    #[cfg(target_arch = "x86_64")]
    x86_64::init_power_management();
}

pub fn cpu_idle() {
    #[cfg(target_arch = "x86_64")]
    x86_64::cpu_idle();
}

pub fn cpu_relax() {
    #[cfg(target_arch = "x86_64")]
    x86_64::cpu_relax();
}

pub fn set_cpu_throttling(duty_percent: u8) {
    #[cfg(target_arch = "x86_64")]
    x86_64::set_cpu_throttling(duty_percent);
}

pub fn get_cpu_throttling() -> u8 {
    #[cfg(target_arch = "x86_64")]
    {
        x86_64::get_cpu_throttling()
    }
    #[cfg(not(target_arch = "x86_64"))]
    0
}

pub fn test_cpu_throttling() -> bool {
    #[cfg(target_arch = "x86_64")]
    {
        x86_64::test_cpu_throttling()
    }
    #[cfg(not(target_arch = "x86_64"))]
    true
}

pub fn test_power_subsystem() -> bool {
    #[cfg(target_arch = "x86_64")]
    {
        x86_64::test_power_subsystem()
    }
    #[cfg(not(target_arch = "x86_64"))]
    true
}

pub fn test_hpet_subsystem() -> bool {
    #[cfg(target_arch = "x86_64")]
    {
        x86_64::test_hpet_subsystem()
    }
    #[cfg(not(target_arch = "x86_64"))]
    true
}

pub fn test_smp_and_timer_subsystem() -> bool {
    #[cfg(target_arch = "x86_64")]
    {
        x86_64::test_smp_and_timer_subsystem()
    }
    #[cfg(not(target_arch = "x86_64"))]
    true
}

pub fn test_debug_trap_subsystem() -> bool {
    #[cfg(target_arch = "x86_64")]
    {
        x86_64::test_debug_trap_and_watchpoints()
    }
    #[cfg(not(target_arch = "x86_64"))]
    true
}
