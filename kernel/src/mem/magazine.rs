//! Bonwick-Style Magazine Layer & Per-CPU Local Object Caches.
//! Implements the classic Bonwick & Adams magazine caching architecture to eliminate

use super::slab::{SlabCache, SlabError};
use alloc::vec::Vec;

/// Standard capacity of a magazine (number of object pointers buffered per magazine).
pub const MAGAZINE_CAPACITY: usize = 32;

/// Maximum number of CPUs supported by the per-CPU magazine layer.
pub const MAX_CPUS: usize = 8;

/// A Bonwick-style fixed-capacity magazine holding object pointers.
#[derive(Clone, Copy)]
pub struct Magazine {
    /// Object pointer stack buffer.
    ptrs: [*mut u8; MAGAZINE_CAPACITY],
    /// Number of pointers currently stored (rounds).
    rounds: usize,
}

// Safety: Magazine holds raw pointers transferred across CPUs safely.
unsafe impl Send for Magazine {}
unsafe impl Sync for Magazine {}

impl Magazine {
    /// Creates a new empty magazine with zero rounds.
    pub const fn new() -> Self {
        Self {
            ptrs: [core::ptr::null_mut(); MAGAZINE_CAPACITY],
            rounds: 0,
        }
    }

    /// Checks if the magazine contains no object pointers.
    #[inline]
    pub const fn is_empty(&self) -> bool {
        self.rounds == 0
    }

    /// Checks if the magazine is full to capacity.
    #[inline]
    pub const fn is_full(&self) -> bool {
        self.rounds >= MAGAZINE_CAPACITY
    }

    /// Number of object pointers currently held in this magazine.
    #[inline]
    pub const fn rounds(&self) -> usize {
        self.rounds
    }

    /// Maximum capacity of this magazine.
    #[inline]
    pub const fn capacity(&self) -> usize {
        MAGAZINE_CAPACITY
    }

    /// Pushes an object pointer into the magazine. Returns `Err(ptr)` if the magazine is full.
    #[inline]
    pub fn push(&mut self, ptr: *mut u8) -> Result<(), *mut u8> {
        if self.is_full() {
            Err(ptr)
        } else {
            self.ptrs[self.rounds] = ptr;
            self.rounds += 1;
            Ok(())
        }
    }

    /// Pops an object pointer from the magazine. Returns `None` if the magazine is empty.
    #[inline]
    pub fn pop(&mut self) -> Option<*mut u8> {
        if self.is_empty() {
            None
        } else {
            self.rounds -= 1;
            let ptr = self.ptrs[self.rounds];
            self.ptrs[self.rounds] = core::ptr::null_mut();
            Some(ptr)
        }
    }

    /// Clears and resets the magazine, returning all stored pointers as an array slice.
    pub fn drain(&mut self) -> ([*mut u8; MAGAZINE_CAPACITY], usize) {
        let count = self.rounds;
        let mut copy = [core::ptr::null_mut(); MAGAZINE_CAPACITY];
        copy[..count].copy_from_slice(&self.ptrs[..count]);
        self.rounds = 0;
        (copy, count)
    }
}

/// Shared depot managing full and empty magazines for a specific slab cache.
pub struct MagazineDepot {
    /// Stack of completely full magazines (`rounds == MAGAZINE_CAPACITY`).
    full_magazines: Vec<Magazine>,
    /// Stack of completely empty magazines (`rounds == 0`).
    empty_magazines: Vec<Magazine>,
    /// Total exchanges from empty to full.
    exchanges_empty_to_full: usize,
    /// Total exchanges from full to empty.
    exchanges_full_to_empty: usize,
}

impl MagazineDepot {
    /// Creates a new magazine depot.
    pub fn new() -> Self {
        Self {
            full_magazines: Vec::new(),
            empty_magazines: Vec::new(),
            exchanges_empty_to_full: 0,
            exchanges_full_to_empty: 0,
        }
    }

    /// Exchanges an exhausted (empty) magazine for a full magazine.
    /// If no full magazine is available in the depot, populates one from the underlying `slab_cache`.
    pub fn exchange_empty_for_full(
        &mut self,
        empty_mag: Magazine,
        slab_cache: &mut SlabCache,
    ) -> Option<Magazine> {
        self.exchanges_empty_to_full += 1;

        // Store the empty magazine in the depot
        if empty_mag.is_empty() {
            self.empty_magazines.push(empty_mag);
        }

        // Check if the depot already has a full magazine available
        if let Some(full) = self.full_magazines.pop() {
            return Some(full);
        }

        // Depot has no full magazines; allocate directly from slab cache to construct a new full magazine
        let mut new_mag = self.empty_magazines.pop().unwrap_or_else(Magazine::new);
        while !new_mag.is_full() {
            match slab_cache.alloc() {
                Some(ptr) => {
                    let _ = new_mag.push(ptr);
                }
                None => {
                    // Out of slab memory; if we got at least some objects, return what we have
                    if !new_mag.is_empty() {
                        return Some(new_mag);
                    } else {
                        return None;
                    }
                }
            }
        }

        Some(new_mag)
    }

    /// Exchanges a full magazine for an empty magazine.
    /// Stores the full magazine in the depot. If depot exceeds upper threshold, reaps
    pub fn exchange_full_for_empty(
        &mut self,
        full_mag: Magazine,
        slab_cache: &mut SlabCache,
    ) -> Magazine {
        self.exchanges_full_to_empty += 1;

        // Check if depot full stack exceeds threshold (e.g. 16 full magazines = 512 objects cached)
        const MAX_DEPOT_FULL_MAGAZINES: usize = 16;
        if self.full_magazines.len() >= MAX_DEPOT_FULL_MAGAZINES {
            // Reap excess full magazine directly back to slab cache
            let mut mag_to_reap = full_mag;
            while let Some(ptr) = mag_to_reap.pop() {
                let _ = slab_cache.free(ptr);
            }
            return self.empty_magazines.pop().unwrap_or_else(Magazine::new);
        }

        self.full_magazines.push(full_mag);

        // Return an empty magazine from the depot or construct a fresh one
        self.empty_magazines.pop().unwrap_or_else(Magazine::new)
    }

    /// Number of full magazines currently buffered in the depot.
    #[inline]
    pub fn full_count(&self) -> usize {
        self.full_magazines.len()
    }

    /// Number of empty magazines currently buffered in the depot.
    #[inline]
    pub fn empty_count(&self) -> usize {
        self.empty_magazines.len()
    }

    /// Lifetime exchanges from empty to full.
    #[inline]
    pub fn exchanges_empty_to_full(&self) -> usize {
        self.exchanges_empty_to_full
    }

    /// Lifetime exchanges from full to empty.
    #[inline]
    pub fn exchanges_full_to_empty(&self) -> usize {
        self.exchanges_full_to_empty
    }
}

/// Per-CPU local object cache managing `active` and `prev` magazines.
pub struct PerCpuMagazineCache {
    /// CPU identifier owning this cache.
    cpu_id: usize,
    /// Primary active magazine for fast-path LIFO allocations and deallocations.
    active: Magazine,
    /// Secondary backup magazine.
    prev: Magazine,
    /// Number of fast-path allocations served directly from local magazines.
    fast_allocations: usize,
    /// Number of allocations requiring depot interaction.
    depot_allocations: usize,
    /// Number of fast-path deallocations absorbed directly by local magazines.
    fast_deallocations: usize,
    /// Number of deallocations requiring depot interaction.
    depot_deallocations: usize,
}

impl PerCpuMagazineCache {
    /// Creates a new per-CPU magazine cache for the specified CPU.
    pub fn new(cpu_id: usize) -> Self {
        Self {
            cpu_id,
            active: Magazine::new(),
            prev: Magazine::new(),
            fast_allocations: 0,
            depot_allocations: 0,
            fast_deallocations: 0,
            depot_deallocations: 0,
        }
    }

    /// Allocates an object pointer using Bonwick's magazine protocol:
    /// 1. Pop from `active` magazine (fast path).
    pub fn alloc(
        &mut self,
        depot: &mut MagazineDepot,
        slab_cache: &mut SlabCache,
    ) -> Option<*mut u8> {
        // Fast path 1: pop from active
        if let Some(ptr) = self.active.pop() {
            self.fast_allocations += 1;
            return Some(ptr);
        }

        // Fast path 2: active is empty, check if prev has objects
        if !self.prev.is_empty() {
            core::mem::swap(&mut self.active, &mut self.prev);
            if let Some(ptr) = self.active.pop() {
                self.fast_allocations += 1;
                return Some(ptr);
            }
        }

        // Slow path: both active and prev are empty; exchange prev with depot for a full magazine
        self.depot_allocations += 1;
        let empty_mag = core::mem::replace(&mut self.prev, Magazine::new());
        let full_mag = depot.exchange_empty_for_full(empty_mag, slab_cache)?;

        self.active = full_mag;
        self.active.pop()
    }

    /// Deallocates an object pointer using Bonwick's magazine protocol:
    /// 1. Push to `active` magazine (fast path).
    pub fn free(
        &mut self,
        ptr: *mut u8,
        depot: &mut MagazineDepot,
        slab_cache: &mut SlabCache,
    ) -> Result<(), SlabError> {
        // Fast path 1: push to active
        if self.active.push(ptr).is_ok() {
            self.fast_deallocations += 1;
            return Ok(());
        }

        // Fast path 2: active is full, check if prev has space
        if !self.prev.is_full() {
            core::mem::swap(&mut self.active, &mut self.prev);
            if self.active.push(ptr).is_ok() {
                self.fast_deallocations += 1;
                return Ok(());
            }
        }

        // Slow path: both active and prev are full; exchange prev with depot for an empty magazine
        self.depot_deallocations += 1;
        let full_mag = core::mem::replace(&mut self.prev, Magazine::new());
        let empty_mag = depot.exchange_full_for_empty(full_mag, slab_cache);

        self.prev = self.active;
        self.active = empty_mag;
        self.active.push(ptr).map_err(|_| SlabError::OutOfMemory)?;

        Ok(())
    }

    /// CPU identifier for this cache.
    #[inline]
    pub fn cpu_id(&self) -> usize {
        self.cpu_id
    }

    /// Fast path allocation count.
    #[inline]
    pub fn fast_allocations(&self) -> usize {
        self.fast_allocations
    }

    /// Depot path allocation count.
    #[inline]
    pub fn depot_allocations(&self) -> usize {
        self.depot_allocations
    }

    /// Fast path deallocation count.
    #[inline]
    pub fn fast_deallocations(&self) -> usize {
        self.fast_deallocations
    }

    /// Depot path deallocation count.
    #[inline]
    pub fn depot_deallocations(&self) -> usize {
        self.depot_deallocations
    }

    /// Rounds currently buffered in active magazine.
    #[inline]
    pub fn active_rounds(&self) -> usize {
        self.active.rounds()
    }

    /// Rounds currently buffered in prev magazine.
    #[inline]
    pub fn prev_rounds(&self) -> usize {
        self.prev.rounds()
    }

    /// Total objects cached locally on this CPU.
    #[inline]
    pub fn local_cached_objects(&self) -> usize {
        self.active.rounds() + self.prev.rounds()
    }
}

/// Bonwick magazine layer managing per-CPU caches and depot for a single size class.
pub struct PerCpuMagazineLayer {
    /// Object size class.
    object_size: usize,
    /// Shared magazine depot.
    depot: MagazineDepot,
    /// Per-CPU magazine caches.
    per_cpu: [PerCpuMagazineCache; MAX_CPUS],
}

impl PerCpuMagazineLayer {
    /// Creates a new per-CPU magazine layer for the given object size class.
    pub fn new(object_size: usize) -> Self {
        Self {
            object_size,
            depot: MagazineDepot::new(),
            per_cpu: [
                PerCpuMagazineCache::new(0),
                PerCpuMagazineCache::new(1),
                PerCpuMagazineCache::new(2),
                PerCpuMagazineCache::new(3),
                PerCpuMagazineCache::new(4),
                PerCpuMagazineCache::new(5),
                PerCpuMagazineCache::new(6),
                PerCpuMagazineCache::new(7),
            ],
        }
    }

    /// Allocates an object for the specified CPU core.
    pub fn alloc(&mut self, cpu_id: usize, slab_cache: &mut SlabCache) -> Option<*mut u8> {
        let cpu_idx = cpu_id % MAX_CPUS;
        self.per_cpu[cpu_idx].alloc(&mut self.depot, slab_cache)
    }

    /// Deallocates an object on the specified CPU core.
    pub fn free(
        &mut self,
        cpu_id: usize,
        ptr: *mut u8,
        slab_cache: &mut SlabCache,
    ) -> Result<(), SlabError> {
        let cpu_idx = cpu_id % MAX_CPUS;
        self.per_cpu[cpu_idx].free(ptr, &mut self.depot, slab_cache)
    }

    /// Access the depot for inspection.
    #[inline]
    pub fn depot(&self) -> &MagazineDepot {
        &self.depot
    }

    /// Access a specific CPU's local magazine cache.
    #[inline]
    pub fn per_cpu_cache(&self, cpu_id: usize) -> Option<&PerCpuMagazineCache> {
        let idx = cpu_id % MAX_CPUS;
        self.per_cpu.get(idx)
    }

    /// Object size handled by this layer.
    #[inline]
    pub fn object_size(&self) -> usize {
        self.object_size
    }
}

/// Unit test verifying Bonwick magazine LIFO operations, per-CPU caches, and depot exchanges.
pub fn test_bonwick_magazines_and_per_cpu_caches() -> bool {
    // 1. Single Magazine LIFO tests
    let mut mag = Magazine::new();
    if !mag.is_empty() || mag.is_full() || mag.rounds() != 0 {
        return false;
    }

    // Push pointers up to capacity (32)
    for i in 0..MAGAZINE_CAPACITY {
        let fake_ptr = (0x1000 + i * 64) as *mut u8;
        if mag.push(fake_ptr).is_err() {
            return false;
        }
    }

    if !mag.is_full() || mag.rounds() != MAGAZINE_CAPACITY {
        return false;
    }

    // 33rd push must fail
    if mag.push(0x9999 as *mut u8).is_ok() {
        return false;
    }

    // Pop in LIFO order
    for i in (0..MAGAZINE_CAPACITY).rev() {
        let expected = (0x1000 + i * 64) as *mut u8;
        match mag.pop() {
            Some(p) => {
                if p != expected {
                    return false;
                }
            }
            None => return false,
        }
    }

    if !mag.is_empty() || mag.rounds() != 0 {
        return false;
    }

    // 2. Per-CPU Magazine Cache and Depot Integration Test
    let mut slab_cache = SlabCache::new(64);
    let mut depot = MagazineDepot::new();
    let mut cpu0 = PerCpuMagazineCache::new(0);

    // Initial alloc on cpu0: both active and prev are empty -> triggers depot exchange
    let p0 = match cpu0.alloc(&mut depot, &mut slab_cache) {
        Some(p) => p,
        None => return false,
    };

    if cpu0.depot_allocations() != 1 {
        return false;
    }

    // The active magazine now has 31 items remaining (1 was popped)
    if cpu0.active_rounds() != MAGAZINE_CAPACITY - 1 {
        return false;
    }

    // Allocate remaining 31 items -> all must hit the fast path!
    let mut allocs = Vec::new();
    allocs.push(p0);
    for _ in 0..(MAGAZINE_CAPACITY - 1) {
        let p = cpu0.alloc(&mut depot, &mut slab_cache).unwrap();
        allocs.push(p);
    }

    if cpu0.fast_allocations() != MAGAZINE_CAPACITY - 1 {
        return false;
    }
    if cpu0.active_rounds() != 0 {
        return false;
    }

    // Free all 32 objects back to CPU0
    for &p in &allocs {
        if cpu0.free(p, &mut depot, &mut slab_cache).is_err() {
            return false;
        }
    }

    // Active magazine is now completely full (32 items)
    if !cpu0.active.is_full() || cpu0.active_rounds() != MAGAZINE_CAPACITY {
        return false;
    }

    // 3. Multi-CPU Layer Test (simulate CPU 0 and CPU 1 cross-freeing)
    let mut layer = PerCpuMagazineLayer::new(64);

    // CPU 0 allocates 16 objects
    let mut cpu0_ptrs = Vec::new();
    for _ in 0..16 {
        let ptr = layer.alloc(0, &mut slab_cache).unwrap();
        cpu0_ptrs.push(ptr);
    }

    // Cross-CPU transfer: CPU 1 frees the objects allocated by CPU 0
    for &ptr in &cpu0_ptrs {
        if layer.free(1, ptr, &mut slab_cache).is_err() {
            return false;
        }
    }

    // CPU 1 local cache should now contain 16 objects
    if layer.per_cpu_cache(1).unwrap().local_cached_objects() != 16 {
        return false;
    }

    // CPU 1 allocates 16 objects -> all fast-path hits
    for _ in 0..16 {
        if layer.alloc(1, &mut slab_cache).is_none() {
            return false;
        }
    }

    true
}
