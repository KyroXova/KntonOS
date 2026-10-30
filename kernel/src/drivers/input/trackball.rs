//! Virtual Trackball Physics Simulator & Kinetic Momentum Engine.
//!
//! Simulates a physical bearing trackball under the thumb touchpad:
//! - Physical mass, radius, and spherical moment of inertia ($I = \frac{2}{5} m r^2$)
//! - Dynamic viscous friction damping ($v_{t+1} = v_t \cdot (1 - \mu \cdot \Delta t)$)
//! - Kinetic momentum spin-down after finger release
//! - Instantaneous thumb "catch" halting kinetic spin on touch re-engagement

use crate::graphics::sqrt_f32;

// -----------------------------------------------------------------------------
// Types and Constants
// -----------------------------------------------------------------------------

/// Operating state of the virtual trackball.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrackballState {
    /// Trackball is stationary at rest.
    Idle,
    /// Thumb is actively contacting the surface and driving rotation.
    Tracking,
    /// Thumb was released; ball is freely spinning with kinetic inertia under friction damping.
    Spinning,
}

/// Configuration parameters for the virtual trackball physics model.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TrackballConfig {
    /// Virtual physical mass of the trackball in grams (e.g. 25.0g).
    pub mass_grams: f32,
    /// Physical radius of the virtual trackball in millimeters (e.g. 15.0mm).
    pub radius_mm: f32,
    /// Viscous friction damping coefficient per second (e.g. 3.0 s^-1).
    pub friction_coefficient: f32,
    /// Minimum linear surface speed threshold below which spinning stops (units/second).
    pub min_spin_velocity: f32,
    /// Maximum clamping speed for flick throws (units/second).
    pub max_velocity: f32,
}

impl TrackballConfig {
    /// Default physics configuration optimized for handheld thumb trackball emulation.
    pub const fn default_handheld() -> Self {
        Self {
            mass_grams: 25.0,
            radius_mm: 15.0,
            friction_coefficient: 3.5,
            min_spin_velocity: 1.5,
            max_velocity: 600.0,
        }
    }
}

/// Step output result from a virtual trackball simulation update.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TrackballStep {
    /// Simulated displacement delta X along surface.
    pub dx: f32,
    /// Simulated displacement delta Y along surface.
    pub dy: f32,
    /// Instantaneous linear surface speed.
    pub speed: f32,
    /// Instantaneous angular velocity in radians per second ($\omega = v / r$).
    pub angular_velocity: f32,
    /// Rotational kinetic energy in micro-joules ($E_k = \frac{1}{5} m v^2$).
    pub kinetic_energy_uj: f32,
    /// Current trackball state.
    pub state: TrackballState,
}

impl TrackballStep {
    pub const fn zero(state: TrackballState) -> Self {
        Self {
            dx: 0.0,
            dy: 0.0,
            speed: 0.0,
            angular_velocity: 0.0,
            kinetic_energy_uj: 0.0,
            state,
        }
    }
}

// -----------------------------------------------------------------------------
// Virtual Trackball Simulation Engine
// -----------------------------------------------------------------------------

/// Physics simulator tracking virtual trackball kinetic state.
#[derive(Debug, Clone)]
pub struct VirtualTrackball {
    /// Physical configuration parameters.
    pub config: TrackballConfig,
    /// Current state machine mode.
    state: TrackballState,
    /// Current surface velocity X (units / sec).
    velocity_x: f32,
    /// Current surface velocity Y (units / sec).
    velocity_y: f32,
    /// Filtered velocity X during active thumb tracking.
    tracked_vx: f32,
    /// Filtered velocity Y during active thumb tracking.
    tracked_vy: f32,
    /// Cumulative rotation angle in radians around motion axis.
    total_rotation_rad: f32,
    /// Total simulated distance rolled in units.
    total_distance: f32,
    /// Distance accumulated since last haptic tick or step.
    haptic_tick_accumulator: f32,
}

impl VirtualTrackball {
    /// Creates a new virtual trackball physics engine with the given configuration.
    pub const fn new(config: TrackballConfig) -> Self {
        Self {
            config,
            state: TrackballState::Idle,
            velocity_x: 0.0,
            velocity_y: 0.0,
            tracked_vx: 0.0,
            tracked_vy: 0.0,
            total_rotation_rad: 0.0,
            total_distance: 0.0,
            haptic_tick_accumulator: 0.0,
        }
    }

    /// Current operational state.
    pub fn state(&self) -> TrackballState {
        self.state
    }

    /// Current surface velocity vector (vx, vy).
    pub fn velocity(&self) -> (f32, f32) {
        (self.velocity_x, self.velocity_y)
    }

    /// Instantaneous linear surface speed.
    pub fn speed(&self) -> f32 {
        sqrt_f32(self.velocity_x * self.velocity_x + self.velocity_y * self.velocity_y)
    }

    /// Moment of inertia of virtual solid sphere: $I = \frac{2}{5} m r^2$.
    /// Expressed in g*mm^2.
    pub fn moment_of_inertia(&self) -> f32 {
        0.4 * self.config.mass_grams * self.config.radius_mm * self.config.radius_mm
    }

    /// Angular velocity $\omega$ in radians per second: $\omega = v / r$.
    pub fn angular_velocity(&self) -> f32 {
        let spd = self.speed();
        if self.config.radius_mm > 0.0 {
            spd / self.config.radius_mm
        } else {
            0.0
        }
    }

    /// Rotational kinetic energy $E_k = \frac{1}{2} I \omega^2 = \frac{1}{5} m v^2$.
    /// Expressed in micro-joules (for mass in grams and v in mm/s).
    pub fn kinetic_energy(&self) -> f32 {
        let spd = self.speed();
        0.2 * self.config.mass_grams * spd * spd
    }

    /// Cumulative distance rolled across all movements.
    pub fn total_distance(&self) -> f32 {
        self.total_distance
    }

    /// Total rotation angle accumulated in radians.
    pub fn total_rotation_rad(&self) -> f32 {
        self.total_rotation_rad
    }

    /// Handles thumb touch down:
    /// In a physical trackball, placing a thumb on a freely spinning ball immediately
    /// arrests (catches) its momentum.
    pub fn on_touch_down(&mut self) {
        self.state = TrackballState::Tracking;
        self.velocity_x = 0.0;
        self.velocity_y = 0.0;
        self.tracked_vx = 0.0;
        self.tracked_vy = 0.0;
    }

    /// Handles continuous thumb movement across the touchpad.
    ///
    /// Directly follows thumb motion deltas, while filtering velocity for kinetic release throw.
    pub fn on_touch_move(&mut self, dx: f32, dy: f32, dt_sec: f32) -> TrackballStep {
        self.state = TrackballState::Tracking;

        if dt_sec > 0.0001 {
            let inst_vx = dx / dt_sec;
            let inst_vy = dy / dt_sec;
            // Smooth release velocity with low-pass filter
            self.tracked_vx = self.tracked_vx * 0.35 + inst_vx * 0.65;
            self.tracked_vy = self.tracked_vy * 0.35 + inst_vy * 0.65;
        }

        self.velocity_x = self.tracked_vx;
        self.velocity_y = self.tracked_vy;

        let step_dist = sqrt_f32(dx * dx + dy * dy);
        self.total_distance += step_dist;
        self.haptic_tick_accumulator += step_dist;
        if self.config.radius_mm > 0.0 {
            self.total_rotation_rad += step_dist / self.config.radius_mm;
        }

        let spd = self.speed();
        let omega = self.angular_velocity();
        let ke = self.kinetic_energy();

        TrackballStep {
            dx,
            dy,
            speed: spd,
            angular_velocity: omega,
            kinetic_energy_uj: ke,
            state: self.state,
        }
    }

    /// Handles thumb release:
    ///
    /// If release velocity exceeds `min_spin_velocity`, transitions to `Spinning`
    /// to simulate kinetic momentum coasting. Otherwise halts to `Idle`.
    pub fn on_touch_up(&mut self) {
        let spd = sqrt_f32(self.tracked_vx * self.tracked_vx + self.tracked_vy * self.tracked_vy);
        if spd >= self.config.min_spin_velocity {
            self.state = TrackballState::Spinning;
            // Clamp release velocity to max_velocity
            if spd > self.config.max_velocity {
                let scale = self.config.max_velocity / spd;
                self.velocity_x = self.tracked_vx * scale;
                self.velocity_y = self.tracked_vy * scale;
            } else {
                self.velocity_x = self.tracked_vx;
                self.velocity_y = self.tracked_vy;
            }
        } else {
            self.stop();
        }
    }

    /// Advances the physics simulation for a time delta `dt_sec`.
    ///
    /// When `state == TrackballState::Spinning`, computes friction damping and
    /// generates coasting displacements.
    pub fn step_simulation(&mut self, dt_sec: f32) -> TrackballStep {
        if self.state != TrackballState::Spinning || dt_sec <= 0.0 {
            return TrackballStep::zero(self.state);
        }

        // Apply viscous friction damping: v_{t+1} = v_t * (1.0 - friction * dt)
        let damping = (1.0 - self.config.friction_coefficient * dt_sec).clamp(0.0, 1.0);
        self.velocity_x *= damping;
        self.velocity_y *= damping;

        let spd = self.speed();

        // Check if velocity has decayed below minimum spin threshold
        if spd < self.config.min_spin_velocity {
            self.stop();
            return TrackballStep::zero(TrackballState::Idle);
        }

        let dx = self.velocity_x * dt_sec;
        let dy = self.velocity_y * dt_sec;
        let step_dist = sqrt_f32(dx * dx + dy * dy);

        self.total_distance += step_dist;
        self.haptic_tick_accumulator += step_dist;
        if self.config.radius_mm > 0.0 {
            self.total_rotation_rad += step_dist / self.config.radius_mm;
        }

        let omega = self.angular_velocity();
        let ke = self.kinetic_energy();

        TrackballStep {
            dx,
            dy,
            speed: spd,
            angular_velocity: omega,
            kinetic_energy_uj: ke,
            state: self.state,
        }
    }

    /// Checks and consumes haptic tick pulses when accumulated distance exceeds `interval_units`.
    pub fn consume_haptic_ticks(&mut self, interval_units: f32) -> u32 {
        if interval_units <= 0.0 || self.haptic_tick_accumulator < interval_units {
            return 0;
        }
        let ticks = (self.haptic_tick_accumulator / interval_units) as u32;
        self.haptic_tick_accumulator -= (ticks as f32) * interval_units;
        ticks
    }

    /// Stops all kinetic motion and transitions to `Idle`.
    pub fn stop(&mut self) {
        self.state = TrackballState::Idle;
        self.velocity_x = 0.0;
        self.velocity_y = 0.0;
        self.tracked_vx = 0.0;
        self.tracked_vy = 0.0;
    }

    /// Resets all internal positions, accumulators, and velocity states.
    pub fn reset(&mut self) {
        self.stop();
        self.total_rotation_rad = 0.0;
        self.total_distance = 0.0;
        self.haptic_tick_accumulator = 0.0;
    }
}

// -----------------------------------------------------------------------------
// Unit Tests & Verification
// -----------------------------------------------------------------------------

/// Unit test verifying virtual trackball physics model, friction damping, and kinetic spin.
pub fn verify_virtual_trackball_physics() -> bool {
    let config = TrackballConfig::default_handheld();
    let mut trackball = VirtualTrackball::new(config);

    // 1. Initial State: Idle with zero velocity and kinetic energy
    if trackball.state() != TrackballState::Idle {
        return false;
    }
    if trackball.speed() != 0.0 || trackball.kinetic_energy() != 0.0 {
        return false;
    }

    // Verify moment of inertia: I = 0.4 * 25.0 * 15.0^2 = 0.4 * 25 * 225 = 2250.0 g*mm^2
    let moi = trackball.moment_of_inertia();
    if (moi - 2250.0).abs() > 0.01 {
        return false;
    }

    // 2. Touch down: transitions to Tracking
    trackball.on_touch_down();
    if trackball.state() != TrackballState::Tracking {
        return false;
    }

    // 3. Touch move: flick throw simulation (move dx = 2.0mm per 10ms = 200 mm/s)
    let dt = 0.010; // 10ms
    for _ in 0..10 {
        let step = trackball.on_touch_move(2.0, 0.0, dt);
        if step.state != TrackballState::Tracking {
            return false;
        }
    }
    if trackball.speed() < 150.0 {
        return false;
    }

    // 4. Touch release: flick velocity >= 1.5 -> transitions to Spinning
    trackball.on_touch_up();
    if trackball.state() != TrackballState::Spinning {
        return false;
    }

    // 5. Kinetic coasting under viscous friction damping
    let initial_speed = trackball.speed();
    let initial_ke = trackball.kinetic_energy();

    // Step simulation 20 times (200ms)
    let mut coasted_distance = 0.0;
    for _ in 0..20 {
        let step = trackball.step_simulation(0.010);
        coasted_distance += step.dx;
    }

    // Velocity and kinetic energy must have decayed
    let mid_speed = trackball.speed();
    let mid_ke = trackball.kinetic_energy();
    if mid_speed >= initial_speed || mid_ke >= initial_ke {
        return false;
    }
    if coasted_distance <= 0.0 {
        return false;
    }

    // 6. Natural spin-down cutoff to Idle
    // Continue stepping until ball comes to rest
    let mut steps = 0;
    while trackball.state() == TrackballState::Spinning && steps < 500 {
        trackball.step_simulation(0.010);
        steps += 1;
    }
    if trackball.state() != TrackballState::Idle {
        return false;
    }
    if trackball.speed() != 0.0 {
        return false;
    }

    // 7. Instantaneous thumb "catch" test:
    // Start another flick spin, then touch down mid-spin
    trackball.on_touch_down();
    for _ in 0..5 {
        trackball.on_touch_move(3.0, 0.0, dt);
    }
    trackball.on_touch_up();
    if trackball.state() != TrackballState::Spinning {
        return false;
    }

    // Thumb catches the spinning ball!
    trackball.on_touch_down();
    if trackball.state() != TrackballState::Tracking {
        return false;
    }
    if trackball.speed() != 0.0 {
        return false;
    }

    true
}
