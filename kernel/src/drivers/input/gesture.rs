//! Multi-Finger Gesture Recognition Engine & Edge-Swipe Detector.
//!
//! Provides kernel-level touch gesture interpretation for handheld gaming PCs:
//! - Taps: SingleTap, DoubleTap, LongPress (>500ms)
//! - Pan & Inertial Momentum: 1-finger drag, 2-finger scroll, Fling velocity
//! - Transformation: 2-finger Pinch-to-zoom ($S = d / d_0$) and 2-finger Rotate
//! - System Task Control: 3-finger swipe (task / window switcher)
//! - Quick-Access Menus: EdgeSwipe originating within screen borders (Left, Right, Top, Bottom)

extern crate alloc;
use alloc::vec::Vec;
use crate::drivers::input::touch::{TouchContact, TouchFrame, ContactStatus, MAX_TOUCH_CONTACTS};
use crate::drivers::input::imu::atan2_f32;
use crate::graphics::sqrt_f32;

// -----------------------------------------------------------------------------
// Gesture Events & Types
// -----------------------------------------------------------------------------

/// Direction of multi-finger swipe or fling.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SwipeDirection {
    Left,
    Right,
    Up,
    Down,
}

/// Border edge of the display screen.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScreenEdge {
    Left,
    Right,
    Top,
    Bottom,
}

/// Gesture events emitted by the gesture recognition engine.
#[derive(Debug, Clone, PartialEq)]
pub enum GestureEvent {
    /// Discrete single tap down and up within duration and distance bounds.
    SingleTap { x: f32, y: f32 },
    /// Double tap within interval and spatial proximity.
    DoubleTap { x: f32, y: f32 },
    /// Stationary press held for longer than long_press_duration_ms.
    LongPress { x: f32, y: f32, duration_ms: u32 },
    /// Continuous 1-finger translation across the screen surface.
    OneFingerPan {
        current_x: f32,
        current_y: f32,
        dx: f32,
        dy: f32,
        velocity_x: f32,
        velocity_y: f32,
    },
    /// Continuous 2-finger scroll translation.
    TwoFingerScroll {
        dx: f32,
        dy: f32,
        velocity_x: f32,
        velocity_y: f32,
    },
    /// Fast flick release carrying inertial velocity.
    Fling {
        velocity_x: f32,
        velocity_y: f32,
        speed: f32,
    },
    /// 2-finger pinch scaling (S = d / d0).
    Pinch {
        scale: f32,
        delta_scale: f32,
        center_x: f32,
        center_y: f32,
    },
    /// 2-finger rotational twist.
    Rotate {
        delta_angle_rad: f32,
        center_x: f32,
        center_y: f32,
    },
    /// 3-finger horizontal or vertical swipe for task / window switching.
    ThreeFingerSwipe {
        direction: SwipeDirection,
        distance_px: f32,
        velocity_px_s: f32,
    },
    /// 1-finger swipe originating from screen bezel edge (for OS quick menus).
    EdgeSwipe {
        edge: ScreenEdge,
        progress: f32,
        is_completed: bool,
    },
}

// -----------------------------------------------------------------------------
// Gesture Engine Configuration
// -----------------------------------------------------------------------------

/// Configuration thresholds for gesture recognition.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GestureConfig {
    /// Maximum duration for a tap in milliseconds (default 250 ms).
    pub tap_max_duration_ms: u32,
    /// Maximum displacement for a tap in pixels (default 15.0 px).
    pub tap_max_distance_px: f32,
    /// Maximum interval between consecutive taps for a double tap (default 300 ms).
    pub double_tap_interval_ms: u32,
    /// Minimum stationary duration for long press in milliseconds (default 500 ms).
    pub long_press_duration_ms: u32,
    /// Displacement threshold to commence pan tracking (default 8.0 px).
    pub pan_threshold_px: f32,
    /// Distance change threshold to commence pinch tracking (default 12.0 px).
    pub pinch_threshold_px: f32,
    /// Screen edge zone width in pixels (default 25.0 px).
    pub edge_zone_width_px: f32,
    /// Edge swipe drag distance required to commit the action (default 80.0 px).
    pub edge_swipe_commit_distance_px: f32,
    /// 3-finger swipe distance threshold (default 40.0 px).
    pub three_finger_swipe_threshold_px: f32,
    /// Minimum flick speed in pixels/second to trigger inertial Fling (default 250.0 px/s).
    pub min_fling_speed_px_s: f32,
    /// Display width in pixels (default 1280.0).
    pub display_width: f32,
    /// Display height in pixels (default 800.0).
    pub display_height: f32,
}

impl GestureConfig {
    pub const fn default_handheld() -> Self {
        Self {
            tap_max_duration_ms: 250,
            tap_max_distance_px: 15.0,
            double_tap_interval_ms: 300,
            long_press_duration_ms: 500,
            pan_threshold_px: 8.0,
            pinch_threshold_px: 12.0,
            edge_zone_width_px: 25.0,
            edge_swipe_commit_distance_px: 80.0,
            three_finger_swipe_threshold_px: 40.0,
            min_fling_speed_px_s: 250.0,
            display_width: 1280.0,
            display_height: 800.0,
        }
    }
}

// -----------------------------------------------------------------------------
// Gesture Engine
// -----------------------------------------------------------------------------

/// Kernel-level multi-finger gesture recognition engine.
pub struct GestureEngine {
    /// Engine configuration.
    pub config: GestureConfig,

    // 1-Finger Tracking State
    finger1_start_x: f32,
    finger1_start_y: f32,
    finger1_last_x: f32,
    finger1_last_y: f32,
    finger1_start_time_us: u64,
    finger1_is_panning: bool,
    finger1_is_edge_swipe: bool,
    finger1_edge_origin: Option<ScreenEdge>,
    finger1_long_press_emitted: bool,

    // Tap & Double Tap History
    last_tap_time_us: u64,
    last_tap_x: f32,
    last_tap_y: f32,

    // 2-Finger Tracking State
    two_finger_initial_dist: f32,
    two_finger_last_dist: f32,
    two_finger_last_center_x: f32,
    two_finger_last_center_y: f32,
    two_finger_last_angle: f32,
    two_finger_is_pinching: bool,
    two_finger_is_scrolling: bool,

    // 3-Finger Tracking State
    three_finger_start_x: f32,
    three_finger_start_y: f32,
    three_finger_is_swiping: bool,
}

impl GestureEngine {
    /// Creates a new gesture engine with specified configuration.
    pub fn new(config: GestureConfig) -> Self {
        Self {
            config,
            finger1_start_x: 0.0,
            finger1_start_y: 0.0,
            finger1_last_x: 0.0,
            finger1_last_y: 0.0,
            finger1_start_time_us: 0,
            finger1_is_panning: false,
            finger1_is_edge_swipe: false,
            finger1_edge_origin: None,
            finger1_long_press_emitted: false,

            last_tap_time_us: 0,
            last_tap_x: 0.0,
            last_tap_y: 0.0,

            two_finger_initial_dist: 0.0,
            two_finger_last_dist: 0.0,
            two_finger_last_center_x: 0.0,
            two_finger_last_center_y: 0.0,
            two_finger_last_angle: 0.0,
            two_finger_is_pinching: false,
            two_finger_is_scrolling: false,

            three_finger_start_x: 0.0,
            three_finger_start_y: 0.0,
            three_finger_is_swiping: false,
        }
    }

    /// Evaluates which screen edge a contact point falls into (if any).
    pub fn detect_screen_edge(&self, x: f32, y: f32) -> Option<ScreenEdge> {
        let edge_w = self.config.edge_zone_width_px;
        if x <= edge_w {
            Some(ScreenEdge::Left)
        } else if x >= (self.config.display_width - edge_w) {
            Some(ScreenEdge::Right)
        } else if y <= edge_w {
            Some(ScreenEdge::Top)
        } else if y >= (self.config.display_height - edge_w) {
            Some(ScreenEdge::Bottom)
        } else {
            None
        }
    }

    /// Ingests a touch frame and processes multi-finger gesture state machines.
    pub fn process_frame(&mut self, frame: &TouchFrame) -> Vec<GestureEvent> {
        let mut events = Vec::new();
        let timestamp_us = frame.timestamp_us;

        // Collect active contacts
        let mut active_contacts = [TouchContact::inactive(0); MAX_TOUCH_CONTACTS];
        let mut count = 0usize;
        for c in &frame.contacts {
            if c.active {
                active_contacts[count] = *c;
                count += 1;
            }
        }

        match count {
            1 => {
                let c = active_contacts[0];
                self.process_one_finger(&c, timestamp_us, &mut events);
                // Reset multi-finger states
                self.two_finger_is_pinching = false;
                self.two_finger_is_scrolling = false;
                self.three_finger_is_swiping = false;
            }
            2 => {
                let c0 = active_contacts[0];
                let c1 = active_contacts[1];
                self.process_two_finger(&c0, &c1, &mut events);
                self.finger1_is_panning = false;
                self.finger1_is_edge_swipe = false;
                self.three_finger_is_swiping = false;
            }
            3 => {
                let c0 = active_contacts[0];
                let c1 = active_contacts[1];
                let c2 = active_contacts[2];
                self.process_three_finger(&c0, &c1, &c2, &mut events);
                self.finger1_is_panning = false;
                self.finger1_is_edge_swipe = false;
                self.two_finger_is_pinching = false;
                self.two_finger_is_scrolling = false;
            }
            0 => {
                // Check if 1-finger released this frame (Up)
                for c in &frame.contacts {
                    if c.status == ContactStatus::Up {
                        self.process_one_finger_release(c, timestamp_us, &mut events);
                        break;
                    }
                }
                // Reset all states
                self.finger1_is_panning = false;
                self.finger1_is_edge_swipe = false;
                self.finger1_edge_origin = None;
                self.finger1_long_press_emitted = false;
                self.two_finger_is_pinching = false;
                self.two_finger_is_scrolling = false;
                self.three_finger_is_swiping = false;
            }
            _ => {
                // 4+ fingers: reset gesture tracking
            }
        }

        events
    }

    /// Handles 1-finger tracking (Tap, Pan, LongPress, EdgeSwipe).
    fn process_one_finger(
        &mut self,
        c: &TouchContact,
        timestamp_us: u64,
        events: &mut Vec<GestureEvent>,
    ) {
        if c.status == ContactStatus::Down {
            self.finger1_start_x = c.filtered_x;
            self.finger1_start_y = c.filtered_y;
            self.finger1_last_x = c.filtered_x;
            self.finger1_last_y = c.filtered_y;
            self.finger1_start_time_us = timestamp_us;
            self.finger1_is_panning = false;
            self.finger1_long_press_emitted = false;

            if let Some(edge) = self.detect_screen_edge(c.filtered_x, c.filtered_y) {
                self.finger1_is_edge_swipe = true;
                self.finger1_edge_origin = Some(edge);
            } else {
                self.finger1_is_edge_swipe = false;
                self.finger1_edge_origin = None;
            }
            return;
        }

        let dx_from_start = c.filtered_x - self.finger1_start_x;
        let dy_from_start = c.filtered_y - self.finger1_start_y;
        let dist_from_start = sqrt_f32(dx_from_start * dx_from_start + dy_from_start * dy_from_start);
        let duration_ms = ((timestamp_us.saturating_sub(self.finger1_start_time_us)) / 1000) as u32;

        let delta_x = c.filtered_x - self.finger1_last_x;
        let delta_y = c.filtered_y - self.finger1_last_y;
        self.finger1_last_x = c.filtered_x;
        self.finger1_last_y = c.filtered_y;

        // Long Press Check
        if !self.finger1_long_press_emitted
            && !self.finger1_is_panning
            && !self.finger1_is_edge_swipe
            && dist_from_start <= self.config.tap_max_distance_px
            && duration_ms >= self.config.long_press_duration_ms
        {
            self.finger1_long_press_emitted = true;
            events.push(GestureEvent::LongPress {
                x: c.filtered_x,
                y: c.filtered_y,
                duration_ms,
            });
            return;
        }

        // Edge Swipe Check
        if self.finger1_is_edge_swipe {
            if let Some(edge) = self.finger1_edge_origin {
                let drag_dist = match edge {
                    ScreenEdge::Left => dx_from_start.max(0.0),
                    ScreenEdge::Right => (-dx_from_start).max(0.0),
                    ScreenEdge::Top => dy_from_start.max(0.0),
                    ScreenEdge::Bottom => (-dy_from_start).max(0.0),
                };

                let progress = (drag_dist / self.config.edge_swipe_commit_distance_px).clamp(0.0, 1.0);
                events.push(GestureEvent::EdgeSwipe {
                    edge,
                    progress,
                    is_completed: progress >= 1.0,
                });
                return;
            }
        }

        // 1-Finger Pan Check
        if dist_from_start > self.config.pan_threshold_px {
            self.finger1_is_panning = true;
            events.push(GestureEvent::OneFingerPan {
                current_x: c.filtered_x,
                current_y: c.filtered_y,
                dx: delta_x,
                dy: delta_y,
                velocity_x: c.velocity_x,
                velocity_y: c.velocity_y,
            });
        }
    }

    /// Handles 1-finger release (SingleTap, DoubleTap, Fling).
    fn process_one_finger_release(
        &mut self,
        c: &TouchContact,
        timestamp_us: u64,
        events: &mut Vec<GestureEvent>,
    ) {
        let dx = c.filtered_x - self.finger1_start_x;
        let dy = c.filtered_y - self.finger1_start_y;
        let dist = sqrt_f32(dx * dx + dy * dy);
        let duration_ms = ((timestamp_us.saturating_sub(self.finger1_start_time_us)) / 1000) as u32;

        if self.finger1_is_edge_swipe {
            if let Some(edge) = self.finger1_edge_origin {
                let drag_dist = match edge {
                    ScreenEdge::Left => dx.max(0.0),
                    ScreenEdge::Right => (-dx).max(0.0),
                    ScreenEdge::Top => dy.max(0.0),
                    ScreenEdge::Bottom => (-dy).max(0.0),
                };
                let progress = (drag_dist / self.config.edge_swipe_commit_distance_px).clamp(0.0, 1.0);
                events.push(GestureEvent::EdgeSwipe {
                    edge,
                    progress,
                    is_completed: progress >= 1.0,
                });
            }
            return;
        }

        // Tap or Double Tap Check
        if !self.finger1_is_panning
            && !self.finger1_long_press_emitted
            && dist <= self.config.tap_max_distance_px
            && duration_ms <= self.config.tap_max_duration_ms
        {
            let interval_since_last_tap = ((timestamp_us.saturating_sub(self.last_tap_time_us)) / 1000) as u32;
            let tap_dx = c.filtered_x - self.last_tap_x;
            let tap_dy = c.filtered_y - self.last_tap_y;
            let tap_dist = sqrt_f32(tap_dx * tap_dx + tap_dy * tap_dy);

            if interval_since_last_tap <= self.config.double_tap_interval_ms && tap_dist <= 30.0 {
                events.push(GestureEvent::DoubleTap {
                    x: c.filtered_x,
                    y: c.filtered_y,
                });
                self.last_tap_time_us = 0; // Reset after double tap
            } else {
                events.push(GestureEvent::SingleTap {
                    x: c.filtered_x,
                    y: c.filtered_y,
                });
                self.last_tap_time_us = timestamp_us;
                self.last_tap_x = c.filtered_x;
                self.last_tap_y = c.filtered_y;
            }
            return;
        }

        // Fling / Inertial Momentum Check
        if self.finger1_is_panning && c.speed >= self.config.min_fling_speed_px_s {
            events.push(GestureEvent::Fling {
                velocity_x: c.velocity_x,
                velocity_y: c.velocity_y,
                speed: c.speed,
            });
        }
    }

    /// Handles 2-finger gestures (Pinch, Rotate, TwoFingerScroll).
    fn process_two_finger(
        &mut self,
        c0: &TouchContact,
        c1: &TouchContact,
        events: &mut Vec<GestureEvent>,
    ) {
        let dx = c1.filtered_x - c0.filtered_x;
        let dy = c1.filtered_y - c0.filtered_y;
        let current_dist = sqrt_f32(dx * dx + dy * dy);
        let center_x = (c0.filtered_x + c1.filtered_x) * 0.5;
        let center_y = (c0.filtered_y + c1.filtered_y) * 0.5;
        let current_angle = atan2_f32(dy, dx);

        if !self.two_finger_is_pinching && !self.two_finger_is_scrolling {
            self.two_finger_initial_dist = current_dist;
            self.two_finger_last_dist = current_dist;
            self.two_finger_last_center_x = center_x;
            self.two_finger_last_center_y = center_y;
            self.two_finger_last_angle = current_angle;

            let dist_change = (current_dist - self.two_finger_initial_dist).abs();
            if dist_change > self.config.pinch_threshold_px {
                self.two_finger_is_pinching = true;
            } else {
                self.two_finger_is_scrolling = true;
            }
            return;
        }

        let delta_dist = (current_dist - self.two_finger_initial_dist).abs();
        if delta_dist > self.config.pinch_threshold_px {
            self.two_finger_is_pinching = true;
        }

        if self.two_finger_is_pinching && self.two_finger_initial_dist > 1.0 {
            let scale = current_dist / self.two_finger_initial_dist;
            let delta_scale = current_dist / self.two_finger_last_dist;
            events.push(GestureEvent::Pinch {
                scale,
                delta_scale,
                center_x,
                center_y,
            });

            let delta_angle = current_angle - self.two_finger_last_angle;
            if delta_angle.abs() > 0.02 {
                events.push(GestureEvent::Rotate {
                    delta_angle_rad: delta_angle,
                    center_x,
                    center_y,
                });
            }
        } else {
            // Two-finger scroll translation
            let scroll_dx = center_x - self.two_finger_last_center_x;
            let scroll_dy = center_y - self.two_finger_last_center_y;
            let avg_vx = (c0.velocity_x + c1.velocity_x) * 0.5;
            let avg_vy = (c0.velocity_y + c1.velocity_y) * 0.5;

            events.push(GestureEvent::TwoFingerScroll {
                dx: scroll_dx,
                dy: scroll_dy,
                velocity_x: avg_vx,
                velocity_y: avg_vy,
            });
        }

        self.two_finger_last_dist = current_dist;
        self.two_finger_last_center_x = center_x;
        self.two_finger_last_center_y = center_y;
        self.two_finger_last_angle = current_angle;
    }

    /// Handles 3-finger gestures (Task/Window Switcher Swipe).
    fn process_three_finger(
        &mut self,
        c0: &TouchContact,
        c1: &TouchContact,
        c2: &TouchContact,
        events: &mut Vec<GestureEvent>,
    ) {
        let avg_x = (c0.filtered_x + c1.filtered_x + c2.filtered_x) / 3.0;
        let avg_y = (c0.filtered_y + c1.filtered_y + c2.filtered_y) / 3.0;
        let avg_vx = (c0.velocity_x + c1.velocity_x + c2.velocity_x) / 3.0;
        let avg_vy = (c0.velocity_y + c1.velocity_y + c2.velocity_y) / 3.0;

        if !self.three_finger_is_swiping {
            self.three_finger_start_x = avg_x;
            self.three_finger_start_y = avg_y;
            self.three_finger_is_swiping = true;
            return;
        }

        let dx = avg_x - self.three_finger_start_x;
        let dy = avg_y - self.three_finger_start_y;
        let thresh = self.config.three_finger_swipe_threshold_px;

        if dx.abs() >= thresh && dx.abs() > dy.abs() {
            let direction = if dx > 0.0 { SwipeDirection::Right } else { SwipeDirection::Left };
            events.push(GestureEvent::ThreeFingerSwipe {
                direction,
                distance_px: dx.abs(),
                velocity_px_s: avg_vx.abs(),
            });
            // Reset start to avoid continuous spam
            self.three_finger_start_x = avg_x;
            self.three_finger_start_y = avg_y;
        } else if dy.abs() >= thresh && dy.abs() > dx.abs() {
            let direction = if dy > 0.0 { SwipeDirection::Down } else { SwipeDirection::Up };
            events.push(GestureEvent::ThreeFingerSwipe {
                direction,
                distance_px: dy.abs(),
                velocity_px_s: avg_vy.abs(),
            });
            self.three_finger_start_x = avg_x;
            self.three_finger_start_y = avg_y;
        }
    }
}

// -----------------------------------------------------------------------------
// Unit Tests & Verification
// -----------------------------------------------------------------------------

/// Unit test verifying multi-finger gesture recognition state machine and edge swipe detector.
pub fn verify_gesture_recognition_and_edge_swipe() -> bool {
    let config = GestureConfig::default_handheld();
    let mut engine = GestureEngine::new(config);

    // 1. Single Tap Test
    // Finger down at (500, 400) at t = 10ms
    let mut frame = TouchFrame::empty();
    frame.timestamp_us = 10_000;
    frame.active_count = 1;
    frame.contacts[0] = TouchContact {
        id: 0,
        x: 500.0,
        y: 400.0,
        filtered_x: 500.0,
        filtered_y: 400.0,
        velocity_x: 0.0,
        velocity_y: 0.0,
        acceleration_x: 0.0,
        acceleration_y: 0.0,
        speed: 0.0,
        norm_x: 500.0 / 1280.0,
        norm_y: 400.0 / 800.0,
        pressure: 0.5,
        major_axis_px: 20.0,
        minor_axis_px: 20.0,
        status: ContactStatus::Down,
        active: true,
        timestamp_us: 10_000,
    };
    let ev1 = engine.process_frame(&frame);
    if !ev1.is_empty() {
        return false; // No tap on down
    }

    // Finger up at (502, 401) at t = 100ms (90ms duration <= 250ms, dist < 15px)
    frame.timestamp_us = 100_000;
    frame.active_count = 0;
    frame.contacts[0].active = false;
    frame.contacts[0].status = ContactStatus::Up;
    frame.contacts[0].x = 502.0;
    frame.contacts[0].y = 401.0;
    frame.contacts[0].filtered_x = 502.0;
    frame.contacts[0].filtered_y = 401.0;

    let ev2 = engine.process_frame(&frame);
    let tap_detected = ev2.iter().any(|e| matches!(e, GestureEvent::SingleTap { x, y } if (*x - 502.0).abs() < 1.0 && (*y - 401.0).abs() < 1.0));
    if !tap_detected {
        return false;
    }

    // 2. Double Tap Test (second tap at 200ms)
    frame.timestamp_us = 200_000;
    frame.active_count = 1;
    frame.contacts[0].active = true;
    frame.contacts[0].status = ContactStatus::Down;
    engine.process_frame(&frame);

    frame.timestamp_us = 250_000;
    frame.active_count = 0;
    frame.contacts[0].active = false;
    frame.contacts[0].status = ContactStatus::Up;
    let ev3 = engine.process_frame(&frame);
    let double_tap_detected = ev3.iter().any(|e| matches!(e, GestureEvent::DoubleTap { .. }));
    if !double_tap_detected {
        return false;
    }

    // 3. 2-Finger Pinch-to-Zoom Test
    // Initial fingers at (400, 300) and (600, 300) -> initial distance = 200 px
    frame.timestamp_us = 400_000;
    frame.active_count = 2;
    frame.contacts[0].active = true;
    frame.contacts[0].status = ContactStatus::Down;
    frame.contacts[0].filtered_x = 400.0;
    frame.contacts[0].filtered_y = 300.0;
    frame.contacts[1].active = true;
    frame.contacts[1].status = ContactStatus::Down;
    frame.contacts[1].filtered_x = 600.0;
    frame.contacts[1].filtered_y = 300.0;
    engine.process_frame(&frame);

    // Spread fingers to (350, 300) and (650, 300) -> distance = 300 px (+100 px pinch out)
    frame.timestamp_us = 450_000;
    frame.contacts[0].status = ContactStatus::Move;
    frame.contacts[0].filtered_x = 350.0;
    frame.contacts[1].status = ContactStatus::Move;
    frame.contacts[1].filtered_x = 650.0;
    let ev4 = engine.process_frame(&frame);
    // Pinch ratio S = 300 / 200 = 1.5
    let pinch_detected = ev4.iter().any(|e| matches!(e, GestureEvent::Pinch { scale, .. } if (*scale - 1.5).abs() < 0.05));
    if !pinch_detected {
        return false;
    }

    // 4. Edge Swipe Test from Left Screen Border (x = 10 <= 25 px edge zone)
    frame.active_count = 1;
    frame.contacts[1].active = false;
    frame.contacts[0].status = ContactStatus::Down;
    frame.contacts[0].filtered_x = 10.0;
    frame.contacts[0].filtered_y = 400.0;
    frame.timestamp_us = 600_000;
    engine.process_frame(&frame);

    // Drag 90 px to the right (x = 100 > 80 px commit threshold)
    frame.contacts[0].status = ContactStatus::Move;
    frame.contacts[0].filtered_x = 100.0;
    frame.timestamp_us = 650_000;
    let ev5 = engine.process_frame(&frame);
    let edge_completed = ev5.iter().any(|e| matches!(e, GestureEvent::EdgeSwipe { edge: ScreenEdge::Left, is_completed: true, .. }));
    if !edge_completed {
        return false;
    }

    // 5. 3-Finger Swipe Test
    frame.active_count = 3;
    frame.contacts[0].active = true;
    frame.contacts[0].status = ContactStatus::Down;
    frame.contacts[0].filtered_x = 400.0;
    frame.contacts[0].filtered_y = 400.0;
    frame.contacts[1].active = true;
    frame.contacts[1].status = ContactStatus::Down;
    frame.contacts[1].filtered_x = 500.0;
    frame.contacts[1].filtered_y = 400.0;
    frame.contacts[2].active = true;
    frame.contacts[2].status = ContactStatus::Down;
    frame.contacts[2].filtered_x = 600.0;
    frame.contacts[2].filtered_y = 400.0;
    frame.timestamp_us = 800_000;
    engine.process_frame(&frame);

    // Swipe all 3 fingers 60 px to the left
    frame.contacts[0].filtered_x = 340.0;
    frame.contacts[1].filtered_x = 440.0;
    frame.contacts[2].filtered_x = 540.0;
    frame.contacts[0].velocity_x = -400.0;
    frame.contacts[1].velocity_x = -400.0;
    frame.contacts[2].velocity_x = -400.0;
    frame.timestamp_us = 850_000;
    let ev6 = engine.process_frame(&frame);
    let swipe_left_detected = ev6.iter().any(|e| matches!(e, GestureEvent::ThreeFingerSwipe { direction: SwipeDirection::Left, .. }));
    if !swipe_left_detected {
        return false;
    }

    true
}
