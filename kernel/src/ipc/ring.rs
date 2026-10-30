//! Lockless Single-Producer Single-Consumer (SPSC) Shared Memory Ring Buffer.
//! Provides high-throughput, low-latency inter-process and inter-core communication

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

// Dual-Ring Bidirectional Shared-Memory Channel (knton-ipc-ring)

pub const DUAL_RING_MAGIC: u64 = 0x4452_494E_4743_484E; // "DRINGCHN"
pub const DUAL_RING_VERSION: u32 = 1;

/// Cacheline-isolated head/tail pointers for a unidirectional sub-ring.
#[repr(C, align(64))]
pub struct DualRingPointers {
    pub head: CachelineAlignedAtomicUsize,
    pub tail: CachelineAlignedAtomicUsize,
}

impl DualRingPointers {
    pub const fn new() -> Self {
        Self {
            head: CachelineAlignedAtomicUsize::new(0),
            tail: CachelineAlignedAtomicUsize::new(0),
        }
    }
}

/// Control header for full-duplex dual-ring shared memory IPC channels.
#[repr(C, align(64))]
pub struct DualRingChannelHeader {
    pub magic: u64,
    pub version: u32,
    pub channel_id: u32,
    pub capacity: usize,
    pub peer0_alive: AtomicUsize,
    pub peer1_alive: AtomicUsize,
    pub ring0_to_1: DualRingPointers, // Forward ring (Peer 0 -> Peer 1)
    pub ring1_to_0: DualRingPointers, // Reverse ring (Peer 1 -> Peer 0)
}

/// Full-duplex bidirectional circular shared-memory IPC channel.
/// Features dedicated TX and RX rings between two peers, eliminating
#[repr(C, align(64))]
pub struct DualRingChannel<const CAP: usize> {
    pub header: DualRingChannelHeader,
    pub buf_0_to_1: [u8; CAP],
    pub buf_1_to_0: [u8; CAP],
}

impl<const CAP: usize> DualRingChannel<CAP> {
    pub const fn new(channel_id: u32) -> Self {
        Self {
            header: DualRingChannelHeader {
                magic: DUAL_RING_MAGIC,
                version: DUAL_RING_VERSION,
                channel_id,
                capacity: CAP,
                peer0_alive: AtomicUsize::new(1),
                peer1_alive: AtomicUsize::new(1),
                ring0_to_1: DualRingPointers::new(),
                ring1_to_0: DualRingPointers::new(),
            },
            buf_0_to_1: [0u8; CAP],
            buf_1_to_0: [0u8; CAP],
        }
    }

    #[inline(always)]
    pub fn endpoint_peer0<'a>(&'a self) -> DualRingEndpoint<'a, CAP> {
        DualRingEndpoint {
            peer_id: 0,
            channel: self,
        }
    }

    #[inline(always)]
    pub fn endpoint_peer1<'a>(&'a self) -> DualRingEndpoint<'a, CAP> {
        DualRingEndpoint {
            peer_id: 1,
            channel: self,
        }
    }
}

/// Peer endpoint handle interacting with the bidirectional dual-ring channel.
pub struct DualRingEndpoint<'a, const CAP: usize> {
    pub peer_id: u8,
    pub channel: &'a DualRingChannel<CAP>,
}

impl<'a, const CAP: usize> DualRingEndpoint<'a, CAP> {
    #[inline(always)]
    fn tx_pointers(&self) -> &DualRingPointers {
        if self.peer_id == 0 {
            &self.channel.header.ring0_to_1
        } else {
            &self.channel.header.ring1_to_0
        }
    }

    #[inline(always)]
    fn rx_pointers(&self) -> &DualRingPointers {
        if self.peer_id == 0 {
            &self.channel.header.ring1_to_0
        } else {
            &self.channel.header.ring0_to_1
        }
    }

    #[inline(always)]
    fn tx_buffer(&self) -> *mut u8 {
        if self.peer_id == 0 {
            self.channel.buf_0_to_1.as_ptr() as *mut u8
        } else {
            self.channel.buf_1_to_0.as_ptr() as *mut u8
        }
    }

    #[inline(always)]
    fn rx_buffer(&self) -> *const u8 {
        if self.peer_id == 0 {
            self.channel.buf_1_to_0.as_ptr()
        } else {
            self.channel.buf_0_to_1.as_ptr()
        }
    }

    #[inline(always)]
    pub fn available_tx(&self) -> usize {
        let tx = self.tx_pointers();
        let tail = tx.tail.value.load(Ordering::Acquire);
        let head = tx.head.value.load(Ordering::Relaxed);
        let occupied = tail.wrapping_sub(head);
        CAP.saturating_sub(occupied)
    }

    #[inline(always)]
    pub fn available_rx(&self) -> usize {
        let rx = self.rx_pointers();
        let tail = rx.tail.value.load(Ordering::Acquire);
        let head = rx.head.value.load(Ordering::Relaxed);
        tail.wrapping_sub(head)
    }

    #[inline(always)]
    pub fn tx_tail_ptr(&self) -> *const AtomicUsize {
        &self.tx_pointers().tail.value
    }

    #[inline(always)]
    pub fn rx_tail_ptr(&self) -> *const AtomicUsize {
        &self.rx_pointers().tail.value
    }

    /// Enqueues a packet into the outbound transmit ring.
    pub fn send(&self, flags: u16, payload: &[u8]) -> Result<usize, RingBufferError> {
        let required = PACKET_HEADER_SIZE + payload.len();
        if self.available_tx() < required {
            return Err(RingBufferError::BufferFull);
        }

        let header = PacketHeader {
            magic: PACKET_MAGIC,
            flags,
            length: payload.len() as u32,
        };

        let tx = self.tx_pointers();
        let current_tail = tx.tail.value.load(Ordering::Relaxed);

        unsafe {
            let buf_ptr = self.tx_buffer();
            let header_slice = core::slice::from_raw_parts(
                &header as *const PacketHeader as *const u8,
                PACKET_HEADER_SIZE,
            );
            Self::write_raw_circular(buf_ptr, CAP, current_tail, header_slice);
            Self::write_raw_circular(
                buf_ptr,
                CAP,
                current_tail.wrapping_add(PACKET_HEADER_SIZE),
                payload,
            );
        }

        tx.tail
            .value
            .store(current_tail.wrapping_add(required), Ordering::Release);

        Ok(payload.len())
    }

    /// Dequeues a packet from the inbound receive ring.
    pub fn recv(&self, dest_buf: &mut [u8]) -> Result<(usize, u16), RingBufferError> {
        let avail = self.available_rx();
        if avail < PACKET_HEADER_SIZE {
            return Err(RingBufferError::BufferEmpty);
        }

        let rx = self.rx_pointers();
        let current_head = rx.head.value.load(Ordering::Relaxed);

        let mut header = PacketHeader {
            magic: 0,
            flags: 0,
            length: 0,
        };

        unsafe {
            let buf_ptr = self.rx_buffer();
            let header_slice = core::slice::from_raw_parts_mut(
                &mut header as *mut PacketHeader as *mut u8,
                PACKET_HEADER_SIZE,
            );
            Self::read_raw_circular(buf_ptr, CAP, current_head, header_slice);

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

            Self::read_raw_circular(
                buf_ptr,
                CAP,
                current_head.wrapping_add(PACKET_HEADER_SIZE),
                &mut dest_buf[..payload_len],
            );

            rx.head
                .value
                .store(current_head.wrapping_add(total_packet_size), Ordering::Release);

            Ok((payload_len, header.flags))
        }
    }

    unsafe fn write_raw_circular(buf_ptr: *mut u8, cap: usize, offset: usize, src: &[u8]) {
        let mut written = 0;
        let mut curr_offset = offset % cap;
        let len = src.len();

        while written < len {
            let chunk = core::cmp::min(len - written, cap - curr_offset);
            let dst_ptr = buf_ptr.add(curr_offset);
            core::ptr::copy_nonoverlapping(src.as_ptr().add(written), dst_ptr, chunk);
            written += chunk;
            curr_offset = 0;
        }
    }

    unsafe fn read_raw_circular(buf_ptr: *const u8, cap: usize, offset: usize, dst: &mut [u8]) {
        let mut read_bytes = 0;
        let mut curr_offset = offset % cap;
        let len = dst.len();

        while read_bytes < len {
            let chunk = core::cmp::min(len - read_bytes, cap - curr_offset);
            let src_ptr = buf_ptr.add(curr_offset);
            core::ptr::copy_nonoverlapping(src_ptr, dst.as_mut_ptr().add(read_bytes), chunk);
            read_bytes += chunk;
            curr_offset = 0;
        }
    }
}

/// Unit test verifying dual-ring circular channel layout and bidirectional transmission.
pub fn test_dual_ring_channel() -> bool {
    static mut DUAL_CHANNEL: DualRingChannel<2048> = DualRingChannel::new(42);
    let channel = unsafe { &mut *core::ptr::addr_of_mut!(DUAL_CHANNEL) };

    let p0 = channel.endpoint_peer0();
    let p1 = channel.endpoint_peer1();

    // Verify initial emptiness and capacity
    if p0.available_rx() != 0 || p0.available_tx() != 2048 {
        return false;
    }
    if p1.available_rx() != 0 || p1.available_tx() != 2048 {
        return false;
    }

    // Peer 0 sends to Peer 1
    let msg_0_to_1 = b"Hello from Peer 0 to Peer 1!";
    if p0.send(0x10, msg_0_to_1) != Ok(msg_0_to_1.len()) {
        return false;
    }

    // Peer 1 sends to Peer 0
    let msg_1_to_0 = b"Acknowledged from Peer 1 back to Peer 0!";
    if p1.send(0x20, msg_1_to_0) != Ok(msg_1_to_0.len()) {
        return false;
    }

    // Peer 1 receives from Peer 0
    let mut rx_buf = [0u8; 128];
    match p1.recv(&mut rx_buf) {
        Ok((len, flags)) => {
            if len != msg_0_to_1.len() || flags != 0x10 || &rx_buf[..len] != msg_0_to_1 {
                return false;
            }
        }
        Err(_) => return false,
    }

    // Peer 0 receives from Peer 1
    match p0.recv(&mut rx_buf) {
        Ok((len, flags)) => {
            if len != msg_1_to_0.len() || flags != 0x20 || &rx_buf[..len] != msg_1_to_0 {
                return false;
            }
        }
        Err(_) => return false,
    }

    // Both rings should be empty again
    if p0.available_rx() != 0 || p1.available_rx() != 0 {
        return false;
    }

    true
}

// Zero-Copy Shared-Memory Packet Descriptors & Batch Ring API

/// Transfer ownership flag: indicates the descriptor currently belongs to the consumer.
pub const DESC_OWNERSHIP_CONSUMER: u32 = 1 << 0;
/// Transfer ownership flag: indicates the descriptor currently belongs to the producer.
pub const DESC_OWNERSHIP_PRODUCER: u32 = 0;
/// Flag indicating end-of-packet/chain.
pub const DESC_FLAG_EOP: u32 = 1 << 1;
/// Flag indicating zero-copy out-of-band buffer reference (physical or shared memory offset).
pub const DESC_FLAG_ZERO_COPY_REF: u32 = 1 << 2;

/// Fixed-size 32-byte zero-copy shared-memory packet descriptor.
/// Enables descriptors to transfer buffer ownership across address spaces without
#[repr(C, align(32))]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PacketDescriptor {
    /// Buffer offset in shared memory region or physical address.
    pub buffer_addr: u64,
    /// Length of payload in bytes.
    pub length: u32,
    /// Ownership and transfer control flags (`DESC_OWNERSHIP_*`, `DESC_FLAG_*`).
    pub flags: u32,
    /// Application-defined transaction or sequence identifier.
    pub cookie: u64,
    /// Reserved for hardware DMA / alignment padding.
    pub reserved: u64,
}

impl PacketDescriptor {
    pub const fn empty() -> Self {
        Self {
            buffer_addr: 0,
            length: 0,
            flags: 0,
            cookie: 0,
            reserved: 0,
        }
    }

    pub const fn new(buffer_addr: u64, length: u32, flags: u32, cookie: u64) -> Self {
        Self {
            buffer_addr,
            length,
            flags,
            cookie,
            reserved: 0,
        }
    }

    #[inline(always)]
    pub fn is_owned_by_consumer(&self) -> bool {
        (self.flags & DESC_OWNERSHIP_CONSUMER) != 0
    }

    #[inline(always)]
    pub fn is_owned_by_producer(&self) -> bool {
        !self.is_owned_by_consumer()
    }

    #[inline(always)]
    pub fn transfer_to_consumer(&mut self) {
        self.flags |= DESC_OWNERSHIP_CONSUMER;
    }

    #[inline(always)]
    pub fn transfer_to_producer(&mut self) {
        self.flags &= !DESC_OWNERSHIP_CONSUMER;
    }
}

/// Circular descriptor ring tracking zero-copy buffer handoffs.
#[repr(C, align(64))]
pub struct DescriptorRing<const SLOTS: usize> {
    pub head: CachelineAlignedAtomicUsize,
    pub tail: CachelineAlignedAtomicUsize,
    pub descriptors: [PacketDescriptor; SLOTS],
}

impl<const SLOTS: usize> DescriptorRing<SLOTS> {
    pub const fn new() -> Self {
        Self {
            head: CachelineAlignedAtomicUsize::new(0),
            tail: CachelineAlignedAtomicUsize::new(0),
            descriptors: [PacketDescriptor::empty(); SLOTS],
        }
    }

    #[inline(always)]
    pub const fn capacity(&self) -> usize {
        SLOTS
    }

    #[inline(always)]
    pub fn available_read(&self) -> usize {
        let t = self.tail.value.load(Ordering::Acquire);
        let h = self.head.value.load(Ordering::Relaxed);
        t.wrapping_sub(h)
    }

    #[inline(always)]
    pub fn available_write(&self) -> usize {
        SLOTS.saturating_sub(self.available_read())
    }

    /// Enqueues a batch of zero-copy packet descriptors in a single atomic sequence.
    pub fn enqueue_batch(&mut self, batch: &[PacketDescriptor]) -> usize {
        let count = core::cmp::min(batch.len(), self.available_write());
        if count == 0 {
            return 0;
        }

        let current_tail = self.tail.value.load(Ordering::Relaxed);
        for i in 0..count {
            let slot = (current_tail + i) % SLOTS;
            let mut desc = batch[i];
            desc.transfer_to_consumer();
            self.descriptors[slot] = desc;
        }

        self.tail
            .value
            .store(current_tail.wrapping_add(count), Ordering::Release);
        count
    }

    /// Dequeues a batch of zero-copy packet descriptors into `dest_batch`.
    pub fn dequeue_batch(&mut self, dest_batch: &mut [PacketDescriptor]) -> usize {
        let count = core::cmp::min(dest_batch.len(), self.available_read());
        if count == 0 {
            return 0;
        }

        let current_head = self.head.value.load(Ordering::Relaxed);
        for i in 0..count {
            let slot = (current_head + i) % SLOTS;
            let mut desc = self.descriptors[slot];
            desc.transfer_to_producer();
            dest_batch[i] = desc;
        }

        self.head
            .value
            .store(current_head.wrapping_add(count), Ordering::Release);
        count
    }
}

/// Unit test verifying zero-copy descriptor ownership transfer and batch operations.
pub fn test_descriptor_ring_batch() -> bool {
    static mut DESC_RING: DescriptorRing<16> = DescriptorRing::new();
    let ring = unsafe { &mut *core::ptr::addr_of_mut!(DESC_RING) };

    if ring.available_read() != 0 || ring.available_write() != 16 {
        return false;
    }

    let mut send_batch = [PacketDescriptor::empty(); 4];
    for i in 0..4 {
        send_batch[i] = PacketDescriptor::new(
            0x0000_7FFF_1000_0000 + (i as u64) * 4096,
            4096,
            DESC_FLAG_ZERO_COPY_REF | DESC_FLAG_EOP,
            100 + (i as u64),
        );
    }

    let enqueued = ring.enqueue_batch(&send_batch);
    if enqueued != 4 || ring.available_read() != 4 || ring.available_write() != 12 {
        return false;
    }

    let mut recv_batch = [PacketDescriptor::empty(); 4];
    let dequeued = ring.dequeue_batch(&mut recv_batch);
    if dequeued != 4 || ring.available_read() != 0 || ring.available_write() != 16 {
        return false;
    }

    for i in 0..4 {
        if recv_batch[i].buffer_addr != send_batch[i].buffer_addr {
            return false;
        }
        if recv_batch[i].cookie != send_batch[i].cookie {
            return false;
        }
        if !recv_batch[i].is_owned_by_producer() {
            return false;
        }
    }

    true
}
