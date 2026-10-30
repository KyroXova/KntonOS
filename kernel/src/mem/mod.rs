//! Micro-Kernel Memory Subsystem: Slab Allocator & Bonwick Magazine Object Pool.

pub mod slab;
pub mod magazine;

/// Re-exports for convenient kernel-wide access.
pub use slab::{
    MultiClassSlabAllocator, Slab, SlabCache, SlabError, SlabState,
    SLAB_SIZE_CLASSES, NUM_SIZE_CLASSES, SLAB_PAGE_SIZE,
    TaggedSlot, LockFreeFreelist, LockFreeObjectPool, INVALID_SLOT_INDEX,
};
pub use magazine::{
    Magazine, MagazineDepot, PerCpuMagazineCache, PerCpuMagazineLayer,
    MAGAZINE_CAPACITY, MAX_CPUS,
};

/// Comprehensive verification suite for micro-kernel lock-free ring buffer object pool & slab cache allocator.
pub fn test_slab_allocator_subsystem() -> bool {
    crate::serial_write_str("Testing Slab Allocator Subsystem (TASK-KNT-085)...\r\n");

    // 1. Verify fixed-size object slab cache and bitmap free-slot indexer
    if !slab::test_slab_cache_and_bitmap_indexer() {
        crate::serial_write_str("  [FAIL] test_slab_cache_and_bitmap_indexer failed\r\n");
        return false;
    }
    crate::serial_write_str("  * Fixed-size slab cache and 64-bit bitmap free-slot indexing verified\r\n");

    // 2. Verify Bonwick-style magazine layer and per-CPU local object caches
    if !magazine::test_bonwick_magazines_and_per_cpu_caches() {
        crate::serial_write_str("  [FAIL] test_bonwick_magazines_and_per_cpu_caches failed\r\n");
        return false;
    }
    crate::serial_write_str("  * Bonwick magazine layer and per-CPU local object caches verified\r\n");

    // 3. Verify lock-free ABA-resistant freelist and global slab frame reaping
    if !slab::test_lock_free_aba_freelist_and_reaping() {
        crate::serial_write_str("  [FAIL] test_lock_free_aba_freelist_and_reaping failed\r\n");
        return false;
    }
    crate::serial_write_str("  * Lock-free ABA-resistant generational freelist and frame reaping verified\r\n");

    // 4. Integrated multi-core simulation stress test across power-of-two size classes
    let mut multi = MultiClassSlabAllocator::new();
    let mut layer_64 = PerCpuMagazineLayer::new(64);
    let mut layer_256 = PerCpuMagazineLayer::new(256);

    for cpu in 0..4 {
        let mut ptrs_64 = alloc::vec::Vec::new();
        let mut ptrs_256 = alloc::vec::Vec::new();

        let cache_64 = multi.cache_mut(1).unwrap();
        for _ in 0..16 {
            if let Some(p) = layer_64.alloc(cpu, cache_64) {
                ptrs_64.push(p);
            }
        }

        let cache_256 = multi.cache_mut(3).unwrap();
        for _ in 0..16 {
            if let Some(p) = layer_256.alloc(cpu, cache_256) {
                ptrs_256.push(p);
            }
        }

        if ptrs_64.len() != 16 || ptrs_256.len() != 16 {
            return false;
        }

        let cache_64 = multi.cache_mut(1).unwrap();
        for p in ptrs_64 {
            if layer_64.free(cpu, p, cache_64).is_err() {
                return false;
            }
        }

        let cache_256 = multi.cache_mut(3).unwrap();
        for p in ptrs_256 {
            if layer_256.free(cpu, p, cache_256).is_err() {
                return false;
            }
        }
    }

    crate::serial_write_str("  * Integrated multi-core magazine caching and multi-class slab stress test verified\r\n");
    crate::serial_write_str("[KNT-MEM-SLAB-PASS] Micro-kernel lock-free slab cache and magazine allocator operational\r\n");

    true
}
