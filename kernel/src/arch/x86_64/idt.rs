#![allow(dead_code)]

use super::gdt::KERNEL_CODE_SEL;
use super::tss::DOUBLE_FAULT_IST_INDEX;
use crate::{serial_write_byte, serial_write_str};

#[repr(C, packed)]
#[derive(Clone, Copy)]
pub struct IdtEntry {
    offset_low: u16,
    selector: u16,
    ist: u8,
    type_attr: u8,
    offset_mid: u16,
    offset_high: u32,
    reserved: u32,
}

impl IdtEntry {
    pub const fn missing() -> Self {
        Self {
            offset_low: 0,
            selector: 0,
            ist: 0,
            type_attr: 0,
            offset_mid: 0,
            offset_high: 0,
            reserved: 0,
        }
    }

    pub fn set_handler(&mut self, handler: usize, ist_index: u8) {
        self.offset_low = (handler & 0xFFFF) as u16;
        self.selector = KERNEL_CODE_SEL;
        self.ist = ist_index & 0x07;
        self.type_attr = 0x8E; // Present=1, DPL=0, Type=0xE (64-bit Interrupt Gate)
        self.offset_mid = ((handler >> 16) & 0xFFFF) as u16;
        self.offset_high = ((handler >> 32) & 0xFFFFFFFF) as u32;
        self.reserved = 0;
    }
}

#[repr(C, packed)]
pub struct IdtDescriptor {
    limit: u16,
    base: u64,
}

#[repr(C)]
pub struct ExceptionContext {
    pub r15: u64,
    pub r14: u64,
    pub r13: u64,
    pub r12: u64,
    pub r11: u64,
    pub r10: u64,
    pub r9: u64,
    pub r8: u64,
    pub rbp: u64,
    pub rdi: u64,
    pub rsi: u64,
    pub rdx: u64,
    pub rcx: u64,
    pub rbx: u64,
    pub rax: u64,
    pub vector: u64,
    pub error_code: u64,
    pub rip: u64,
    pub cs: u64,
    pub rflags: u64,
    pub rsp: u64,
    pub ss: u64,
}

static mut IDT: [IdtEntry; 256] = [IdtEntry::missing(); 256];

const EXCEPTION_NAMES: [&str; 32] = [
    "#DE: Divide Error",
    "#DB: Debug Exception",
    "#NMI: Non-Maskable Interrupt",
    "#BP: Breakpoint",
    "#OF: Overflow",
    "#BR: Bound Range Exceeded",
    "#UD: Invalid Opcode",
    "#NM: Device Not Available",
    "#DF: Double Fault",
    "Coprocessor Segment Overrun",
    "#TS: Invalid TSS",
    "#NP: Segment Not Present",
    "#SS: Stack-Segment Fault",
    "#GP: General Protection Fault",
    "#PF: Page Fault",
    "Reserved",
    "#MF: x87 FPU Floating-Point Error",
    "#AC: Alignment Check",
    "#MC: Machine Check",
    "#XM: SIMD Floating-Point Exception",
    "#VE: Virtualization Exception",
    "#CP: Control Protection Exception",
    "Reserved",
    "Reserved",
    "Reserved",
    "Reserved",
    "Reserved",
    "Reserved",
    "Reserved",
    "Reserved",
    "#SX: Security Exception",
    "Reserved",
];

fn write_hex64(val: u64) {
    const HEX_CHARS: &[u8; 16] = b"0123456789ABCDEF";
    serial_write_str("0x");
    for i in (0..16).rev() {
        let nibble = ((val >> (i * 4)) & 0xF) as usize;
        serial_write_byte(HEX_CHARS[nibble]);
    }
}

#[no_mangle]
pub extern "C" fn exception_dispatch(ctx: *const ExceptionContext) -> ! {
    let frame = unsafe { &*ctx };

    serial_write_str("\r\n=======================================================\r\n");
    serial_write_str("[KNT-EXCEPTION] CPU Exception Encountered: ");
    if (frame.vector as usize) < EXCEPTION_NAMES.len() {
        serial_write_str(EXCEPTION_NAMES[frame.vector as usize]);
    } else {
        serial_write_str("Interrupt");
    }
    serial_write_str("\r\n");

    if frame.vector == 14 {
        // Page fault: read CR2
        let cr2: u64;
        unsafe {
            core::arch::asm!("mov {}, cr2", out(reg) cr2, options(nomem, nostack, preserves_flags));
        }
        serial_write_str("  Faulting Linear Address (CR2): ");
        write_hex64(cr2);
        serial_write_str("\r\n");
    }

    serial_write_str("  Vector:     "); write_hex64(frame.vector); serial_write_str("\r\n");
    serial_write_str("  Error Code: "); write_hex64(frame.error_code); serial_write_str("\r\n");
    serial_write_str("  RIP:        "); write_hex64(frame.rip); serial_write_str("\r\n");
    serial_write_str("  CS:         "); write_hex64(frame.cs); serial_write_str("\r\n");
    serial_write_str("  RFLAGS:     "); write_hex64(frame.rflags); serial_write_str("\r\n");
    serial_write_str("  RSP:        "); write_hex64(frame.rsp); serial_write_str("\r\n");
    serial_write_str("  SS:         "); write_hex64(frame.ss); serial_write_str("\r\n");
    serial_write_str("=======================================================\r\n");

    loop {
        unsafe {
            core::arch::asm!("hlt", options(nomem, nostack, preserves_flags));
        }
    }
}

// Macro to generate raw exception entry stubs
macro_rules! exception_stub_no_err {
    ($name:ident, $vec:expr) => {
        #[unsafe(naked)]
        unsafe extern "C" fn $name() {
            core::arch::naked_asm!(
                "push 0",               // Dummy error code
                concat!("push ", $vec), // Vector number
                "push rax",
                "push rbx",
                "push rcx",
                "push rdx",
                "push rsi",
                "push rdi",
                "push rbp",
                "push r8",
                "push r9",
                "push r10",
                "push r11",
                "push r12",
                "push r13",
                "push r14",
                "push r15",
                "mov rcx, rsp",         // arg0 for Windows/SysV calling convention
                "mov rdi, rsp",
                "call {dispatch}",
                "ud2",
                dispatch = sym exception_dispatch,
            );
        }
    };
}

macro_rules! exception_stub_err {
    ($name:ident, $vec:expr) => {
        #[unsafe(naked)]
        unsafe extern "C" fn $name() {
            core::arch::naked_asm!(
                concat!("push ", $vec), // Vector number (error code already pushed by CPU)
                "push rax",
                "push rbx",
                "push rcx",
                "push rdx",
                "push rsi",
                "push rdi",
                "push rbp",
                "push r8",
                "push r9",
                "push r10",
                "push r11",
                "push r12",
                "push r13",
                "push r14",
                "push r15",
                "mov rcx, rsp",
                "mov rdi, rsp",
                "call {dispatch}",
                "ud2",
                dispatch = sym exception_dispatch,
            );
        }
    };
}

exception_stub_no_err!(isr0, 0);
exception_stub_no_err!(isr1, 1);
exception_stub_no_err!(isr2, 2);
exception_stub_no_err!(isr3, 3);
exception_stub_no_err!(isr4, 4);
exception_stub_no_err!(isr5, 5);
exception_stub_no_err!(isr6, 6);
exception_stub_no_err!(isr7, 7);
exception_stub_err!(isr8, 8);   // Double Fault (#DF)
exception_stub_no_err!(isr9, 9);
exception_stub_err!(isr10, 10); // Invalid TSS (#TS)
exception_stub_err!(isr11, 11); // Segment Not Present (#NP)
exception_stub_err!(isr12, 12); // Stack-Segment Fault (#SS)
exception_stub_err!(isr13, 13); // General Protection Fault (#GP)
exception_stub_err!(isr14, 14); // Page Fault (#PF)
exception_stub_no_err!(isr15, 15);
exception_stub_no_err!(isr16, 16);
exception_stub_err!(isr17, 17); // Alignment Check (#AC)
exception_stub_no_err!(isr18, 18);
exception_stub_no_err!(isr19, 19);
exception_stub_no_err!(isr20, 20);
exception_stub_err!(isr21, 21); // Control Protection (#CP)
exception_stub_no_err!(isr22, 22);
exception_stub_no_err!(isr23, 23);
exception_stub_no_err!(isr24, 24);
exception_stub_no_err!(isr25, 25);
exception_stub_no_err!(isr26, 26);
exception_stub_no_err!(isr27, 27);
exception_stub_no_err!(isr28, 28);
exception_stub_no_err!(isr29, 29);
exception_stub_err!(isr30, 30); // Security Exception (#SX)
exception_stub_no_err!(isr31, 31);

pub fn init() {
    unsafe {
        IDT[0].set_handler(isr0 as *const () as usize, 0);
        IDT[1].set_handler(isr1 as *const () as usize, 0);
        IDT[2].set_handler(isr2 as *const () as usize, 0);
        IDT[3].set_handler(isr3 as *const () as usize, 0);
        IDT[4].set_handler(isr4 as *const () as usize, 0);
        IDT[5].set_handler(isr5 as *const () as usize, 0);
        IDT[6].set_handler(isr6 as *const () as usize, 0);
        IDT[7].set_handler(isr7 as *const () as usize, 0);
        // Double Fault (#DF) uses dedicated IST[0] stack
        IDT[8].set_handler(isr8 as *const () as usize, DOUBLE_FAULT_IST_INDEX);
        IDT[9].set_handler(isr9 as *const () as usize, 0);
        IDT[10].set_handler(isr10 as *const () as usize, 0);
        IDT[11].set_handler(isr11 as *const () as usize, 0);
        IDT[12].set_handler(isr12 as *const () as usize, 0);
        IDT[13].set_handler(isr13 as *const () as usize, 0);
        IDT[14].set_handler(isr14 as *const () as usize, 0);
        IDT[15].set_handler(isr15 as *const () as usize, 0);
        IDT[16].set_handler(isr16 as *const () as usize, 0);
        IDT[17].set_handler(isr17 as *const () as usize, 0);
        IDT[18].set_handler(isr18 as *const () as usize, 0);
        IDT[19].set_handler(isr19 as *const () as usize, 0);
        IDT[20].set_handler(isr20 as *const () as usize, 0);
        IDT[21].set_handler(isr21 as *const () as usize, 0);
        IDT[22].set_handler(isr22 as *const () as usize, 0);
        IDT[23].set_handler(isr23 as *const () as usize, 0);
        IDT[24].set_handler(isr24 as *const () as usize, 0);
        IDT[25].set_handler(isr25 as *const () as usize, 0);
        IDT[26].set_handler(isr26 as *const () as usize, 0);
        IDT[27].set_handler(isr27 as *const () as usize, 0);
        IDT[28].set_handler(isr28 as *const () as usize, 0);
        IDT[29].set_handler(isr29 as *const () as usize, 0);
        IDT[30].set_handler(isr30 as *const () as usize, 0);
        IDT[31].set_handler(isr31 as *const () as usize, 0);

        let idt_desc = IdtDescriptor {
            limit: (core::mem::size_of::<[IdtEntry; 256]>() - 1) as u16,
            base: core::ptr::addr_of!(IDT) as *const IdtEntry as u64,
        };

        core::arch::asm!(
            "lidt [{}]",
            in(reg) &idt_desc,
            options(readonly, nostack, preserves_flags)
        );
    }
}
