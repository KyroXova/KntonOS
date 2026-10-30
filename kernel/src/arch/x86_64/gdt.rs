#![allow(dead_code)]

use super::tss::{self, TSS};

pub const KERNEL_CODE_SEL: u16 = 0x08;
pub const KERNEL_DATA_SEL: u16 = 0x10;
pub const USER_DATA_SEL: u16 = 0x18 | 3;
pub const USER_CODE_SEL: u16 = 0x20 | 3;
pub const TSS_SEL: u16 = 0x28;

#[repr(C, packed)]
struct GdtDescriptor {
    limit: u16,
    base: u64,
}

// GDT: 0=Null, 1=KernelCode(0x08), 2=KernelData(0x10), 3=UserData(0x18), 4=UserCode(0x20), 5/6=TSS(0x28).
static mut GDT: [u64; 7] = [0; 7];

pub fn init() {
    tss::init_tss();

    unsafe {
        // Entry 0: Null
        GDT[0] = 0;

        // Entry 1: Kernel Code 64-bit (0x08, Present=1, Ring 0, Exec/Read=1, LongMode=1)
        GDT[1] = 0x00209A0000000000;

        // Entry 2: Kernel Data (0x10, Present=1, Ring 0, Writable=1)
        GDT[2] = 0x0000920000000000;

        // Entry 3: User Data (0x18, Present=1, Ring 3, Writable=1)
        GDT[3] = 0x0000F20000000000;

        // Entry 4: User Code 64-bit (0x20, Present=1, Ring 3, Exec/Read=1, LongMode=1)
        GDT[4] = 0x0020FA0000000000;

        // Entries 5 and 6: 64-bit TSS descriptor (16 bytes)
        let tss_ptr = core::ptr::addr_of!(TSS) as u64;
        let tss_limit = (core::mem::size_of::<tss::TaskStateSegment>() - 1) as u64;

        // TSS low 8 bytes: limit, base 0..23, present, type 0x9 (64-bit TSS Available)
        let mut tss_low: u64 = tss_limit & 0xFFFF;
        tss_low |= (tss_ptr & 0x00FFFFFF) << 16;
        tss_low |= 0x89_u64 << 40;
        tss_low |= ((tss_limit >> 16) & 0x0F) << 48;
        tss_low |= ((tss_ptr >> 24) & 0xFF) << 56;

        // TSS high 8 bytes: base bits 32..63
        let tss_high: u64 = (tss_ptr >> 32) & 0xFFFFFFFF;

        GDT[5] = tss_low;
        GDT[6] = tss_high;

        let gdt_desc = GdtDescriptor {
            limit: (core::mem::size_of::<[u64; 7]>() - 1) as u16,
            base: core::ptr::addr_of!(GDT) as *const u64 as u64,
        };

        // Load GDT
        core::arch::asm!(
            "lgdt [{}]",
            in(reg) &gdt_desc,
            options(readonly, nostack, preserves_flags)
        );

        // Reload segment registers & perform far return to reload CS
        core::arch::asm!(
            "push {code_sel}",
            "lea {tmp}, [2f + rip]",
            "push {tmp}",
            "retfq",
            "2:",
            "mov ds, {data_sel:x}",
            "mov es, {data_sel:x}",
            "mov fs, {data_sel:x}",
            "mov gs, {data_sel:x}",
            "mov ss, {data_sel:x}",
            code_sel = in(reg) KERNEL_CODE_SEL as u64,
            data_sel = in(reg) KERNEL_DATA_SEL,
            tmp = out(reg) _,
            options(preserves_flags)
        );

        // Load Task Register (TSS)
        core::arch::asm!(
            "ltr ax",
            in("ax") TSS_SEL,
            options(nomem, nostack, preserves_flags)
        );
    }
}
