//! Kernel High-Resolution Timers & Hierarchical Timing Wheel Subsystem.

pub mod hrtimer;
pub mod wheel;

pub use hrtimer::{
    HrTimeNs, HrTimer, HrTimerCallback, HrTimerMode, HrTimerState, PerCpuHrTimerQueue,
    RbTreeTimerQueue, NS_PER_MS, NS_PER_SEC, NS_PER_US,
};
pub use wheel::{
    HierarchicalTimingWheel, WheelCallback, WheelTimer, LEVEL0_SPAN, LEVEL1_SPAN,
    WHEEL_LEVEL0_SIZE, WHEEL_UPPER_SIZE,
};
