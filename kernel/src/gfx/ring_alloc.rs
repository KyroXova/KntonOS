//! Direct3D 12 Constant Buffer Ring Allocator Engine.
//! Provides lock-free bump-pointer suballocation for transient shader constants:

extern crate alloc;
use alloc::vec::Vec;

// Constants and Errors

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

// Suballocation Slices

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

// Constant Buffer Ring Allocator

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

// Unit Tests & Verification

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


// Multi-Frame Ring Chunk Retirement

/// Metadata for a recorded frame's constant buffer allocations in the ring.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FrameChunk {
    /// Incremental frame index (e.g. 1, 2, 3...).
    pub frame_index: u64,
    /// 64-bit GPU timeline fence value that signals frame completion.
    pub fence_value: u64,
    /// Ring offset where this frame's allocations started.
    pub ring_start_offset: usize,
    /// Ring offset where this frame's allocations ended (head at frame end).
    pub ring_end_offset: usize,
    /// Total bytes allocated within this frame.
    pub allocated_bytes: usize,
    /// Whether this frame encountered a ring buffer wrap-around.
    pub wrapped: bool,
    /// Wrap-around padding bytes at buffer end (if wrapped).
    pub wrap_padding: usize,
}

/// Tracks multi-frame constant buffer allocations and retires completed chunks
/// based on monotonic GPU timeline fence progression.
pub struct FrameRetirementTracker {
    /// Active frame currently being recorded.
    pub current_frame_index: u64,
    /// Start head offset of the active frame.
    pub current_frame_start: usize,
    /// Accumulated allocated bytes in the active frame.
    pub current_frame_bytes: usize,
    /// Whether the active frame has wrapped around to 0.
    pub current_frame_wrapped: bool,
    /// Wrap padding recorded in the active frame.
    pub current_frame_padding: usize,
    /// Queue of pending in-flight frame chunks awaiting GPU retirement.
    pub pending_chunks: Vec<FrameChunk>,
    /// Highest fence value retired so far.
    pub last_retired_fence_value: u64,
    /// Total frames successfully retired.
    pub total_frames_retired: u64,
}

impl FrameRetirementTracker {
    /// Creates a new frame retirement tracker.
    pub fn new() -> Self {
        Self {
            current_frame_index: 0,
            current_frame_start: 0,
            current_frame_bytes: 0,
            current_frame_wrapped: false,
            current_frame_padding: 0,
            pending_chunks: Vec::new(),
            last_retired_fence_value: 0,
            total_frames_retired: 0,
        }
    }

    /// Marks the start of a new frame's command buffer recording.
    pub fn begin_frame(&mut self, frame_index: u64, ring: &ConstantBufferRingAllocator) {
        self.current_frame_index = frame_index;
        self.current_frame_start = ring.head;
        self.current_frame_bytes = 0;
        self.current_frame_wrapped = false;
        self.current_frame_padding = 0;
    }

    /// Notifies the tracker if an allocation within the current frame caused a ring wrap-around.
    pub fn record_wrap(&mut self, padding_bytes: usize) {
        self.current_frame_wrapped = true;
        self.current_frame_padding = padding_bytes;
    }

    /// Ends recording for the current frame and registers its chunk with the specified GPU fence value.
    pub fn end_frame(&mut self, ring: &ConstantBufferRingAllocator, fence_value: u64) {
        let chunk = FrameChunk {
            frame_index: self.current_frame_index,
            fence_value,
            ring_start_offset: self.current_frame_start,
            ring_end_offset: ring.head,
            allocated_bytes: self.current_frame_bytes,
            wrapped: self.current_frame_wrapped,
            wrap_padding: self.current_frame_padding,
        };

        self.pending_chunks.push(chunk);

        // Advance frame start for next frame
        self.current_frame_start = ring.head;
        self.current_frame_bytes = 0;
        self.current_frame_wrapped = false;
        self.current_frame_padding = 0;
    }

    /// Retires all completed frame chunks whose fence values are <= `completed_fence_value`.
    /// Reclaims memory by advancing the ring allocator's `tail` pointer to the newest
    pub fn retire_completed_frames(
        &mut self,
        ring: &mut ConstantBufferRingAllocator,
        completed_fence_value: u64,
    ) -> usize {
        let mut total_reclaimed_bytes = 0usize;
        let mut newest_tail = ring.tail;
        let mut retired_count = 0usize;

        while let Some(chunk) = self.pending_chunks.first() {
            if chunk.fence_value <= completed_fence_value {
                newest_tail = chunk.ring_end_offset;
                total_reclaimed_bytes += chunk.allocated_bytes + chunk.wrap_padding;
                self.last_retired_fence_value = chunk.fence_value;
                self.total_frames_retired += 1;
                retired_count += 1;
                self.pending_chunks.remove(0);
            } else {
                break;
            }
        }

        if retired_count > 0 {
            ring.tail = newest_tail;
            ring.allocated_bytes = ring.allocated_bytes.saturating_sub(total_reclaimed_bytes);
            if ring.head == ring.tail {
                // Ring is completely empty; reset pointers to 0 for maximum contiguous headroom
                ring.head = 0;
                ring.tail = 0;
                ring.end_padding = 0;
                ring.allocated_bytes = 0;
            }
        }

        total_reclaimed_bytes
    }

    /// Number of frames currently in-flight on the GPU.
    pub fn in_flight_frame_count(&self) -> usize {
        self.pending_chunks.len()
    }
}

/// Unit test verifying multi-frame ring chunk retirement synchronized with timeline fence progression.
pub fn verify_frame_retirement_and_fence_sync() -> bool {
    let capacity = 32 * 1024; // 32 KiB
    let base_gpu_va = 0x9000_0000_u64;
    let mut ring = ConstantBufferRingAllocator::new(capacity, base_gpu_va);
    let mut tracker = FrameRetirementTracker::new();

    // 1. Frame 1: Allocates 2 x 256 bytes (512 bytes total), Fence value = 10
    tracker.begin_frame(1, &ring);
    let s1 = ring.allocate(256).expect("F1 slice 1 failed");
    let s2 = ring.allocate(256).expect("F1 slice 2 failed");
    tracker.current_frame_bytes += s1.aligned_size_bytes + s2.aligned_size_bytes;
    tracker.end_frame(&ring, 10);

    if ring.head != 512 || ring.tail != 0 || tracker.in_flight_frame_count() != 1 {
        return false;
    }

    // 2. Frame 2: Allocates 1024 bytes, Fence value = 20
    tracker.begin_frame(2, &ring);
    let s3 = ring.allocate(1024).expect("F2 slice failed");
    tracker.current_frame_bytes += s3.aligned_size_bytes;
    tracker.end_frame(&ring, 20);

    if ring.head != 1536 || ring.tail != 0 || tracker.in_flight_frame_count() != 2 {
        return false;
    }

    // 3. Frame 3: Allocates 512 bytes, Fence value = 30
    tracker.begin_frame(3, &ring);
    let s4 = ring.allocate(512).expect("F3 slice failed");
    tracker.current_frame_bytes += s4.aligned_size_bytes;
    tracker.end_frame(&ring, 30);

    if ring.head != 2048 || ring.tail != 0 || tracker.in_flight_frame_count() != 3 {
        return false;
    }

    // 4. Test Fence Progression:
    // Signal GPU completed fence value = 5 (less than Frame 1's fence 10)
    let reclaimed_0 = tracker.retire_completed_frames(&mut ring, 5);
    if reclaimed_0 != 0 || ring.tail != 0 || tracker.in_flight_frame_count() != 3 {
        return false;
    }

    // Signal GPU completed fence value = 15 (covers Frame 1 with fence 10)
    // Frame 1 must retire: tail advances to 512, reclaimed 512 bytes
    let reclaimed_f1 = tracker.retire_completed_frames(&mut ring, 15);
    if reclaimed_f1 != 512 || ring.tail != 512 || tracker.in_flight_frame_count() != 2 {
        return false;
    }
    if ring.allocated_bytes != 1536 {
        return false;
    }

    // Signal GPU completed fence value = 30 (covers Frame 2 and Frame 3)
    // Both frames retire: tail catches up to head (2048 -> resets to 0 when empty)
    let reclaimed_f2_f3 = tracker.retire_completed_frames(&mut ring, 30);
    if reclaimed_f2_f3 != 1536 || tracker.in_flight_frame_count() != 0 {
        return false;
    }
    // Ring should be completely clean and reset
    if ring.head != 0 || ring.tail != 0 || ring.allocated_bytes != 0 {
        return false;
    }
    if tracker.total_frames_retired != 3 {
        return false;
    }

    true
}
