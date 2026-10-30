//! Dynamic VBLANK Presentation Timing & Frame Pacing Engine.
//!
//! Provides nanosecond-precision line timing, frame duration calculations,
//! hardware VBLANK interrupt tracking, dynamic front-porch modulation,
//! and frame presentation pacing queues.

extern crate alloc;
use alloc::vec::Vec;

use super::drm::DrmModeInfo;
use super::vrr::VrrRange;

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

// -----------------------------------------------------------------------------
// Hardware VBLANK Interrupt Tracking
// -----------------------------------------------------------------------------

/// State tracker for hardware VBLANK interrupts, intervals, and drift monitoring.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VblankTracker {
    pub crtc_id: u32,
    pub sequence: u64,
    pub last_timestamp_ns: u64,
    pub last_interval_ns: u64,
    pub avg_interval_ns: u64,
    pub nominal_interval_ns: u64,
    pub vblank_enabled: bool,
    pub drift_detected: bool,
}

impl VblankTracker {
    pub const fn new(crtc_id: u32, nominal_interval_ns: u64) -> Self {
        Self {
            crtc_id,
            sequence: 0,
            last_timestamp_ns: 0,
            last_interval_ns: nominal_interval_ns,
            avg_interval_ns: nominal_interval_ns,
            nominal_interval_ns,
            vblank_enabled: true,
            drift_detected: false,
        }
    }

    /// Invoked by the display engine upon receiving a hardware VBLANK interrupt.
    pub fn on_vblank_interrupt(&mut self, timestamp_ns: u64) {
        self.sequence = self.sequence.wrapping_add(1);

        if self.last_timestamp_ns > 0 && timestamp_ns > self.last_timestamp_ns {
            let interval = timestamp_ns - self.last_timestamp_ns;
            self.last_interval_ns = interval;

            // Exponential moving average: (3 * avg + interval) / 4
            self.avg_interval_ns = (self.avg_interval_ns * 3 + interval) / 4;

            // Drift detection: more than 20% variance from nominal when fixed refresh
            let delta = if interval > self.nominal_interval_ns {
                interval - self.nominal_interval_ns
            } else {
                self.nominal_interval_ns - interval
            };

            self.drift_detected = delta > (self.nominal_interval_ns / 5);
        }

        self.last_timestamp_ns = timestamp_ns;
    }

    #[inline(always)]
    pub fn next_expected_vblank_ns(&self) -> u64 {
        self.last_timestamp_ns + self.avg_interval_ns
    }

    #[inline(always)]
    pub fn time_until_next_vblank_ns(&self, now_ns: u64) -> i64 {
        let next = self.next_expected_vblank_ns();
        (next as i64) - (now_ns as i64)
    }
}

// -----------------------------------------------------------------------------
// Vertical Front-Porch Duration Modulator
// -----------------------------------------------------------------------------

/// Computes the exact vertical front-porch extension lines needed for dynamic VRR frame duration.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FrontPorchModulator {
    pub timing: VblankTimingInfo,
    pub vrr_range: VrrRange,
    pub base_front_porch: u32,
}

impl FrontPorchModulator {
    pub const fn new(timing: VblankTimingInfo, vrr_range: VrrRange, base_front_porch: u32) -> Self {
        Self {
            timing,
            vrr_range,
            base_front_porch,
        }
    }

    /// Calculates required (vtotal, front_porch) for a desired target frame duration in nanoseconds.
    pub fn calculate_porch_for_duration(&self, target_duration_ns: u64) -> (u32, u32) {
        let line_time = self.timing.line_time_ns;
        if line_time == 0 {
            return (self.timing.vtotal, self.base_front_porch);
        }

        let desired_vtotal = ((target_duration_ns + line_time / 2) / line_time) as u32;
        let clamped_vtotal = desired_vtotal.clamp(self.vrr_range.min_vtotal, self.vrr_range.max_vtotal);

        let delta = (clamped_vtotal as i32) - (self.timing.vtotal as i32);
        let adjusted_porch = (self.base_front_porch as i32) + delta;
        let final_porch = adjusted_porch.max(1) as u32;

        (clamped_vtotal, final_porch)
    }

    /// Calculates required (vtotal, front_porch) based on when a frame finished rendering.
    pub fn calculate_porch_for_ready_time(
        &self,
        ready_timestamp_ns: u64,
        last_vblank_ns: u64,
    ) -> (u32, u32) {
        let render_duration = if ready_timestamp_ns > last_vblank_ns {
            ready_timestamp_ns - last_vblank_ns
        } else {
            self.timing.frame_duration_ns
        };

        self.calculate_porch_for_duration(render_duration)
    }
}

// -----------------------------------------------------------------------------
// Frame Pacing Queue
// -----------------------------------------------------------------------------

/// Paced presentation frame candidate queued for hardware scanout.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PacedFrame {
    pub frame_id: u64,
    pub fb_id: u32,
    pub ready_timestamp_ns: u64,
    pub target_present_timestamp_ns: u64,
    pub modulated_vtotal: u32,
    pub modulated_front_porch: u32,
    pub is_presented: bool,
    pub presented_vblank_seq: u64,
}

impl PacedFrame {
    pub const fn new(
        frame_id: u64,
        fb_id: u32,
        ready_timestamp_ns: u64,
        target_present_timestamp_ns: u64,
        vtotal: u32,
        front_porch: u32,
    ) -> Self {
        Self {
            frame_id,
            fb_id,
            ready_timestamp_ns,
            target_present_timestamp_ns,
            modulated_vtotal: vtotal,
            modulated_front_porch: front_porch,
            is_presented: false,
            presented_vblank_seq: 0,
        }
    }
}

pub const MAX_PACED_FRAMES: usize = 16;

/// Queue managing submission deadlines, dynamic front-porch timing, and VBLANK dispatch.
pub struct FramePacingQueue {
    pub frames: Vec<PacedFrame>,
    pub next_frame_id: u64,
    pub frames_presented: u64,
    pub dropped_frames: u64,
}

impl FramePacingQueue {
    pub fn new() -> Self {
        Self {
            frames: Vec::with_capacity(MAX_PACED_FRAMES),
            next_frame_id: 1,
            frames_presented: 0,
            dropped_frames: 0,
        }
    }

    /// Enqueues a render-ready framebuffer with dynamically modulated front porch.
    pub fn schedule_frame(
        &mut self,
        fb_id: u32,
        ready_ts_ns: u64,
        tracker: &VblankTracker,
        modulator: &FrontPorchModulator,
    ) -> u64 {
        let frame_id = self.next_frame_id;
        self.next_frame_id = self.next_frame_id.wrapping_add(1);

        let (vtotal, porch) = modulator.calculate_porch_for_ready_time(ready_ts_ns, tracker.last_timestamp_ns);
        let frame_duration = modulator.timing.line_time_ns * (vtotal as u64);
        let target_present_ts = tracker.last_timestamp_ns + frame_duration;

        let frame = PacedFrame::new(
            frame_id,
            fb_id,
            ready_ts_ns,
            target_present_ts,
            vtotal,
            porch,
        );

        if self.frames.len() >= MAX_PACED_FRAMES {
            // Drop oldest unpresented frame to preserve low input latency
            self.frames.remove(0);
            self.dropped_frames += 1;
        }

        self.frames.push(frame);
        frame_id
    }

    /// Processes a VBLANK interrupt, retiring the active frame and advancing the presentation queue.
    pub fn on_vblank(&mut self, vblank_seq: u64, timestamp_ns: u64) -> Option<PacedFrame> {
        if self.frames.is_empty() {
            return None;
        }

        // Search for the frame matching or earliest ready for presentation at or before this timestamp
        let mut best_idx = None;
        for i in 0..self.frames.len() {
            if !self.frames[i].is_presented && self.frames[i].ready_timestamp_ns <= timestamp_ns {
                best_idx = Some(i);
                break;
            }
        }

        if let Some(idx) = best_idx {
            let mut frame = self.frames.remove(idx);
            frame.is_presented = true;
            frame.presented_vblank_seq = vblank_seq;
            self.frames_presented += 1;
            Some(frame)
        } else {
            None
        }
    }

    pub fn queue_depth(&self) -> usize {
        self.frames.len()
    }
}

// -----------------------------------------------------------------------------
// Unit Verification for VBLANK Tracking & Frame Pacing
// -----------------------------------------------------------------------------

/// Unit test verifying hardware VBLANK interrupt tracking, front-porch modulation, and frame pacing queue.
pub fn verify_vblank_tracking_and_pacing() -> bool {
    // 1. Setup video mode: 1920x1080@144Hz (clock = 356400 kHz, htotal = 2200, vtotal = 1125)
    let mode_144 = DrmModeInfo::new(
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

    let timing = VblankTimingInfo::from_mode(&mode_144);
    // line_time = 2200 * 1_000_000 / 356400 = 6172 ns
    assert_eq!(timing.line_time_ns, 6172);
    // frame_duration = 6172 * 1125 = 6_943_500 ns (~6.94 ms = 144 Hz)
    assert_eq!(timing.frame_duration_ns, 6_943_500);

    // 2. Setup VRR range: 48 Hz - 144 Hz
    let limits = super::vrr::EdidRangeLimits {
        min_v_rate_hz: 48,
        max_v_rate_hz: 144,
        min_h_rate_khz: 30,
        max_h_rate_khz: 160,
        max_pixel_clock_khz: 360_000,
        supports_continuous_frequency: true,
    };
    let vrr_range = VrrRange::from_mode_and_limits(&mode_144, &limits);
    assert_eq!(vrr_range.min_vtotal, 1125);
    assert_eq!(vrr_range.max_vtotal, 3375);

    // 3. Test FrontPorchModulator
    let modulator = FrontPorchModulator::new(timing, vrr_range, 4);

    // Fast render (6.94 ms = 144 Hz) -> minimum vtotal = 1125, front porch = 4 lines
    let (vtotal_144, porch_144) = modulator.calculate_porch_for_duration(6_943_500);
    assert_eq!(vtotal_144, 1125);
    assert_eq!(porch_144, 4);

    // Medium render (13.88 ms = 72 Hz) -> vtotal = 2250, front porch = 4 + (2250 - 1125) = 1129 lines
    let (vtotal_72, porch_72) = modulator.calculate_porch_for_duration(13_887_000);
    assert_eq!(vtotal_72, 2250);
    assert_eq!(porch_72, 1129);

    // Slow render (20.83 ms = 48 Hz) -> maximum vtotal = 3375, front porch = 4 + (3375 - 1125) = 2254 lines
    let (vtotal_48, porch_48) = modulator.calculate_porch_for_duration(20_830_500);
    assert_eq!(vtotal_48, 3375);
    assert_eq!(porch_48, 2254);

    // 4. Test VblankTracker
    let mut tracker = VblankTracker::new(1, timing.frame_duration_ns);
    assert_eq!(tracker.sequence, 0);

    // Fire first VBLANK interrupt at t = 10_000_000 ns
    tracker.on_vblank_interrupt(10_000_000);
    assert_eq!(tracker.sequence, 1);
    assert_eq!(tracker.last_timestamp_ns, 10_000_000);

    // Fire second VBLANK interrupt at t = 16_943_500 ns (6.94 ms interval)
    tracker.on_vblank_interrupt(16_943_500);
    assert_eq!(tracker.sequence, 2);
    assert_eq!(tracker.last_interval_ns, 6_943_500);
    assert_eq!(tracker.next_expected_vblank_ns(), 16_943_500 + tracker.avg_interval_ns);

    // 5. Test FramePacingQueue
    let mut queue = FramePacingQueue::new();
    assert_eq!(queue.queue_depth(), 0);

    // Schedule frame 1 (ready at t = 12_000_000 ns)
    let fid1 = queue.schedule_frame(101, 12_000_000, &tracker, &modulator);
    assert_eq!(fid1, 1);
    assert_eq!(queue.queue_depth(), 1);

    // Schedule frame 2 (ready at t = 15_000_000 ns)
    let fid2 = queue.schedule_frame(102, 15_000_000, &tracker, &modulator);
    assert_eq!(fid2, 2);
    assert_eq!(queue.queue_depth(), 2);

    // Process VBLANK at t = 16_943_500 ns -> Should retire frame 1
    let presented_frame1 = queue.on_vblank(3, 16_943_500).expect("Frame 1 not presented");
    assert_eq!(presented_frame1.frame_id, 1);
    assert_eq!(presented_frame1.fb_id, 101);
    assert_eq!(presented_frame1.presented_vblank_seq, 3);
    assert!(presented_frame1.is_presented);
    assert_eq!(queue.queue_depth(), 1);

    // Process next VBLANK at t = 23_887_000 ns -> Should retire frame 2
    let presented_frame2 = queue.on_vblank(4, 23_887_000).expect("Frame 2 not presented");
    assert_eq!(presented_frame2.frame_id, 2);
    assert_eq!(presented_frame2.fb_id, 102);
    assert_eq!(presented_frame2.presented_vblank_seq, 4);
    assert_eq!(queue.queue_depth(), 0);

    assert_eq!(queue.frames_presented, 2);
    assert_eq!(queue.dropped_frames, 0);

    true
}
