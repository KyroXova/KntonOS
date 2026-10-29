//! Local APIC Calibrated Periodic Timer for Preemptive Scheduling Time Slicing.

use super::apic::{LocalApic, LAPIC_LVT_TIMER, LAPIC_TIMER_DIV, LAPIC_TIMER_INIT_COUNT};
use core::sync::atomic::{AtomicU64, Ordering};
use crate::serial_write_str;

pub const TIMER_INTERRUPT_VECTOR: u8 = 0x20; // Vector 32 (first user IRQ vector above exceptions)
pub const TIMER_PERIODIC_MODE: u32 = 1 << 17; // Bit 17 = Periodic mode
pub const TIMER_DIVIDE_BY_16: u32 = 0x03;

/// Default initial count for ~100 Hz (10 ms) scheduler quantum tick rate in QEMU/KVM.
pub const DEFAULT_INITIAL_COUNT: u32 = 1_000_000;

static TIMER_TICKS: AtomicU64 = AtomicU64::new(0);

pub struct LapicTimer;

impl LapicTimer {
    /// Configures the Local APIC timer in periodic mode with specified initial count and divisor.
    pub fn init_periodic(initial_count: u32) {
        serial_write_str("[KNT-TIMER] Initializing Local APIC timer in periodic mode...\r\n");

        unsafe {
            // 1. Set divide configuration register (Divide by 16)
            LocalApic::write_reg(LAPIC_TIMER_DIV, TIMER_DIVIDE_BY_16);

            // 2. Set LVT Timer Register: Vector 0x20 | Periodic Mode (unmasked)
            LocalApic::write_reg(
                LAPIC_LVT_TIMER,
                (TIMER_INTERRUPT_VECTOR as u32) | TIMER_PERIODIC_MODE,
            );

            // 3. Set Initial Count Register to start periodic countdown
            LocalApic::write_reg(LAPIC_TIMER_INIT_COUNT, initial_count);
        }

        serial_write_str("  * Local APIC timer active (Vector: 0x20, Mode: Periodic, Div: 16)\r\n");
    }

    /// Disables the Local APIC timer.
    pub fn disable() {
        unsafe {
            LocalApic::write_reg(LAPIC_TIMER_INIT_COUNT, 0);
            LocalApic::write_reg(LAPIC_LVT_TIMER, 1 << 16); // Mask timer
        }
    }

    /// Returns the total number of timer interrupts handled.
    pub fn ticks() -> u64 {
        TIMER_TICKS.load(Ordering::Relaxed)
    }

    /// Increments timer tick counter and triggers scheduler quantum accounting.
    pub fn on_tick() {
        let count = TIMER_TICKS.fetch_add(1, Ordering::Relaxed);
        if count == 0 {
            serial_write_str("[KNT-TIMER-TICK] First Local APIC timer interrupt received\r\n");
        }
    }
}

#[no_mangle]
pub extern "win64" fn handle_lapic_timer_tick() {
    LapicTimer::on_tick();
    LocalApic::eoi();
    crate::process::timer_tick();
}

/// Raw assembly interrupt handler stub for Vector 0x20 (Local APIC Timer).
#[unsafe(naked)]
pub unsafe extern "C" fn lapic_timer_isr() {
    core::arch::naked_asm!(
        // Save scratch registers
        "push rax",
        "push rcx",
        "push rdx",
        "push rsi",
        "push rdi",
        "push r8",
        "push r9",
        "push r10",
        "push r11",
        // Allocate 32-byte Win64 shadow space on 16-byte aligned stack (112 + 32 = 144 bytes)
        "sub rsp, 32",
        "call {handler}",
        "add rsp, 32",
        // Restore scratch registers
        "pop r11",
        "pop r10",
        "pop r9",
        "pop r8",
        "pop rdi",
        "pop rsi",
        "pop rdx",
        "pop rcx",
        "pop rax",
        "iretq",
        handler = sym handle_lapic_timer_tick,
    );
}
