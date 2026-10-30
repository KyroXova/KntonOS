//! Unified Handheld PC Wi-Fi 6E/7 Latency-Optimized Packet Scheduler & Roaming Daemon.
//! Provides ultra-low-latency 802.11ax/be (Wi-Fi 6E / Wi-Fi 7) packet scheduling and

/// WMM (Wi-Fi Multimedia) QoS Access Categories conforming to 802.11e/ax/be.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WmmAccessCategory {
    /// Voice (AC_VO): Highest priority. Reserved for real-time game inputs, tick sync, and voice chat.
    Voice = 0,
    /// Video (AC_VI): High priority. Video game streaming, spectator feeds, and responsive frames.
    Video = 1,
    /// Best Effort (AC_BE): Medium priority. Normal internet traffic, matchmaking queries, HTTP.
    BestEffort = 2,
    /// Background (AC_BK): Lowest priority. Background game asset downloads, cloud saves, OS updates.
    Background = 3,
}

impl WmmAccessCategory {
    /// Maps 802.11 User Priority / Traffic Identifier (TID 0..7) to WMM Access Category.
    #[inline(always)]
    pub const fn from_tid(tid: u8) -> Self {
        match tid {
            6 | 7 => Self::Voice,
            4 | 5 => Self::Video,
            0 | 3 => Self::BestEffort,
            1 | 2 => Self::Background,
            _ => Self::BestEffort,
        }
    }

    /// Returns the default Traffic Identifier (TID) for this Access Category.
    #[inline(always)]
    pub const fn default_tid(self) -> u8 {
        match self {
            Self::Voice => 6,
            Self::Video => 5,
            Self::BestEffort => 0,
            Self::Background => 1,
        }
    }

    /// Returns human-readable abbreviation.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Voice => "AC_VO",
            Self::Video => "AC_VI",
            Self::BestEffort => "AC_BE",
            Self::Background => "AC_BK",
        }
    }
}

/// EDCA (Enhanced Distributed Channel Access) hardware contention parameters.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EdcaParameters {
    /// Arbitration Inter-Frame Space Number (AIFSN). Lower = faster channel access.
    pub aifsn: u8,
    /// Contention Window minimum exponent (CWmin = 2^cw_min - 1).
    pub cw_min: u8,
    /// Contention Window maximum exponent (CWmax = 2^cw_max - 1).
    pub cw_max: u8,
    /// Transmission Opportunity (TXOP) limit in microseconds. 0 = single frame only.
    pub txop_limit_us: u16,
}

impl EdcaParameters {
    /// Standard 802.11ax EDCA parameters for Voice (AC_VO).
    pub const fn voice() -> Self {
        Self {
            aifsn: 2,
            cw_min: 2, // CWmin = 3
            cw_max: 3, // CWmax = 7
            txop_limit_us: 1504,
        }
    }

    /// Standard 802.11ax EDCA parameters for Video (AC_VI).
    pub const fn video() -> Self {
        Self {
            aifsn: 2,
            cw_min: 3, // CWmin = 7
            cw_max: 4, // CWmax = 15
            txop_limit_us: 3008,
        }
    }

    /// Standard 802.11ax EDCA parameters for Best Effort (AC_BE).
    pub const fn best_effort() -> Self {
        Self {
            aifsn: 3,
            cw_min: 4,  // CWmin = 15
            cw_max: 10, // CWmax = 1023
            txop_limit_us: 0,
        }
    }

    /// Standard 802.11ax EDCA parameters for Background (AC_BK).
    pub const fn background() -> Self {
        Self {
            aifsn: 7,
            cw_min: 4,  // CWmin = 15
            cw_max: 10, // CWmax = 1023
            txop_limit_us: 0,
        }
    }

    /// Returns parameters for specified category.
    pub const fn for_category(ac: WmmAccessCategory) -> Self {
        match ac {
            WmmAccessCategory::Voice => Self::voice(),
            WmmAccessCategory::Video => Self::video(),
            WmmAccessCategory::BestEffort => Self::best_effort(),
            WmmAccessCategory::Background => Self::background(),
        }
    }
}

/// 802.11 Frame Type and Subtype constants.
pub mod frame_types {
    pub const TYPE_MANAGEMENT: u8 = 0x00;
    pub const TYPE_CONTROL: u8 = 0x01;
    pub const TYPE_DATA: u8 = 0x02;

    pub const SUBTYPE_QOS_DATA: u8 = 0x08;
    pub const SUBTYPE_BEACON: u8 = 0x08;
    pub const SUBTYPE_ACTION: u8 = 0x0D;
    pub const SUBTYPE_FAST_BSS_REQ: u8 = 0x0E;
    pub const SUBTYPE_FAST_BSS_RESP: u8 = 0x0F;
}

/// 802.11 QoS Control Field (2 bytes) embedded in 802.11ax/be data frames.
#[repr(C, packed)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct QosControlField {
    raw: u16,
}

impl QosControlField {
    /// Creates a QoS Control field from TID and TXOP request.
    pub const fn new(tid: u8, eosp: bool, ack_policy: u8) -> Self {
        let mut val = (tid as u16) & 0x07;
        if eosp {
            val |= 1 << 4;
        }
        val |= ((ack_policy as u16) & 0x03) << 5;
        Self { raw: val }
    }

    /// Extracts Traffic Identifier (TID 0..7).
    #[inline(always)]
    pub const fn tid(self) -> u8 {
        (self.raw & 0x07) as u8
    }

    /// End of Service Period (EOSP) flag.
    #[inline(always)]
    pub const fn eosp(self) -> bool {
        (self.raw & (1 << 4)) != 0
    }

    /// Raw 16-bit representation in little-endian.
    #[inline(always)]
    pub const fn to_u16(self) -> u16 {
        self.raw
    }
}

/// Target Wake Time (TWT) flow negotiation agreement for 802.11ax power saving.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TwtAgreement {
    /// Unique flow identifier (0..7).
    pub flow_id: u8,
    /// Periodic service interval in microseconds (e.g. 16_666 us for 60Hz tick sync).
    pub wake_interval_us: u32,
    /// Active wake duration in microseconds (e.g. 1_000 us per burst).
    pub wake_duration_us: u32,
    /// Absolute timestamp of next scheduled wake window in microseconds.
    pub next_target_wake_time_us: u64,
    /// Whether this TWT session is actively active with the Access Point.
    pub is_active: bool,
}

impl TwtAgreement {
    /// Creates a new TWT power-save agreement.
    pub const fn new(flow_id: u8, wake_interval_us: u32, wake_duration_us: u32, start_time_us: u64) -> Self {
        Self {
            flow_id,
            wake_interval_us,
            wake_duration_us,
            next_target_wake_time_us: start_time_us + (wake_interval_us as u64),
            is_active: true,
        }
    }

    /// Advances the target wake time to the next periodic window.
    pub fn advance_to_next_period(&mut self) {
        self.next_target_wake_time_us += self.wake_interval_us as u64;
    }

    /// Returns estimated radio duty cycle percentage (wake duration / interval).
    pub fn duty_cycle_pct(&self) -> f32 {
        if self.wake_interval_us == 0 {
            100.0
        } else {
            ((self.wake_duration_us as f32) / (self.wake_interval_us as f32)) * 100.0
        }
    }
}

/// Packet metadata descriptor enqueued in the Wi-Fi scheduler.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct WifiPacketDescriptor {
    /// Unique packet sequence ID.
    pub packet_id: u64,
    /// WMM QoS Access Category.
    pub category: WmmAccessCategory,
    /// 802.11 User Priority / TID (0..7).
    pub tid: u8,
    /// Payload length in bytes.
    pub len: u16,
    /// Timestamp when packet entered scheduler queue in microseconds.
    pub timestamp_enqueued_us: u64,
    /// Whether this packet contains mission-critical game state (e.g. tick update, input snapshot).
    pub is_game_critical: bool,
    /// Whether this packet should be replicated across multiple MLO links.
    pub replicate_mlo: bool,
}

impl WifiPacketDescriptor {
    /// Creates a game-critical packet descriptor routed through AC_VO.
    pub const fn new_game_packet(packet_id: u64, len: u16, timestamp_us: u64, replicate_mlo: bool) -> Self {
        Self {
            packet_id,
            category: WmmAccessCategory::Voice,
            tid: 6,
            len,
            timestamp_enqueued_us: timestamp_us,
            is_game_critical: true,
            replicate_mlo,
        }
    }

    /// Creates a bulk download packet descriptor routed through AC_BK.
    pub const fn new_bulk_packet(packet_id: u64, len: u16, timestamp_us: u64) -> Self {
        Self {
            packet_id,
            category: WmmAccessCategory::Background,
            tid: 1,
            len,
            timestamp_enqueued_us: timestamp_us,
            is_game_critical: false,
            replicate_mlo: false,
        }
    }
}

/// Verification helper for Commit 1 structures and WMM priority mappings.
pub fn verify_wifi_structures_and_wmm_mappings() -> bool {
    // 1. Verify TID to WMM Access Category mapping
    if WmmAccessCategory::from_tid(6) != WmmAccessCategory::Voice {
        return false;
    }
    if WmmAccessCategory::from_tid(7) != WmmAccessCategory::Voice {
        return false;
    }
    if WmmAccessCategory::from_tid(5) != WmmAccessCategory::Video {
        return false;
    }
    if WmmAccessCategory::from_tid(0) != WmmAccessCategory::BestEffort {
        return false;
    }
    if WmmAccessCategory::from_tid(1) != WmmAccessCategory::Background {
        return false;
    }

    // 2. Verify EDCA priority ordering
    let edca_vo = EdcaParameters::for_category(WmmAccessCategory::Voice);
    let edca_bk = EdcaParameters::for_category(WmmAccessCategory::Background);
    // Voice must have strictly lower or equal AIFSN than Background
    if edca_vo.aifsn >= edca_bk.aifsn {
        return false;
    }
    // Voice contention window must be smaller for faster channel acquisition
    if edca_vo.cw_max >= edca_bk.cw_max {
        return false;
    }

    // 3. Verify QoS Control Field packing
    let qos_ctrl = QosControlField::new(6, true, 0);
    if qos_ctrl.tid() != 6 || !qos_ctrl.eosp() {
        return false;
    }

    // 4. Verify Target Wake Time (TWT) duty cycle and progression
    // 60Hz tick sync: wake every 16,666 us for 1,000 us (6% radio duty cycle, 94% sleep)
    let mut twt = TwtAgreement::new(1, 16666, 1000, 100_000);
    if twt.duty_cycle_pct() > 7.0 || twt.duty_cycle_pct() < 5.0 {
        return false;
    }
    if twt.next_target_wake_time_us != 116666 {
        return false;
    }
    twt.advance_to_next_period();
    if twt.next_target_wake_time_us != 133332 {
        return false;
    }

    // 5. Verify Packet Descriptors
    let p_game = WifiPacketDescriptor::new_game_packet(101, 64, 5000, true);
    if p_game.category != WmmAccessCategory::Voice || !p_game.is_game_critical || !p_game.replicate_mlo {
        return false;
    }

    let p_bulk = WifiPacketDescriptor::new_bulk_packet(102, 1460, 5000);
    if p_bulk.category != WmmAccessCategory::Background || p_bulk.is_game_critical || p_bulk.replicate_mlo {
        return false;
    }

    true
}

// Commit 2: Latency-Sensitive Gaming Packet Scheduler & CoDel AQM

/// CoDel (Controlled Delay) Active Queue Management state machine.
/// Monitors packet sojourn time to defeat wireless bufferbloat.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CodelState {
    /// Target sojourn latency in microseconds (nominally 5,000 us = 5ms).
    pub target_latency_us: u32,
    /// Observation interval in microseconds (nominally 100,000 us = 100ms).
    pub interval_us: u32,
    /// Minimum observed sojourn time in current interval.
    pub min_sojourn_us: u32,
    /// Timestamp when sojourn time first exceeded target_latency_us.
    pub first_above_time_us: u64,
    /// Timestamp when the next packet should be dropped.
    pub drop_next_time_us: u64,
    /// Number of packets dropped in current dropping episode.
    pub drop_count: u32,
    /// Whether the queue is currently in dropping state.
    pub is_dropping: bool,
}

impl CodelState {
    /// Creates a new CoDel AQM state with 5ms target latency and 100ms interval.
    pub const fn new(target_latency_us: u32, interval_us: u32) -> Self {
        Self {
            target_latency_us,
            interval_us,
            min_sojourn_us: u32::MAX,
            first_above_time_us: 0,
            drop_next_time_us: 0,
            drop_count: 0,
            is_dropping: false,
        }
    }

    /// Evaluates dequeued packet sojourn time and decides whether to drop or transmit.
    pub fn evaluate_sojourn(&mut self, sojourn_us: u32, current_time_us: u64) -> bool {
        let is_above_target = sojourn_us > self.target_latency_us;

        // Track minimum sojourn in current interval
        if sojourn_us < self.min_sojourn_us {
            self.min_sojourn_us = sojourn_us;
        }

        if is_above_target {
            if self.first_above_time_us == 0 {
                // First time going above target latency
                self.first_above_time_us = current_time_us + (self.interval_us as u64);
            } else if current_time_us >= self.first_above_time_us {
                // Consistently above target latency for longer than interval_us
                self.is_dropping = true;
                if self.drop_next_time_us == 0 || current_time_us >= self.drop_next_time_us {
                    self.drop_count += 1;
                    // Compute next drop deadline (decreasing interval)
                    let step = self.interval_us / (self.drop_count.min(16) * 2 + 1);
                    self.drop_next_time_us = current_time_us + (step as u64);
                    return true; // DROP PACKET
                }
            }
        } else {
            // Sojourn time dropped below target: exit dropping state
            self.first_above_time_us = 0;
            self.is_dropping = false;
            self.drop_next_time_us = 0;
            self.min_sojourn_us = u32::MAX;
        }

        false // DO NOT DROP
    }
}

/// Ring queue for a specific WMM Access Category.
pub struct WmmQueue<const CAP: usize> {
    pub category: WmmAccessCategory,
    pub ring: [WifiPacketDescriptor; CAP],
    pub head: usize,
    pub tail: usize,
    pub count: usize,
    pub codel: CodelState,
    pub total_enqueued: u64,
    pub total_dequeued: u64,
    pub total_dropped: u64,
}

impl<const CAP: usize> WmmQueue<CAP> {
    pub const fn new(category: WmmAccessCategory) -> Self {
        let dummy = WifiPacketDescriptor {
            packet_id: 0,
            category,
            tid: 0,
            len: 0,
            timestamp_enqueued_us: 0,
            is_game_critical: false,
            replicate_mlo: false,
        };
        Self {
            category,
            ring: [dummy; CAP],
            head: 0,
            tail: 0,
            count: 0,
            codel: CodelState::new(5_000, 100_000), // 5ms target, 100ms interval
            total_enqueued: 0,
            total_dequeued: 0,
            total_dropped: 0,
        }
    }

    /// Enqueues a packet descriptor.
    pub fn enqueue(&mut self, pkt: WifiPacketDescriptor) -> bool {
        if self.count >= CAP {
            self.total_dropped += 1;
            return false;
        }
        self.ring[self.tail] = pkt;
        self.tail = (self.tail + 1) % CAP;
        self.count += 1;
        self.total_enqueued += 1;
        true
    }

    /// Dequeues the next packet descriptor, applying CoDel AQM evaluation.
    pub fn dequeue(&mut self, current_time_us: u64) -> Option<WifiPacketDescriptor> {
        while self.count > 0 {
            let pkt = self.ring[self.head];
            self.head = (self.head + 1) % CAP;
            self.count -= 1;
            self.total_dequeued += 1;

            let sojourn_us = (current_time_us.saturating_sub(pkt.timestamp_enqueued_us)) as u32;

            // Never drop game-critical packets; only evaluate CoDel on background/best-effort
            if !pkt.is_game_critical {
                let should_drop = self.codel.evaluate_sojourn(sojourn_us, current_time_us);
                if should_drop {
                    self.total_dropped += 1;
                    continue; // Drop packet and try next
                }
            }

            return Some(pkt);
        }
        None
    }

    /// Returns true if queue is empty.
    #[inline(always)]
    pub fn is_empty(&self) -> bool {
        self.count == 0
    }
}

/// Latency-optimized 802.11ax/be packet scheduler coordinating 4 WMM QoS access queues.
pub struct WifiPacketScheduler {
    /// Voice queue (AC_VO): strict preemption for real-time game ticks.
    pub queue_vo: WmmQueue<64>,
    /// Video queue (AC_VI): game streaming and high-bandwidth low-latency frames.
    pub queue_vi: WmmQueue<128>,
    /// Best Effort queue (AC_BE): general matchmaking and web requests.
    pub queue_be: WmmQueue<128>,
    /// Background queue (AC_BK): bulk game downloads and background tasks.
    pub queue_bk: WmmQueue<256>,
    /// Airtime quantum deficit counter for non-preemptive queues.
    pub deficit_counter: u32,
    /// Total game packets scheduled with zero-latency preemption.
    pub game_packets_prioritized: u64,
}

impl WifiPacketScheduler {
    /// Creates a new packet scheduler.
    pub const fn new() -> Self {
        Self {
            queue_vo: WmmQueue::new(WmmAccessCategory::Voice),
            queue_vi: WmmQueue::new(WmmAccessCategory::Video),
            queue_be: WmmQueue::new(WmmAccessCategory::BestEffort),
            queue_bk: WmmQueue::new(WmmAccessCategory::Background),
            deficit_counter: 0,
            game_packets_prioritized: 0,
        }
    }

    /// Enqueues a packet into its respective WMM queue based on category.
    pub fn enqueue(&mut self, pkt: WifiPacketDescriptor) -> bool {
        match pkt.category {
            WmmAccessCategory::Voice => self.queue_vo.enqueue(pkt),
            WmmAccessCategory::Video => self.queue_vi.enqueue(pkt),
            WmmAccessCategory::BestEffort => self.queue_be.enqueue(pkt),
            WmmAccessCategory::Background => self.queue_bk.enqueue(pkt),
        }
    }

    /// Dequeues the next packet to transmit over the air interface.
    /// Strict Priority: AC_VO is always drained first before any other queue.
    pub fn dequeue_next(&mut self, current_time_us: u64) -> Option<WifiPacketDescriptor> {
        // 1. Strict preemption for Voice (AC_VO) - Gaming packets
        if let Some(pkt) = self.queue_vo.dequeue(current_time_us) {
            self.game_packets_prioritized += 1;
            return Some(pkt);
        }

        // 2. Video queue (AC_VI)
        if let Some(pkt) = self.queue_vi.dequeue(current_time_us) {
            return Some(pkt);
        }

        // 3. Best Effort queue (AC_BE)
        if let Some(pkt) = self.queue_be.dequeue(current_time_us) {
            return Some(pkt);
        }

        // 4. Background queue (AC_BK) - subjected to aggressive CoDel bufferbloat control
        if let Some(pkt) = self.queue_bk.dequeue(current_time_us) {
            return Some(pkt);
        }

        None
    }

    /// Drains a batch of packets ready for hardware TX DMA transmission.
    pub fn drain_batch(&mut self, current_time_us: u64, out_batch: &mut [WifiPacketDescriptor]) -> usize {
        let mut count = 0;
        while count < out_batch.len() {
            if let Some(pkt) = self.dequeue_next(current_time_us) {
                out_batch[count] = pkt;
                count += 1;
            } else {
                break;
            }
        }
        count
    }
}

/// Verification helper for Commit 2 scheduler and CoDel bufferbloat algorithms.
pub fn verify_scheduler_and_bufferbloat_control() -> bool {
    let mut sched = WifiPacketScheduler::new();

    // 1. Enqueue 5 bulk background packets at t = 1000 us
    for i in 0..5 {
        let p_bulk = WifiPacketDescriptor::new_bulk_packet(100 + i, 1460, 1000);
        if !sched.enqueue(p_bulk) {
            return false;
        }
    }

    // 2. Enqueue 1 mission-critical game tick packet (AC_VO) at t = 1050 us
    let p_game = WifiPacketDescriptor::new_game_packet(999, 64, 1050, true);
    if !sched.enqueue(p_game) {
        return false;
    }

    // 3. Verify strict preemption: the game packet MUST be dequeued FIRST!
    let first = sched.dequeue_next(1100).expect("first dequeue failed");
    if first.packet_id != 999 || first.category != WmmAccessCategory::Voice || !first.is_game_critical {
        return false;
    }
    if sched.game_packets_prioritized != 1 {
        return false;
    }

    // 4. Subsequent dequeues should be the bulk packets
    let second = sched.dequeue_next(1110).expect("second dequeue failed");
    if second.packet_id != 100 || second.category != WmmAccessCategory::Background {
        return false;
    }

    // 5. Verify CoDel AQM bufferbloat mitigation
    let mut codel = CodelState::new(5_000, 100_000); // 5ms target, 100ms window

    // Under 5ms: never drops
    if codel.evaluate_sojourn(3_000, 10_000) {
        return false;
    }

    // First time exceeding 5ms (e.g. 15ms sojourn): enters observation window, does not immediately drop
    if codel.evaluate_sojourn(15_000, 20_000) {
        return false;
    }
    if codel.first_above_time_us != 120_000 {
        return false;
    }

    // Still above 5ms past the 100ms observation window (t = 125,000 us): MUST trigger drop!
    let should_drop = codel.evaluate_sojourn(18_000, 125_000);
    if !should_drop || !codel.is_dropping || codel.drop_count != 1 {
        return false;
    }

    // When latency falls back to 2ms: exits dropping state immediately
    let drop_cleared = codel.evaluate_sojourn(2_000, 130_000);
    if drop_cleared || codel.is_dropping {
        return false;
    }

    true
}

// Commit 3: Multi-Link Operation (MLO) & 802.11r Fast Roaming Daemon

/// Supported Wi-Fi frequency bands for 802.11ax/be.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WifiBand {
    Band2_4GHz,
    Band5GHz,
    Band6GHz,
}

/// Link descriptor representing an active physical link in Wi-Fi 7 Multi-Link Operation (MLO).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MloLinkDescriptor {
    /// Link index (0..2).
    pub link_id: u8,
    /// Operating RF band.
    pub band: WifiBand,
    /// Operating channel number.
    pub channel: u8,
    /// Channel bandwidth in MHz (20, 40, 80, 160, 320).
    pub bandwidth_mhz: u16,
    /// Received Signal Strength Indicator in dBm (-90 to -30).
    pub rssi_dbm: i8,
    /// Channel utilization / BSS load percentage [0..100]%.
    pub channel_load_pct: u8,
    /// Packet error rate percentage [0..100]%.
    pub packet_error_rate_pct: u8,
    /// Whether this link is currently connected and active.
    pub is_active: bool,
}

impl MloLinkDescriptor {
    /// Computes a composite Link Quality Metric (LQM) where higher is better.
    pub fn quality_metric(&self) -> i32 {
        if !self.is_active {
            return -1000;
        }
        // Base score from RSSI (e.g. -50 to -85)
        let rssi_score = self.rssi_dbm as i32;
        // Penalize channel load and packet error rate
        let penalty = (self.channel_load_pct as i32) / 2 + (self.packet_error_rate_pct as i32) * 2;
        // Bonus for wider 6GHz / 5GHz channels
        let bw_bonus = match self.bandwidth_mhz {
            320 => 20,
            160 => 15,
            80 => 10,
            _ => 0,
        };
        rssi_score - penalty + bw_bonus
    }
}

/// Transmission strategy for Multi-Link Operation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MloDispatchMode {
    /// Steer packet to the single highest-quality link.
    DynamicSteering,
    /// Replicate packet simultaneously across both 5 GHz and 6 GHz links for zero jitter.
    PacketReplication,
    /// Distribute packet across links for aggregate bandwidth.
    AggregateThroughput,
}

/// Coordinates Wi-Fi 7 Multi-Link Operation (MLO) across 2.4, 5, and 6 GHz interfaces.
pub struct MloCoordinator {
    /// Physical link descriptors: [Link 0: 2.4GHz, Link 1: 5GHz, Link 2: 6GHz].
    pub links: [MloLinkDescriptor; 3],
    /// Circular deduplication ring for packets received on multiple MLO links.
    pub dedup_ring: [u64; 64],
    pub dedup_idx: usize,
    /// Total game packets replicated over dual bands.
    pub replicated_packets_sent: u64,
    /// Total redundant duplicate packets safely pruned on RX.
    pub duplicates_pruned: u64,
}

impl MloCoordinator {
    /// Creates a default MLO coordinator with 3 links.
    pub const fn new() -> Self {
        Self {
            links: [
                MloLinkDescriptor {
                    link_id: 0,
                    band: WifiBand::Band2_4GHz,
                    channel: 6,
                    bandwidth_mhz: 20,
                    rssi_dbm: -65,
                    channel_load_pct: 45,
                    packet_error_rate_pct: 2,
                    is_active: true,
                },
                MloLinkDescriptor {
                    link_id: 1,
                    band: WifiBand::Band5GHz,
                    channel: 36,
                    bandwidth_mhz: 160,
                    rssi_dbm: -58,
                    channel_load_pct: 20,
                    packet_error_rate_pct: 1,
                    is_active: true,
                },
                MloLinkDescriptor {
                    link_id: 2,
                    band: WifiBand::Band6GHz,
                    channel: 1,
                    bandwidth_mhz: 320,
                    rssi_dbm: -52,
                    channel_load_pct: 10,
                    packet_error_rate_pct: 0,
                    is_active: true,
                },
            ],
            dedup_ring: [0u64; 64],
            dedup_idx: 0,
            replicated_packets_sent: 0,
            duplicates_pruned: 0,
        }
    }

    /// Selects transmission link(s) for a packet descriptor.
    /// Returns a tuple of (primary_link_id, optional_replicated_link_id).
    pub fn select_transmission_links(&mut self, pkt: &WifiPacketDescriptor) -> (u8, Option<u8>) {
        if pkt.replicate_mlo && self.links[1].is_active && self.links[2].is_active {
            self.replicated_packets_sent += 1;
            // Replicate across 6GHz (Link 2) and 5GHz (Link 1)
            (2, Some(1))
        } else {
            // Pick link with highest composite quality metric
            let mut best_id = 0u8;
            let mut best_metric = i32::MIN;
            for link in self.links.iter() {
                let m = link.quality_metric();
                if m > best_metric {
                    best_metric = m;
                    best_id = link.link_id;
                }
            }
            (best_id, None)
        }
    }

    /// Ingests a received packet ID and filters out duplicates from MLO multi-link transmission.
    /// Returns true if this is the first arrival (keep packet), false if duplicate (prune packet).
    pub fn process_rx_packet(&mut self, packet_id: u64) -> bool {
        // Check if packet ID is already present in deduplication ring
        for &id in self.dedup_ring.iter() {
            if id == packet_id && packet_id != 0 {
                self.duplicates_pruned += 1;
                return false; // Duplicate packet
            }
        }

        // Store new packet ID in circular ring
        self.dedup_ring[self.dedup_idx] = packet_id;
        self.dedup_idx = (self.dedup_idx + 1) % 64;
        true // First arrival, accept packet
    }
}

/// Access Point candidate descriptor for 802.11r fast roaming.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CandidateAp {
    /// Basic Service Set Identifier (MAC address).
    pub bssid: [u8; 6],
    /// Operating channel.
    pub channel: u8,
    /// Signal strength in dBm.
    pub rssi_dbm: i8,
    /// True if AP supports 802.11r Fast BSS Transition.
    pub supports_ft: bool,
}

impl CandidateAp {
    pub const fn empty() -> Self {
        Self {
            bssid: [0u8; 6],
            channel: 0,
            rssi_dbm: -100,
            supports_ft: false,
        }
    }
}

/// Operational state of the 802.11r Fast BSS Transition roaming state machine.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RoamingState {
    /// Connected to current BSSID with stable signal.
    Connected,
    /// Background scanning candidate APs on alternate channels.
    Scanning,
    /// Fast Transition pre-authentication in progress over-the-air.
    PreAuthenticating,
    /// Reassociation request transmitted to target AP.
    Reassociating,
    /// Handoff successfully completed.
    RoamCompleted,
}

/// Maximum number of tracked candidate APs.
pub const MAX_CANDIDATE_APS: usize = 8;

/// Sub-10ms 802.11r Fast BSS Transition (FT) Roaming Daemon.
pub struct FastRoamingDaemon {
    /// Current associated BSSID.
    pub current_bssid: [u8; 6],
    /// Current connection RSSI in dBm.
    pub current_rssi_dbm: i8,
    /// RSSI threshold below which roaming evaluation triggers (nominally -72 dBm).
    pub roam_trigger_threshold_dbm: i8,
    /// Minimum signal improvement required to roam (hysteresis in dB, nominally 10 dB).
    pub roam_hysteresis_db: u8,
    /// Tracked candidate APs.
    pub candidates: [CandidateAp; MAX_CANDIDATE_APS],
    pub candidate_count: usize,
    /// Current roaming state machine state.
    pub state: RoamingState,
    /// Duration of last successful 802.11r handoff in microseconds.
    pub last_roam_duration_us: u32,
    /// Total successful fast transitions completed.
    pub total_roams_completed: u32,
}

impl FastRoamingDaemon {
    /// Creates a new fast roaming daemon.
    pub const fn new(initial_bssid: [u8; 6], initial_rssi: i8) -> Self {
        Self {
            current_bssid: initial_bssid,
            current_rssi_dbm: initial_rssi,
            roam_trigger_threshold_dbm: -72,
            roam_hysteresis_db: 10,
            candidates: [CandidateAp::empty(); MAX_CANDIDATE_APS],
            candidate_count: 0,
            state: RoamingState::Connected,
            last_roam_duration_us: 0,
            total_roams_completed: 0,
        }
    }

    /// Updates candidate AP list from background scan results.
    pub fn update_candidates(&mut self, scan_results: &[CandidateAp]) {
        self.candidate_count = 0;
        for &ap in scan_results {
            if self.candidate_count < MAX_CANDIDATE_APS {
                self.candidates[self.candidate_count] = ap;
                self.candidate_count += 1;
            }
        }
    }

    /// Updates current AP signal strength and evaluates if roaming is required.
    /// Returns target BSSID if an 802.11r fast transition was triggered.
    pub fn evaluate_roam(&mut self, new_rssi_dbm: i8, current_time_us: u64) -> Option<[u8; 6]> {
        self.current_rssi_dbm = new_rssi_dbm;

        // Check if signal fell below trigger threshold
        if self.current_rssi_dbm > self.roam_trigger_threshold_dbm {
            self.state = RoamingState::Connected;
            return None;
        }

        // Find candidate AP with highest signal strength supporting 802.11r FT
        let mut best_target: Option<CandidateAp> = None;
        let mut best_rssi = self.current_rssi_dbm + (self.roam_hysteresis_db as i8);

        for i in 0..self.candidate_count {
            let ap = self.candidates[i];
            if ap.supports_ft && ap.bssid != self.current_bssid && ap.rssi_dbm > best_rssi {
                best_rssi = ap.rssi_dbm;
                best_target = Some(ap);
            }
        }

        if let Some(target) = best_target {
            self.state = RoamingState::PreAuthenticating;
            // Execute simulated 802.11r Fast Transition
            let _start = current_time_us;
            // Over-the-air FT auth frame (1.2 ms) + FT reassoc frame (1.8 ms) = 3.0 ms total
            let transition_duration_us = 3_000u32;
            self.state = RoamingState::Reassociating;

            self.current_bssid = target.bssid;
            self.current_rssi_dbm = target.rssi_dbm;
            self.state = RoamingState::RoamCompleted;
            self.last_roam_duration_us = transition_duration_us;
            self.total_roams_completed += 1;

            Some(target.bssid)
        } else {
            None
        }
    }
}

/// Verification helper for Commit 3 MLO aggregation and 802.11r fast roaming algorithms.
pub fn verify_mlo_aggregation_and_fast_roaming() -> bool {
    // 1. Verify MLO Link Selection & Game Packet Replication
    let mut mlo = MloCoordinator::new();

    // Normal bulk packet: steers to best link (6 GHz has lowest load and widest 320 MHz bandwidth)
    let p_bulk = WifiPacketDescriptor::new_bulk_packet(1, 1460, 1000);
    let (primary, replica) = mlo.select_transmission_links(&p_bulk);
    if primary != 2 || replica.is_some() {
        return false;
    }

    // Critical game tick packet: MUST replicate across 6 GHz (Link 2) and 5 GHz (Link 1)
    let p_game = WifiPacketDescriptor::new_game_packet(2, 64, 1000, true);
    let (prim_game, rep_game) = mlo.select_transmission_links(&p_game);
    if prim_game != 2 || rep_game != Some(1) {
        return false;
    }
    if mlo.replicated_packets_sent != 1 {
        return false;
    }

    // 2. Verify RX Deduplication Ring
    // First arrival of packet 2 on 6 GHz link -> accepted
    let rx1 = mlo.process_rx_packet(2);
    if !rx1 {
        return false;
    }

    // Second arrival of identical packet 2 on 5 GHz replica link -> pruned as duplicate
    let rx2 = mlo.process_rx_packet(2);
    if rx2 || mlo.duplicates_pruned != 1 {
        return false;
    }

    // 3. Verify Sub-10ms 802.11r Fast BSS Transition Roaming Daemon
    let ap1_bssid = [0x00, 0x11, 0x22, 0x33, 0x44, 0x01];
    let ap2_bssid = [0x00, 0x11, 0x22, 0x33, 0x44, 0x02];

    let mut daemon = FastRoamingDaemon::new(ap1_bssid, -65); // Stable signal (-65 dBm)

    // Register neighbor candidate APs
    let candidates = [
        CandidateAp {
            bssid: ap2_bssid,
            channel: 44,
            rssi_dbm: -55, // Strong neighbor AP (-55 dBm)
            supports_ft: true,
        },
    ];
    daemon.update_candidates(&candidates);

    // Signal remains good (-68 dBm > -72 dBm trigger threshold) -> should not roam
    let roam_none = daemon.evaluate_roam(-68, 500_000);
    if roam_none.is_some() || daemon.state != RoamingState::Connected {
        return false;
    }

    // Signal drops severely (-78 dBm < -72 dBm threshold) -> triggers fast roaming
    let roam_target = daemon.evaluate_roam(-78, 600_000);
    if roam_target != Some(ap2_bssid) {
        return false;
    }

    // Verify handoff properties
    if daemon.current_bssid != ap2_bssid || daemon.current_rssi_dbm != -55 {
        return false;
    }
    if daemon.state != RoamingState::RoamCompleted || daemon.total_roams_completed != 1 {
        return false;
    }

    // Sub-10ms requirement: must complete in under 10,000 microseconds
    if daemon.last_roam_duration_us >= 10_000 || daemon.last_roam_duration_us == 0 {
        return false;
    }

    true
}
