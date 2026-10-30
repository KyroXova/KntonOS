//! Zero-Copy DMA RX/TX Ring Buffer Descriptors & Cacheline-Aligned Rings.
//!
//! Provides hardware-compatible network ring buffer descriptors, 64-byte
//! cacheline alignment to avoid false sharing across CPU cores, hardware/software
//! ownership bits, cyclic head/tail cursors, and zero-copy packet buffer slicing.

pub const DEFAULT_RING_SIZE: usize = 64;
pub const PACKET_BUF_SIZE: usize = 2048; // Standard 2KB buffer per RX/TX packet
pub const CACHELINE_BYTES: usize = 64;

// --- Descriptor Flags ---
pub const DESC_FLAG_OWN_HW: u16 = 1 << 0;     // 1 = Hardware owned, 0 = Software owned
pub const DESC_FLAG_EOP: u16 = 1 << 1;        // End of Packet
pub const DESC_FLAG_SOP: u16 = 1 << 2;        // Start of Packet
pub const DESC_FLAG_CSUM_OFFLOAD: u16 = 1 << 3; // Hardware Checksum Offload
pub const DESC_FLAG_ERR: u16 = 1 << 15;       // Packet Error

/// 16-byte Hardware Network DMA Descriptor matching modern NICs (Intel e1000/e1000e/i210 / VirtIO).
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DmaPacketDescriptor {
    pub buffer_addr: u64,
    pub length: u16,
    pub flags: u16,
    pub status: u16,
    pub vlan_tag: u16,
}

impl DmaPacketDescriptor {
    pub const fn empty() -> Self {
        Self {
            buffer_addr: 0,
            length: 0,
            flags: 0,
            status: 0,
            vlan_tag: 0,
        }
    }

    #[inline(always)]
    pub fn is_hw_owned(&self) -> bool {
        (self.flags & DESC_FLAG_OWN_HW) != 0
    }

    #[inline(always)]
    pub fn set_hw_owned(&mut self, hw: bool) {
        if hw {
            self.flags |= DESC_FLAG_OWN_HW;
        } else {
            self.flags &= !DESC_FLAG_OWN_HW;
        }
    }

    #[inline(always)]
    pub fn is_eop(&self) -> bool {
        (self.flags & DESC_FLAG_EOP) != 0
    }
}

/// Zero-copy packet buffer reference slice.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PacketSlice {
    pub phys_addr: u64,
    pub offset: u16,
    pub length: u16,
    pub checksum_valid: bool,
}

impl PacketSlice {
    pub const fn new(phys_addr: u64, length: u16) -> Self {
        Self {
            phys_addr,
            offset: 0,
            length,
            checksum_valid: false,
        }
    }
}

/// Cacheline-aligned (64-byte) DMA Ring Buffer for RX or TX.
///
/// Ensures head and tail pointers and descriptor arrays reside on distinct
/// cache lines to eliminate false sharing between DMA engines and producer/consumer cores.
#[repr(C, align(64))]
pub struct DmaRingBuffer<const N: usize = DEFAULT_RING_SIZE> {
    pub descriptors: [DmaPacketDescriptor; N],
    pub ring_size: u32,
    pub mask: u32,
    pub head: u32, // Hardware DMA cursor
    pub tail: u32, // Software driver cursor
    pub dropped_packets: u64,
    pub total_processed: u64,
    pub _pad: [u8; 32], // Ensures cacheline separation
}

impl<const N: usize> DmaRingBuffer<N> {
    pub const fn new() -> Self {
        Self {
            descriptors: [DmaPacketDescriptor::empty(); N],
            ring_size: N as u32,
            mask: (N as u32) - 1,
            head: 0,
            tail: 0,
            dropped_packets: 0,
            total_processed: 0,
            _pad: [0u8; 32],
        }
    }

    /// Number of descriptors currently pending hardware processing or completion.
    #[inline(always)]
    pub fn count(&self) -> u32 {
        self.tail.wrapping_sub(self.head)
    }

    /// Available descriptor slots in the ring.
    #[inline(always)]
    pub fn available(&self) -> u32 {
        self.ring_size.saturating_sub(self.count())
    }

    /// Submits a TX packet buffer into the ring for DMA transfer.
    pub fn submit_tx(&mut self, buf_phys: u64, len: u16, csum_offload: bool) -> bool {
        if self.count() >= self.ring_size {
            self.dropped_packets += 1;
            return false;
        }

        let idx = (self.tail & self.mask) as usize;
        let mut flags = DESC_FLAG_OWN_HW | DESC_FLAG_SOP | DESC_FLAG_EOP;
        if csum_offload {
            flags |= DESC_FLAG_CSUM_OFFLOAD;
        }

        self.descriptors[idx] = DmaPacketDescriptor {
            buffer_addr: buf_phys,
            length: len,
            flags,
            status: 0,
            vlan_tag: 0,
        };

        self.tail = self.tail.wrapping_add(1);
        self.total_processed += 1;
        true
    }

    /// Pre-allocates an RX descriptor ready for hardware reception.
    pub fn supply_rx_buffer(&mut self, buf_phys: u64) -> bool {
        if self.count() >= self.ring_size {
            return false;
        }

        let idx = (self.tail & self.mask) as usize;
        self.descriptors[idx] = DmaPacketDescriptor {
            buffer_addr: buf_phys,
            length: PACKET_BUF_SIZE as u16,
            flags: DESC_FLAG_OWN_HW,
            status: 0,
            vlan_tag: 0,
        };

        self.tail = self.tail.wrapping_add(1);
        true
    }

    /// Reaps a completed packet from the ring.
    pub fn reap_completed(&mut self) -> Option<PacketSlice> {
        if self.head == self.tail {
            return None;
        }

        let idx = (self.head & self.mask) as usize;
        let desc = &mut self.descriptors[idx];

        // If still owned by hardware, DMA has not completed
        if desc.is_hw_owned() {
            return None;
        }

        let slice = PacketSlice {
            phys_addr: desc.buffer_addr,
            offset: 0,
            length: desc.length,
            checksum_valid: (desc.flags & DESC_FLAG_CSUM_OFFLOAD) != 0,
        };

        self.head = self.head.wrapping_add(1);
        Some(slice)
    }

    /// Simulates hardware completing a DMA transfer (for self-tests and virtual loopback).
    pub fn simulate_hw_completion(&mut self, actual_length: u16) -> bool {
        if self.head == self.tail {
            return false;
        }

        let idx = (self.head & self.mask) as usize;
        let desc = &mut self.descriptors[idx];
        if !desc.is_hw_owned() {
            return false;
        }

        desc.set_hw_owned(false);
        if actual_length > 0 {
            desc.length = actual_length;
        }
        true
    }
}

/// Unit test verifying DMA ring descriptor structures, cacheline alignment, and zero-copy slicing.
pub fn verify_dma_ring_buffers() -> bool {
    // 1. Verify descriptor size and alignment
    if core::mem::size_of::<DmaPacketDescriptor>() != 16 {
        return false;
    }
    if core::mem::align_of::<DmaRingBuffer<4>>() != 64 {
        return false;
    }

    // 2. Test TX submission ring
    let mut tx_ring = DmaRingBuffer::<4>::new();
    if tx_ring.count() != 0 || tx_ring.available() != 4 {
        return false;
    }

    // Submit 4 packets
    for i in 0..4 {
        let buf = 0x1000_0000 + (i as u64 * 2048);
        if !tx_ring.submit_tx(buf, 128 + i as u16, true) {
            return false;
        }
    }
    if tx_ring.count() != 4 || tx_ring.available() != 0 {
        return false;
    }

    // 5th submit should fail
    if tx_ring.submit_tx(0x9999, 64, false) {
        return false;
    }
    if tx_ring.dropped_packets != 1 {
        return false;
    }

    // 3. Hardware simulation and reaping
    // Reaping without completion returns None because descriptors are HW-owned
    if tx_ring.reap_completed().is_some() {
        return false;
    }

    // Simulate completion for packet 0
    if !tx_ring.simulate_hw_completion(128) {
        return false;
    }
    let p0 = tx_ring.reap_completed().expect("p0 reap failed");
    if p0.phys_addr != 0x1000_0000 || p0.length != 128 || !p0.checksum_valid {
        return false;
    }

    // Simulate completion for remaining 3 packets
    for i in 1..4 {
        tx_ring.simulate_hw_completion(128 + i as u16);
        let p = tx_ring.reap_completed().expect("reap failed");
        if p.length != 128 + i as u16 {
            return false;
        }
    }

    if tx_ring.count() != 0 {
        return false;
    }

    // 4. Test ring wraparound with 2 more packets
    tx_ring.submit_tx(0x2000_0000, 256, false);
    tx_ring.submit_tx(0x2000_1000, 512, false);
    tx_ring.simulate_hw_completion(256);
    tx_ring.simulate_hw_completion(512);

    let wrap0 = tx_ring.reap_completed().expect("wrap0 failed");
    let wrap1 = tx_ring.reap_completed().expect("wrap1 failed");
    if wrap0.length != 256 || wrap1.length != 512 {
        return false;
    }

    true
}
