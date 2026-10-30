//! Kernel High-Resolution Timers (hrtimer) & Ordered Red-Black Tree Queue.
//! Provides sub-microsecond precision monotonic timers organized in an ordered

use alloc::vec::Vec;

/// High-resolution monotonic timestamp in nanoseconds.
pub type HrTimeNs = u64;

/// Nanosecond conversion factors.
pub const NS_PER_US: u64 = 1_000;
pub const NS_PER_MS: u64 = 1_000_000;
pub const NS_PER_SEC: u64 = 1_000_000_000;

/// Operational mode of a high-resolution timer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HrTimerMode {
    /// Fires once at the specified monotonic deadline and becomes inactive.
    OneShot,
    /// Fires periodically, automatically reloading by adding its interval to its deadline.
    Periodic,
}

/// Execution and queue state of a high-resolution timer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HrTimerState {
    /// Timer is unqueued and inactive.
    Inactive,
    /// Timer is armed and waiting in the Red-Black tree queue.
    Enqueued,
    /// Timer has reached its deadline and is ready for dispatch.
    Expired,
    /// Timer callback is currently executing.
    Running,
}

/// Callback function signature invoked when a timer expires.
pub type HrTimerCallback = fn(timer_id: u64, now_ns: HrTimeNs, user_data: usize);

/// High-resolution timer descriptor.
#[derive(Clone, Copy)]
pub struct HrTimer {
    /// Unique timer identifier.
    pub id: u64,
    /// Absolute monotonic expiration deadline in nanoseconds.
    pub expires_ns: HrTimeNs,
    /// Periodic repeat interval in nanoseconds (0 if OneShot).
    pub interval_ns: HrTimeNs,
    /// Operational mode.
    pub mode: HrTimerMode,
    /// Current timer state.
    pub state: HrTimerState,
    /// Optional expiration callback function.
    pub callback: Option<HrTimerCallback>,
    /// User context data passed to the callback.
    pub user_data: usize,
    /// Target CPU core ID.
    pub cpu_id: usize,
}

impl HrTimer {
    /// Creates a new one-shot high-resolution timer with the given deadline.
    pub const fn new_oneshot(id: u64, expires_ns: HrTimeNs, cpu_id: usize) -> Self {
        Self {
            id,
            expires_ns,
            interval_ns: 0,
            mode: HrTimerMode::OneShot,
            state: HrTimerState::Inactive,
            callback: None,
            user_data: 0,
            cpu_id,
        }
    }

    /// Creates a new periodic high-resolution timer with start deadline and period.
    pub const fn new_periodic(
        id: u64,
        start_deadline_ns: HrTimeNs,
        interval_ns: HrTimeNs,
        cpu_id: usize,
    ) -> Self {
        Self {
            id,
            expires_ns: start_deadline_ns,
            interval_ns,
            mode: HrTimerMode::Periodic,
            state: HrTimerState::Inactive,
            callback: None,
            user_data: 0,
            cpu_id,
        }
    }

    /// Sets the expiration callback and user data.
    #[inline]
    pub fn with_callback(mut self, cb: HrTimerCallback, user_data: usize) -> Self {
        self.callback = Some(cb);
        self.user_data = user_data;
        self
    }
}

/// Color node attribute for Red-Black tree balancing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RbColor {
    Red,
    Black,
}

/// An entry in the Red-Black tree timer queue.
#[derive(Clone)]
struct RbNode {
    timer: HrTimer,
    color: RbColor,
    left: Option<usize>,
    right: Option<usize>,
    parent: Option<usize>,
}

/// Ordered Red-Black Tree priority queue for high-resolution timers.
/// Keys are ordered strictly by `(expires_ns, id)`.
pub struct RbTreeTimerQueue {
    nodes: Vec<Option<RbNode>>,
    root: Option<usize>,
    count: usize,
    free_head: Option<usize>,
}

impl RbTreeTimerQueue {
    /// Creates an empty Red-Black tree timer queue.
    pub fn new() -> Self {
        Self {
            nodes: Vec::new(),
            root: None,
            count: 0,
            free_head: None,
        }
    }

    /// Number of timers currently armed in the queue.
    #[inline]
    pub fn len(&self) -> usize {
        self.count
    }

    /// True if no timers are currently armed.
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.count == 0
    }

    /// Earliest deadline across all active timers in the queue.
    pub fn earliest_deadline(&self) -> Option<HrTimeNs> {
        self.peek_min_index().and_then(|idx| {
            self.nodes[idx].as_ref().map(|n| n.timer.expires_ns)
        })
    }

    /// Peeks at the earliest timer without removing it.
    pub fn peek_earliest(&self) -> Option<&HrTimer> {
        self.peek_min_index().and_then(|idx| {
            self.nodes[idx].as_ref().map(|n| &n.timer)
        })
    }

    /// Inserts a timer into the Red-Black tree queue.
    pub fn insert(&mut self, mut timer: HrTimer) -> bool {
        timer.state = HrTimerState::Enqueued;
        let new_idx = self.allocate_node(timer);

        if self.root.is_none() {
            if let Some(ref mut node) = self.nodes[new_idx] {
                node.color = RbColor::Black;
            }
            self.root = Some(new_idx);
            self.count += 1;
            return true;
        }

        let mut curr_idx = self.root.unwrap();
        let target_key = (timer.expires_ns, timer.id);

        loop {
            let curr_key = {
                let curr_node = self.nodes[curr_idx].as_ref().unwrap();
                (curr_node.timer.expires_ns, curr_node.timer.id)
            };

            if target_key < curr_key {
                let left_opt = self.nodes[curr_idx].as_ref().unwrap().left;
                if let Some(left) = left_opt {
                    curr_idx = left;
                } else {
                    self.nodes[curr_idx].as_mut().unwrap().left = Some(new_idx);
                    self.nodes[new_idx].as_mut().unwrap().parent = Some(curr_idx);
                    break;
                }
            } else {
                let right_opt = self.nodes[curr_idx].as_ref().unwrap().right;
                if let Some(right) = right_opt {
                    curr_idx = right;
                } else {
                    self.nodes[curr_idx].as_mut().unwrap().right = Some(new_idx);
                    self.nodes[new_idx].as_mut().unwrap().parent = Some(curr_idx);
                    break;
                }
            }
        }

        self.insert_fixup(new_idx);
        self.count += 1;
        true
    }

    /// Cancels and removes a timer by its unique identifier.
    pub fn cancel(&mut self, id: u64) -> Option<HrTimer> {
        let node_idx = self.find_by_id(id)?;
        let timer = self.nodes[node_idx].as_ref().unwrap().timer;
        self.delete_node(node_idx);
        Some(timer)
    }

    /// Pops the timer with the earliest deadline.
    pub fn pop_earliest(&mut self) -> Option<HrTimer> {
        let min_idx = self.peek_min_index()?;
        let timer = self.nodes[min_idx].as_ref().unwrap().timer;
        self.delete_node(min_idx);
        Some(timer)
    }

    // --- Internal Red-Black Tree Helpers ---

    fn allocate_node(&mut self, timer: HrTimer) -> usize {
        let node = RbNode {
            timer,
            color: RbColor::Red,
            left: None,
            right: None,
            parent: None,
        };

        if let Some(free_idx) = self.free_head {
            self.free_head = self.nodes[free_idx].as_ref().and_then(|n| n.left);
            self.nodes[free_idx] = Some(node);
            free_idx
        } else {
            let idx = self.nodes.len();
            self.nodes.push(Some(node));
            idx
        }
    }

    fn free_node(&mut self, idx: usize) {
        if let Some(ref mut n) = self.nodes[idx] {
            n.left = self.free_head;
            n.right = None;
            n.parent = None;
        }
        self.free_head = Some(idx);
    }

    fn peek_min_index(&self) -> Option<usize> {
        let mut curr = self.root?;
        while let Some(left) = self.nodes[curr].as_ref().and_then(|n| n.left) {
            curr = left;
        }
        Some(curr)
    }

    fn find_by_id(&self, id: u64) -> Option<usize> {
        // Linear scan of allocated slots (safe & simple for cancellation)
        for (idx, opt) in self.nodes.iter().enumerate() {
            if let Some(node) = opt {
                if node.timer.id == id && node.timer.state == HrTimerState::Enqueued {
                    return Some(idx);
                }
            }
        }
        None
    }

    fn rotate_left(&mut self, x: usize) {
        let y = match self.nodes[x].as_ref().and_then(|n| n.right) {
            Some(y_idx) => y_idx,
            None => return,
        };

        let y_left = self.nodes[y].as_ref().and_then(|n| n.left);
        self.nodes[x].as_mut().unwrap().right = y_left;
        if let Some(yl) = y_left {
            self.nodes[yl].as_mut().unwrap().parent = Some(x);
        }

        let x_parent = self.nodes[x].as_ref().and_then(|n| n.parent);
        self.nodes[y].as_mut().unwrap().parent = x_parent;

        if x_parent.is_none() {
            self.root = Some(y);
        } else if self.nodes[x_parent.unwrap()].as_ref().unwrap().left == Some(x) {
            self.nodes[x_parent.unwrap()].as_mut().unwrap().left = Some(y);
        } else {
            self.nodes[x_parent.unwrap()].as_mut().unwrap().right = Some(y);
        }

        self.nodes[y].as_mut().unwrap().left = Some(x);
        self.nodes[x].as_mut().unwrap().parent = Some(y);
    }

    fn rotate_right(&mut self, y: usize) {
        let x = match self.nodes[y].as_ref().and_then(|n| n.left) {
            Some(x_idx) => x_idx,
            None => return,
        };

        let x_right = self.nodes[x].as_ref().and_then(|n| n.right);
        self.nodes[y].as_mut().unwrap().left = x_right;
        if let Some(xr) = x_right {
            self.nodes[xr].as_mut().unwrap().parent = Some(y);
        }

        let y_parent = self.nodes[y].as_ref().and_then(|n| n.parent);
        self.nodes[x].as_mut().unwrap().parent = y_parent;

        if y_parent.is_none() {
            self.root = Some(x);
        } else if self.nodes[y_parent.unwrap()].as_ref().unwrap().right == Some(y) {
            self.nodes[y_parent.unwrap()].as_mut().unwrap().right = Some(x);
        } else {
            self.nodes[y_parent.unwrap()].as_mut().unwrap().left = Some(x);
        }

        self.nodes[x].as_mut().unwrap().right = Some(y);
        self.nodes[y].as_mut().unwrap().parent = Some(x);
    }

    fn insert_fixup(&mut self, mut z: usize) {
        while z != self.root.unwrap_or(z) {
            let parent = self.nodes[z].as_ref().and_then(|n| n.parent);
            if parent.is_none() || self.nodes[parent.unwrap()].as_ref().unwrap().color == RbColor::Black {
                break;
            }
            let p = parent.unwrap();
            let grandparent = self.nodes[p].as_ref().and_then(|n| n.parent);
            if grandparent.is_none() {
                break;
            }
            let g = grandparent.unwrap();

            if self.nodes[g].as_ref().unwrap().left == Some(p) {
                let uncle = self.nodes[g].as_ref().unwrap().right;
                if uncle.is_some() && self.nodes[uncle.unwrap()].as_ref().unwrap().color == RbColor::Red {
                    self.nodes[p].as_mut().unwrap().color = RbColor::Black;
                    self.nodes[uncle.unwrap()].as_mut().unwrap().color = RbColor::Black;
                    self.nodes[g].as_mut().unwrap().color = RbColor::Red;
                    z = g;
                } else {
                    if self.nodes[p].as_ref().unwrap().right == Some(z) {
                        z = p;
                        self.rotate_left(z);
                    }
                    let p2 = self.nodes[z].as_ref().unwrap().parent.unwrap();
                    let g2 = self.nodes[p2].as_ref().unwrap().parent.unwrap();
                    self.nodes[p2].as_mut().unwrap().color = RbColor::Black;
                    self.nodes[g2].as_mut().unwrap().color = RbColor::Red;
                    self.rotate_right(g2);
                }
            } else {
                let uncle = self.nodes[g].as_ref().unwrap().left;
                if uncle.is_some() && self.nodes[uncle.unwrap()].as_ref().unwrap().color == RbColor::Red {
                    self.nodes[p].as_mut().unwrap().color = RbColor::Black;
                    self.nodes[uncle.unwrap()].as_mut().unwrap().color = RbColor::Black;
                    self.nodes[g].as_mut().unwrap().color = RbColor::Red;
                    z = g;
                } else {
                    if self.nodes[p].as_ref().unwrap().left == Some(z) {
                        z = p;
                        self.rotate_right(z);
                    }
                    let p2 = self.nodes[z].as_ref().unwrap().parent.unwrap();
                    let g2 = self.nodes[p2].as_ref().unwrap().parent.unwrap();
                    self.nodes[p2].as_mut().unwrap().color = RbColor::Black;
                    self.nodes[g2].as_mut().unwrap().color = RbColor::Red;
                    self.rotate_left(g2);
                }
            }
        }

        if let Some(r) = self.root {
            self.nodes[r].as_mut().unwrap().color = RbColor::Black;
        }
    }

    fn delete_node(&mut self, z: usize) {
        let mut y = z;
        let y_original_color = self.nodes[y].as_ref().unwrap().color;
        let x: Option<usize>;
        let x_parent: Option<usize>;

        let z_left = self.nodes[z].as_ref().unwrap().left;
        let z_right = self.nodes[z].as_ref().unwrap().right;

        if z_left.is_none() {
            x = z_right;
            x_parent = self.nodes[z].as_ref().unwrap().parent;
            self.rb_transplant(z, z_right);
        } else if z_right.is_none() {
            x = z_left;
            x_parent = self.nodes[z].as_ref().unwrap().parent;
            self.rb_transplant(z, z_left);
        } else {
            // Find successor (minimum of right subtree)
            let mut succ = z_right.unwrap();
            while let Some(left) = self.nodes[succ].as_ref().unwrap().left {
                succ = left;
            }
            y = succ;
            let y_succ_original_color = self.nodes[y].as_ref().unwrap().color;
            let y_right = self.nodes[y].as_ref().unwrap().right;
            x = y_right;

            if self.nodes[y].as_ref().unwrap().parent == Some(z) {
                x_parent = Some(y);
            } else {
                x_parent = self.nodes[y].as_ref().unwrap().parent;
                self.rb_transplant(y, y_right);
                self.nodes[y].as_mut().unwrap().right = self.nodes[z].as_ref().unwrap().right;
                if let Some(r) = self.nodes[y].as_ref().unwrap().right {
                    self.nodes[r].as_mut().unwrap().parent = Some(y);
                }
            }

            self.rb_transplant(z, Some(y));
            self.nodes[y].as_mut().unwrap().left = self.nodes[z].as_ref().unwrap().left;
            if let Some(l) = self.nodes[y].as_ref().unwrap().left {
                self.nodes[l].as_mut().unwrap().parent = Some(y);
            }
            self.nodes[y].as_mut().unwrap().color = self.nodes[z].as_ref().unwrap().color;

            if y_succ_original_color == RbColor::Black {
                self.delete_fixup(x, x_parent);
            }

            self.free_node(z);
            self.count = self.count.saturating_sub(1);
            return;
        }

        if y_original_color == RbColor::Black {
            self.delete_fixup(x, x_parent);
        }

        self.free_node(z);
        self.count = self.count.saturating_sub(1);
    }

    fn rb_transplant(&mut self, u: usize, v: Option<usize>) {
        let u_parent = self.nodes[u].as_ref().unwrap().parent;
        if u_parent.is_none() {
            self.root = v;
        } else if self.nodes[u_parent.unwrap()].as_ref().unwrap().left == Some(u) {
            self.nodes[u_parent.unwrap()].as_mut().unwrap().left = v;
        } else {
            self.nodes[u_parent.unwrap()].as_mut().unwrap().right = v;
        }

        if let Some(v_idx) = v {
            self.nodes[v_idx].as_mut().unwrap().parent = u_parent;
        }
    }

    fn delete_fixup(&mut self, mut x: Option<usize>, mut parent: Option<usize>) {
        while x != self.root && x.map_or(RbColor::Black, |idx| self.nodes[idx].as_ref().unwrap().color) == RbColor::Black {
            if parent.is_none() {
                break;
            }
            let p = parent.unwrap();

            if x == self.nodes[p].as_ref().unwrap().left {
                let mut w = self.nodes[p].as_ref().unwrap().right;
                if w.is_some() && self.nodes[w.unwrap()].as_ref().unwrap().color == RbColor::Red {
                    self.nodes[w.unwrap()].as_mut().unwrap().color = RbColor::Black;
                    self.nodes[p].as_mut().unwrap().color = RbColor::Red;
                    self.rotate_left(p);
                    w = self.nodes[p].as_ref().unwrap().right;
                }

                let w_idx = match w {
                    Some(idx) => idx,
                    None => {
                        x = parent;
                        parent = x.and_then(|idx| self.nodes[idx].as_ref().unwrap().parent);
                        continue;
                    }
                };

                let w_left_color = self.nodes[w_idx].as_ref().unwrap().left
                    .map_or(RbColor::Black, |idx| self.nodes[idx].as_ref().unwrap().color);
                let w_right_color = self.nodes[w_idx].as_ref().unwrap().right
                    .map_or(RbColor::Black, |idx| self.nodes[idx].as_ref().unwrap().color);

                if w_left_color == RbColor::Black && w_right_color == RbColor::Black {
                    self.nodes[w_idx].as_mut().unwrap().color = RbColor::Red;
                    x = parent;
                    parent = x.and_then(|idx| self.nodes[idx].as_ref().unwrap().parent);
                } else {
                    if w_right_color == RbColor::Black {
                        if let Some(wl) = self.nodes[w_idx].as_ref().unwrap().left {
                            self.nodes[wl].as_mut().unwrap().color = RbColor::Black;
                        }
                        self.nodes[w_idx].as_mut().unwrap().color = RbColor::Red;
                        self.rotate_right(w_idx);
                        w = self.nodes[p].as_ref().unwrap().right;
                    }

                    if let Some(w_final) = w {
                        let p_color = self.nodes[p].as_ref().unwrap().color;
                        self.nodes[w_final].as_mut().unwrap().color = p_color;
                        self.nodes[p].as_mut().unwrap().color = RbColor::Black;
                        if let Some(wr) = self.nodes[w_final].as_ref().unwrap().right {
                            self.nodes[wr].as_mut().unwrap().color = RbColor::Black;
                        }
                        self.rotate_left(p);
                    }
                    x = self.root;
                    break;
                }
            } else {
                let mut w = self.nodes[p].as_ref().unwrap().left;
                if w.is_some() && self.nodes[w.unwrap()].as_ref().unwrap().color == RbColor::Red {
                    self.nodes[w.unwrap()].as_mut().unwrap().color = RbColor::Black;
                    self.nodes[p].as_mut().unwrap().color = RbColor::Red;
                    self.rotate_right(p);
                    w = self.nodes[p].as_ref().unwrap().left;
                }

                let w_idx = match w {
                    Some(idx) => idx,
                    None => {
                        x = parent;
                        parent = x.and_then(|idx| self.nodes[idx].as_ref().unwrap().parent);
                        continue;
                    }
                };

                let w_left_color = self.nodes[w_idx].as_ref().unwrap().left
                    .map_or(RbColor::Black, |idx| self.nodes[idx].as_ref().unwrap().color);
                let w_right_color = self.nodes[w_idx].as_ref().unwrap().right
                    .map_or(RbColor::Black, |idx| self.nodes[idx].as_ref().unwrap().color);

                if w_left_color == RbColor::Black && w_right_color == RbColor::Black {
                    self.nodes[w_idx].as_mut().unwrap().color = RbColor::Red;
                    x = parent;
                    parent = x.and_then(|idx| self.nodes[idx].as_ref().unwrap().parent);
                } else {
                    if w_left_color == RbColor::Black {
                        if let Some(wr) = self.nodes[w_idx].as_ref().unwrap().right {
                            self.nodes[wr].as_mut().unwrap().color = RbColor::Black;
                        }
                        self.nodes[w_idx].as_mut().unwrap().color = RbColor::Red;
                        self.rotate_left(w_idx);
                        w = self.nodes[p].as_ref().unwrap().left;
                    }

                    if let Some(w_final) = w {
                        let p_color = self.nodes[p].as_ref().unwrap().color;
                        self.nodes[w_final].as_mut().unwrap().color = p_color;
                        self.nodes[p].as_mut().unwrap().color = RbColor::Black;
                        if let Some(wl) = self.nodes[w_final].as_ref().unwrap().left {
                            self.nodes[wl].as_mut().unwrap().color = RbColor::Black;
                        }
                        self.rotate_right(p);
                    }
                    x = self.root;
                    break;
                }
            }
        }

        if let Some(x_idx) = x {
            if let Some(ref mut n) = self.nodes[x_idx] {
                n.color = RbColor::Black;
            }
        }
    }
}

/// Per-CPU high-resolution timer queue managing ordered deadlines and dispatch.
pub struct PerCpuHrTimerQueue {
    cpu_id: usize,
    queue: RbTreeTimerQueue,
    total_armed: usize,
    total_expired: usize,
    total_cancelled: usize,
}

impl PerCpuHrTimerQueue {
    /// Creates a timer queue for the designated CPU.
    pub fn new(cpu_id: usize) -> Self {
        Self {
            cpu_id,
            queue: RbTreeTimerQueue::new(),
            total_armed: 0,
            total_expired: 0,
            total_cancelled: 0,
        }
    }

    /// Arms a high-resolution timer.
    pub fn arm(&mut self, timer: HrTimer) -> bool {
        self.total_armed += 1;
        self.queue.insert(timer)
    }

    /// Cancels an active timer by ID.
    pub fn cancel(&mut self, id: u64) -> Option<HrTimer> {
        let res = self.queue.cancel(id);
        if res.is_some() {
            self.total_cancelled += 1;
        }
        res
    }

    /// Returns the earliest absolute expiration timestamp in nanoseconds.
    #[inline]
    pub fn earliest_deadline(&self) -> Option<HrTimeNs> {
        self.queue.earliest_deadline()
    }

    /// Advances the timer queue to `now_ns`, popping and returning all expired timers.
    /// Periodic timers are automatically rescheduled for their next deadline.
    pub fn tick(&mut self, now_ns: HrTimeNs) -> Vec<HrTimer> {
        let mut expired = Vec::new();

        while let Some(deadline) = self.queue.earliest_deadline() {
            if deadline <= now_ns {
                if let Some(mut timer) = self.queue.pop_earliest() {
                    timer.state = HrTimerState::Expired;
                    self.total_expired += 1;

                    // If periodic, re-arm with updated deadline
                    if timer.mode == HrTimerMode::Periodic && timer.interval_ns > 0 {
                        let mut next_timer = timer;
                        next_timer.expires_ns = deadline + timer.interval_ns;
                        next_timer.state = HrTimerState::Enqueued;
                        self.queue.insert(next_timer);
                    }

                    expired.push(timer);
                }
            } else {
                break;
            }
        }

        expired
    }

    /// Target CPU ID.
    #[inline]
    pub fn cpu_id(&self) -> usize {
        self.cpu_id
    }

    /// Number of active timers.
    #[inline]
    pub fn len(&self) -> usize {
        self.queue.len()
    }

    /// True if no timers are active.
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.queue.is_empty()
    }

    /// Total timers armed over lifetime.
    #[inline]
    pub fn total_armed(&self) -> usize {
        self.total_armed
    }

    /// Total timers expired over lifetime.
    #[inline]
    pub fn total_expired(&self) -> usize {
        self.total_expired
    }

    /// Total timers cancelled over lifetime.
    #[inline]
    pub fn total_cancelled(&self) -> usize {
        self.total_cancelled
    }
}

/// Standard target display refresh rates for gaming workloads.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FrameRatePreset {
    Fps60,
    Fps120,
    Fps144,
    Fps165,
    Fps240,
    Custom(u32),
}

impl FrameRatePreset {
    /// Returns the target frame interval in nanoseconds for this refresh rate.
    pub const fn target_interval_ns(&self) -> HrTimeNs {
        match *self {
            FrameRatePreset::Fps60 => 16_666_666,   // 16.666 ms
            FrameRatePreset::Fps120 => 8_333_333,   // 8.333 ms
            FrameRatePreset::Fps144 => 6_944_444,   // 6.944 ms
            FrameRatePreset::Fps165 => 6_060_606,   // 6.060 ms
            FrameRatePreset::Fps240 => 4_166_666,   // 4.166 ms
            FrameRatePreset::Custom(hz) => {
                if hz == 0 {
                    16_666_666
                } else {
                    NS_PER_SEC / (hz as u64)
                }
            }
        }
    }

    /// Refresh rate in Hz.
    pub const fn hz(&self) -> u32 {
        match *self {
            FrameRatePreset::Fps60 => 60,
            FrameRatePreset::Fps120 => 120,
            FrameRatePreset::Fps144 => 144,
            FrameRatePreset::Fps165 => 165,
            FrameRatePreset::Fps240 => 240,
            FrameRatePreset::Custom(hz) => hz,
        }
    }
}

/// Result returned after evaluating frame presentation timing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FramePaceResult {
    /// Monotonic frame sequence index.
    pub frame_index: u64,
    /// Absolute target presentation deadline in nanoseconds.
    pub deadline_ns: HrTimeNs,
    /// True if the frame was presented before or at the deadline.
    pub deadline_met: bool,
    /// Temporal jitter in nanoseconds.
    pub jitter_ns: u64,
    /// Number of frames missed due to render overrun.
    pub missed_frames: u32,
}

/// Ultra-low jitter game frame pacing governor.
pub struct GameFramePacer {
    preset: FrameRatePreset,
    target_interval_ns: HrTimeNs,
    next_deadline_ns: HrTimeNs,
    frame_index: u64,
    total_frames: u64,
    total_missed_frames: u64,
    accumulated_jitter_ns: u64,
    max_jitter_ns: u64,
}

impl GameFramePacer {
    /// Creates a new game frame pacer initialized at `start_time_ns`.
    pub fn new(preset: FrameRatePreset, start_time_ns: HrTimeNs) -> Self {
        let target_interval_ns = preset.target_interval_ns();
        Self {
            preset,
            target_interval_ns,
            next_deadline_ns: start_time_ns + target_interval_ns,
            frame_index: 0,
            total_frames: 0,
            total_missed_frames: 0,
            accumulated_jitter_ns: 0,
            max_jitter_ns: 0,
        }
    }

    /// Dynamically switches the display refresh rate preset.
    pub fn set_preset(&mut self, preset: FrameRatePreset, now_ns: HrTimeNs) {
        self.preset = preset;
        self.target_interval_ns = preset.target_interval_ns();
        self.next_deadline_ns = now_ns + self.target_interval_ns;
    }

    /// Evaluates presentation of the current frame and advances deadline to next frame.
    pub fn advance_frame(&mut self, presentation_time_ns: HrTimeNs) -> FramePaceResult {
        let deadline = self.next_deadline_ns;
        self.frame_index += 1;
        self.total_frames += 1;

        let (deadline_met, jitter, missed) = if presentation_time_ns <= deadline {
            let j = deadline - presentation_time_ns;
            self.next_deadline_ns += self.target_interval_ns;
            (true, j, 0)
        } else {
            let lateness = presentation_time_ns - deadline;
            if lateness > self.target_interval_ns {
                let m = (lateness / self.target_interval_ns) as u32;
                self.next_deadline_ns = presentation_time_ns + self.target_interval_ns;
                (false, lateness, m)
            } else {
                self.next_deadline_ns += self.target_interval_ns;
                (false, lateness, 0)
            }
        };

        self.total_missed_frames += missed as u64;
        self.accumulated_jitter_ns += jitter;
        if jitter > self.max_jitter_ns {
            self.max_jitter_ns = jitter;
        }

        FramePaceResult {
            frame_index: self.frame_index,
            deadline_ns: deadline,
            deadline_met,
            jitter_ns: jitter,
            missed_frames: missed,
        }
    }

    /// Average frame delivery jitter in nanoseconds.
    #[inline]
    pub fn average_jitter_ns(&self) -> u64 {
        if self.total_frames == 0 {
            0
        } else {
            self.accumulated_jitter_ns / self.total_frames
        }
    }

    /// Peak jitter observed in nanoseconds.
    #[inline]
    pub fn max_jitter_ns(&self) -> u64 {
        self.max_jitter_ns
    }

    /// Total frames presented.
    #[inline]
    pub fn total_frames(&self) -> u64 {
        self.total_frames
    }

    /// Total frames dropped or missed.
    #[inline]
    pub fn total_missed_frames(&self) -> u64 {
        self.total_missed_frames
    }

    /// Active refresh rate preset.
    #[inline]
    pub fn preset(&self) -> FrameRatePreset {
        self.preset
    }

    /// Target frame interval in nanoseconds.
    #[inline]
    pub fn target_interval_ns(&self) -> HrTimeNs {
        self.target_interval_ns
    }

    /// Next presentation deadline in nanoseconds.
    #[inline]
    pub fn next_deadline_ns(&self) -> HrTimeNs {
        self.next_deadline_ns
    }
}
pub fn test_hrtimer_rb_tree_queue() -> bool {
    let mut q = RbTreeTimerQueue::new();
    if !q.is_empty() || q.len() != 0 || q.earliest_deadline().is_some() {
        return false;
    }

    // Insert out of order: 500ms, 100ms, 800ms, 200ms, 50ms (in ns)
    let deadlines = [
        500 * NS_PER_MS,
        100 * NS_PER_MS,
        800 * NS_PER_MS,
        200 * NS_PER_MS,
        50 * NS_PER_MS,
    ];

    for (i, &d) in deadlines.iter().enumerate() {
        let t = HrTimer::new_oneshot((i + 1) as u64, d, 0);
        q.insert(t);
    }

    if q.len() != 5 || q.is_empty() {
        return false;
    }

    // Earliest must be 50ms
    if q.earliest_deadline() != Some(50 * NS_PER_MS) {
        return false;
    }

    // Cancel timer with id = 4 (deadline 200ms)
    let cancelled = q.cancel(4);
    if cancelled.is_none() || cancelled.unwrap().expires_ns != 200 * NS_PER_MS {
        return false;
    }
    if q.len() != 4 {
        return false;
    }

    // Pop remaining timers; must emerge in strict ascending order: 50ms, 100ms, 500ms, 800ms
    let expected_order = [
        50 * NS_PER_MS,
        100 * NS_PER_MS,
        500 * NS_PER_MS,
        800 * NS_PER_MS,
    ];

    for &exp in &expected_order {
        match q.pop_earliest() {
            Some(t) => {
                if t.expires_ns != exp {
                    return false;
                }
            }
            None => return false,
        }
    }

    if !q.is_empty() || q.len() != 0 {
        return false;
    }

    // Test PerCpuHrTimerQueue tick dispatch and periodic reload
    let mut cpu_q = PerCpuHrTimerQueue::new(0);

    // One-shot at 100ms
    cpu_q.arm(HrTimer::new_oneshot(10, 100 * NS_PER_MS, 0));
    // Periodic at 60ms with 60ms period
    cpu_q.arm(HrTimer::new_periodic(20, 60 * NS_PER_MS, 60 * NS_PER_MS, 0));

    // Tick at 50ms -> 0 timers expired
    let exp0 = cpu_q.tick(50 * NS_PER_MS);
    if !exp0.is_empty() || cpu_q.len() != 2 {
        return false;
    }

    // Tick at 70ms -> periodic timer 20 expired and re-armed for 120ms
    let exp1 = cpu_q.tick(70 * NS_PER_MS);
    if exp1.len() != 1 || exp1[0].id != 20 {
        return false;
    }
    // Queue should still have 2 timers (timer 10 at 100ms, timer 20 at 120ms)
    if cpu_q.len() != 2 || cpu_q.earliest_deadline() != Some(100 * NS_PER_MS) {
        return false;
    }

    // Tick at 150ms -> both timer 10 (100ms) and timer 20 (120ms) expire
    let exp2 = cpu_q.tick(150 * NS_PER_MS);
    if exp2.len() != 2 {
        return false;
    }

    // Timer 10 was OneShot, so it was consumed; Timer 20 was reloaded to 180ms
    if cpu_q.len() != 1 || cpu_q.earliest_deadline() != Some(180 * NS_PER_MS) {
        return false;
    }

    true
}

/// Unit test verifying dynamic TSC-Deadline programmer and game frame pacing synchronizer.
pub fn test_tsc_deadline_and_frame_pacer() -> bool {
    // 1. Test GameFramePacer presets and jitter calculation
    let mut pacer = GameFramePacer::new(FrameRatePreset::Fps60, 0);

    if pacer.preset() != FrameRatePreset::Fps60
        || pacer.target_interval_ns() != 16_666_666
        || pacer.total_frames() != 0
    {
        return false;
    }

    // Frame 1 presented at 16,660,000 ns (6,666 ns before deadline)
    let res1 = pacer.advance_frame(16_660_000);
    if !res1.deadline_met || res1.missed_frames != 0 || res1.jitter_ns != 6_666 {
        return false;
    }

    // Switch to 144Hz preset
    pacer.set_preset(FrameRatePreset::Fps144, 16_660_000);
    if pacer.target_interval_ns() != 6_944_444 {
        return false;
    }

    // Simulate 100 frames at 144Hz with low jitter (< 100 microseconds)
    let mut curr_time = 16_660_000;
    for i in 0..100 {
        // Deterministic variation between -20us and +20us
        let var_ns: i64 = ((i % 5) as i64 - 2) * 10_000;
        curr_time += (6_944_444 as i64 + var_ns) as u64;
        pacer.advance_frame(curr_time);
    }

    if pacer.total_missed_frames() != 0 {
        return false;
    }

    // Simulate major render stall: 50ms spike (> 7 frames missed on 144Hz)
    curr_time += 50 * NS_PER_MS;
    let res_stall = pacer.advance_frame(curr_time);
    if res_stall.deadline_met || res_stall.missed_frames == 0 {
        return false;
    }
    if pacer.total_missed_frames() == 0 {
        return false;
    }

    true
}
