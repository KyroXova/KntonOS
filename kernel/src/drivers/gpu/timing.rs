//! Dynamic VBLANK Presentation Timing & Frame Pacing Engine.
//!
//! Provides nanosecond-precision line timing, frame duration calculations,
//! hardware VBLANK interrupt tracking, and dynamic front-porch modulation.

use super::drm::DrmModeInfo;

/// Nanosecond timing parameters computed from a video mode modeline.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VblankTimingInfo {
    pub pixel_clock_khz: u32,
    pub htotal: u32,
    pub vdisplay: u32,
    pub vtotal: u32,
    pub vrefresh: u32,
    pub line_time_ns: u64,
    pub frame_duration_ns: u64,
}

impl VblankTimingInfo {
    pub fn from_mode(mode: &DrmModeInfo) -> Self {
        let clock = mode.clock;
        let htotal = mode.htotal as u32;
        let vdisplay = mode.vdisplay as u32;
        let vtotal = mode.vtotal as u32;
        let vrefresh = mode.vrefresh;

        let line_time_ns = calculate_line_time_ns(clock, htotal);
        let frame_duration_ns = calculate_frame_duration_ns(clock, htotal, vtotal);

        Self {
            pixel_clock_khz: clock,
            htotal,
            vdisplay,
            vtotal,
            vrefresh,
            line_time_ns,
            frame_duration_ns,
        }
    }
}

/// Calculates the duration of a single horizontal scanline in nanoseconds.
#[inline(always)]
pub fn calculate_line_time_ns(clock_khz: u32, htotal: u32) -> u64 {
    if clock_khz == 0 {
        return 0;
    }
    // line_time_ns = (htotal * 1_000_000_000) / (clock_khz * 1000) = (htotal * 1_000_000) / clock_khz
    ((htotal as u64) * 1_000_000) / (clock_khz as u64)
}

/// Calculates the duration of a full frame in nanoseconds.
#[inline(always)]
pub fn calculate_frame_duration_ns(clock_khz: u32, htotal: u32, vtotal: u32) -> u64 {
    let line_time = calculate_line_time_ns(clock_khz, htotal);
    line_time * (vtotal as u64)
}

/// Calculates the required vertical total line count to achieve a desired frame duration.
#[inline(always)]
pub fn calculate_target_vtotal(clock_khz: u32, htotal: u32, target_duration_ns: u64) -> u32 {
    let line_time = calculate_line_time_ns(clock_khz, htotal);
    if line_time == 0 {
        return 0;
    }
    ((target_duration_ns + line_time / 2) / line_time) as u32
}
