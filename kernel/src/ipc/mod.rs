//! Inter-Process Communication (IPC) Subsystem for KntonOS.
//!
//! Provides zero-copy SPSC shared memory rings, fast user-space futex synchronization,
//! and NT section mapping primitives.

pub mod ring;
pub mod futex;
pub mod section;

pub use futex::{
    futex_wait, futex_wake, futex_wake_tid, test_futex_engine, FutexBucket, FutexError, FutexKey,
    FutexTable, FutexWaiter, FUTEX_BUCKETS, FUTEX_TABLE,
};
pub use ring::{
    test_spsc_ring, PacketHeader, RingBufferError, SharedMemoryRing, SharedRingHeader,
    SpscRingBuffer, CACHELINE_SIZE, PACKET_HEADER_SIZE, PACKET_MAGIC,
};
pub use section::{SectionManager, SharedSection, SECTION_MANAGER};
