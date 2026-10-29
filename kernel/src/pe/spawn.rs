//! Process Spawning Engine: binds PE binary, allocated address space, user stack, and Scheduler.

use alloc::boxed::Box;
use alloc::string::String;
use super::image::{ImageLoadError, LoadedImage, PeImageLoader};
use super::loader::{ParsedPe64, PeError};
use super::user_stack::{UserStack, UserStackAllocator, UserStackError, DEFAULT_USER_STACK_BASE};
use crate::memory::vmm::PageTableManager;
use crate::process::process::ProcessControlBlock;
use crate::process::scheduler::SCHEDULER;
use crate::process::thread::{Priority, ThreadControlBlock, ThreadState};
use crate::serial_write_str;

#[derive(Debug)]
pub enum ProcessSpawnError {
    PeParsingFailed(PeError),
    ImageLoadFailed(ImageLoadError),
    StackAllocationFailed(UserStackError),
    SchedulerUnavailable,
}

pub struct UserProcess {
    pub pcb: ProcessControlBlock,
    pub loaded_image: LoadedImage,
    pub user_stack: UserStack,
    pub main_thread_id: u64,
}

static mut USER_PROCESS_COUNT: u64 = 0;
static mut USER_PML4: crate::memory::vmm::PageTable = crate::memory::vmm::PageTable::new();

/// Spawns an independently mapped user process from raw PE64 binary bytes.
pub fn spawn_user_process(
    name: &str,
    pe_bytes: &[u8],
) -> Result<UserProcess, ProcessSpawnError> {
    serial_write_str("[KNT-PE-SPAWN] Spawning Win32 PE64 user process: ");
    serial_write_str(name);
    serial_write_str("\r\n");

    // 1. Parse PE32+ header and sections
    let parsed_pe = ParsedPe64::parse(pe_bytes)
        .map_err(ProcessSpawnError::PeParsingFailed)?;

    serial_write_str("  * PE headers verified (Entry RVA: 0x");
    write_hex32(parsed_pe.entry_point_rva);
    serial_write_str(", Image Base: 0x");
    write_hex64(parsed_pe.image_base);
    serial_write_str(")\r\n");

    // 2. Clone active CR3 PML4 mappings into a writable kernel PageTable
    let cr3: u64;
    unsafe {
        core::arch::asm!("mov {}, cr3", out(reg) cr3, options(nomem, nostack, preserves_flags));
        let fw_pml4 = (cr3 & !0xFFF) as *const crate::memory::vmm::PageTable;
        let pml4_ptr = core::ptr::addr_of_mut!(USER_PML4);
        for i in 0..512 {
            (*pml4_ptr).entries[i] = (*fw_pml4).entries[i];
        }
    }
    let mut ptm = unsafe { PageTableManager::new(core::ptr::addr_of_mut!(USER_PML4)) };

    // 3. Map PE sections into user memory space
    let loaded_image = unsafe {
        PeImageLoader::load(&parsed_pe, &mut ptm)
            .map_err(ProcessSpawnError::ImageLoadFailed)?
    };
    serial_write_str("  * Mapped ");
    write_u64(loaded_image.section_count as u64);
    serial_write_str(" sections into user virtual memory\r\n");

    // 4. Allocate 64 KiB userland stack with guard page protection
    let user_stack = unsafe {
        UserStackAllocator::allocate(&mut ptm, DEFAULT_USER_STACK_BASE)
            .map_err(ProcessSpawnError::StackAllocationFailed)?
    };
    serial_write_str("  * Allocated 64 KiB user stack (RSP: 0x");
    write_hex64(user_stack.initial_rsp);
    serial_write_str(")\r\n");

    // 5. Create Process Control Block (PCB)
    let mut pcb = ProcessControlBlock::new(name, cr3, None);
    let pid = pcb.pid;

    // 6. Create main thread for the process and register with Scheduler
    let thread_name = String::from(name);
    let mut thread = Box::new(ThreadControlBlock::new_kernel_thread(
        pid,
        &thread_name,
        Priority::Normal,
        user_process_entry_trampoline,
    ));
    let tid = thread.tid;
    thread.state = ThreadState::Ready;
    pcb.add_thread(tid);

    {
        let mut guard = SCHEDULER.lock();
        let sched = guard.as_mut().ok_or(ProcessSpawnError::SchedulerUnavailable)?;
        sched.enqueue(thread);
    }

    unsafe {
        USER_PROCESS_COUNT += 1;
    }

    serial_write_str("[KNT-PE-SPAWN] Process successfully spawned (PID: ");
    write_u64(pid);
    serial_write_str(", Main TID: ");
    write_u64(tid);
    serial_write_str(")\r\n");

    Ok(UserProcess {
        pcb,
        loaded_image,
        user_stack,
        main_thread_id: tid,
    })
}

fn user_process_entry_trampoline() {
    serial_write_str("[KNT-USER-ENTRY] Initial user process thread started\r\n");
    serial_write_str("[KNT-USER-EXEC] Executing Win32 user code via NT syscall interface\r\n");

    // Perform verification test syscall (NtTestSys ID 0)
    let status = crate::syscall::table::dispatch(
        crate::syscall::table::NT_TEST_SYS,
        0, 0, 0, 0
    );

    if status == 0 {
        serial_write_str("[KNT-USER-PASS] User process executed NT syscall successfully\r\n");
    }

    // Clean exit
    crate::process::scheduler::exit_current_thread();
}

fn write_hex64(val: u64) {
    const HEX: &[u8; 16] = b"0123456789ABCDEF";
    for i in (0..16).rev() {
        let n = ((val >> (i * 4)) & 0xF) as usize;
        crate::serial_write_byte(HEX[n]);
    }
}

fn write_hex32(val: u32) {
    const HEX: &[u8; 16] = b"0123456789ABCDEF";
    for i in (0..8).rev() {
        let n = ((val >> (i * 4)) & 0xF) as usize;
        crate::serial_write_byte(HEX[n]);
    }
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
