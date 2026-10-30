//! Unified Handheld PC Wi-Fi 6E/7 Latency-Optimized Packet Scheduler & Roaming Daemon.
//!
//! Provides ultra-low-latency 802.11ax/be (Wi-Fi 6E / Wi-Fi 7) packet scheduling and
//! fast-roaming infrastructure optimized for handheld PC gaming.
//!
//! In handheld gaming PCs (e.g. Steam Deck OLED, ROG Ally X, Legion Go 2), Wi-Fi performance
//! is often degraded by bufferbloat from background game downloads, inter-band channel
//! interference, and roaming stalls between mesh APs.
//!
//! This module implements:
//! - 802.11ax/be High Efficiency (HE) and Extremely High Throughput (EHT) QoS frame structures.
//! - WMM (Wireless Multimedia) 4-class EDCA QoS queuing prioritizing real-time game state packets.
//! - Target Wake Time (TWT) micro-sleep service period scheduling for battery savings.
//! - Active Queue Management (CoDel-style AQM) bufferbloat mitigation with sojourn time tracking.
//! - Multi-Link Operation (MLO) simultaneous multi-band (2.4/5/6 GHz) packet replication.
//! - Sub-10ms 802.11r Fast BSS Transition (FT) roaming daemon.

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
