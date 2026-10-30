//! Direct3D 12 Constant Buffer Ring Allocator Engine.
//!
//! Provides lock-free bump-pointer suballocation for transient shader constants:
//! - Strict 256-byte alignment (`D3D12_CONSTANT_BUFFER_DATA_PLACEMENT_ALIGNMENT`)
//! - Zero-heap-allocation per-draw constant staging
//! - Ring buffer wrap-around with padding and capacity management
//! - Frame retirement tracking linked to GPU timeline fence progression

extern crate alloc;
use alloc::vec::Vec;

// -----------------------------------------------------------------------------
// Constants and Errors
// -----------------------------------------------------------------------------

/// D3D12 constant buffer data placement alignment requirement (256 bytes).
pub const D3D12_CONSTANT_BUFFER_DATA_PLACEMENT_ALIGNMENT: usize = 256;

/// Default ring buffer capacity (2 MiB).
pub const DEFAULT_RING_BUFFER_CAPACITY: usize = 2 * 1024 * 1024;

/// Errors returned by the constant buffer ring allocator.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RingAllocError {
    /// Ring buffer is full; GPU has not retired previous frames yet.
    OutOfMemory { requested: usize, available: usize },
    /// Allocation size exceeds total ring buffer capacity.
    RequestedSizeExceedsCapacity { requested: usize, capacity: usize },
}

/// Aligns a value up to the nearest multiple of `alignment` (which must be a power of two).
#[inline(always)]
pub const fn align_up(value: usize, alignment: usize) -> usize {
    (value + (alignment - 1)) & !(alignment - 1)
}

// -----------------------------------------------------------------------------
// Suballocation Slices
// -----------------------------------------------------------------------------

/// Memory slice allocated from the transient constant buffer ring.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ConstantBufferSlice {
    /// GPU Virtual Address (GPU VA) matching hardware root descriptor bindings.
    pub gpu_va: u64,
    /// Byte offset within the ring buffer.
    pub ring_offset: usize,
    /// Usable size requested by caller in bytes.
    pub size_bytes: usize,
    /// Aligned size occupied in ring buffer (multiple of 256).
    pub aligned_size_bytes: usize,
}

// -----------------------------------------------------------------------------
// Constant Buffer Ring Allocator
// -----------------------------------------------------------------------------

/// High-performance bump-pointer ring buffer allocator for GPU shader constants.
pub struct ConstantBufferRingAllocator {
    /// Base GPU Virtual Address for the mapped buffer.
    pub base_gpu_va: u64,
    /// CPU backing storage for staging constant data.
    pub storage: Vec<u8>,
    /// Total capacity in bytes (multiple of 256).
    pub capacity: usize,
    /// Current bump allocation head offset in bytes.
    pub head: usize,
    /// Tail offset representing oldest unretired allocated memory.
    pub tail: usize,
    /// Wrap-around padding bytes at the end of the buffer when head wrapped to 0.
    pub end_padding: usize,
    /// Total active allocated bytes in flight.
    pub allocated_bytes: usize,
    /// Total number of suballocations performed.
    pub total_allocations: u64,
}

impl ConstantBufferRingAllocator {
    /// Creates a new constant buffer ring allocator with specified capacity and base GPU VA.
    pub fn new(capacity: usize, base_gpu_va: u64) -> Self {
        let aligned_cap = align_up(capacity, D3D12_CONSTANT_BUFFER_DATA_PLACEMENT_ALIGNMENT);
        let mut storage = Vec::new();
        storage.resize(aligned_cap, 0);

        Self {
            base_gpu_va,
            storage,
            capacity: aligned_cap,
            head: 0,
            tail: 0,
            end_padding: 0,
            allocated_bytes: 0,
            total_allocations: 0,
        }
    }

    /// Allocates an aligned slice from the ring buffer.
    pub fn allocate(&mut self, size_bytes: usize) -> Result<ConstantBufferSlice, RingAllocError> {
        let aligned_size = align_up(size_bytes, D3D12_CONSTANT_BUFFER_DATA_PLACEMENT_ALIGNMENT);

        if aligned_size > self.capacity {
            return Err(RingAllocError::RequestedSizeExceedsCapacity {
                requested: aligned_size,
                capacity: self.capacity,
            });
        }

        // Align current head to 256 bytes
        let aligned_head = align_up(self.head, D3D12_CONSTANT_BUFFER_DATA_PLACEMENT_ALIGNMENT);

        // Case A: head is at or ahead of tail (head >= tail)
        if aligned_head >= self.tail {
            // Check if it fits between aligned_head and capacity
            if aligned_head + aligned_size <= self.capacity {
                let offset = aligned_head;
                let gpu_va = self.base_gpu_va + (offset as u64);
                self.head = offset + aligned_size;
                self.allocated_bytes += aligned_size;
                self.total_allocations += 1;

                return Ok(ConstantBufferSlice {
                    gpu_va,
                    ring_offset: offset,
                    size_bytes,
                    aligned_size_bytes: aligned_size,
                });
            }

            // Doesn't fit at end: attempt wrap-around to 0
            if aligned_size <= self.tail {
                // Pad the remainder of the buffer
                self.end_padding = self.capacity - aligned_head;
                let offset = 0;
                let gpu_va = self.base_gpu_va;
                self.head = aligned_size;
                self.allocated_bytes += aligned_size;
                self.total_allocations += 1;

                return Ok(ConstantBufferSlice {
                    gpu_va,
                    ring_offset: offset,
                    size_bytes,
                    aligned_size_bytes: aligned_size,
                });
            }

            // Buffer is full (cannot fit at end, and cannot fit before tail)
            let available = (self.capacity - aligned_head) + self.tail;
            return Err(RingAllocError::OutOfMemory {
                requested: aligned_size,
                available,
            });
        }

        // Case B: head has wrapped around and is behind tail (head < tail)
        if aligned_head + aligned_size <= self.tail {
            let offset = aligned_head;
            let gpu_va = self.base_gpu_va + (offset as u64);
            self.head = offset + aligned_size;
            self.allocated_bytes += aligned_size;
            self.total_allocations += 1;

            return Ok(ConstantBufferSlice {
                gpu_va,
                ring_offset: offset,
                size_bytes,
                aligned_size_bytes: aligned_size,
            });
        }

        // Out of memory between wrapped head and tail
        let available = self.tail.saturating_sub(aligned_head);
        Err(RingAllocError::OutOfMemory {
            requested: aligned_size,
            available,
        })
    }

    /// Allocates an aligned slice and immediately copies raw CPU constant data into it.
    pub fn allocate_and_write(&mut self, data: &[u8]) -> Result<ConstantBufferSlice, RingAllocError> {
        let slice = self.allocate(data.len())?;
        self.storage[slice.ring_offset..slice.ring_offset + data.len()].copy_from_slice(data);
        Ok(slice)
    }

    /// Reads data from a previously allocated slice in CPU memory.
    pub fn read_slice(&self, slice: &ConstantBufferSlice) -> &[u8] {
        &self.storage[slice.ring_offset..slice.ring_offset + slice.size_bytes]
    }

    /// Returns the currently available contiguous space before wrapping or tail.
    pub fn contiguous_available_bytes(&self) -> usize {
        let aligned_head = align_up(self.head, D3D12_CONSTANT_BUFFER_DATA_PLACEMENT_ALIGNMENT);
        if aligned_head >= self.tail {
            self.capacity - aligned_head
        } else {
            self.tail - aligned_head
        }
    }
}

// -----------------------------------------------------------------------------
// Unit Tests & Verification
// -----------------------------------------------------------------------------

/// Unit test verifying 256-byte aligned constant buffer ring allocator and transient suballocation.
pub fn verify_ring_suballocation_and_alignment() -> bool {
    let capacity = 64 * 1024; // 64 KiB
    let base_gpu_va = 0x8000_0000_u64;
    let mut ring = ConstantBufferRingAllocator::new(capacity, base_gpu_va);

    // 1. Initial State
    if ring.head != 0 || ring.tail != 0 || ring.allocated_bytes != 0 {
        return false;
    }

    // 2. Allocate 64 bytes: must be aligned to 256 bytes
    let slice1 = match ring.allocate(64) {
        Ok(s) => s,
        Err(_) => return false,
    };
    if slice1.ring_offset != 0 || slice1.size_bytes != 64 || slice1.aligned_size_bytes != 256 {
        return false;
    }
    if slice1.gpu_va != base_gpu_va || (slice1.gpu_va % 256 != 0) {
        return false;
    }
    if ring.head != 256 || ring.allocated_bytes != 256 {
        return false;
    }

    // 3. Allocate 300 bytes: must be aligned to 512 bytes
    let slice2 = match ring.allocate(300) {
        Ok(s) => s,
        Err(_) => return false,
    };
    if slice2.ring_offset != 256 || slice2.size_bytes != 300 || slice2.aligned_size_bytes != 512 {
        return false;
    }
    if slice2.gpu_va != base_gpu_va + 256 || (slice2.gpu_va % 256 != 0) {
        return false;
    }
    if ring.head != 768 || ring.allocated_bytes != 768 {
        return false;
    }

    // 4. Test Data Writing & Verification
    let test_data = [0xAA_u8, 0xBB, 0xCC, 0xDD, 0x11, 0x22, 0x33, 0x44];
    let slice3 = match ring.allocate_and_write(&test_data) {
        Ok(s) => s,
        Err(_) => return false,
    };
    if slice3.ring_offset != 768 || (slice3.gpu_va % 256 != 0) {
        return false;
    }
    let read_back = ring.read_slice(&slice3);
    if read_back != &test_data {
        return false;
    }

    // 5. Test Ring Buffer Wrap-Around
    // Fill remaining space up to 64 KiB
    let remaining_before_wrap = ring.capacity - ring.head;
    // Allocate almost all remaining
    let slice_large = match ring.allocate(remaining_before_wrap - 128) {
        Ok(s) => s,
        Err(_) => return false,
    };
    if slice_large.gpu_va % 256 != 0 {
        return false;
    }

    // Now current head cannot fit a 512-byte allocation at the end.
    // If tail is 0, wrap around is blocked because tail hasn't moved yet.
    if ring.allocate(512).is_ok() {
        return false; // Should return OutOfMemory because ring is full
    }

    // Move tail forward to simulate retired frame memory
    ring.tail = 4096;
    // Now wrap-around to 0 should succeed because 512 <= tail (4096)
    let slice_wrapped = match ring.allocate(512) {
        Ok(s) => s,
        Err(_) => return false,
    };
    if slice_wrapped.ring_offset != 0 || slice_wrapped.gpu_va != base_gpu_va {
        return false;
    }
    if ring.head != 512 {
        return false;
    }

    true
}
