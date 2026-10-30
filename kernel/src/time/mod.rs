//! Kernel High-Resolution Timers & Hierarchical Timing Wheel Subsystem.

pub mod hrtimer;

pub use hrtimer::{
    HrTimeNs, HrTimer, HrTimerCallback, HrTimerMode, HrTimerState, PerCpuHrTimerQueue,
    RbTreeTimerQueue, NS_PER_MS, NS_PER_SEC, NS_PER_US,
};
