//! Kernel Memory Management Subsystem for KntonOS (`kntonk`).
//! Subsystems:

pub mod heap;
pub mod pmm;
pub mod spinlock;
pub mod vmm;
pub mod demand_paging;
pub mod cow;

pub use heap::{KntonAllocator, DEFAULT_HEAP_SIZE};
pub use pmm::{BitmapFrameAllocator, FrameAllocator, PhysAddr, PhysFrame, PAGE_SIZE};
pub use spinlock::Spinlock;
pub use vmm::{PageTable, PageTableEntry, PageTableFlags, PageTableManager, VirtAddr, VmmError};
pub use demand_paging::{
    handle_page_fault, test_cow_and_demand_paging, test_demand_paging_engine, test_vma_and_error_codes, AddressSpace,
    PageFaultErrorCode, PageFaultResolution, VmArea, VmFlags,
};
pub use cow::{
    fork_address_space, handle_cow_fault, test_cow_bitfields, test_cow_fork_and_reference_counter,
    FrameRefCounter, FRAME_REF_COUNTER, MAX_TRACKED_PHYS_FRAMES, PTE_AVL_MASK, PTE_COW_BIT,
};

/// Global Physical Memory Manager singleton.
pub static PMM: Spinlock<BitmapFrameAllocator> = Spinlock::new(BitmapFrameAllocator::new());

/// Global Kernel Heap Allocator singleton.
#[global_allocator]
pub static ALLOCATOR: KntonAllocator = KntonAllocator::new();

/// Initializes the foundational memory subsystems.
pub fn init() {
    // 1. Initialize PMM tracking 32,768 physical frames (128 MiB baseline physical space)
    PMM.lock().init(32768);
    crate::serial_write_str("[KNT-MEM-PMM] Physical memory frame allocator initialized (128 MiB pool)\r\n");

    // 2. Initialize Kernel Global Heap (2 MiB default resident heap)
    ALLOCATOR.init_default();
    crate::serial_write_str("[KNT-MEM-HEAP] Kernel GlobalAlloc heap initialized (2 MiB pool)\r\n");
}

/// Self-test suite verifying PMM, VMM, and dynamic heap allocation in no_std.
pub fn test_memory_subsystem() -> bool {
    crate::serial_write_str("[KNT-MEM-TEST] Executing memory subsystem self-test suite...\r\n");

    // --- TEST 1: Physical Memory Manager (PMM) ---
    {
        let mut pmm = PMM.lock();
        let free_before = pmm.free_frames();

        let frame1 = pmm.allocate_frame().expect("PMM frame 1 allocation failed");
        let frame2 = pmm.allocate_frame().expect("PMM frame 2 allocation failed");

        if frame1 == frame2 {
            crate::serial_write_str("[KNT-PANIC] PMM allocated identical duplicate frames!\r\n");
            return false;
        }

        // Deallocate frame1 and reallocate to test recycling
        pmm.deallocate_frame(frame1);
        let frame1_recycled = pmm.allocate_frame().expect("PMM frame recycling failed");

        if frame1_recycled != frame1 {
            crate::serial_write_str("[KNT-PANIC] PMM did not recycle free frame accurately!\r\n");
            return false;
        }

        pmm.deallocate_frame(frame1_recycled);
        pmm.deallocate_frame(frame2);

        if pmm.free_frames() != free_before {
            crate::serial_write_str("[KNT-PANIC] PMM frame counter mismatch after deallocation!\r\n");
            return false;
        }

        crate::serial_write_str("[KNT-MEM-PMM-PASS] PMM frame allocation, recycling, and tracking verified\r\n");
    }

    // --- TEST 2: Virtual Memory Manager (VMM) ---
    unsafe {
        static mut TEST_PML4: PageTable = PageTable::new();
        let pml4_ptr = core::ptr::addr_of_mut!(TEST_PML4);
        (*pml4_ptr).zero();

        let mut vmm = PageTableManager::new(pml4_ptr);
        let mut pmm = PMM.lock();

        let virt_test = VirtAddr::new(0x0000_7FFF_1000_0000);
        let phys_test = PhysAddr::new(0x0000_0000_0200_0000); // 32 MiB physical mark
        let flags = PageTableFlags::WRITABLE | PageTableFlags::NO_EXECUTE;

        // Map page
        if let Err(_) = vmm.map_page(virt_test, phys_test, flags, &mut *pmm) {
            crate::serial_write_str("[KNT-PANIC] VMM map_page failed!\r\n");
            return false;
        }

        // Translate page
        match vmm.translate(virt_test) {
            Some(translated) => {
                if translated != phys_test {
                    crate::serial_write_str("[KNT-PANIC] VMM translated address does not match target physical address!\r\n");
                    return false;
                }
            }
            None => {
                crate::serial_write_str("[KNT-PANIC] VMM translate returned None for mapped virtual address!\r\n");
                return false;
            }
        }

        // Unmap page
        match vmm.unmap_page(virt_test) {
            Ok(unmapped_phys) => {
                if unmapped_phys != phys_test {
                    crate::serial_write_str("[KNT-PANIC] VMM unmapped wrong physical address!\r\n");
                    return false;
                }
            }
            Err(_) => {
                crate::serial_write_str("[KNT-PANIC] VMM unmap_page failed!\r\n");
                return false;
            }
        }

        // Verify no longer translated
        if vmm.translate(virt_test).is_some() {
            crate::serial_write_str("[KNT-PANIC] VMM page still translated after unmapping!\r\n");
            return false;
        }

        crate::serial_write_str("[KNT-MEM-VMM-PASS] VMM 4-level PML4 mapping and translation verified\r\n");
    }

    // --- TEST 3: Heap & Rust alloc Collections ---
    {
        use alloc::boxed::Box;
        use alloc::string::String;
        use alloc::vec::Vec;

        // Test Box
        let boxed_val = Box::new(0xCAFE_BABE_DEAD_BEEFu64);
        if *boxed_val != 0xCAFE_BABE_DEAD_BEEFu64 {
            crate::serial_write_str("[KNT-PANIC] Heap Box value corrupted!\r\n");
            return false;
        }

        // Test Vec
        let mut test_vec = Vec::new();
        for i in 0..128u32 {
            test_vec.push(i);
        }
        let sum: u32 = test_vec.iter().sum();
        // sum(0..128) = 127 * 128 / 2 = 8128
        if sum != 8128 {
            crate::serial_write_str("[KNT-PANIC] Heap Vec calculation mismatch!\r\n");
            return false;
        }

        // Test String
        let mut test_str = String::from("KntonOS");
        test_str.push_str(" Memory Subsystem Online");
        if test_str.len() != 31 {
            crate::serial_write_str("[KNT-PANIC] Heap String length mismatch!\r\n");
            return false;
        }

        crate::serial_write_str("[KNT-MEM-HEAP-PASS] GlobalAlloc Box, Vec, and String operations verified\r\n");
    }

    crate::serial_write_str("[KNT-MEM-TESTS-PASS] All kernel memory subsystem checks passed\r\n");
    true
}
