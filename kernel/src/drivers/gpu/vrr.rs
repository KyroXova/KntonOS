//! Hardware Variable Refresh Rate (VRR / FreeSync / Adaptive-Sync) Subsystem.
//!
//! Provides EDID range limits parsing, dynamic refresh rate range calculation,
//! and dynamic VBLANK timing state management.

use super::drm::DrmModeInfo;

// -----------------------------------------------------------------------------
// EDID Range Limits & VRR Capability Descriptors
// -----------------------------------------------------------------------------

pub const EDID_BLOCK_SIZE: usize = 128;
pub const EDID_DESCRIPTOR_RANGE_LIMITS_TAG: u8 = 0xFD;

/// Parsed display range limits from an EDID Display Range Limits descriptor (Tag 0xFD).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EdidRangeLimits {
    pub min_v_rate_hz: u16,
    pub max_v_rate_hz: u16,
    pub min_h_rate_khz: u16,
    pub max_h_rate_khz: u16,
    pub max_pixel_clock_khz: u32,
    pub supports_continuous_frequency: bool,
}

impl EdidRangeLimits {
    pub const fn empty() -> Self {
        Self {
            min_v_rate_hz: 0,
            max_v_rate_hz: 0,
            min_h_rate_khz: 0,
            max_h_rate_khz: 0,
            max_pixel_clock_khz: 0,
            supports_continuous_frequency: false,
        }
    }
}

/// Parses an EDID 1.3/1.4 128-byte block to locate and extract the Display Range Limits descriptor.
pub fn parse_edid_vrr_descriptor(edid: &[u8]) -> Option<EdidRangeLimits> {
    if edid.len() < EDID_BLOCK_SIZE {
        return None;
    }

    // Verify EDID Header: 00 FF FF FF FF FF FF 00
    let valid_header = edid[0] == 0x00
        && edid[1] == 0xFF
        && edid[2] == 0xFF
        && edid[3] == 0xFF
        && edid[4] == 0xFF
        && edid[5] == 0xFF
        && edid[6] == 0xFF
        && edid[7] == 0x00;

    if !valid_header {
        return None;
    }

    // Verify EDID Checksum (all 128 bytes sum to 0 mod 256)
    let mut checksum: u8 = 0;
    for &b in &edid[..EDID_BLOCK_SIZE] {
        checksum = checksum.wrapping_add(b);
    }
    if checksum != 0 {
        return None;
    }

    // Iterate over the 4 descriptor blocks starting at offset 54, 72, 90, 108
    let descriptor_offsets = [54, 72, 90, 108];
    for &offset in &descriptor_offsets {
        let block = &edid[offset..offset + 18];

        // A display descriptor has pixels clock = 0 (bytes 0..2 are 0)
        if block[0] == 0 && block[1] == 0 && block[2] == 0 {
            let tag = block[3];
            if tag == EDID_DESCRIPTOR_RANGE_LIMITS_TAG {
                let min_v = block[5] as u16;
                let max_v = block[6] as u16;
                let min_h = block[7] as u16;
                let max_h = block[8] as u16;
                let max_clock_10mhz = block[9] as u32;

                // Byte 10: flags (bit 0 = default GTF, bit 2 = CV-T, 0x04 / extended flags)
                let flags = block[10];
                let continuous = flags == 0x04 || (max_v > min_v);

                return Some(EdidRangeLimits {
                    min_v_rate_hz: min_v,
                    max_v_rate_hz: max_v,
                    min_h_rate_khz: min_h,
                    max_h_rate_khz: max_h,
                    max_pixel_clock_khz: max_clock_10mhz * 10_000,
                    supports_continuous_frequency: continuous,
                });
            }
        }
    }

    None
}

// -----------------------------------------------------------------------------
// VRR Dynamic Refresh Rate Range
// -----------------------------------------------------------------------------

/// Variable Refresh Rate range boundary constraints for a video mode and display panel.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VrrRange {
    pub min_refresh_hz: u32,
    pub max_refresh_hz: u32,
    pub min_vtotal: u32,
    pub max_vtotal: u32,
    pub nominal_vtotal: u32,
    pub lfc_supported: bool,
}

impl VrrRange {
    pub const fn empty() -> Self {
        Self {
            min_refresh_hz: 60,
            max_refresh_hz: 60,
            min_vtotal: 0,
            max_vtotal: 0,
            nominal_vtotal: 0,
            lfc_supported: false,
        }
    }

    /// Constructs a VRR range from the nominal video mode and display limits.
    pub fn from_mode_and_limits(mode: &DrmModeInfo, limits: &EdidRangeLimits) -> Self {
        let min_refresh = (limits.min_v_rate_hz as u32).max(30);
        let max_refresh = (limits.max_v_rate_hz as u32).max(min_refresh).max(mode.vrefresh);

        let pixel_clock_hz = (mode.clock as u64) * 1000;
        let htotal = mode.htotal as u64;

        // At max_refresh_hz: vtotal is at its minimum
        let min_vtotal = if max_refresh > 0 && htotal > 0 {
            (pixel_clock_hz / (htotal * max_refresh as u64)) as u32
        } else {
            mode.vtotal as u32
        };

        // At min_refresh_hz: vtotal is at its maximum
        let max_vtotal = if min_refresh > 0 && htotal > 0 {
            (pixel_clock_hz / (htotal * min_refresh as u64)) as u32
        } else {
            mode.vtotal as u32
        };

        // LFC is supported if the maximum refresh rate is at least 2x the minimum refresh rate (e.g. 48 to 144 Hz)
        let lfc_supported = max_refresh >= (2 * min_refresh);

        Self {
            min_refresh_hz: min_refresh,
            max_refresh_hz: max_refresh,
            min_vtotal: min_vtotal.min(mode.vtotal as u32),
            max_vtotal: max_vtotal.max(mode.vtotal as u32),
            nominal_vtotal: mode.vtotal as u32,
            lfc_supported,
        }
    }

    #[inline(always)]
    pub fn is_in_range(&self, refresh_hz: u32) -> bool {
        refresh_hz >= self.min_refresh_hz && refresh_hz <= self.max_refresh_hz
    }

    #[inline(always)]
    pub fn clamp_refresh(&self, refresh_hz: u32) -> u32 {
        if refresh_hz < self.min_refresh_hz {
            self.min_refresh_hz
        } else if refresh_hz > self.max_refresh_hz {
            self.max_refresh_hz
        } else {
            refresh_hz
        }
    }

    /// Computes the exact vertical total line count required for a given target refresh rate.
    pub fn vtotal_for_refresh(&self, pixel_clock_khz: u32, htotal: u32, refresh_hz: u32) -> u32 {
        let clamped_hz = self.clamp_refresh(refresh_hz);
        let pixel_clock_hz = (pixel_clock_khz as u64) * 1000;
        let htotal_u64 = htotal as u64;

        if htotal_u64 == 0 || clamped_hz == 0 {
            return self.nominal_vtotal;
        }

        let calculated_vtotal = (pixel_clock_hz / (htotal_u64 * clamped_hz as u64)) as u32;
        calculated_vtotal.clamp(self.min_vtotal, self.max_vtotal)
    }

    /// Computes the effective refresh rate in Hz for a given vertical total line count.
    pub fn refresh_for_vtotal(&self, pixel_clock_khz: u32, htotal: u32, vtotal: u32) -> u32 {
        let pixel_clock_hz = (pixel_clock_khz as u64) * 1000;
        let denom = (htotal as u64) * (vtotal as u64);

        if denom == 0 {
            return self.min_refresh_hz;
        }

        ((pixel_clock_hz + denom / 2) / denom) as u32
    }
}

// -----------------------------------------------------------------------------
// VRR Timing State
// -----------------------------------------------------------------------------

/// Dynamic hardware timing state modulated across frames during VRR presentation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VrrTimingState {
    pub enabled: bool,
    pub range: VrrRange,
    pub current_vtotal: u32,
    pub nominal_vtotal: u32,
    pub base_front_porch: u32,
    pub current_front_porch: u32,
    pub target_refresh_hz: u32,
    pub lfc_active: bool,
    pub frame_multiplier: u32,
}

impl VrrTimingState {
    pub const fn new(range: VrrRange, base_front_porch: u32) -> Self {
        Self {
            enabled: false,
            range,
            current_vtotal: range.nominal_vtotal,
            nominal_vtotal: range.nominal_vtotal,
            base_front_porch,
            current_front_porch: base_front_porch,
            target_refresh_hz: range.max_refresh_hz,
            lfc_active: false,
            frame_multiplier: 1,
        }
    }

    pub fn enable(&mut self) {
        self.enabled = true;
    }

    pub fn disable(&mut self) {
        self.enabled = false;
        self.current_vtotal = self.nominal_vtotal;
        self.current_front_porch = self.base_front_porch;
        self.lfc_active = false;
        self.frame_multiplier = 1;
    }

    /// Updates target refresh rate and modulates the vertical front porch lines.
    pub fn set_target_refresh(&mut self, refresh_hz: u32, htotal: u32, clock_khz: u32) {
        self.target_refresh_hz = refresh_hz;
        if !self.enabled {
            self.current_vtotal = self.nominal_vtotal;
            self.current_front_porch = self.base_front_porch;
            return;
        }

        let new_vtotal = self.range.vtotal_for_refresh(clock_khz, htotal, refresh_hz);
        self.current_vtotal = new_vtotal;

        // Modulation: front porch expands by (new_vtotal - nominal_vtotal)
        let delta = (new_vtotal as i32) - (self.nominal_vtotal as i32);
        let adjusted_porch = (self.base_front_porch as i32) + delta;
        self.current_front_porch = adjusted_porch.max(1) as u32;
    }
}

// -----------------------------------------------------------------------------
// Unit Verification for EDID VRR Descriptor & Dynamic Ranges
// -----------------------------------------------------------------------------

/// Unit test verifying EDID Range Limits parsing and VRR dynamic refresh range calculations.
pub fn verify_vrr_descriptor_and_ranges() -> bool {
    // 1. Build a synthetic valid 128-byte EDID block containing Tag 0xFD Display Range Limits (48 - 144 Hz)
    let mut edid = [0u8; EDID_BLOCK_SIZE];
    // Header: 00 FF FF FF FF FF FF 00
    edid[0] = 0x00;
    edid[1] = 0xFF;
    edid[2] = 0xFF;
    edid[3] = 0xFF;
    edid[4] = 0xFF;
    edid[5] = 0xFF;
    edid[6] = 0xFF;
    edid[7] = 0x00;

    // Display Range Limits Descriptor in block 1 (offset 54)
    let desc_offset = 54;
    edid[desc_offset] = 0x00;     // Flag
    edid[desc_offset + 1] = 0x00;
    edid[desc_offset + 2] = 0x00; // Reserved
    edid[desc_offset + 3] = 0xFD; // Tag: Range Limits
    edid[desc_offset + 4] = 0x00;
    edid[desc_offset + 5] = 48;   // Min Vertical Rate: 48 Hz
    edid[desc_offset + 6] = 144;  // Max Vertical Rate: 144 Hz
    edid[desc_offset + 7] = 30;   // Min Horizontal: 30 kHz
    edid[desc_offset + 8] = 160;  // Max Horizontal: 160 kHz
    edid[desc_offset + 9] = 36;   // Max Pixel Clock: 36 * 10 MHz = 360 MHz
    edid[desc_offset + 10] = 0x04;// Continuous Frequency (Adaptive-Sync)

    // Compute checksum so sum(edid) % 256 == 0
    let mut sum: u8 = 0;
    for i in 0..127 {
        sum = sum.wrapping_add(edid[i]);
    }
    edid[127] = (0u8).wrapping_sub(sum);

    // 2. Parse EDID block
    let parsed = parse_edid_vrr_descriptor(&edid).expect("Failed to parse valid EDID VRR descriptor");
    assert_eq!(parsed.min_v_rate_hz, 48);
    assert_eq!(parsed.max_v_rate_hz, 144);
    assert_eq!(parsed.min_h_rate_khz, 30);
    assert_eq!(parsed.max_h_rate_khz, 160);
    assert_eq!(parsed.max_pixel_clock_khz, 360_000);
    assert!(parsed.supports_continuous_frequency);

    // 3. Construct VRR range for 1920x1080@144Hz timing mode
    // Mode: clock = 356400 kHz, htotal = 2200, vtotal = 1125 -> 356400 * 1000 / (2200 * 1125) = 144 Hz
    let mode_1080p_144 = DrmModeInfo::new(
        "1920x1080@144",
        356400,
        1920,
        2008,
        2052,
        2200,
        1080,
        1084,
        1089,
        1125,
        144,
        0,
        0,
    );

    let vrr_range = VrrRange::from_mode_and_limits(&mode_1080p_144, &parsed);
    assert_eq!(vrr_range.min_refresh_hz, 48);
    assert_eq!(vrr_range.max_refresh_hz, 144);
    assert_eq!(vrr_range.min_vtotal, 1125); // At 144 Hz
    assert_eq!(vrr_range.max_vtotal, 3375); // At 48 Hz: 356400 * 1000 / (2200 * 48) = 3375
    assert!(vrr_range.lfc_supported);       // 144 >= 2 * 48 -> LFC supported!

    // 4. Test dynamic vtotal calculations
    // At 144 Hz -> vtotal = 1125
    assert_eq!(vrr_range.vtotal_for_refresh(356400, 2200, 144), 1125);
    // At 72 Hz -> vtotal = 2250
    assert_eq!(vrr_range.vtotal_for_refresh(356400, 2200, 72), 2250);
    // At 48 Hz -> vtotal = 3375
    assert_eq!(vrr_range.vtotal_for_refresh(356400, 2200, 48), 3375);

    // Roundtrip refresh rate calculation
    assert_eq!(vrr_range.refresh_for_vtotal(356400, 2200, 1125), 144);
    assert_eq!(vrr_range.refresh_for_vtotal(356400, 2200, 2250), 72);
    assert_eq!(vrr_range.refresh_for_vtotal(356400, 2200, 3375), 48);

    // 5. Test VRR Timing State modulation
    let mut timing_state = VrrTimingState::new(vrr_range, 4);
    assert!(!timing_state.enabled);
    assert_eq!(timing_state.current_front_porch, 4);

    timing_state.enable();
    assert!(timing_state.enabled);

    // Modulate to 60 Hz: vtotal = 2700, delta = 2700 - 1125 = 1575 lines
    timing_state.set_target_refresh(60, 2200, 356400);
    assert_eq!(timing_state.current_vtotal, 2700);
    assert_eq!(timing_state.current_front_porch, 4 + 1575);

    timing_state.disable();
    assert_eq!(timing_state.current_front_porch, 4);

    true
}
