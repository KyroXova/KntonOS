#![allow(dead_code)]

#[repr(C, packed)]
#[derive(Clone, Copy)]
pub struct TaskStateSegment {
    pub reserved0: u32,
    pub rsp0: u64,
    pub rsp1: u64,
    pub rsp2: u64,
    pub reserved1: u64,
    pub ist: [u64; 7],
    pub reserved2: u64,
    pub reserved3: u16,
    pub iomap_base: u16,
}

impl TaskStateSegment {
    pub const fn new() -> Self {
        Self {
            reserved0: 0,
            rsp0: 0,
            rsp1: 0,
            rsp2: 0,
            reserved1: 0,
            ist: [0; 7],
            reserved2: 0,
            reserved3: 0,
            iomap_base: core::mem::size_of::<Self>() as u16,
        }
    }
}

pub const DOUBLE_FAULT_IST_INDEX: u8 = 1; // 1-based IST index in x86-64 IDT

// Statically allocate kernel privilege stack (RSP0) and double fault stack (IST1)
const STACK_SIZE: usize = 16384; // 16 KiB
static mut KERNEL_RSP0_STACK: [u8; STACK_SIZE] = [0; STACK_SIZE];
static mut DOUBLE_FAULT_STACK: [u8; STACK_SIZE] = [0; STACK_SIZE];

pub static mut TSS: TaskStateSegment = TaskStateSegment::new();

pub fn init_tss() {
    unsafe {
        let rsp0_end = (core::ptr::addr_of!(KERNEL_RSP0_STACK) as *const u8 as u64) + STACK_SIZE as u64;
        let df_end = (core::ptr::addr_of!(DOUBLE_FAULT_STACK) as *const u8 as u64) + STACK_SIZE as u64;

        TSS.rsp0 = rsp0_end;
        TSS.ist[(DOUBLE_FAULT_IST_INDEX - 1) as usize] = df_end;
    }
}
