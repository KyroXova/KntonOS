//! Hierarchical Timing Wheel Engine with O(1) Cascade Insertion & Expiry Dispatch.
//!
//! Implements a 5-level hierarchical timing wheel (Level 0 through Level 4) providing
//! O(1) timer insertion and deterministic cascading expiry dispatch. Ideal for coarse-
//! to-medium kernel timeouts (networking, I/O timeouts, delayed work queues, and task deadlines).

use alloc::vec::Vec;

/// Level 0 wheel capacity (256 slots, 8-bit index mask).
pub const WHEEL_LEVEL0_BITS: usize = 8;
pub const WHEEL_LEVEL0_SIZE: usize = 1 << WHEEL_LEVEL0_BITS; // 256
pub const WHEEL_LEVEL0_MASK: u64 = (WHEEL_LEVEL0_SIZE - 1) as u64;

/// Upper level wheel capacity (64 slots per level, 6-bit index mask).
pub const WHEEL_UPPER_BITS: usize = 6;
pub const WHEEL_UPPER_SIZE: usize = 1 << WHEEL_UPPER_BITS; // 64
pub const WHEEL_UPPER_MASK: u64 = (WHEEL_UPPER_SIZE - 1) as u64;

/// Shift offsets for each hierarchical level.
pub const LEVEL0_SHIFT: usize = 0;
pub const LEVEL1_SHIFT: usize = WHEEL_LEVEL0_BITS; // 8
pub const LEVEL2_SHIFT: usize = LEVEL1_SHIFT + WHEEL_UPPER_BITS; // 14
pub const LEVEL3_SHIFT: usize = LEVEL2_SHIFT + WHEEL_UPPER_BITS; // 20
pub const LEVEL4_SHIFT: usize = LEVEL3_SHIFT + WHEEL_UPPER_BITS; // 26

/// Spans covered by each level.
pub const LEVEL0_SPAN: u64 = 1 << LEVEL1_SHIFT; // 256 ticks
pub const LEVEL1_SPAN: u64 = 1 << LEVEL2_SHIFT; // 16,384 ticks
pub const LEVEL2_SPAN: u64 = 1 << LEVEL3_SHIFT; // 1,048,576 ticks
pub const LEVEL3_SPAN: u64 = 1 << LEVEL4_SHIFT; // 67,108,864 ticks

/// Callback invoked on timing wheel timer expiry.
pub type WheelCallback = fn(id: u64, current_ticks: u64, user_data: usize);

/// Timing wheel timer descriptor.
#[derive(Clone, Copy)]
pub struct WheelTimer {
    /// Unique timer identifier.
    pub id: u64,
    /// Absolute expiration tick count.
    pub expires_ticks: u64,
    /// Periodic repeat interval in ticks (0 if one-shot).
    pub interval_ticks: u64,
    /// Context data passed to the callback.
    pub user_data: usize,
    /// Optional callback function.
    pub callback: Option<WheelCallback>,
}

impl WheelTimer {
    /// Creates a new one-shot wheel timer.
    pub const fn new_oneshot(id: u64, expires_ticks: u64) -> Self {
        Self {
            id,
            expires_ticks,
            interval_ticks: 0,
            user_data: 0,
            callback: None,
        }
    }

    /// Creates a new periodic wheel timer.
    pub const fn new_periodic(id: u64, start_ticks: u64, interval_ticks: u64) -> Self {
        Self {
            id,
            expires_ticks: start_ticks,
            interval_ticks,
            user_data: 0,
            callback: None,
        }
    }

    /// Attaches an expiration callback.
    #[inline]
    pub fn with_callback(mut self, cb: WheelCallback, user_data: usize) -> Self {
        self.callback = Some(cb);
        self.user_data = user_data;
        self
    }
}

/// 5-Level Hierarchical Timing Wheel.
pub struct HierarchicalTimingWheel {
    /// Current monotonic tick count of the wheel.
    current_ticks: u64,
    /// Level 0: 256 slots (granularity: 1 tick, range: 0..255).
    level0: [Vec<WheelTimer>; WHEEL_LEVEL0_SIZE],
    /// Level 1: 64 slots (granularity: 256 ticks, range: 256..16,383).
    level1: [Vec<WheelTimer>; WHEEL_UPPER_SIZE],
    /// Level 2: 64 slots (granularity: 16,384 ticks, range: 16,384..1,048,575).
    level2: [Vec<WheelTimer>; WHEEL_UPPER_SIZE],
    /// Level 3: 64 slots (granularity: 1,048,576 ticks, range: 1,048,576..67,108,863).
    level3: [Vec<WheelTimer>; WHEEL_UPPER_SIZE],
    /// Level 4: 64 slots (granularity: 67,108,864 ticks, range: 67,108,864..4,294,967,295).
    level4: [Vec<WheelTimer>; WHEEL_UPPER_SIZE],
    /// Total active timers across all levels.
    active_count: usize,
    /// Total timers expired.
    total_expired: usize,
    /// Total cascades performed across upper levels.
    total_cascades: usize,
}

impl HierarchicalTimingWheel {
    /// Creates a new hierarchical timing wheel starting at `initial_ticks`.
    pub fn new(initial_ticks: u64) -> Self {
        const EMPTY_L0: Vec<WheelTimer> = Vec::new();
        const EMPTY_UPPER: Vec<WheelTimer> = Vec::new();

        Self {
            current_ticks: initial_ticks,
            level0: [EMPTY_L0; WHEEL_LEVEL0_SIZE],
            level1: [EMPTY_UPPER; WHEEL_UPPER_SIZE],
            level2: [EMPTY_UPPER; WHEEL_UPPER_SIZE],
            level3: [EMPTY_UPPER; WHEEL_UPPER_SIZE],
            level4: [EMPTY_UPPER; WHEEL_UPPER_SIZE],
            active_count: 0,
            total_expired: 0,
            total_cascades: 0,
        }
    }

    /// Schedules a timer into the appropriate hierarchical level based on deadline delta.
    pub fn schedule(&mut self, timer: WheelTimer) {
        let delta = timer.expires_ticks.saturating_sub(self.current_ticks);

        if delta < LEVEL0_SPAN {
            let slot = (timer.expires_ticks & WHEEL_LEVEL0_MASK) as usize;
            self.level0[slot].push(timer);
        } else if delta < LEVEL1_SPAN {
            let slot = ((timer.expires_ticks >> LEVEL1_SHIFT) & WHEEL_UPPER_MASK) as usize;
            self.level1[slot].push(timer);
        } else if delta < LEVEL2_SPAN {
            let slot = ((timer.expires_ticks >> LEVEL2_SHIFT) & WHEEL_UPPER_MASK) as usize;
            self.level2[slot].push(timer);
        } else if delta < LEVEL3_SPAN {
            let slot = ((timer.expires_ticks >> LEVEL3_SHIFT) & WHEEL_UPPER_MASK) as usize;
            self.level3[slot].push(timer);
        } else {
            let slot = ((timer.expires_ticks >> LEVEL4_SHIFT) & WHEEL_UPPER_MASK) as usize;
            self.level4[slot].push(timer);
        }

        self.active_count += 1;
    }

    /// Cancels an active timer by its identifier across all wheel levels.
    pub fn cancel(&mut self, id: u64) -> Option<WheelTimer> {
        // Search level 0
        for slot in &mut self.level0 {
            if let Some(pos) = slot.iter().position(|t| t.id == id) {
                let timer = slot.remove(pos);
                self.active_count = self.active_count.saturating_sub(1);
                return Some(timer);
            }
        }
        // Search level 1
        for slot in &mut self.level1 {
            if let Some(pos) = slot.iter().position(|t| t.id == id) {
                let timer = slot.remove(pos);
                self.active_count = self.active_count.saturating_sub(1);
                return Some(timer);
            }
        }
        // Search level 2
        for slot in &mut self.level2 {
            if let Some(pos) = slot.iter().position(|t| t.id == id) {
                let timer = slot.remove(pos);
                self.active_count = self.active_count.saturating_sub(1);
                return Some(timer);
            }
        }
        // Search level 3
        for slot in &mut self.level3 {
            if let Some(pos) = slot.iter().position(|t| t.id == id) {
                let timer = slot.remove(pos);
                self.active_count = self.active_count.saturating_sub(1);
                return Some(timer);
            }
        }
        // Search level 4
        for slot in &mut self.level4 {
            if let Some(pos) = slot.iter().position(|t| t.id == id) {
                let timer = slot.remove(pos);
                self.active_count = self.active_count.saturating_sub(1);
                return Some(timer);
            }
        }

        None
    }

    /// Cascades timers from upper wheel levels down to lower levels when lower levels wrap.
    fn cascade_level(&mut self, level_idx: usize, slot_idx: usize) {
        self.total_cascades += 1;
        let timers_to_reinsert = match level_idx {
            1 => core::mem::take(&mut self.level1[slot_idx]),
            2 => core::mem::take(&mut self.level2[slot_idx]),
            3 => core::mem::take(&mut self.level3[slot_idx]),
            4 => core::mem::take(&mut self.level4[slot_idx]),
            _ => return,
        };

        self.active_count = self.active_count.saturating_sub(timers_to_reinsert.len());
        for timer in timers_to_reinsert {
            self.schedule(timer);
        }
    }

    /// Advances the wheel by a single tick, cascading upper levels and returning expired timers.
    pub fn step(&mut self) -> Vec<WheelTimer> {
        let slot = (self.current_ticks & WHEEL_LEVEL0_MASK) as usize;

        // When level 0 completes a revolution (slot == 0), cascade upper levels
        if slot == 0 {
            let l1_slot = ((self.current_ticks >> LEVEL1_SHIFT) & WHEEL_UPPER_MASK) as usize;
            self.cascade_level(1, l1_slot);

            if l1_slot == 0 {
                let l2_slot = ((self.current_ticks >> LEVEL2_SHIFT) & WHEEL_UPPER_MASK) as usize;
                self.cascade_level(2, l2_slot);

                if l2_slot == 0 {
                    let l3_slot = ((self.current_ticks >> LEVEL3_SHIFT) & WHEEL_UPPER_MASK) as usize;
                    self.cascade_level(3, l3_slot);

                    if l3_slot == 0 {
                        let l4_slot = ((self.current_ticks >> LEVEL4_SHIFT) & WHEEL_UPPER_MASK) as usize;
                        self.cascade_level(4, l4_slot);
                    }
                }
            }
        }

        // Collect expired timers in the current level 0 slot
        let current_slot_timers = core::mem::take(&mut self.level0[slot]);
        self.active_count = self.active_count.saturating_sub(current_slot_timers.len());

        let mut expired = Vec::new();
        for timer in current_slot_timers {
            if timer.expires_ticks <= self.current_ticks {
                self.total_expired += 1;
                // If periodic, re-arm with updated deadline
                if timer.interval_ticks > 0 {
                    let mut next = timer;
                    next.expires_ticks = self.current_ticks + timer.interval_ticks;
                    self.schedule(next);
                }
                expired.push(timer);
            } else {
                // Not yet expired (e.g. cascaded into same slot for future revolution)
                self.schedule(timer);
            }
        }

        self.current_ticks += 1;
        expired
    }

    /// Advances the wheel until reaching `target_ticks`, collecting all expired timers.
    pub fn advance_to(&mut self, target_ticks: u64) -> Vec<WheelTimer> {
        let mut all_expired = Vec::new();
        while self.current_ticks <= target_ticks {
            let exp = self.step();
            all_expired.extend(exp);
        }
        all_expired
    }

    /// Current monotonic ticks.
    #[inline]
    pub fn current_ticks(&self) -> u64 {
        self.current_ticks
    }

    /// Total active timers currently buffered.
    #[inline]
    pub fn active_count(&self) -> usize {
        self.active_count
    }

    /// Total lifetime expired timers dispatched.
    #[inline]
    pub fn total_expired(&self) -> usize {
        self.total_expired
    }

    /// Total upper-level cascade events executed.
    #[inline]
    pub fn total_cascades(&self) -> usize {
        self.total_cascades
    }

    /// Earliest expiration tick across all active timers (scans active slots).
    pub fn earliest_expiry(&self) -> Option<u64> {
        if self.active_count == 0 {
            return None;
        }

        let mut earliest: Option<u64> = None;
        let mut check = |t: &WheelTimer| {
            if earliest.is_none() || t.expires_ticks < earliest.unwrap() {
                earliest = Some(t.expires_ticks);
            }
        };

        for slot in &self.level0 {
            for t in slot {
                check(t);
            }
        }
        for slot in &self.level1 {
            for t in slot {
                check(t);
            }
        }
        for slot in &self.level2 {
            for t in slot {
                check(t);
            }
        }
        for slot in &self.level3 {
            for t in slot {
                check(t);
            }
        }
        for slot in &self.level4 {
            for t in slot {
                check(t);
            }
        }

        earliest
    }
}

/// Unit test verifying hierarchical timing wheel cascade operations, multi-level distribution, and dispatch.
pub fn test_timing_wheel_cascade_and_dispatch() -> bool {
    let mut wheel = HierarchicalTimingWheel::new(0);

    if wheel.current_ticks() != 0 || wheel.active_count() != 0 {
        return false;
    }

    // Schedule:
    // - timer 1 at tick 10 (Level 0)
    // - timer 2 at tick 50 (Level 0)
    // - timer 3 at tick 300 (Level 1, delta = 300 >= 256)
    // - timer 4 at tick 20,000 (Level 2, delta = 20,000 >= 16,384)
    // - timer 5 (periodic) at tick 20 with interval 20
    wheel.schedule(WheelTimer::new_oneshot(1, 10));
    wheel.schedule(WheelTimer::new_oneshot(2, 50));
    wheel.schedule(WheelTimer::new_oneshot(3, 300));
    wheel.schedule(WheelTimer::new_oneshot(4, 20_000));
    wheel.schedule(WheelTimer::new_periodic(5, 20, 20));

    if wheel.active_count() != 5 {
        return false;
    }

    // Advance to tick 15 -> timer 1 must have fired
    let exp1 = wheel.advance_to(15);
    if exp1.len() != 1 || exp1[0].id != 1 {
        return false;
    }

    // Advance to tick 25 -> periodic timer 5 must have fired once and re-armed for tick 40
    let exp2 = wheel.advance_to(25);
    if exp2.len() != 1 || exp2[0].id != 5 {
        return false;
    }

    // Advance to tick 60 -> periodic timer 5 fires again at 40, timer 2 fires at 50, timer 5 fires again at 60
    let exp3 = wheel.advance_to(60);
    if exp3.len() != 3 {
        return false;
    }

    // Cancel timer 4 (at tick 20,000 in Level 2)
    let cancelled = wheel.cancel(4);
    if cancelled.is_none() || cancelled.unwrap().id != 4 {
        return false;
    }

    // Advance to tick 310:
    // Crossing tick 256 triggers Level 1 cascade! Timer 3 (tick 300) cascades into Level 0 and fires at tick 300.
    let exp4 = wheel.advance_to(310);

    // Verify timer 3 fired
    let timer3_fired = exp4.iter().any(|t| t.id == 3);
    if !timer3_fired {
        return false;
    }

    if wheel.total_cascades() == 0 {
        return false;
    }

    true
}
