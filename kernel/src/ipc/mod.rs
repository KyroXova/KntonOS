//! Inter-Process Communication (IPC) Subsystem for KntonOS.
//!
//! Provides zero-copy SPSC shared memory rings, fast user-space futex synchronization,
//! and NT section mapping primitives.

pub mod ring;

pub use ring::{
    test_spsc_ring, PacketHeader, RingBufferError, SharedMemoryRing, SharedRingHeader,
    SpscRingBuffer, CACHELINE_SIZE, PACKET_HEADER_SIZE, PACKET_MAGIC,
};
