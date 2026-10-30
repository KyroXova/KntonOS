//! Micro-Kernel Memory Subsystem: Slab Allocator & Bonwick Magazine Object Pool.

pub mod slab;

/// Re-exports for convenient kernel-wide access.
pub use slab::{
    MultiClassSlabAllocator, Slab, SlabCache, SlabError, SlabState,
    SLAB_SIZE_CLASSES, NUM_SIZE_CLASSES, SLAB_PAGE_SIZE,
};
