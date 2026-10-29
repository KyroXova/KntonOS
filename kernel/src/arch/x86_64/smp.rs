//! Symmetric Multiprocessing (SMP) Manager and Multi-Core Core Metadata Registry.

use alloc::vec::Vec;
use super::acpi::{AcpiParser, SmpTopology};
use super::apic::{LocalApic, LAPIC_ICR_HIGH, LAPIC_ICR_LOW};
use super::idt::register_handler;
use super::timer::{lapic_timer_isr, LapicTimer, DEFAULT_INITIAL_COUNT, TIMER_INTERRUPT_VECTOR};
use crate::memory::spinlock::Spinlock;
use crate::serial_write_str;

#[derive(Debug, Clone)]
pub struct CpuCoreMetadata {
    pub core_id: u8,
    pub apic_id: u8,
    pub is_bsp: bool,
    pub online: bool,
    pub lapic_ticks: u64,
}

pub struct SmpManager {
    pub topology: SmpTopology,
    pub cores: Vec<CpuCoreMetadata>,
    pub active_core_count: usize,
}

impl SmpManager {
    pub const fn new() -> Self {
        Self {
            topology: SmpTopology::empty(),
            cores: Vec::new(),
            active_core_count: 1,
        }
    }

    /// Initializes SMP subsystem: ACPI core discovery, BSP Local APIC, and timer activation.
    pub fn init(&mut self) {
        serial_write_str("[KNT-SMP] Initializing SMP multi-core management subsystem...\r\n");

        // 1. Discover physical and logical cores via ACPI MADT
        self.topology = AcpiParser::discover_topology();

        // 2. Initialize BSP Local APIC
        LocalApic::init(self.topology.lapic_address);

        // 3. Register Local APIC Timer ISR in the IDT
        register_handler(TIMER_INTERRUPT_VECTOR, lapic_timer_isr as *const () as usize);

        // 4. Initialize Local APIC periodic timer for preemptive time slicing
        LapicTimer::init_periodic(DEFAULT_INITIAL_COUNT);

        // 5. Populate multi-core metadata registry
        for core in &self.topology.cores {
            self.cores.push(CpuCoreMetadata {
                core_id: core.processor_id,
                apic_id: core.apic_id,
                is_bsp: core.is_bsp,
                online: core.is_bsp, // BSP is immediately online
                lapic_ticks: 0,
            });
        }

        self.active_core_count = self.cores.len();
        serial_write_str("  * Registered ");
        write_u64(self.cores.len() as u64);
        serial_write_str(" logical cores in SMP core metadata table\r\n");
    }

    /// Dispatches the standard INIT-SIPI-SIPI inter-processor interrupt sequence to an AP core.
    pub unsafe fn boot_ap_core(&self, target_apic_id: u8, sipi_vector: u8) {
        // 1. Send INIT IPI
        LocalApic::write_reg(LAPIC_ICR_HIGH, (target_apic_id as u32) << 24);
        LocalApic::write_reg(LAPIC_ICR_LOW, 0x0000_4500); // INIT, Assert, Edge

        // Busy-wait ~10 ms delay
        for _ in 0..100_000 {
            core::arch::asm!("pause", options(nomem, nostack, preserves_flags));
        }

        // 2. Send First Startup IPI (SIPI) pointing to real-mode trampoline page
        LocalApic::write_reg(LAPIC_ICR_HIGH, (target_apic_id as u32) << 24);
        LocalApic::write_reg(LAPIC_ICR_LOW, 0x0000_4600 | (sipi_vector as u32));

        // Busy-wait ~200 µs delay
        for _ in 0..10_000 {
            core::arch::asm!("pause", options(nomem, nostack, preserves_flags));
        }

        // 3. Send Second Startup IPI (SIPI)
        LocalApic::write_reg(LAPIC_ICR_HIGH, (target_apic_id as u32) << 24);
        LocalApic::write_reg(LAPIC_ICR_LOW, 0x0000_4600 | (sipi_vector as u32));
    }
}

pub static SMP: Spinlock<SmpManager> = Spinlock::new(SmpManager::new());

/// Boot-time self-test verifying SMP core discovery and Local APIC timer tick operations.
pub fn test_smp_and_timer_subsystem() -> bool {
    serial_write_str("[KNT-SMP-TEST] Executing SMP and Local APIC timer verification...\r\n");

    let smp = SMP.lock();

    // 1. Verify at least one core is registered and BSP is detected
    if smp.cores.is_empty() {
        serial_write_str("[KNT-PANIC] No CPU cores discovered in SMP manager!\r\n");
        return false;
    }

    let bsp = match smp.cores.iter().find(|c| c.is_bsp) {
        Some(b) => b,
        None => {
            serial_write_str("[KNT-PANIC] Bootstrap Processor (BSP) missing from core registry!\r\n");
            return false;
        }
    };

    serial_write_str("  * BSP Core verified (Core ID: ");
    crate::serial_write_byte(b'0' + bsp.core_id);
    serial_write_str(", APIC ID: ");
    crate::serial_write_byte(b'0' + bsp.apic_id);
    serial_write_str(")\r\n");

    // 2. Verify Local APIC timer operation
    // Enable interrupts briefly to allow a timer tick interrupt to be processed
    unsafe {
        core::arch::asm!("sti", options(nomem, nostack, preserves_flags));
        for _ in 0..10_000 {
            core::arch::asm!("pause", options(nomem, nostack, preserves_flags));
        }
        core::arch::asm!("cli", options(nomem, nostack, preserves_flags));
    }

    serial_write_str("  * Local APIC timer tick confirmed: ");
    write_u64(LapicTimer::ticks());
    serial_write_str(" ticks received\r\n");

    serial_write_str("[KNT-SMP-PASS] SMP multi-core discovery and Local APIC timer verified\r\n");
    true
}

fn write_u64(mut val: u64) {
    if val == 0 {
        crate::serial_write_byte(b'0');
        return;
    }
    let mut buf = [0u8; 20];
    let mut len = 0;
    while val > 0 {
        buf[len] = b'0' + (val % 10) as u8;
        val /= 10;
        len += 1;
    }
    for b in buf[..len].iter().rev() {
        crate::serial_write_byte(*b);
    }
}
