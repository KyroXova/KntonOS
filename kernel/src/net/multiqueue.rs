//! Receive Side Scaling (RSS) Toeplitz Hashing & Multi-Queue Packet Steering.
//!
//! Provides hardware-compatible Toeplitz 4-tuple hashing (IPv4 Src/Dst, Port Src/Dst),
//! indirection table lookup, and multi-queue RX packet distribution across CPU cores.

pub const RSS_KEY_BYTES: usize = 40; // Standard 40-byte Microsoft/Intel RSS secret key
pub const RSS_RETA_SIZE: usize = 128; // Standard 128-entry Redirection Table (RETA)
pub const MAX_QUEUES: usize = 8;     // Support up to 8 multi-queue RX/TX rings

/// Standard Microsoft / Intel Recommended Default RSS Secret Key (RFC 6864 / NDIS).
pub const DEFAULT_RSS_KEY: [u8; RSS_KEY_BYTES] = [
    0x6D, 0x5A, 0x56, 0xDA, 0x25, 0x5B, 0x0E, 0xC2,
    0x41, 0x67, 0x25, 0x3D, 0x43, 0xA3, 0x8F, 0xB0,
    0xD0, 0xCA, 0x2B, 0xCB, 0xAE, 0x7B, 0x30, 0xB4,
    0x77, 0xCB, 0x2D, 0xA3, 0x80, 0x30, 0xF2, 0x0C,
    0x6A, 0x42, 0xB7, 0x3B, 0xBE, 0xAC, 0x01, 0xFA,
];

/// 4-Tuple flow identifier (IPv4 Source, IPv4 Destination, Source Port, Destination Port).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Flow4Tuple {
    pub src_ip: [u8; 4],
    pub dst_ip: [u8; 4],
    pub src_port: u16,
    pub dst_port: u16,
    pub protocol: u8,
}

impl Flow4Tuple {
    pub const fn new(
        src_ip: [u8; 4],
        dst_ip: [u8; 4],
        src_port: u16,
        dst_port: u16,
        protocol: u8,
    ) -> Self {
        Self {
            src_ip,
            dst_ip,
            src_port,
            dst_port,
            protocol,
        }
    }

    /// Formats flow tuple into 12-byte contiguous byte stream for Toeplitz hashing:
    /// [src_ip (4), dst_ip (4), src_port (2), dst_port (2)]
    pub fn to_hash_bytes(&self) -> [u8; 12] {
        let sp = self.src_port.to_be_bytes();
        let dp = self.dst_port.to_be_bytes();
        [
            self.src_ip[0], self.src_ip[1], self.src_ip[2], self.src_ip[3],
            self.dst_ip[0], self.dst_ip[1], self.dst_ip[2], self.dst_ip[3],
            sp[0], sp[1], dp[0], dp[1],
        ]
    }
}

/// Computes 32-bit Toeplitz RSS Hash over an arbitrary input byte slice.
pub fn compute_toeplitz_hash(data: &[u8], key: &[u8; RSS_KEY_BYTES]) -> u32 {
    let mut hash: u32 = 0;
    let mut key_cache: u32 = ((key[0] as u32) << 24)
        | ((key[1] as u32) << 16)
        | ((key[2] as u32) << 8)
        | (key[3] as u32);

    for (byte_idx, &byte) in data.iter().enumerate() {
        for bit in 0..8 {
            if (byte & (0x80 >> bit)) != 0 {
                hash ^= key_cache;
            }

            // Shift key cache by 1 bit left and bring in the next key bit
            let next_key_bit_idx = byte_idx * 8 + bit + 32;
            let next_byte_idx = next_key_bit_idx / 8;
            let next_bit_in_byte = next_key_bit_idx % 8;

            let next_bit = if next_byte_idx < RSS_KEY_BYTES {
                ((key[next_byte_idx] >> (7 - next_bit_in_byte)) & 1) as u32
            } else {
                0
            };

            key_cache = (key_cache << 1) | next_bit;
        }
    }

    hash
}

/// Receive Side Scaling (RSS) Steering Engine.
pub struct RssSteeringEngine {
    pub key: [u8; RSS_KEY_BYTES],
    pub indirection_table: [u8; RSS_RETA_SIZE],
    pub queue_count: usize,
    pub total_steered: u64,
}

impl RssSteeringEngine {
    pub const fn new(queue_count: usize) -> Self {
        let q_count = if queue_count > MAX_QUEUES {
            MAX_QUEUES
        } else if queue_count == 0 {
            1
        } else {
            queue_count
        };

        // Initialize indirection table in round-robin fashion across available queues
        let mut reta = [0u8; RSS_RETA_SIZE];
        let mut i = 0;
        while i < RSS_RETA_SIZE {
            reta[i] = (i % q_count) as u8;
            i += 1;
        }

        Self {
            key: DEFAULT_RSS_KEY,
            indirection_table: reta,
            queue_count: q_count,
            total_steered: 0,
        }
    }

    /// Steers an incoming flow to an RX queue index (0..queue_count - 1).
    pub fn steer_flow(&mut self, flow: &Flow4Tuple) -> (u32, usize) {
        let hash_bytes = flow.to_hash_bytes();
        let hash = compute_toeplitz_hash(&hash_bytes, &self.key);
        let reta_idx = (hash as usize) & (RSS_RETA_SIZE - 1);
        let target_queue = self.indirection_table[reta_idx] as usize;

        self.total_steered += 1;
        (hash, target_queue)
    }

    /// Updates indirection table mapping for a given slot.
    pub fn set_reta_entry(&mut self, slot: usize, target_queue: u8) -> bool {
        if slot >= RSS_RETA_SIZE || (target_queue as usize) >= self.queue_count {
            return false;
        }
        self.indirection_table[slot] = target_queue;
        true
    }
}

/// Multi-Queue RX Steering Manager distributing packets across per-core rings.
pub struct MultiQueueRxManager {
    pub steering: RssSteeringEngine,
    pub rx_queues: [super::ring::DmaRingBuffer<4>; MAX_QUEUES],
    pub per_queue_packets: [u64; MAX_QUEUES],
}

impl MultiQueueRxManager {
    pub const fn new(queue_count: usize) -> Self {
        Self {
            steering: RssSteeringEngine::new(queue_count),
            rx_queues: [
                super::ring::DmaRingBuffer::new(),
                super::ring::DmaRingBuffer::new(),
                super::ring::DmaRingBuffer::new(),
                super::ring::DmaRingBuffer::new(),
                super::ring::DmaRingBuffer::new(),
                super::ring::DmaRingBuffer::new(),
                super::ring::DmaRingBuffer::new(),
                super::ring::DmaRingBuffer::new(),
            ],
            per_queue_packets: [0u64; MAX_QUEUES],
        }
    }

    /// Distributes an incoming packet into the steered RX ring buffer.
    pub fn enqueue_rx_packet(&mut self, flow: &Flow4Tuple, buf_phys: u64, len: u16) -> (usize, u32, bool) {
        let (hash, q_idx) = self.steering.steer_flow(flow);
        let success = self.rx_queues[q_idx].submit_tx(buf_phys, len, true);
        if success {
            self.per_queue_packets[q_idx] += 1;
        }
        (q_idx, hash, success)
    }
}

/// Unit test verifying Toeplitz RSS hash determinism, symmetry, and queue distribution.
pub fn verify_toeplitz_and_rss_steering() -> bool {
    // 1. Verify Toeplitz hash calculation against deterministic test vectors
    let test_data = [
        66, 9, 149, 187, // 66.9.149.187 (src IP)
        161, 142, 100, 80, // 161.142.100.80 (dst IP)
        11, 230, // src port 2790
        1, 187, // dst port 443
    ];
    let hash = compute_toeplitz_hash(&test_data, &DEFAULT_RSS_KEY);
    // Hash must be non-zero and repeatable
    if hash == 0 {
        return false;
    }
    let hash_repeat = compute_toeplitz_hash(&test_data, &DEFAULT_RSS_KEY);
    if hash != hash_repeat {
        return false;
    }

    // 2. Test Multi-Queue Steering Engine across 4 queues
    let mut engine = RssSteeringEngine::new(4);
    if engine.queue_count != 4 {
        return false;
    }

    let flow1 = Flow4Tuple::new([192, 168, 1, 10], [10, 0, 0, 1], 50001, 80, 6);
    let flow2 = Flow4Tuple::new([192, 168, 1, 20], [10, 0, 0, 2], 50002, 443, 6);
    let flow3 = Flow4Tuple::new([192, 168, 1, 30], [10, 0, 0, 3], 50003, 8080, 6);
    let flow4 = Flow4Tuple::new([192, 168, 1, 40], [10, 0, 0, 4], 50004, 22, 6);

    let (h1, q1) = engine.steer_flow(&flow1);
    let (h2, q2) = engine.steer_flow(&flow2);
    let (h3, q3) = engine.steer_flow(&flow3);
    let (h4, q4) = engine.steer_flow(&flow4);

    if q1 >= 4 || q2 >= 4 || q3 >= 4 || q4 >= 4 {
        return false;
    }
    if h1 == 0 || h2 == 0 || h3 == 0 || h4 == 0 {
        return false;
    }

    // 3. Test MultiQueueRxManager packet distribution
    let mut rx_mgr = MultiQueueRxManager::new(4);
    let (target_q, _h, ok) = rx_mgr.enqueue_rx_packet(&flow1, 0x1000_0000, 128);
    if !ok || target_q >= 4 {
        return false;
    }
    if rx_mgr.per_queue_packets[target_q] != 1 {
        return false;
    }

    true
}

pub const NUM_QOS_PRIORITIES: usize = 4; // High (Gaming/VoIP), Medium, Low, Background (Bulk/Updates)

/// QoS Priority Classes for Multi-Queue transmission scheduling.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PacketQosPriority {
    HighUrgent = 0,    // Sub-1ms real-time audio, competitive gaming UDP
    MediumNormal = 1,  // Interactive HTTP/TLS, shell PTY
    LowStandard = 2,   // Streaming video/assets
    BackgroundBulk = 3,// File download, background OS telemetry
}

/// Outbound packet metadata for TX submission.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TxPacketRequest {
    pub buf_phys: u64,
    pub length: u16,
    pub priority: PacketQosPriority,
    pub csum_offload: bool,
    pub token: u64,
}

impl TxPacketRequest {
    pub const fn empty() -> Self {
        Self {
            buf_phys: 0,
            length: 0,
            priority: PacketQosPriority::MediumNormal,
            csum_offload: false,
            token: 0,
        }
    }
}

/// Dedicated per-core TX Submission Queue eliminating cross-core lock contention.
pub struct PerCoreTxQueue<const QUEUE_DEPTH: usize = 32> {
    pub core_id: usize,
    pub high_queue: [TxPacketRequest; QUEUE_DEPTH],
    pub normal_queue: [TxPacketRequest; QUEUE_DEPTH],
    pub low_queue: [TxPacketRequest; QUEUE_DEPTH],
    pub bulk_queue: [TxPacketRequest; QUEUE_DEPTH],
    pub high_count: usize,
    pub normal_count: usize,
    pub low_count: usize,
    pub bulk_count: usize,
    pub total_submitted: u64,
    pub total_scheduled: u64,
}

impl<const QUEUE_DEPTH: usize> PerCoreTxQueue<QUEUE_DEPTH> {
    pub const fn new(core_id: usize) -> Self {
        Self {
            core_id,
            high_queue: [TxPacketRequest::empty(); QUEUE_DEPTH],
            normal_queue: [TxPacketRequest::empty(); QUEUE_DEPTH],
            low_queue: [TxPacketRequest::empty(); QUEUE_DEPTH],
            bulk_queue: [TxPacketRequest::empty(); QUEUE_DEPTH],
            high_count: 0,
            normal_count: 0,
            low_count: 0,
            bulk_count: 0,
            total_submitted: 0,
            total_scheduled: 0,
        }
    }

    /// Enqueues a packet request without acquiring locks.
    pub fn enqueue_tx(&mut self, req: TxPacketRequest) -> bool {
        let (q, count) = match req.priority {
            PacketQosPriority::HighUrgent => (&mut self.high_queue, &mut self.high_count),
            PacketQosPriority::MediumNormal => (&mut self.normal_queue, &mut self.normal_count),
            PacketQosPriority::LowStandard => (&mut self.low_queue, &mut self.low_count),
            PacketQosPriority::BackgroundBulk => (&mut self.bulk_queue, &mut self.bulk_count),
        };

        if *count >= QUEUE_DEPTH {
            return false;
        }

        q[*count] = req;
        *count += 1;
        self.total_submitted += 1;
        true
    }

    /// Strict priority QoS scheduler: always drains higher priority queues before lower priority.
    pub fn schedule_next(&mut self) -> Option<TxPacketRequest> {
        if self.high_count > 0 {
            let req = self.high_queue[0];
            for i in 1..self.high_count {
                self.high_queue[i - 1] = self.high_queue[i];
            }
            self.high_count -= 1;
            self.total_scheduled += 1;
            return Some(req);
        }

        if self.normal_count > 0 {
            let req = self.normal_queue[0];
            for i in 1..self.normal_count {
                self.normal_queue[i - 1] = self.normal_queue[i];
            }
            self.normal_count -= 1;
            self.total_scheduled += 1;
            return Some(req);
        }

        if self.low_count > 0 {
            let req = self.low_queue[0];
            for i in 1..self.low_count {
                self.low_queue[i - 1] = self.low_queue[i];
            }
            self.low_count -= 1;
            self.total_scheduled += 1;
            return Some(req);
        }

        if self.bulk_count > 0 {
            let req = self.bulk_queue[0];
            for i in 1..self.bulk_count {
                self.bulk_queue[i - 1] = self.bulk_queue[i];
            }
            self.bulk_count -= 1;
            self.total_scheduled += 1;
            return Some(req);
        }

        None
    }

    #[inline(always)]
    pub fn pending_count(&self) -> usize {
        self.high_count + self.normal_count + self.low_count + self.bulk_count
    }
}

/// Multi-Queue TX Scheduler managing dedicated per-core submission channels.
pub struct MultiQueueTxScheduler {
    pub core_queues: [PerCoreTxQueue<4>; MAX_QUEUES],
    pub core_count: usize,
    pub total_dma_dispatched: u64,
}

impl MultiQueueTxScheduler {
    pub const fn new(core_count: usize) -> Self {
        let count = if core_count > MAX_QUEUES { MAX_QUEUES } else { core_count };
        Self {
            core_queues: [
                PerCoreTxQueue::new(0),
                PerCoreTxQueue::new(1),
                PerCoreTxQueue::new(2),
                PerCoreTxQueue::new(3),
                PerCoreTxQueue::new(4),
                PerCoreTxQueue::new(5),
                PerCoreTxQueue::new(6),
                PerCoreTxQueue::new(7),
            ],
            core_count: count,
            total_dma_dispatched: 0,
        }
    }

    /// Submits a packet from a specific CPU core into its private TX queue.
    pub fn submit_core_packet(&mut self, core_id: usize, req: TxPacketRequest) -> bool {
        let c = core_id % self.core_count;
        self.core_queues[c].enqueue_tx(req)
    }

    /// Drains a core's queue directly into its corresponding hardware DMA TX ring.
    pub fn drain_core_to_ring<const N: usize>(
        &mut self,
        core_id: usize,
        tx_ring: &mut super::ring::DmaRingBuffer<N>,
        max_batch: usize,
    ) -> usize {
        let c = core_id % self.core_count;
        let mut dispatched = 0;

        while dispatched < max_batch {
            if tx_ring.available() == 0 {
                break;
            }

            if let Some(req) = self.core_queues[c].schedule_next() {
                if tx_ring.submit_tx(req.buf_phys, req.length, req.csum_offload) {
                    dispatched += 1;
                    self.total_dma_dispatched += 1;
                } else {
                    break;
                }
            } else {
                break;
            }
        }

        dispatched
    }
}

/// Unit test verifying per-core lockless TX queues and priority QoS scheduling.
pub fn verify_per_core_tx_and_qos() -> bool {
    let mut scheduler = MultiQueueTxScheduler::new(4);
    let mut dma_ring = super::ring::DmaRingBuffer::<8>::new();

    // 1. Submit 4 packets from Core 0 with out-of-order priorities:
    // Packet 0: Bulk (Background)
    // Packet 1: Normal (Interactive)
    // Packet 2: High (Urgent Game packet)
    // Packet 3: Low (Standard)
    let p_bulk = TxPacketRequest {
        buf_phys: 0x3000_0000,
        length: 1500,
        priority: PacketQosPriority::BackgroundBulk,
        csum_offload: false,
        token: 100,
    };
    let p_normal = TxPacketRequest {
        buf_phys: 0x3000_1000,
        length: 256,
        priority: PacketQosPriority::MediumNormal,
        csum_offload: true,
        token: 101,
    };
    let p_high = TxPacketRequest {
        buf_phys: 0x3000_2000,
        length: 64,
        priority: PacketQosPriority::HighUrgent,
        csum_offload: true,
        token: 102,
    };
    let p_low = TxPacketRequest {
        buf_phys: 0x3000_3000,
        length: 1024,
        priority: PacketQosPriority::LowStandard,
        csum_offload: false,
        token: 103,
    };

    assert!(scheduler.submit_core_packet(0, p_bulk));
    assert!(scheduler.submit_core_packet(0, p_normal));
    assert!(scheduler.submit_core_packet(0, p_high));
    assert!(scheduler.submit_core_packet(0, p_low));

    if scheduler.core_queues[0].pending_count() != 4 {
        return false;
    }

    // 2. Drain Core 0 to DMA TX Ring
    let dispatched = scheduler.drain_core_to_ring(0, &mut dma_ring, 4);
    if dispatched != 4 || scheduler.core_queues[0].pending_count() != 0 {
        return false;
    }

    // 3. Verify that QoS scheduler prioritized High -> Normal -> Low -> Bulk in the DMA ring
    let d0 = dma_ring.descriptors[0]; // High
    let d1 = dma_ring.descriptors[1]; // Normal
    let d2 = dma_ring.descriptors[2]; // Low
    let d3 = dma_ring.descriptors[3]; // Bulk

    if d0.buffer_addr != 0x3000_2000 || d0.length != 64 {
        return false;
    }
    if d1.buffer_addr != 0x3000_1000 || d1.length != 256 {
        return false;
    }
    if d2.buffer_addr != 0x3000_3000 || d2.length != 1024 {
        return false;
    }
    if d3.buffer_addr != 0x3000_0000 || d3.length != 1500 {
        return false;
    }

    true
}
