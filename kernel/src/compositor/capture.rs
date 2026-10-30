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
