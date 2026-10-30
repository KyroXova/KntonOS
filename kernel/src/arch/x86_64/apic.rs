//! Local APIC (Advanced Programmable Interrupt Controller) driver.

use super::syscall::{rdmsr, wrmsr};
use crate::memory::pmm::PhysAddr;
use crate::memory::vmm::{PageTableFlags, PageTableManager, VirtAddr};
use crate::memory::PMM;
use crate::serial_write_str;

pub const IA32_APIC_BASE_MSR: u32 = 0x1B;
pub const APIC_BASE_MSR_ENABLE: u64 = 1 << 11;
pub const APIC_BASE_MSR_BSP: u64 = 1 << 8;

// Local APIC Register Offsets
pub const LAPIC_ID: u32 = 0x0020;
pub const LAPIC_VERSION: u32 = 0x0030;
pub const LAPIC_TPR: u32 = 0x0080;
pub const LAPIC_EOI: u32 = 0x00B0;
pub const LAPIC_LDR: u32 = 0x00D0;
pub const LAPIC_DFR: u32 = 0x00E0;
pub const LAPIC_SPURIOUS: u32 = 0x00F0;
pub const LAPIC_ICR_LOW: u32 = 0x0300;
pub const LAPIC_ICR_HIGH: u32 = 0x0310;
pub const LAPIC_LVT_TIMER: u32 = 0x0320;
pub const LAPIC_TIMER_INIT_COUNT: u32 = 0x0380;
pub const LAPIC_TIMER_CURR_COUNT: u32 = 0x0390;
pub const LAPIC_TIMER_DIV: u32 = 0x03E0;

pub const SPURIOUS_VECTOR: u32 = 0xFF;
pub const APIC_SW_ENABLE: u32 = 1 << 8;

pub static mut LAPIC_BASE_ADDR: usize = 0xFEE0_0000;

pub struct LocalApic;

impl LocalApic {
    /// Reads a 32-bit register from the memory-mapped Local APIC register space.
    #[inline(always)]
    pub unsafe fn read_reg(offset: u32) -> u32 {
        let addr = LAPIC_BASE_ADDR + (offset as usize);
        core::ptr::read_volatile(addr as *const u32)
    }

    /// Writes a 32-bit value to a register in the Local APIC space.
    #[inline(always)]
    pub unsafe fn write_reg(offset: u32, val: u32) {
        let addr = LAPIC_BASE_ADDR + (offset as usize);
        core::ptr::write_volatile(addr as *mut u32, val);
    }

    /// Acknowledges an interrupt by writing zero to the End of Interrupt (EOI) register.
    #[inline(always)]
    pub fn eoi() {
        unsafe {
            Self::write_reg(LAPIC_EOI, 0);
        }
    }

    /// Returns the APIC ID of the calling logical processor.
    pub fn id() -> u8 {
        unsafe {
            ((Self::read_reg(LAPIC_ID) >> 24) & 0xFF) as u8
        }
    }

    /// Returns true if this logical processor is the Bootstrap Processor (BSP).
    pub fn is_bsp() -> bool {
        unsafe {
            (rdmsr(IA32_APIC_BASE_MSR) & APIC_BASE_MSR_BSP) != 0
        }
    }

    /// Ensures the physical MMIO page for LAPIC (default 0xFEE0_0000) is mapped in active page tables.
    pub fn map_lapic_mmio(base_addr: u64) {
        unsafe {
            // Temporarily clear CR0.WP (bit 16) to allow writing intermediate paging structures
            let cr0: u64;
            core::arch::asm!("mov {}, cr0", out(reg) cr0, options(nomem, nostack, preserves_flags));
            core::arch::asm!("mov cr0, {}", in(reg) (cr0 & !(1 << 16)), options(nomem, nostack, preserves_flags));

            let cr3: u64;
            core::arch::asm!("mov {}, cr3", out(reg) cr3, options(nomem, nostack, preserves_flags));
            let root_pml4 = (cr3 & !0xFFF) as *mut crate::memory::vmm::PageTable;

            let mut ptm = PageTableManager::new(root_pml4);
            let mut pmm = PMM.lock();

            let virt = VirtAddr::new(base_addr as usize);
            let phys = PhysAddr::new(base_addr as usize);
            let flags = PageTableFlags::PRESENT
                | PageTableFlags::WRITABLE
                | PageTableFlags::WRITE_THROUGH
                | PageTableFlags::NO_CACHE;

            if ptm.translate(virt).is_none() {
                let _ = ptm.map_page(virt, phys, flags, &mut *pmm);
            }

            core::arch::asm!("mov cr0, {}", in(reg) cr0, options(nomem, nostack, preserves_flags));
        }
    }

    /// Initializes and enables the Local APIC on the current processor core.
    pub fn init(base_addr: u64) {
        Self::map_lapic_mmio(base_addr);

        unsafe {
            LAPIC_BASE_ADDR = base_addr as usize;

            // 1. Enable Local APIC globally via IA32_APIC_BASE MSR
            let apic_base = rdmsr(IA32_APIC_BASE_MSR);
            wrmsr(IA32_APIC_BASE_MSR, apic_base | APIC_BASE_MSR_ENABLE);

            // 2. Set Spurious Interrupt Vector register (Vector 0xFF | APIC Software Enable)
            Self::write_reg(LAPIC_SPURIOUS, SPURIOUS_VECTOR | APIC_SW_ENABLE);

            // 3. Clear Task Priority Register (TPR = 0 accepts all interrupt priorities)
            Self::write_reg(LAPIC_TPR, 0);

            // 4. Send initial EOI to flush any pending stale interrupts
            Self::eoi();
        }

        serial_write_str("[KNT-LAPIC] Local APIC enabled on core (ID: ");
        crate::serial_write_byte(b'0' + (Self::id() % 10));
        serial_write_str(", Spurious Vector: 0xFF)\r\n");
    }
}
