//! Micro-Kernel Fixed-Size Object Slab Cache & Bitmap Free-Slot Indexer.
//!
//! Provides deterministic O(1) object allocation and deallocation for small-to-medium
//! kernel objects, eliminating heap fragmentation through power-of-two size classes
//! and 64-bit word bitmap free-slot indexers.

use alloc::alloc::{alloc, dealloc};
use alloc::vec::Vec;
use core::alloc::Layout;

/// Standard micro-kernel power-of-two slab size classes (in bytes).
pub const SLAB_SIZE_CLASSES: [usize; 8] = [32, 64, 128, 256, 512, 1024, 2048, 4096];

/// Number of size classes supported.
pub const NUM_SIZE_CLASSES: usize = SLAB_SIZE_CLASSES.len();

/// Standard backing page frame size for slabs (4 KiB).
pub const SLAB_PAGE_SIZE: usize = 4096;

/// Maximum number of 64-bit bitmap words per slab (supports up to 256 slots per slab).
pub const BITMAP_WORDS_PER_SLAB: usize = 4;

/// Maximum number of slots supported in a single 4KB slab (4096 / 32 = 128 slots).
pub const MAX_SLOTS_PER_SLAB: usize = BITMAP_WORDS_PER_SLAB * 64;

/// Errors returned by slab cache operations.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SlabError {
    /// Object size requested is zero or exceeds maximum supported size.
    InvalidSize,
    /// Slab is exhausted and cannot allocate additional objects.
    OutOfMemory,
    /// Pointer address does not belong to this slab or is not aligned to object boundary.
    PointerNotInSlab,
    /// Object slot was already free (detected double-free attempt).
    DoubleFree,
    /// Slot index is out of bounds for this slab.
    SlotOutOfBounds,
}

/// Occupancy state of a slab page frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SlabState {
    /// No slots allocated; candidate for frame reclamation.
    Empty,
    /// Some slots allocated and some slots free.
    Partial,
    /// All slots allocated; no slots available.
    Full,
}

/// A single slab backed by a page-aligned physical/virtual memory frame.
///
/// Uses an embedded bitmap (`[u64; BITMAP_WORDS_PER_SLAB]`) where bit 0 indicates
/// a free slot and bit 1 indicates an allocated slot.
pub struct Slab {
    /// Unique identifier for this slab frame.
    id: usize,
    /// Pointer to the page-aligned backing buffer.
    buffer: *mut u8,
    /// Size of the backing buffer in bytes (always page-aligned, typically 4096).
    buffer_size: usize,
    /// Size of individual objects allocated from this slab.
    object_size: usize,
    /// Total number of object slots available in this slab.
    total_slots: usize,
    /// Number of currently allocated object slots.
    allocated_slots: usize,
    /// Bitmap free-slot indexer (bit 0 = free, bit 1 = allocated).
    bitmap: [u64; BITMAP_WORDS_PER_SLAB],
    /// Current occupancy state.
    state: SlabState,
}

// Safety: Slab manages its own backing buffer and memory safety invariant.
unsafe impl Send for Slab {}
unsafe impl Sync for Slab {}

impl Slab {
    /// Creates a new page-aligned slab for objects of `object_size` bytes.
    pub fn new(id: usize, object_size: usize) -> Option<Self> {
        if object_size == 0 || object_size > SLAB_PAGE_SIZE {
            return None;
        }

        let total_slots = SLAB_PAGE_SIZE / object_size;
        if total_slots == 0 || total_slots > MAX_SLOTS_PER_SLAB {
            return None;
        }

        let layout = Layout::from_size_align(SLAB_PAGE_SIZE, SLAB_PAGE_SIZE).ok()?;
        let buffer = unsafe { alloc(layout) };
        if buffer.is_null() {
            return None;
        }

        // Initialize backing memory to 0
        unsafe {
            core::ptr::write_bytes(buffer, 0, SLAB_PAGE_SIZE);
        }

        Some(Self {
            id,
            buffer,
            buffer_size: SLAB_PAGE_SIZE,
            object_size,
            total_slots,
            allocated_slots: 0,
            bitmap: [0u64; BITMAP_WORDS_PER_SLAB],
            state: SlabState::Empty,
        })
    }

    /// Allocates an object slot from this slab using fast bitwise scan.
    /// Returns the raw pointer to the object or None if the slab is full.
    pub fn alloc_slot(&mut self) -> Option<*mut u8> {
        if self.allocated_slots >= self.total_slots {
            self.state = SlabState::Full;
            return None;
        }

        // Scan bitmap words for the first zero bit
        for word_idx in 0..BITMAP_WORDS_PER_SLAB {
            let word = self.bitmap[word_idx];
            if word != !0u64 {
                // Invert word to find first free bit using trailing zeros
                let bit_idx = (!word).trailing_zeros() as usize;
                let slot_idx = word_idx * 64 + bit_idx;

                if slot_idx < self.total_slots {
                    // Mark slot as allocated
                    self.bitmap[word_idx] |= 1u64 << bit_idx;
                    self.allocated_slots += 1;

                    self.state = if self.allocated_slots == self.total_slots {
                        SlabState::Full
                    } else {
                        SlabState::Partial
                    };

                    let offset = slot_idx * self.object_size;
                    let ptr = unsafe { self.buffer.add(offset) };
                    return Some(ptr);
                }
            }
        }

        self.state = SlabState::Full;
        None
    }

    /// Deallocates an object slot by its pointer.
    pub fn free_slot(&mut self, ptr: *mut u8) -> Result<(), SlabError> {
        let ptr_addr = ptr as usize;
        let buf_addr = self.buffer as usize;

        // Boundary and alignment checks
        if ptr_addr < buf_addr || ptr_addr >= buf_addr + self.buffer_size {
            return Err(SlabError::PointerNotInSlab);
        }

        let offset = ptr_addr - buf_addr;
        if offset % self.object_size != 0 {
            return Err(SlabError::PointerNotInSlab);
        }

        let slot_idx = offset / self.object_size;
        if slot_idx >= self.total_slots {
            return Err(SlabError::SlotOutOfBounds);
        }

        let word_idx = slot_idx / 64;
        let bit_idx = slot_idx % 64;

        if (self.bitmap[word_idx] & (1u64 << bit_idx)) == 0 {
            return Err(SlabError::DoubleFree);
        }

        // Mark slot as free
        self.bitmap[word_idx] &= !(1u64 << bit_idx);
        self.allocated_slots = self.allocated_slots.saturating_sub(1);

        self.state = if self.allocated_slots == 0 {
            SlabState::Empty
        } else {
            SlabState::Partial
        };

        Ok(())
    }

    /// Checks if a pointer falls within this slab's backing frame.
    #[inline]
    pub fn contains(&self, ptr: *mut u8) -> bool {
        let ptr_addr = ptr as usize;
        let buf_addr = self.buffer as usize;
        ptr_addr >= buf_addr && ptr_addr < buf_addr + self.buffer_size
    }

    /// Unique slab ID.
    #[inline]
    pub fn id(&self) -> usize {
        self.id
    }

    /// Object size handled by this slab.
    #[inline]
    pub fn object_size(&self) -> usize {
        self.object_size
    }

    /// Total capacity in slots.
    #[inline]
    pub fn total_slots(&self) -> usize {
        self.total_slots
    }

    /// Currently allocated slot count.
    #[inline]
    pub fn allocated_slots(&self) -> usize {
        self.allocated_slots
    }

    /// Free slot count.
    #[inline]
    pub fn free_slots(&self) -> usize {
        self.total_slots - self.allocated_slots
    }

    /// Current slab state.
    #[inline]
    pub fn state(&self) -> SlabState {
        self.state
    }

    /// Returns the base pointer of the backing memory buffer.
    #[inline]
    pub fn buffer_base(&self) -> *mut u8 {
        self.buffer
    }
}

impl Drop for Slab {
    fn drop(&mut self) {
        if !self.buffer.is_null() {
            unsafe {
                let layout = Layout::from_size_align_unchecked(self.buffer_size, SLAB_PAGE_SIZE);
                dealloc(self.buffer, layout);
                self.buffer = core::ptr::null_mut();
            }
        }
    }
}

/// Slab cache managing a collection of slabs for a specific object size class.
pub struct SlabCache {
    /// Object size class for all slabs in this cache.
    object_size: usize,
    /// Next monotonic slab ID.
    next_slab_id: usize,
    /// Collection of slabs managed by this cache.
    slabs: Vec<Slab>,
    /// Lifetime allocation counter.
    total_allocations: usize,
    /// Lifetime deallocation counter.
    total_deallocations: usize,
}

impl SlabCache {
    /// Creates a new slab cache for a specific object size class.
    pub fn new(object_size: usize) -> Self {
        Self {
            object_size,
            next_slab_id: 1,
            slabs: Vec::new(),
            total_allocations: 0,
            total_deallocations: 0,
        }
    }

    /// Allocates an object from this slab cache.
    /// First attempts to allocate from an existing partial slab, then an empty slab,
    /// and finally creates a new slab if all current slabs are full.
    pub fn alloc(&mut self) -> Option<*mut u8> {
        // Step 1: Search for a partial slab
        for slab in &mut self.slabs {
            if slab.state() == SlabState::Partial {
                if let Some(ptr) = slab.alloc_slot() {
                    self.total_allocations += 1;
                    return Some(ptr);
                }
            }
        }

        // Step 2: Search for an empty slab
        for slab in &mut self.slabs {
            if slab.state() == SlabState::Empty {
                if let Some(ptr) = slab.alloc_slot() {
                    self.total_allocations += 1;
                    return Some(ptr);
                }
            }
        }

        // Step 3: All existing slabs are full; allocate a new slab frame
        let mut new_slab = Slab::new(self.next_slab_id, self.object_size)?;
        self.next_slab_id += 1;

        let ptr = new_slab.alloc_slot()?;
        self.total_allocations += 1;
        self.slabs.push(new_slab);
        Some(ptr)
    }

    /// Deallocates an object pointer, returning it to its owning slab.
    pub fn free(&mut self, ptr: *mut u8) -> Result<(), SlabError> {
        for slab in &mut self.slabs {
            if slab.contains(ptr) {
                slab.free_slot(ptr)?;
                self.total_deallocations += 1;
                return Ok(());
            }
        }
        Err(SlabError::PointerNotInSlab)
    }

    /// Object size managed by this cache.
    #[inline]
    pub fn object_size(&self) -> usize {
        self.object_size
    }

    /// Total number of slabs currently retained.
    #[inline]
    pub fn slab_count(&self) -> usize {
        self.slabs.len()
    }

    /// Total number of active allocated objects across all slabs.
    pub fn active_objects(&self) -> usize {
        self.slabs.iter().map(|s| s.allocated_slots()).sum()
    }

    /// Total object slot capacity across all slabs.
    pub fn total_capacity(&self) -> usize {
        self.slabs.iter().map(|s| s.total_slots()).sum()
    }

    /// Lifetime allocations performed.
    #[inline]
    pub fn total_allocations(&self) -> usize {
        self.total_allocations
    }

    /// Lifetime deallocations performed.
    #[inline]
    pub fn total_deallocations(&self) -> usize {
        self.total_deallocations
    }

    /// Memory utilization ratio (0.0 to 1.0).
    pub fn utilization(&self) -> f32 {
        let cap = self.total_capacity();
        if cap == 0 {
            0.0
        } else {
            self.active_objects() as f32 / cap as f32
        }
    }

    /// Internal access to slabs for frame reaping.
    #[inline]
    #[allow(dead_code)]
    pub(crate) fn slabs_mut(&mut self) -> &mut Vec<Slab> {
        &mut self.slabs
    }

    /// Internal access to slabs for inspection.
    #[inline]
    #[allow(dead_code)]
    pub(crate) fn slabs(&self) -> &Vec<Slab> {
        &self.slabs
    }
}

/// Micro-Kernel multi-class slab allocator managing standard power-of-two size classes.
pub struct MultiClassSlabAllocator {
    caches: [SlabCache; NUM_SIZE_CLASSES],
}

impl MultiClassSlabAllocator {
    /// Creates a new multi-class slab allocator initialized for all 8 standard size classes.
    pub fn new() -> Self {
        Self {
            caches: [
                SlabCache::new(SLAB_SIZE_CLASSES[0]), // 32
                SlabCache::new(SLAB_SIZE_CLASSES[1]), // 64
                SlabCache::new(SLAB_SIZE_CLASSES[2]), // 128
                SlabCache::new(SLAB_SIZE_CLASSES[3]), // 256
                SlabCache::new(SLAB_SIZE_CLASSES[4]), // 512
                SlabCache::new(SLAB_SIZE_CLASSES[5]), // 1024
                SlabCache::new(SLAB_SIZE_CLASSES[6]), // 2048
                SlabCache::new(SLAB_SIZE_CLASSES[7]), // 4096
            ],
        }
    }

    /// Finds the index of the smallest power-of-two size class that fits `size` bytes.
    #[inline]
    pub fn find_size_class_index(size: usize) -> Option<usize> {
        if size == 0 || size > SLAB_PAGE_SIZE {
            return None;
        }
        for (i, &class_size) in SLAB_SIZE_CLASSES.iter().enumerate() {
            if size <= class_size {
                return Some(i);
            }
        }
        None
    }

    /// Allocates memory for an object of `size` bytes from the appropriate size class.
    pub fn alloc(&mut self, size: usize) -> Option<*mut u8> {
        let class_idx = Self::find_size_class_index(size)?;
        self.caches[class_idx].alloc()
    }

    /// Deallocates memory previously allocated with `alloc`.
    pub fn free(&mut self, ptr: *mut u8, size: usize) -> Result<(), SlabError> {
        let class_idx = Self::find_size_class_index(size).ok_or(SlabError::InvalidSize)?;
        self.caches[class_idx].free(ptr)
    }

    /// Obtains a reference to the slab cache for a specific size class index.
    pub fn cache(&self, class_idx: usize) -> Option<&SlabCache> {
        self.caches.get(class_idx)
    }

    /// Obtains a mutable reference to the slab cache for a specific size class index.
    pub fn cache_mut(&mut self, class_idx: usize) -> Option<&mut SlabCache> {
        self.caches.get_mut(class_idx)
    }

    /// Total active objects allocated across all size classes.
    pub fn total_active_objects(&self) -> usize {
        self.caches.iter().map(|c| c.active_objects()).sum()
    }

    /// Total slot capacity across all size classes.
    pub fn total_capacity(&self) -> usize {
        self.caches.iter().map(|c| c.total_capacity()).sum()
    }
}

/// Unit test verifying slab cache allocation, bitmap free-slot indexing, and double-free detection.
pub fn test_slab_cache_and_bitmap_indexer() -> bool {
    // 1. Test single Slab with 64-byte objects (4096 / 64 = 64 slots)
    let mut slab = match Slab::new(1, 64) {
        Some(s) => s,
        None => return false,
    };

    if slab.total_slots() != 64 || slab.allocated_slots() != 0 || slab.state() != SlabState::Empty {
        return false;
    }

    let mut ptrs: [*mut u8; 64] = [core::ptr::null_mut(); 64];

    // Allocate all 64 slots
    for i in 0..64 {
        match slab.alloc_slot() {
            Some(p) => {
                ptrs[i] = p;
                // Write magic pattern to verify accessibility and no overlaps
                unsafe {
                    core::ptr::write_volatile(p as *mut u64, 0xCAFE_BABE_0000_0000 | (i as u64));
                }
            }
            None => return false,
        }
    }

    if slab.allocated_slots() != 64 || slab.state() != SlabState::Full {
        return false;
    }

    // 65th allocation must fail
    if slab.alloc_slot().is_some() {
        return false;
    }

    // Verify all 64 written values
    for i in 0..64 {
        let val = unsafe { core::ptr::read_volatile(ptrs[i] as *const u64) };
        if val != (0xCAFE_BABE_0000_0000 | (i as u64)) {
            return false;
        }
    }

    // Free slot 0, slot 31, and slot 63
    if slab.free_slot(ptrs[0]).is_err() {
        return false;
    }
    if slab.free_slot(ptrs[31]).is_err() {
        return false;
    }
    if slab.free_slot(ptrs[63]).is_err() {
        return false;
    }

    if slab.allocated_slots() != 61 || slab.state() != SlabState::Partial {
        return false;
    }

    // Double free detection on slot 31 must return DoubleFree error
    if slab.free_slot(ptrs[31]) != Err(SlabError::DoubleFree) {
        return false;
    }

    // Re-allocate 3 slots: bitmap must reuse the freed slots in order (0, 31, 63)
    let realloc_0 = slab.alloc_slot().unwrap();
    let realloc_31 = slab.alloc_slot().unwrap();
    let realloc_63 = slab.alloc_slot().unwrap();

    if realloc_0 != ptrs[0] || realloc_31 != ptrs[31] || realloc_63 != ptrs[63] {
        return false;
    }

    if slab.allocated_slots() != 64 || slab.state() != SlabState::Full {
        return false;
    }

    // Free all slots
    for i in 0..64 {
        let p = if i == 0 {
            realloc_0
        } else if i == 31 {
            realloc_31
        } else if i == 63 {
            realloc_63
        } else {
            ptrs[i]
        };
        if slab.free_slot(p).is_err() {
            return false;
        }
    }

    if slab.allocated_slots() != 0 || slab.state() != SlabState::Empty {
        return false;
    }

    // 2. Test SlabCache multi-slab scaling (128-byte objects = 32 slots per slab)
    let mut cache = SlabCache::new(128);

    // Allocate 100 objects (requires 4 slabs: 32 * 3 = 96 < 100 <= 128)
    let mut cache_ptrs = Vec::new();
    for i in 0..100 {
        match cache.alloc() {
            Some(p) => {
                unsafe {
                    core::ptr::write_volatile(p as *mut usize, 0xD00D_0000 + i);
                }
                cache_ptrs.push(p);
            }
            None => return false,
        }
    }

    if cache.slab_count() != 4 || cache.active_objects() != 100 {
        return false;
    }

    // Verify all 100 values
    for i in 0..100 {
        let val = unsafe { core::ptr::read_volatile(cache_ptrs[i] as *const usize) };
        if val != 0xD00D_0000 + i {
            return false;
        }
    }

    // Free odd index objects (50 objects)
    for i in (1..100).step_by(2) {
        if cache.free(cache_ptrs[i]).is_err() {
            return false;
        }
    }

    if cache.active_objects() != 50 {
        return false;
    }

    // Verify even index objects remain uncorrupted
    for i in (0..100).step_by(2) {
        let val = unsafe { core::ptr::read_volatile(cache_ptrs[i] as *const usize) };
        if val != 0xD00D_0000 + i {
            return false;
        }
    }

    // Reallocate 50 objects
    for i in 0..50 {
        let p = cache.alloc().unwrap();
        unsafe {
            core::ptr::write_volatile(p as *mut usize, 0xBEEF_0000 + i);
        }
    }

    if cache.active_objects() != 100 {
        return false;
    }

    // 3. Test MultiClassSlabAllocator across varied size classes
    let mut multi = MultiClassSlabAllocator::new();

    let p32 = multi.alloc(24).unwrap();     // Class 32
    let p128 = multi.alloc(100).unwrap();   // Class 128
    let p512 = multi.alloc(300).unwrap();   // Class 512
    let p2048 = multi.alloc(1500).unwrap(); // Class 2048
    let p4096 = multi.alloc(4000).unwrap(); // Class 4096

    unsafe {
        core::ptr::write_volatile(p32 as *mut u32, 0x3232_3232);
        core::ptr::write_volatile(p128 as *mut u32, 0x1281_2812);
        core::ptr::write_volatile(p512 as *mut u32, 0x5125_1251);
        core::ptr::write_volatile(p2048 as *mut u32, 0x2048_2048);
        core::ptr::write_volatile(p4096 as *mut u32, 0x4096_4096);
    }

    if unsafe { core::ptr::read_volatile(p32 as *const u32) } != 0x3232_3232 {
        return false;
    }
    if unsafe { core::ptr::read_volatile(p128 as *const u32) } != 0x1281_2812 {
        return false;
    }
    if unsafe { core::ptr::read_volatile(p512 as *const u32) } != 0x5125_1251 {
        return false;
    }
    if unsafe { core::ptr::read_volatile(p2048 as *const u32) } != 0x2048_2048 {
        return false;
    }
    if unsafe { core::ptr::read_volatile(p4096 as *const u32) } != 0x4096_4096 {
        return false;
    }

    if multi.free(p32, 24).is_err() {
        return false;
    }
    if multi.free(p128, 100).is_err() {
        return false;
    }
    if multi.free(p512, 300).is_err() {
        return false;
    }
    if multi.free(p2048, 1500).is_err() {
        return false;
    }
    if multi.free(p4096, 4000).is_err() {
        return false;
    }

    true
}
