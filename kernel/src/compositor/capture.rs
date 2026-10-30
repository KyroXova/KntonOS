//! Hardware-Accelerated Frame Capture & Direct DRM Scanout Grabber.
//!
//! Provides direct frame grabbing from active DRM/KMS scanout buffers or compositor
//! backbuffers with monotonic timestamping, circular ring buffering, zero-copy dirty
//! rectangle extraction, and backpressure drop-frame tracking.

use alloc::vec::Vec;
use crate::compositor::dirty::Rect;
use crate::display::color::Color;

/// Errors that may occur during frame capture.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum CaptureError {
    InvalidDimensions,
    NullBufferPointer,
    BufferTooSmall,
    RingBufferFull,
}

/// Reasons for dropping a frame during capture or streaming.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum DropReason {
    QueueFull,
    EncoderBusy,
    FrameTooOld,
}

/// Policy for handling frame drops when the capture ring buffer is full.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum DropPolicy {
    /// Drop the newly incoming frame when the ring is full (preserves backlog).
    DropNewest,
    /// Evict the oldest unconsumed frame to make room for the latest frame (low-latency streaming).
    DropOldest,
}

/// Metadata describing a dropped frame.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct FrameDropInfo {
    pub dropped_seq: u64,
    pub reason: DropReason,
}

/// Metadata and raw pixel buffer for a captured frame.
#[derive(Clone, Debug)]
pub struct CapturedFrame {
    /// Monotonically increasing frame sequence number.
    pub seq: u64,
    /// Monotonic nanosecond timestamp at capture time.
    pub timestamp_ns: u64,
    /// Width of the frame in pixels.
    pub width: usize,
    /// Height of the frame in pixels.
    pub height: usize,
    /// Stride (pitch) in pixels.
    pub stride: usize,
    /// 32-bit ARGB/XRGB pixel payload.
    pub pixels: Vec<u32>,
    /// Optional bounding dirty rectangle if partial capture was performed.
    pub dirty_rect: Option<Rect>,
}

impl CapturedFrame {
    pub fn new(
        seq: u64,
        timestamp_ns: u64,
        width: usize,
        height: usize,
        stride: usize,
        pixels: Vec<u32>,
        dirty_rect: Option<Rect>,
    ) -> Self {
        Self {
            seq,
            timestamp_ns,
            width,
            height,
            stride,
            pixels,
            dirty_rect,
        }
    }

    /// Total size in bytes of the pixel payload.
    #[inline(always)]
    pub fn byte_len(&self) -> usize {
        self.pixels.len() * 4
    }
}

/// Zero-copy non-owning slice view over a bounding dirty rectangle in memory.
pub struct DirtyFrameSlice<'a> {
    buffer: &'a [u32],
    full_width: usize,
    full_height: usize,
    stride: usize,
    pub rect: Rect,
}

impl<'a> DirtyFrameSlice<'a> {
    pub fn new(
        buffer: &'a [u32],
        full_width: usize,
        full_height: usize,
        stride: usize,
        rect: Rect,
    ) -> Result<Self, CaptureError> {
        if full_width == 0 || full_height == 0 || stride < full_width {
            return Err(CaptureError::InvalidDimensions);
        }
        let clamped = rect.clamp(full_width, full_height);
        if clamped.is_empty() {
            return Err(CaptureError::InvalidDimensions);
        }
        let required_len = full_height * stride;
        if buffer.len() < required_len {
            return Err(CaptureError::BufferTooSmall);
        }
        Ok(Self {
            buffer,
            full_width,
            full_height,
            stride,
            rect: clamped,
        })
    }

    #[inline(always)]
    pub fn full_width(&self) -> usize {
        self.full_width
    }

    #[inline(always)]
    pub fn full_height(&self) -> usize {
        self.full_height
    }

    #[inline(always)]
    pub fn stride(&self) -> usize {
        self.stride
    }

    /// Returns a zero-copy row slice for relative row index `y` within the dirty rect.
    #[inline(always)]
    pub fn row(&self, y: usize) -> Option<&'a [u32]> {
        if y >= self.rect.height {
            return None;
        }
        let abs_y = self.rect.y + y;
        let start = abs_y * self.stride + self.rect.x;
        let end = start + self.rect.width;
        Some(&self.buffer[start..end])
    }

    /// Read a pixel at relative (x, y) within the dirty rect.
    #[inline(always)]
    pub fn pixel(&self, x: usize, y: usize) -> Option<u32> {
        if x >= self.rect.width || y >= self.rect.height {
            None
        } else {
            let abs_y = self.rect.y + y;
            let abs_x = self.rect.x + x;
            Some(self.buffer[abs_y * self.stride + abs_x])
        }
    }
}

/// Backpressure and frame-drop telemetry tracker.
#[derive(Clone, Debug, Default)]
pub struct BackpressureDropTracker {
    pub total_captured: u64,
    pub total_dropped: u64,
    pub consecutive_drops: u64,
    pub max_consecutive_drops: u64,
}

impl BackpressureDropTracker {
    pub const fn new() -> Self {
        Self {
            total_captured: 0,
            total_dropped: 0,
            consecutive_drops: 0,
            max_consecutive_drops: 0,
        }
    }

    pub fn record_capture(&mut self) {
        self.total_captured = self.total_captured.saturating_add(1);
        self.consecutive_drops = 0;
    }

    pub fn record_drop(&mut self, _reason: DropReason) {
        self.total_dropped = self.total_dropped.saturating_add(1);
        self.consecutive_drops = self.consecutive_drops.saturating_add(1);
        if self.consecutive_drops > self.max_consecutive_drops {
            self.max_consecutive_drops = self.consecutive_drops;
        }
    }

    /// Drop rate as parts-per-million (e.g. 50_000 = 5.0%).
    pub fn drop_rate_ppm(&self) -> u32 {
        let total = self.total_captured.saturating_add(self.total_dropped);
        if total == 0 {
            0
        } else {
            ((self.total_dropped as u128 * 1_000_000) / (total as u128)) as u32
        }
    }

    /// Percentage of dropped frames as an integer 0..100.
    pub fn drop_percentage(&self) -> u32 {
        let total = self.total_captured.saturating_add(self.total_dropped);
        if total == 0 {
            0
        } else {
            ((self.total_dropped * 100) / total) as u32
        }
    }

    pub fn reset_stats(&mut self) {
        self.total_captured = 0;
        self.total_dropped = 0;
        self.consecutive_drops = 0;
        self.max_consecutive_drops = 0;
    }
}

/// Fixed-capacity circular ring buffer for captured frames.
pub struct CaptureRingBuffer {
    buffer: Vec<Option<CapturedFrame>>,
    capacity: usize,
    head: usize,
    tail: usize,
    count: usize,
}

impl CaptureRingBuffer {
    pub fn new(capacity: usize) -> Self {
        let cap = if capacity == 0 { 8 } else { capacity };
        let mut buffer = Vec::with_capacity(cap);
        for _ in 0..cap {
            buffer.push(None);
        }
        Self {
            buffer,
            capacity: cap,
            head: 0,
            tail: 0,
            count: 0,
        }
    }

    #[inline(always)]
    pub fn capacity(&self) -> usize {
        self.capacity
    }

    #[inline(always)]
    pub fn len(&self) -> usize {
        self.count
    }

    #[inline(always)]
    pub fn is_empty(&self) -> bool {
        self.count == 0
    }

    #[inline(always)]
    pub fn is_full(&self) -> bool {
        self.count >= self.capacity
    }

    /// Enqueues a captured frame. If full, returns `Err(CaptureError::RingBufferFull)`.
    pub fn push(&mut self, frame: CapturedFrame) -> Result<(), CaptureError> {
        if self.is_full() {
            return Err(CaptureError::RingBufferFull);
        }
        self.buffer[self.tail] = Some(frame);
        self.tail = (self.tail + 1) % self.capacity;
        self.count += 1;
        Ok(())
    }

    /// Enqueues a captured frame using the specified backpressure policy, updating the drop tracker.
    /// Returns `Ok(())` on success, or `Err(FrameDropInfo)` if a frame was dropped.
    pub fn push_with_policy(
        &mut self,
        frame: CapturedFrame,
        policy: DropPolicy,
        tracker: &mut BackpressureDropTracker,
    ) -> Result<(), FrameDropInfo> {
        if !self.is_full() {
            tracker.record_capture();
            let _ = self.push(frame);
            Ok(())
        } else {
            match policy {
                DropPolicy::DropNewest => {
                    let dropped_seq = frame.seq;
                    tracker.record_drop(DropReason::QueueFull);
                    Err(FrameDropInfo {
                        dropped_seq,
                        reason: DropReason::QueueFull,
                    })
                }
                DropPolicy::DropOldest => {
                    let evicted = self.pop().expect("ring full must have elements");
                    let dropped_seq = evicted.seq;
                    tracker.record_drop(DropReason::QueueFull);
                    tracker.record_capture();
                    let _ = self.push(frame);
                    Err(FrameDropInfo {
                        dropped_seq,
                        reason: DropReason::QueueFull,
                    })
                }
            }
        }
    }

    /// Dequeues the oldest captured frame.
    pub fn pop(&mut self) -> Option<CapturedFrame> {
        if self.is_empty() {
            return None;
        }
        let frame = self.buffer[self.head].take();
        self.head = (self.head + 1) % self.capacity;
        self.count -= 1;
        frame
    }

    /// Peeks at the oldest captured frame without removing it.
    pub fn peek(&self) -> Option<&CapturedFrame> {
        if self.is_empty() {
            None
        } else {
            self.buffer[self.head].as_ref()
        }
    }

    /// Clears all frames from the ring buffer.
    pub fn clear(&mut self) {
        for slot in self.buffer.iter_mut() {
            *slot = None;
        }
        self.head = 0;
        self.tail = 0;
        self.count = 0;
    }
}

/// Direct scanout frame grabber.
pub struct FrameGrabber {
    next_seq: u64,
}

impl FrameGrabber {
    pub const fn new() -> Self {
        Self { next_seq: 1 }
    }

    /// Grabs a frame directly from a compositor backbuffer of `Color` pixels.
    pub fn grab_from_backbuffer(
        &mut self,
        backbuffer: &[Color],
        width: usize,
        height: usize,
        timestamp_ns: u64,
    ) -> Result<CapturedFrame, CaptureError> {
        if width == 0 || height == 0 {
            return Err(CaptureError::InvalidDimensions);
        }
        let total_pixels = width * height;
        if backbuffer.len() < total_pixels {
            return Err(CaptureError::BufferTooSmall);
        }

        let mut pixels = Vec::with_capacity(total_pixels);
        for c in &backbuffer[..total_pixels] {
            // ARGB8888 representation: (A << 24) | (R << 16) | (G << 8) | B
            let val = ((c.a as u32) << 24) | ((c.r as u32) << 16) | ((c.g as u32) << 8) | (c.b as u32);
            pixels.push(val);
        }

        let seq = self.next_seq;
        self.next_seq += 1;

        Ok(CapturedFrame::new(
            seq,
            timestamp_ns,
            width,
            height,
            width,
            pixels,
            None,
        ))
    }

    /// Grabs a frame directly from a raw DRM/KMS scanout memory pointer.
    pub fn grab_from_raw_drm(
        &mut self,
        ptr: *const u32,
        width: usize,
        height: usize,
        stride_pixels: usize,
        timestamp_ns: u64,
    ) -> Result<CapturedFrame, CaptureError> {
        if ptr.is_null() {
            return Err(CaptureError::NullBufferPointer);
        }
        if width == 0 || height == 0 || stride_pixels < width {
            return Err(CaptureError::InvalidDimensions);
        }

        let mut pixels = Vec::with_capacity(width * height);
        unsafe {
            for y in 0..height {
                let row_ptr = ptr.add(y * stride_pixels);
                for x in 0..width {
                    pixels.push(*row_ptr.add(x));
                }
            }
        }

        let seq = self.next_seq;
        self.next_seq += 1;

        Ok(CapturedFrame::new(
            seq,
            timestamp_ns,
            width,
            height,
            width,
            pixels,
            None,
        ))
    }

    /// Grabs only a dirty sub-rectangle from a compositor backbuffer of `Color` pixels.
    pub fn grab_dirty_rect_from_backbuffer(
        &mut self,
        backbuffer: &[Color],
        width: usize,
        height: usize,
        dirty: Rect,
        timestamp_ns: u64,
    ) -> Result<CapturedFrame, CaptureError> {
        if width == 0 || height == 0 {
            return Err(CaptureError::InvalidDimensions);
        }
        let clamped = dirty.clamp(width, height);
        if clamped.is_empty() {
            return Err(CaptureError::InvalidDimensions);
        }
        let total_pixels = width * height;
        if backbuffer.len() < total_pixels {
            return Err(CaptureError::BufferTooSmall);
        }

        let mut pixels = Vec::with_capacity(clamped.width * clamped.height);
        for y in clamped.y..clamped.bottom() {
            let row_offset = y * width;
            for x in clamped.x..clamped.right() {
                let c = backbuffer[row_offset + x];
                let val = ((c.a as u32) << 24) | ((c.r as u32) << 16) | ((c.g as u32) << 8) | (c.b as u32);
                pixels.push(val);
            }
        }

        let seq = self.next_seq;
        self.next_seq += 1;

        Ok(CapturedFrame::new(
            seq,
            timestamp_ns,
            clamped.width,
            clamped.height,
            clamped.width,
            pixels,
            Some(clamped),
        ))
    }

    /// Grabs only a dirty sub-rectangle from a raw DRM scanout pointer.
    pub fn grab_dirty_rect_from_raw_drm(
        &mut self,
        ptr: *const u32,
        width: usize,
        height: usize,
        stride_pixels: usize,
        dirty: Rect,
        timestamp_ns: u64,
    ) -> Result<CapturedFrame, CaptureError> {
        if ptr.is_null() {
            return Err(CaptureError::NullBufferPointer);
        }
        if width == 0 || height == 0 || stride_pixels < width {
            return Err(CaptureError::InvalidDimensions);
        }
        let clamped = dirty.clamp(width, height);
        if clamped.is_empty() {
            return Err(CaptureError::InvalidDimensions);
        }

        let mut pixels = Vec::with_capacity(clamped.width * clamped.height);
        unsafe {
            for y in clamped.y..clamped.bottom() {
                let row_ptr = ptr.add(y * stride_pixels + clamped.x);
                for x in 0..clamped.width {
                    pixels.push(*row_ptr.add(x));
                }
            }
        }

        let seq = self.next_seq;
        self.next_seq += 1;

        Ok(CapturedFrame::new(
            seq,
            timestamp_ns,
            clamped.width,
            clamped.height,
            clamped.width,
            pixels,
            Some(clamped),
        ))
    }
}

/// Unit test verifying frame grabber and capture ring buffer.
pub fn test_frame_grabber_and_ring() -> bool {
    let mut grabber = FrameGrabber::new();
    let colors = [
        Color::rgb(255, 0, 0),
        Color::rgb(0, 255, 0),
        Color::rgb(0, 0, 255),
        Color::rgb(255, 255, 255),
    ];
    let frame = grabber
        .grab_from_backbuffer(&colors, 2, 2, 1_000_000_000)
        .expect("grab failed");
    if frame.seq != 1 || frame.width != 2 || frame.height != 2 || frame.pixels.len() != 4 {
        return false;
    }
    // Check ARGB8888 conversion
    if frame.pixels[0] != 0xFF_FF_00_00 || frame.pixels[1] != 0xFF_00_FF_00 {
        return false;
    }

    let mut ring = CaptureRingBuffer::new(2);
    if !ring.is_empty() || ring.is_full() {
        return false;
    }
    ring.push(frame).expect("push failed");
    if ring.len() != 1 || ring.is_empty() {
        return false;
    }
    let frame2 = grabber
        .grab_from_backbuffer(&colors, 2, 2, 1_016_666_666)
        .expect("grab 2 failed");
    ring.push(frame2).expect("push 2 failed");
    if !ring.is_full() {
        return false;
    }
    // Push on full returns RingBufferFull
    let frame3 = grabber
        .grab_from_backbuffer(&colors, 2, 2, 1_033_333_333)
        .expect("grab 3 failed");
    if ring.push(frame3) != Err(CaptureError::RingBufferFull) {
        return false;
    }
    let popped = ring.pop().expect("pop failed");
    if popped.seq != 1 {
        return false;
    }
    if ring.len() != 1 {
        return false;
    }
    true
}

/// Unit test verifying dirty rectangle bounded capture, zero-copy slices, and backpressure tracker.
pub fn test_dirty_rect_and_backpressure() -> bool {
    let mut grabber = FrameGrabber::new();
    let colors = [
        Color::rgb(10, 10, 10), Color::rgb(20, 20, 20), Color::rgb(30, 30, 30),
        Color::rgb(40, 40, 40), Color::rgb(50, 50, 50), Color::rgb(60, 60, 60),
        Color::rgb(70, 70, 70), Color::rgb(80, 80, 80), Color::rgb(90, 90, 90),
    ]; // 3x3
    let dirty = Rect::new(1, 1, 2, 2);
    let subframe = grabber
        .grab_dirty_rect_from_backbuffer(&colors, 3, 3, dirty, 2_000_000_000)
        .expect("dirty grab failed");
    if subframe.width != 2 || subframe.height != 2 || subframe.pixels.len() != 4 {
        return false;
    }
    if subframe.dirty_rect != Some(dirty) {
        return false;
    }

    // Zero-copy DirtyFrameSlice test
    let raw_buf = [
        100, 101, 102,
        200, 201, 202,
        300, 301, 302,
    ];
    let slice = DirtyFrameSlice::new(&raw_buf, 3, 3, 3, dirty).expect("slice creation failed");
    if slice.rect.width != 2 || slice.rect.height != 2 {
        return false;
    }
    let row0 = slice.row(0).expect("row0 missing");
    if row0 != &[201, 202] {
        return false;
    }
    let row1 = slice.row(1).expect("row1 missing");
    if row1 != &[301, 302] {
        return false;
    }

    // Backpressure tracker & policies test
    let mut ring = CaptureRingBuffer::new(2);
    let mut tracker = BackpressureDropTracker::new();

    let f1 = grabber.grab_from_backbuffer(&colors, 3, 3, 100).expect("f1 failed");
    let f1_seq = f1.seq;
    let f2 = grabber.grab_from_backbuffer(&colors, 3, 3, 200).expect("f2 failed");
    let f2_seq = f2.seq;
    let f3 = grabber.grab_from_backbuffer(&colors, 3, 3, 300).expect("f3 failed");

    // Push 2 frames: ring becomes full
    if ring.push_with_policy(f1, DropPolicy::DropNewest, &mut tracker).is_err() {
        return false;
    }
    if ring.push_with_policy(f2, DropPolicy::DropNewest, &mut tracker).is_err() {
        return false;
    }
    if tracker.total_captured != 2 || tracker.total_dropped != 0 {
        return false;
    }

    // Push 3rd with DropNewest: f3 dropped
    let drop_res = ring.push_with_policy(f3.clone(), DropPolicy::DropNewest, &mut tracker);
    if let Err(info) = drop_res {
        if info.dropped_seq != f3.seq {
            return false;
        }
    } else {
        return false;
    }
    if tracker.total_dropped != 1 || tracker.consecutive_drops != 1 {
        return false;
    }

    // Push 4th with DropOldest: f1 evicted, f4 enqueued
    let f4 = grabber.grab_from_backbuffer(&colors, 3, 3, 400).expect("f4 failed");
    let drop_old = ring.push_with_policy(f4.clone(), DropPolicy::DropOldest, &mut tracker);
    if let Err(info) = drop_old {
        // Evicted oldest is f1
        if info.dropped_seq != f1_seq {
            return false;
        }
    } else {
        return false;
    }
    if tracker.total_captured != 3 || tracker.total_dropped != 2 {
        return false;
    }
    // Oldest in ring now is f2
    let popped = ring.pop().expect("pop failed");
    if popped.seq != f2_seq {
        return false;
    }

    true
}
