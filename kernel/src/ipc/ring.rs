//! Lockless Single-Producer Single-Consumer (SPSC) Shared Memory Ring Buffer.
//!
//! Provides high-throughput, low-latency inter-process and inter-core communication
//! with cacheline-isolated atomic head/tail pointers, zero-copy packet framing,
//! and circular wrapping for arbitrary variable-length payloads.

use core::sync::atomic::{AtomicUsize, Ordering};

pub const CACHELINE_SIZE: usize = 64;
pub const PACKET_MAGIC: u16 = 0x504B; // "PK"
pub const PACKET_HEADER_SIZE: usize = core::mem::size_of::<PacketHeader>();

/// 64-byte cacheline aligned atomic counter to eliminate CPU false sharing between cores.
#[repr(C, align(64))]
pub struct CachelineAlignedAtomicUsize {
    pub value: AtomicUsize,
    _pad: [u8; CACHELINE_SIZE - core::mem::size_of::<AtomicUsize>()],
}

impl CachelineAlignedAtomicUsize {
    pub const fn new(val: usize) -> Self {
        Self {
            value: AtomicUsize::new(val),
            _pad: [0u8; CACHELINE_SIZE - core::mem::size_of::<AtomicUsize>()],
        }
    }
}

/// Header prepended to every message framed within the ring buffer.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PacketHeader {
    pub magic: u16,
    pub flags: u16,
    pub length: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RingBufferError {
    BufferFull,
    BufferEmpty,
    DestinationBufferTooSmall,
    CorruptedPacketHeader,
    InvalidCapacity,
    BufferNullOrMisaligned,
}

/// Fixed-capacity, cacheline-isolated SPSC ring buffer for high-speed IPC.
#[repr(C, align(64))]
pub struct SpscRingBuffer<const CAP: usize> {
    head: CachelineAlignedAtomicUsize, // Read position (managed by consumer)
    tail: CachelineAlignedAtomicUsize, // Write position (managed by producer)
    buffer: [u8; CAP],
}

impl<const CAP: usize> SpscRingBuffer<CAP> {
    pub const fn new() -> Self {
        Self {
            head: CachelineAlignedAtomicUsize::new(0),
            tail: CachelineAlignedAtomicUsize::new(0),
            buffer: [0u8; CAP],
        }
    }

    #[inline(always)]
    pub const fn capacity(&self) -> usize {
        CAP
    }

    #[inline(always)]
    pub fn available_read(&self) -> usize {
        let t = self.tail.value.load(Ordering::Acquire);
        let h = self.head.value.load(Ordering::Relaxed);
        t.wrapping_sub(h)
    }

    #[inline(always)]
    pub fn available_write(&self) -> usize {
        CAP.saturating_sub(self.available_read())
    }

    #[inline(always)]
    pub fn is_empty(&self) -> bool {
        self.available_read() == 0
    }

    #[inline(always)]
    pub fn is_full(&self) -> bool {
        self.available_write() < PACKET_HEADER_SIZE + 1
    }

    /// Enqueues a variable-length packet payload with standard framing.
    pub fn enqueue(&self, flags: u16, payload: &[u8]) -> Result<usize, RingBufferError> {
        let required = PACKET_HEADER_SIZE + payload.len();
        if self.available_write() < required {
            return Err(RingBufferError::BufferFull);
        }

        let header = PacketHeader {
            magic: PACKET_MAGIC,
            flags,
            length: payload.len() as u32,
        };

        let current_tail = self.tail.value.load(Ordering::Relaxed);

        // 1. Write 8-byte PacketHeader (handling circular wrap)
        let header_bytes = unsafe {
            core::slice::from_raw_parts(
                &header as *const PacketHeader as *const u8,
                PACKET_HEADER_SIZE,
            )
        };
        self.write_circular(current_tail, header_bytes);

        // 2. Write variable payload
        self.write_circular(current_tail.wrapping_add(PACKET_HEADER_SIZE), payload);

        // 3. Commit new tail pointer with Release semantics
        self.tail
            .value
            .store(current_tail.wrapping_add(required), Ordering::Release);

        Ok(payload.len())
    }

    /// Dequeues the next packet payload into `dest_buf`, returning `(payload_len, flags)`.
    pub fn dequeue(&self, dest_buf: &mut [u8]) -> Result<(usize, u16), RingBufferError> {
        let avail = self.available_read();
        if avail < PACKET_HEADER_SIZE {
            return Err(RingBufferError::BufferEmpty);
        }

        let current_head = self.head.value.load(Ordering::Relaxed);

        // 1. Read packet header
        let mut header = PacketHeader {
            magic: 0,
            flags: 0,
            length: 0,
        };
        let header_slice = unsafe {
            core::slice::from_raw_parts_mut(
                &mut header as *mut PacketHeader as *mut u8,
                PACKET_HEADER_SIZE,
            )
        };
        self.read_circular(current_head, header_slice);

        if header.magic != PACKET_MAGIC {
            return Err(RingBufferError::CorruptedPacketHeader);
        }

        let payload_len = header.length as usize;
        let total_packet_size = PACKET_HEADER_SIZE + payload_len;

        if avail < total_packet_size {
            return Err(RingBufferError::BufferEmpty);
        }

        if dest_buf.len() < payload_len {
            return Err(RingBufferError::DestinationBufferTooSmall);
        }

        // 2. Read payload
        self.read_circular(
            current_head.wrapping_add(PACKET_HEADER_SIZE),
            &mut dest_buf[..payload_len],
        );

        // 3. Commit new head pointer with Release semantics
        self.head
            .value
            .store(current_head.wrapping_add(total_packet_size), Ordering::Release);

        Ok((payload_len, header.flags))
    }

    /// Peeks the next packet header without advancing the consumer head pointer.
    pub fn peek_packet(&self) -> Result<(usize, u16), RingBufferError> {
        if self.available_read() < PACKET_HEADER_SIZE {
            return Err(RingBufferError::BufferEmpty);
        }

        let current_head = self.head.value.load(Ordering::Relaxed);
        let mut header = PacketHeader {
            magic: 0,
            flags: 0,
            length: 0,
        };
        let header_slice = unsafe {
            core::slice::from_raw_parts_mut(
                &mut header as *mut PacketHeader as *mut u8,
                PACKET_HEADER_SIZE,
            )
        };
        self.read_circular(current_head, header_slice);

        if header.magic != PACKET_MAGIC {
            return Err(RingBufferError::CorruptedPacketHeader);
        }

        Ok((header.length as usize, header.flags))
    }

    /// Resets the ring buffer state to empty.
    pub fn clear(&self) {
        let t = self.tail.value.load(Ordering::Relaxed);
        self.head.value.store(t, Ordering::Release);
    }

    fn write_circular(&self, offset: usize, src: &[u8]) {
        let mut written = 0;
        let mut curr_offset = offset % CAP;
        let len = src.len();

        while written < len {
            let chunk = core::cmp::min(len - written, CAP - curr_offset);
            unsafe {
                let dst_ptr = (self.buffer.as_ptr() as *mut u8).add(curr_offset);
                core::ptr::copy_nonoverlapping(src.as_ptr().add(written), dst_ptr, chunk);
            }
            written += chunk;
            curr_offset = 0; // Wrap back to beginning
        }
    }

    fn read_circular(&self, offset: usize, dst: &mut [u8]) {
        let mut read_bytes = 0;
        let mut curr_offset = offset % CAP;
        let len = dst.len();

        while read_bytes < len {
            let chunk = core::cmp::min(len - read_bytes, CAP - curr_offset);
            unsafe {
                let src_ptr = self.buffer.as_ptr().add(curr_offset);
                core::ptr::copy_nonoverlapping(src_ptr, dst.as_mut_ptr().add(read_bytes), chunk);
            }
            read_bytes += chunk;
            curr_offset = 0;
        }
    }
}

/// Raw representation of an SPSC ring in a shared memory section.
#[repr(C)]
pub struct SharedRingHeader {
    pub head: CachelineAlignedAtomicUsize,
    pub tail: CachelineAlignedAtomicUsize,
    pub capacity: usize,
    pub magic: u32,
}

pub const SHARED_RING_MAGIC: u32 = 0x53505343; // "SPSC"

/// Zero-copy adapter over an arbitrary shared memory page or section buffer.
pub struct SharedMemoryRing {
    header: *mut SharedRingHeader,
    buffer_ptr: *mut u8,
    capacity: usize,
}

impl SharedMemoryRing {
    /// Initializes a newly created shared memory ring inside raw buffer storage.
    pub unsafe fn init_in_place(
        raw_storage: *mut u8,
        storage_size: usize,
    ) -> Result<Self, RingBufferError> {
        let header_size = core::mem::size_of::<SharedRingHeader>();
        if storage_size <= header_size + PACKET_HEADER_SIZE {
            return Err(RingBufferError::InvalidCapacity);
        }

        let header = raw_storage as *mut SharedRingHeader;
        let buffer_ptr = raw_storage.add(header_size);
        let capacity = storage_size - header_size;

        (*header).head = CachelineAlignedAtomicUsize::new(0);
        (*header).tail = CachelineAlignedAtomicUsize::new(0);
        (*header).capacity = capacity;
        (*header).magic = SHARED_RING_MAGIC;

        Ok(Self {
            header,
            buffer_ptr,
            capacity,
        })
    }

    /// Attaches to an existing shared memory ring mapped into user or kernel address space.
    pub unsafe fn attach(
        raw_storage: *mut u8,
        storage_size: usize,
    ) -> Result<Self, RingBufferError> {
        let header_size = core::mem::size_of::<SharedRingHeader>();
        if storage_size <= header_size {
            return Err(RingBufferError::InvalidCapacity);
        }

        let header = raw_storage as *mut SharedRingHeader;
        if (*header).magic != SHARED_RING_MAGIC {
            return Err(RingBufferError::CorruptedPacketHeader);
        }

        let buffer_ptr = raw_storage.add(header_size);
        let capacity = (*header).capacity;

        Ok(Self {
            header,
            buffer_ptr,
            capacity,
        })
    }

    #[inline(always)]
    pub fn capacity(&self) -> usize {
        self.capacity
    }

    #[inline(always)]
    pub fn available_read(&self) -> usize {
        unsafe {
            let t = (*self.header).tail.value.load(Ordering::Acquire);
            let h = (*self.header).head.value.load(Ordering::Relaxed);
            t.wrapping_sub(h)
        }
    }

    #[inline(always)]
    pub fn available_write(&self) -> usize {
        self.capacity.saturating_sub(self.available_read())
    }

    pub fn enqueue(&self, flags: u16, payload: &[u8]) -> Result<usize, RingBufferError> {
        let required = PACKET_HEADER_SIZE + payload.len();
        if self.available_write() < required {
            return Err(RingBufferError::BufferFull);
        }

        let header = PacketHeader {
            magic: PACKET_MAGIC,
            flags,
            length: payload.len() as u32,
        };

        unsafe {
            let current_tail = (*self.header).tail.value.load(Ordering::Relaxed);
            let header_bytes = core::slice::from_raw_parts(
                &header as *const PacketHeader as *const u8,
                PACKET_HEADER_SIZE,
            );

            self.write_raw_circular(current_tail, header_bytes);
            self.write_raw_circular(current_tail.wrapping_add(PACKET_HEADER_SIZE), payload);

            (*self.header)
                .tail
                .value
                .store(current_tail.wrapping_add(required), Ordering::Release);
        }

        Ok(payload.len())
    }

    pub fn dequeue(&self, dest_buf: &mut [u8]) -> Result<(usize, u16), RingBufferError> {
        let avail = self.available_read();
        if avail < PACKET_HEADER_SIZE {
            return Err(RingBufferError::BufferEmpty);
        }

        unsafe {
            let current_head = (*self.header).head.value.load(Ordering::Relaxed);
            let mut header = PacketHeader {
                magic: 0,
                flags: 0,
                length: 0,
            };
            let header_slice = core::slice::from_raw_parts_mut(
                &mut header as *mut PacketHeader as *mut u8,
                PACKET_HEADER_SIZE,
            );

            self.read_raw_circular(current_head, header_slice);

            if header.magic != PACKET_MAGIC {
                return Err(RingBufferError::CorruptedPacketHeader);
            }

            let payload_len = header.length as usize;
            let total_packet_size = PACKET_HEADER_SIZE + payload_len;

            if avail < total_packet_size {
                return Err(RingBufferError::BufferEmpty);
            }

            if dest_buf.len() < payload_len {
                return Err(RingBufferError::DestinationBufferTooSmall);
            }

            self.read_raw_circular(
                current_head.wrapping_add(PACKET_HEADER_SIZE),
                &mut dest_buf[..payload_len],
            );

            (*self.header)
                .head
                .value
                .store(current_head.wrapping_add(total_packet_size), Ordering::Release);

            Ok((payload_len, header.flags))
        }
    }

    unsafe fn write_raw_circular(&self, offset: usize, src: &[u8]) {
        let mut written = 0;
        let mut curr_offset = offset % self.capacity;
        let len = src.len();

        while written < len {
            let chunk = core::cmp::min(len - written, self.capacity - curr_offset);
            let dst_ptr = self.buffer_ptr.add(curr_offset);
            core::ptr::copy_nonoverlapping(src.as_ptr().add(written), dst_ptr, chunk);
            written += chunk;
            curr_offset = 0;
        }
    }

    unsafe fn read_raw_circular(&self, offset: usize, dst: &mut [u8]) {
        let mut read_bytes = 0;
        let mut curr_offset = offset % self.capacity;
        let len = dst.len();

        while read_bytes < len {
            let chunk = core::cmp::min(len - read_bytes, self.capacity - curr_offset);
            let src_ptr = self.buffer_ptr.add(curr_offset);
            core::ptr::copy_nonoverlapping(src_ptr, dst.as_mut_ptr().add(read_bytes), chunk);
            read_bytes += chunk;
            curr_offset = 0;
        }
    }
}

/// Standalone verification self-test for SPSC lockless ring buffer.
pub fn test_spsc_ring() -> bool {
    crate::serial_write_str("[KNT-IPC-RING-TEST] Testing lockless SPSC shared memory ring buffer...\r\n");

    static mut RING: SpscRingBuffer<1024> = SpscRingBuffer::new();
    let ring = unsafe { &mut *core::ptr::addr_of_mut!(RING) };
    ring.clear();

    if !ring.is_empty() || ring.available_read() != 0 || ring.available_write() != 1024 {
        crate::serial_write_str("[KNT-PANIC] Fresh SPSC ring is not empty!\r\n");
        return false;
    }

    // 1. Basic packet enqueue and dequeue
    let msg1 = b"KntonOS IPC Fast Ring";
    if ring.enqueue(0x01, msg1) != Ok(msg1.len()) {
        crate::serial_write_str("[KNT-PANIC] Failed to enqueue first packet!\r\n");
        return false;
    }

    let mut rx_buf = [0u8; 128];
    match ring.dequeue(&mut rx_buf) {
        Ok((len, flags)) => {
            if len != msg1.len() || flags != 0x01 || &rx_buf[..len] != msg1 {
                crate::serial_write_str("[KNT-PANIC] Dequeued packet content mismatch!\r\n");
                return false;
            }
        }
        Err(_) => {
            crate::serial_write_str("[KNT-PANIC] Dequeue failed for first packet!\r\n");
            return false;
        }
    }

    // 2. Circular wrapping test across ring boundary
    // Fill and drain 20 times with 120-byte payloads to force wraparounds
    let mut payload = [0u8; 120];
    for round in 0..20u8 {
        for i in 0..payload.len() {
            payload[i] = round.wrapping_add(i as u8);
        }

        if ring.enqueue(round as u16, &payload) != Ok(payload.len()) {
            crate::serial_write_str("[KNT-PANIC] Wraparound enqueue failed!\r\n");
            return false;
        }

        match ring.dequeue(&mut rx_buf) {
            Ok((len, flags)) => {
                if len != payload.len() || flags != (round as u16) || &rx_buf[..len] != &payload[..] {
                    crate::serial_write_str("[KNT-PANIC] Wraparound payload mismatch!\r\n");
                    return false;
                }
            }
            Err(_) => {
                crate::serial_write_str("[KNT-PANIC] Wraparound dequeue failed!\r\n");
                return false;
            }
        }
    }

    crate::serial_write_str("[KNT-IPC-RING-PASS] SPSC lockless ring buffer framing and wraparound verified\r\n");
    true
}
