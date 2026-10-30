//! Real-Time In-Game Performance Telemetry HUD Renderer.
//!
//! Tracks frame presentation timestamps, calculates running framerate (FPS),
//! computes 99th percentile frame time variance (frametime spikes / microstutter),
//! queries GPU engine load and system thermal telemetry, and renders a compact
//! semi-transparent heads-up display into Plane #2.

use alloc::vec::Vec;
use crate::compositor::dirty::Rect;
use crate::compositor::overlay::HardwareOverlayPlane;

// -----------------------------------------------------------------------------
// Constants and Telemetry Bounds
// -----------------------------------------------------------------------------

/// Number of historical frame durations preserved for statistical variance & 99th percentile.
pub const TELEMETRY_HISTORY_CAPACITY: usize = 120;

/// Default HUD Dimensions on Plane #2
pub const HUD_DEFAULT_WIDTH: usize = 280;
pub const HUD_DEFAULT_HEIGHT: usize = 120;

/// Default HUD Colors (ARGB8888)
pub const HUD_COLOR_BACKGROUND: u32 = 0xCC1E1E24; // Fluent Dark Mica (80% opaque)
pub const HUD_COLOR_BORDER: u32 = 0xFF3A3A4C;     // Subtle border outline
pub const HUD_COLOR_TEXT_PRIMARY: u32 = 0xFFFFFFFF; // Pure White
pub const HUD_COLOR_TEXT_ACCENT: u32 = 0xFF00FF7F;  // Spring Green (FPS OK)
pub const HUD_COLOR_TEXT_WARN: u32 = 0xFFFFD700;    // Gold / Warning
pub const HUD_COLOR_TEXT_ALERT: u32 = 0xFFFF4500;   // OrangeRed / Alert

// -----------------------------------------------------------------------------
// Telemetry Data Aggregator
// -----------------------------------------------------------------------------

/// Performance and hardware sensor snapshot.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TelemetrySnapshot {
    pub instantaneous_fps: u32,
    pub average_fps: u32,
    pub frametime_us: u32,
    pub p99_frametime_us: u32,
    pub gpu_load_percent: u8,
    pub gpu_temp_celsius: u8,
    pub cpu_load_percent: u8,
    pub cpu_temp_celsius: u8,
}

impl Default for TelemetrySnapshot {
    fn default() -> Self {
        Self {
            instantaneous_fps: 60,
            average_fps: 60,
            frametime_us: 16_666,
            p99_frametime_us: 17_000,
            gpu_load_percent: 0,
            gpu_temp_celsius: 45,
            cpu_load_percent: 0,
            cpu_temp_celsius: 50,
        }
    }
}

/// Real-time frame statistics engine computing FPS and 99th percentile frametime.
#[derive(Debug, Clone)]
pub struct PerformanceTelemetryTracker {
    pub frame_timestamps_ns: Vec<u64>,
    pub frame_durations_us: Vec<u32>,
    pub last_frame_timestamp_ns: u64,
    pub total_frames_counted: u64,
    pub gpu_load_percent: u8,
    pub gpu_temp_celsius: u8,
    pub cpu_load_percent: u8,
    pub cpu_temp_celsius: u8,
}

impl PerformanceTelemetryTracker {
    pub fn new() -> Self {
        Self {
            frame_timestamps_ns: Vec::with_capacity(TELEMETRY_HISTORY_CAPACITY),
            frame_durations_us: Vec::with_capacity(TELEMETRY_HISTORY_CAPACITY),
            last_frame_timestamp_ns: 0,
            total_frames_counted: 0,
            gpu_load_percent: 0,
            gpu_temp_celsius: 45,
            cpu_load_percent: 0,
            cpu_temp_celsius: 50,
        }
    }

    /// Records the presentation timestamp of a new frame and updates duration history.
    pub fn on_frame_presented(&mut self, timestamp_ns: u64) {
        if self.last_frame_timestamp_ns != 0 && timestamp_ns > self.last_frame_timestamp_ns {
            let duration_ns = timestamp_ns - self.last_frame_timestamp_ns;
            let duration_us = (duration_ns / 1_000) as u32;

            if self.frame_durations_us.len() >= TELEMETRY_HISTORY_CAPACITY {
                self.frame_durations_us.remove(0);
            }
            self.frame_durations_us.push(duration_us);
        }

        self.last_frame_timestamp_ns = timestamp_ns;
        self.total_frames_counted += 1;
    }

    /// Sets simulated or hardware polled hardware sensor readings.
    pub fn update_hardware_metrics(
        &mut self,
        gpu_load: u8,
        gpu_temp: u8,
        cpu_load: u8,
        cpu_temp: u8,
    ) {
        self.gpu_load_percent = gpu_load;
        self.gpu_temp_celsius = gpu_temp;
        self.cpu_load_percent = cpu_load;
        self.cpu_temp_celsius = cpu_temp;
    }

    /// Computes 99th percentile frame time from historical window.
    pub fn compute_p99_frametime_us(&self) -> u32 {
        if self.frame_durations_us.is_empty() {
            return 16_666;
        }

        // Sort copy of durations to extract 99th percentile rank
        let mut sorted = self.frame_durations_us.clone();
        sorted.sort_unstable();

        let rank = (sorted.len() * 99) / 100;
        let p99_idx = rank.min(sorted.len() - 1);
        sorted[p99_idx]
    }

    /// Produces a snapshot of current telemetry metrics.
    pub fn snapshot(&self) -> TelemetrySnapshot {
        let count = self.frame_durations_us.len();
        if count == 0 {
            return TelemetrySnapshot::default();
        }

        let latest_dur_us = *self.frame_durations_us.last().unwrap_or(&16_666);
        let instantaneous_fps = if latest_dur_us > 0 {
            (1_000_000 / latest_dur_us).min(1000)
        } else {
            1000
        };

        let sum_dur: u64 = self.frame_durations_us.iter().map(|&d| d as u64).sum();
        let avg_dur_us = (sum_dur / count as u64) as u32;
        let average_fps = if avg_dur_us > 0 {
            (1_000_000 / avg_dur_us).min(1000)
        } else {
            1000
        };

        let p99_frametime_us = self.compute_p99_frametime_us();

        TelemetrySnapshot {
            instantaneous_fps,
            average_fps,
            frametime_us: latest_dur_us,
            p99_frametime_us,
            gpu_load_percent: self.gpu_load_percent,
            gpu_temp_celsius: self.gpu_temp_celsius,
            cpu_load_percent: self.cpu_load_percent,
            cpu_temp_celsius: self.cpu_temp_celsius,
        }
    }
}

// -----------------------------------------------------------------------------
// In-Game Performance HUD Renderer
// -----------------------------------------------------------------------------

/// Telemetry HUD overlay widget that draws into Plane #2.
pub struct TelemetryHudRenderer {
    pub bounds: Rect,
    pub background_color: u32,
    pub border_color: u32,
}

impl TelemetryHudRenderer {
    pub fn new(x: usize, y: usize, width: usize, height: usize) -> Self {
        Self {
            bounds: Rect::new(x, y, width, height),
            background_color: HUD_COLOR_BACKGROUND,
            border_color: HUD_COLOR_BORDER,
        }
    }

    /// Renders HUD background panel, border, and metric indicators into the overlay plane.
    pub fn render(&self, overlay: &mut HardwareOverlayPlane, telemetry: &TelemetrySnapshot) {
        let x0 = self.bounds.x;
        let y0 = self.bounds.y;
        let w = self.bounds.width;
        let h = self.bounds.height;

        let x1 = x0 + w;
        let y1 = y0 + h;

        // 1. Draw rounded-style rectangular background panel
        for y in y0..y1 {
            for x in x0..x1 {
                let is_border = x == x0 || x == x1 - 1 || y == y0 || y == y1 - 1;
                let color = if is_border {
                    self.border_color
                } else {
                    self.background_color
                };
                overlay.write_pixel(x, y, color);
            }
        }

        // 2. Render color bar indicators representing GPU Load and CPU Load
        // GPU Load bar: line from x = x0 + 10 to x0 + 10 + (gpu_load * 200 / 100) at y = y0 + 35
        let gpu_bar_len = ((telemetry.gpu_load_percent as usize) * (w - 30)) / 100;
        let gpu_color = if telemetry.gpu_load_percent > 90 {
            HUD_COLOR_TEXT_ALERT
        } else if telemetry.gpu_load_percent > 70 {
            HUD_COLOR_TEXT_WARN
        } else {
            HUD_COLOR_TEXT_ACCENT
        };
        for x in (x0 + 15)..(x0 + 15 + gpu_bar_len) {
            overlay.write_pixel(x, y0 + 35, gpu_color);
            overlay.write_pixel(x, y0 + 36, gpu_color);
        }

        // CPU Load bar: line at y = y0 + 75
        let cpu_bar_len = ((telemetry.cpu_load_percent as usize) * (w - 30)) / 100;
        let cpu_color = if telemetry.cpu_load_percent > 90 {
            HUD_COLOR_TEXT_ALERT
        } else if telemetry.cpu_load_percent > 70 {
            HUD_COLOR_TEXT_WARN
        } else {
            HUD_COLOR_TEXT_ACCENT
        };
        for x in (x0 + 15)..(x0 + 15 + cpu_bar_len) {
            overlay.write_pixel(x, y0 + 75, cpu_color);
            overlay.write_pixel(x, y0 + 76, cpu_color);
        }

        // Mark the entire HUD area as dirty
        overlay.mark_dirty_rect(self.bounds);
    }
}

// -----------------------------------------------------------------------------
// Unit Tests & Verification
// -----------------------------------------------------------------------------

/// Unit test verifying telemetry tracking, 99th percentile frametime calculation, and HUD rendering.
pub fn verify_telemetry_hud_tracking() -> bool {
    let mut tracker = PerformanceTelemetryTracker::new();

    // Simulate 100 frames at ~60 FPS (16.6ms / 16,666 µs) with three stutter spikes at 33.3ms (33,333 µs)
    let mut current_ns = 1_000_000_000u64;
    for i in 0..100 {
        let step_us = if i == 50 || i == 75 || i == 90 {
            33_333 // Stutter spike
        } else {
            16_666 // Normal 60 FPS
        };
        current_ns += step_us * 1_000;
        tracker.on_frame_presented(current_ns);
    }

    tracker.update_hardware_metrics(85, 68, 42, 55);

    let snap = tracker.snapshot();

    // Average FPS should be around 58-60 FPS
    if snap.average_fps < 55 || snap.average_fps > 62 {
        return false;
    }

    // 99th percentile frame time should catch the 33,333 µs stutter spike
    if snap.p99_frametime_us != 33_333 {
        return false;
    }

    if snap.gpu_load_percent != 85 || snap.gpu_temp_celsius != 68 {
        return false;
    }

    // Render HUD onto Plane #2
    let mut overlay = HardwareOverlayPlane::new(2, 290, 130);
    overlay.set_visible(true);

    let hud = TelemetryHudRenderer::new(10, 10, HUD_DEFAULT_WIDTH, HUD_DEFAULT_HEIGHT);
    hud.render(&mut overlay, &snap);

    // Verify border pixel and interior background pixel
    let border_px = overlay.read_pixel(10, 10);
    let bg_px = overlay.read_pixel(20, 20);

    if border_px != HUD_COLOR_BORDER || bg_px != HUD_COLOR_BACKGROUND {
        return false;
    }

    // Verify GPU bar pixel rendered inside HUD
    let bar_px = overlay.read_pixel(25, 45); // y0 + 35 = 45
    if bar_px != HUD_COLOR_TEXT_WARN {
        return false; // 85% load -> Warning color
    }

    true
}
