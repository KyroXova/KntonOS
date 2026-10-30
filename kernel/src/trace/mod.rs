//! Zero-Overhead Kernel Trace Ring Buffer & Performance Profiling Event Engine.
//!
//! Provides lockless per-core circular event ring buffers, nanosecond timestamping,
//! dynamic event filtering, and shared-memory userspace exporter.

pub mod events;

pub use events::{
    test_trace_record_serialization, TraceEventMask, TraceEventType, TraceRecord,
    TRACE_RECORD_MAGIC,
};

pub mod ring;

pub use ring::{
    test_trace_ring_buffer, CachelineCursor, TraceRingBuffer, TraceRingHeader,
    DEFAULT_RING_CAPACITY,
};
