//! SDHCI Host Controller Driver & Hardware Abstraction Layer.
//!
//! Provides memory-mapped register access and hardware control for handheld PC
//! MicroSD host controllers (e.g. O2Micro, Realtek RTS5227, Genesys Logic GL9755).

pub use super::sdcard::*;

/// SDHCI Host Controller Instance.
#[derive(Debug)]
pub struct SdhciHost {
    /// Memory-mapped base physical address of the SDHCI controller.
    pub base_addr: usize,
    /// Whether controller operates in 64-bit ADMA2 addressing mode.
    pub is_adma64: bool,
    /// Current bus clock frequency in kilohertz.
    pub current_clock_khz: u32,
    /// Cached physical card insertion status.
    pub card_inserted: bool,
    /// Current operational card state.
    pub card_state: SdCardState,
    /// Assigned Relative Card Address (RCA).
    pub rca: u16,
    /// Capacity in 512-byte blocks.
    pub block_count: u64,
}

impl SdhciHost {
    /// Creates a new SDHCI host controller instance.
    pub const fn new(base_addr: usize) -> Self {
        Self {
            base_addr,
            is_adma64: true,
            current_clock_khz: 400, // 400 kHz identification frequency
            card_inserted: false,
            card_state: SdCardState::Empty,
            rca: 0,
            block_count: 0,
        }
    }
}
