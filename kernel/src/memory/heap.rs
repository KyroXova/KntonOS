//! Kernel Global Heap Allocator implementing core::alloc::GlobalAlloc.

use super::spinlock::Spinlock;
use core::alloc::{GlobalAlloc, Layout};

pub const DEFAULT_HEAP_SIZE: usize = 2 * 1024 * 1024; // 2 MiB default kernel heap

#[repr(align(4096))]
struct HeapMemory([u8; DEFAULT_HEAP_SIZE]);

static mut HEAP_STORAGE: HeapMemory = HeapMemory([0; DEFAULT_HEAP_SIZE]);

#[repr(C, align(16))]
struct BlockHeader {
    size: usize,     // Total size including this header
    is_free: bool,   // True if available
    next: Option<*mut BlockHeader>,
}

pub struct Heap {
    head: Option<*mut BlockHeader>,
    heap_start: usize,
    heap_end: usize,
    allocated_bytes: usize,
    total_bytes: usize,
}

unsafe impl Send for Heap {}

impl Heap {
    pub const fn empty() -> Self {
        Self {
            head: None,
            heap_start: 0,
            heap_end: 0,
            allocated_bytes: 0,
            total_bytes: 0,
        }
    }

    /// Initializes the heap with a contiguous memory region.
    pub unsafe fn init(&mut self, start: usize, size: usize) {
        // Ensure 16-byte alignment of the start address
        let aligned_start = (start + 15) & !15;
        let usable_size = size.saturating_sub(aligned_start - start);

        let initial_block = aligned_start as *mut BlockHeader;
        (*initial_block).size = usable_size;
        (*initial_block).is_free = true;
        (*initial_block).next = None;

        self.head = Some(initial_block);
        self.heap_start = aligned_start;
        self.heap_end = aligned_start + usable_size;
        self.total_bytes = usable_size;
        self.allocated_bytes = 0;
    }

    pub fn total_bytes(&self) -> usize {
        self.total_bytes
    }

    pub fn allocated_bytes(&self) -> usize {
        self.allocated_bytes
    }

    pub fn free_bytes(&self) -> usize {
        self.total_bytes.saturating_sub(self.allocated_bytes)
    }

    /// Allocates memory satisfying the layout requirements.
    pub unsafe fn allocate(&mut self, layout: Layout) -> *mut u8 {
        let align = layout.align().max(16);
        let size = layout.size().max(1);

        let header_size = core::mem::size_of::<BlockHeader>();
        let mut curr_opt = self.head;

        while let Some(curr_ptr) = curr_opt {
            let curr = &mut *curr_ptr;

            if curr.is_free {
                // Calculate payload pointer aligned to requested alignment
                let payload_raw = (curr_ptr as usize) + header_size;
                let payload_aligned = (payload_raw + (align - 1)) & !(align - 1);
                let padding = payload_aligned - payload_raw;
                let needed_size = header_size + padding + size;

                if curr.size >= needed_size {
                    // Check if remaining block is large enough to split
                    let remaining = curr.size - needed_size;
                    let min_split = header_size + 16;

                    if remaining >= min_split {
                        let next_block = (payload_aligned + size) as *mut BlockHeader;
                        (*next_block).size = remaining;
                        (*next_block).is_free = true;
                        (*next_block).next = curr.next;

                        curr.size = needed_size;
                        curr.next = Some(next_block);
                    }

                    curr.is_free = false;
                    self.allocated_bytes += curr.size;
                    return payload_aligned as *mut u8;
                }
            }
            curr_opt = curr.next;
        }

        core::ptr::null_mut()
    }

    /// Deallocates a previously allocated pointer.
    pub unsafe fn deallocate(&mut self, ptr: *mut u8, _layout: Layout) {
        if ptr.is_null() {
            return;
        }

        let ptr_addr = ptr as usize;
        let header_size = core::mem::size_of::<BlockHeader>();

        // Find the block containing this payload
        let mut curr_opt = self.head;
        while let Some(curr_ptr) = curr_opt {
            let curr = &mut *curr_ptr;
            let block_start = curr_ptr as usize;
            let block_end = block_start + curr.size;

            if ptr_addr >= block_start + header_size && ptr_addr < block_end {
                if !curr.is_free {
                    curr.is_free = true;
                    self.allocated_bytes = self.allocated_bytes.saturating_sub(curr.size);
                    self.coalesce();
                }
                return;
            }
            curr_opt = curr.next;
        }
    }

    /// Merges contiguous adjacent free blocks to minimize fragmentation.
    unsafe fn coalesce(&mut self) {
        let mut curr_opt = self.head;
        while let Some(curr_ptr) = curr_opt {
            let curr = &mut *curr_ptr;
            if curr.is_free {
                while let Some(next_ptr) = curr.next {
                    let next = &mut *next_ptr;
                    // Check if adjacent in memory
                    if (curr_ptr as usize) + curr.size == (next_ptr as usize) && next.is_free {
                        curr.size += next.size;
                        curr.next = next.next;
                    } else {
                        break;
                    }
                }
            }
            curr_opt = curr.next;
        }
    }
}

pub struct KntonAllocator {
    inner: Spinlock<Heap>,
}

impl KntonAllocator {
    pub const fn new() -> Self {
        Self {
            inner: Spinlock::new(Heap::empty()),
        }
    }

    /// Initializes default kernel heap memory storage.
    pub fn init_default(&self) {
        unsafe {
            let start = core::ptr::addr_of_mut!(HEAP_STORAGE.0) as *mut u8 as usize;
            self.inner.lock().init(start, DEFAULT_HEAP_SIZE);
        }
    }

    pub fn total_bytes(&self) -> usize {
        self.inner.lock().total_bytes()
    }

    pub fn allocated_bytes(&self) -> usize {
        self.inner.lock().allocated_bytes()
    }

    pub fn free_bytes(&self) -> usize {
        self.inner.lock().free_bytes()
    }
}

unsafe impl GlobalAlloc for KntonAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        self.inner.lock().allocate(layout)
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        self.inner.lock().deallocate(ptr, layout);
    }
}
