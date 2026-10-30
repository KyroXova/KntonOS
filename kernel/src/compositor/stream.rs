//! Hardware Dumb-Buffer Video Streamer Engine & Planar Color Formatter.
//! Provides color space conversion from ARGB8888 to hardware video encoder planar layouts

use alloc::vec;
use alloc::vec::Vec;
use crate::compositor::capture::CapturedFrame;

/// Supported hardware encoder video pixel formats.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum VideoEncoderFormat {
    /// Bi-planar Y + interleaved UV layout (standard for VAAPI, NVENC, QuickSync).
    Nv12,
    /// Tri-planar Y + U + V layout (standard I420).
    I420,
    /// Raw uncompressed 32-bit ARGB8888.
    Argb8888,
}

/// Errors occurring during stream formatting or frame pacing.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum StreamError {
    InvalidDimensions,
    BufferLengthMismatch,
    UnsupportedFormat,
    NoFramesAvailable,
}

/// Planar video packet produced by the streamer pipeline for encoder consumption.
#[derive(Clone, Debug)]
pub struct EncodedVideoPacket {
    pub seq: u64,
    pub timestamp_ns: u64,
    pub width: usize,
    pub height: usize,
    pub format: VideoEncoderFormat,
    pub payload: Vec<u8>,
    pub is_keyframe: bool,
}

impl EncodedVideoPacket {
    #[inline(always)]
    pub fn size_bytes(&self) -> usize {
        self.payload.len()
    }
}

/// High-precision integer color space converter for ARGB8888 to YUV planar layouts.
pub struct VideoFormatConverter;

impl VideoFormatConverter {
    /// Converts a single 32-bit ARGB pixel to standard BT.601 (Y, U, V) components.
    #[inline(always)]
    pub fn rgb_to_yuv(argb: u32) -> (u8, u8, u8) {
        let r = ((argb >> 16) & 0xFF) as i32;
        let g = ((argb >> 8) & 0xFF) as i32;
        let b = (argb & 0xFF) as i32;

        // ITU-R BT.601 integer math with rounding
        let y = ((66 * r + 129 * g + 25 * b + 128) >> 8) + 16;
        let u = ((-38 * r - 74 * g + 112 * b + 128) >> 8) + 128;
        let v = ((112 * r - 94 * g - 18 * b + 128) >> 8) + 128;

        (
            y.clamp(0, 255) as u8,
            u.clamp(0, 255) as u8,
            v.clamp(0, 255) as u8,
        )
    }

    /// Converts ARGB8888 pixels into NV12 bi-planar format (Y plane followed by interleaved UV plane).
    /// Dimensions must be even for 2x2 chroma subsampling.
    pub fn argb_to_nv12(pixels: &[u32], width: usize, height: usize) -> Result<Vec<u8>, StreamError> {
        if width == 0 || height == 0 || width % 2 != 0 || height % 2 != 0 {
            return Err(StreamError::InvalidDimensions);
        }
        let total_pixels = width * height;
        if pixels.len() < total_pixels {
            return Err(StreamError::BufferLengthMismatch);
        }

        let y_size = total_pixels;
        let uv_size = total_pixels / 2;
        let mut out = vec![0u8; y_size + uv_size];

        // 1. Calculate full-resolution Y plane
        for i in 0..total_pixels {
            let (y, _, _) = Self::rgb_to_yuv(pixels[i]);
            out[i] = y;
        }

        // 2. Calculate subsampled UV plane: 2x2 blocks average U and V
        let mut uv_offset = y_size;
        for by in (0..height).step_by(2) {
            let row0 = by * width;
            let row1 = (by + 1) * width;
            for bx in (0..width).step_by(2) {
                let p00 = pixels[row0 + bx];
                let p01 = pixels[row0 + bx + 1];
                let p10 = pixels[row1 + bx];
                let p11 = pixels[row1 + bx + 1];

                let (_, u00, v00) = Self::rgb_to_yuv(p00);
                let (_, u01, v01) = Self::rgb_to_yuv(p01);
                let (_, u10, v10) = Self::rgb_to_yuv(p10);
                let (_, u11, v11) = Self::rgb_to_yuv(p11);

                let avg_u = ((u00 as u16 + u01 as u16 + u10 as u16 + u11 as u16) / 4) as u8;
                let avg_v = ((v00 as u16 + v01 as u16 + v10 as u16 + v11 as u16) / 4) as u8;

                out[uv_offset] = avg_u;
                out[uv_offset + 1] = avg_v;
                uv_offset += 2;
            }
        }

        Ok(out)
    }

    /// Converts ARGB8888 pixels into I420 tri-planar format (Y plane, U plane, V plane).
    pub fn argb_to_i420(pixels: &[u32], width: usize, height: usize) -> Result<Vec<u8>, StreamError> {
        if width == 0 || height == 0 || width % 2 != 0 || height % 2 != 0 {
            return Err(StreamError::InvalidDimensions);
        }
        let total_pixels = width * height;
        if pixels.len() < total_pixels {
            return Err(StreamError::BufferLengthMismatch);
        }

        let y_size = total_pixels;
        let chroma_size = total_pixels / 4;
        let mut out = vec![0u8; y_size + chroma_size * 2];

        // 1. Full-resolution Y plane
        for i in 0..total_pixels {
            let (y, _, _) = Self::rgb_to_yuv(pixels[i]);
            out[i] = y;
        }

        // 2. Subsampled U and V planes
        let u_base = y_size;
        let v_base = y_size + chroma_size;
        let chroma_w = width / 2;

        for by in (0..height).step_by(2) {
            let row0 = by * width;
            let row1 = (by + 1) * width;
            let cy = by / 2;
            for bx in (0..width).step_by(2) {
                let cx = bx / 2;
                let p00 = pixels[row0 + bx];
                let p01 = pixels[row0 + bx + 1];
                let p10 = pixels[row1 + bx];
                let p11 = pixels[row1 + bx + 1];

                let (_, u00, v00) = Self::rgb_to_yuv(p00);
                let (_, u01, v01) = Self::rgb_to_yuv(p01);
                let (_, u10, v10) = Self::rgb_to_yuv(p10);
                let (_, u11, v11) = Self::rgb_to_yuv(p11);

                let avg_u = ((u00 as u16 + u01 as u16 + u10 as u16 + u11 as u16) / 4) as u8;
                let avg_v = ((v00 as u16 + v01 as u16 + v10 as u16 + v11 as u16) / 4) as u8;

                let c_idx = cy * chroma_w + cx;
                out[u_base + c_idx] = avg_u;
                out[v_base + c_idx] = avg_v;
            }
        }

        Ok(out)
    }
}

/// Monotonic nanosecond frame pacer for targeting fixed refresh rates (30, 60, 120 FPS).
pub struct FramePacer {
    target_fps: u32,
    frame_interval_ns: u64,
    last_frame_ts_ns: u64,
}

impl FramePacer {
    pub fn new(target_fps: u32) -> Self {
        let fps = if target_fps == 0 { 60 } else { target_fps };
        let interval = 1_000_000_000 / (fps as u64);
        Self {
            target_fps: fps,
            frame_interval_ns: interval,
            last_frame_ts_ns: 0,
        }
    }

    #[inline(always)]
    pub fn target_fps(&self) -> u32 {
        self.target_fps
    }

    #[inline(always)]
    pub fn frame_interval_ns(&self) -> u64 {
        self.frame_interval_ns
    }

    pub fn set_target_fps(&mut self, target_fps: u32) {
        let fps = if target_fps == 0 { 60 } else { target_fps };
        self.target_fps = fps;
        self.frame_interval_ns = 1_000_000_000 / (fps as u64);
    }

    /// Evaluates whether a new frame should be emitted based on monotonic timestamp.
    pub fn should_emit_frame(&self, current_ts_ns: u64) -> bool {
        if self.last_frame_ts_ns == 0 {
            return true;
        }
        current_ts_ns >= self.last_frame_ts_ns + self.frame_interval_ns
    }

    /// Advances the pacer timestamp clock.
    pub fn advance_pacing(&mut self, current_ts_ns: u64) {
        self.last_frame_ts_ns = current_ts_ns;
    }

    /// Calculates jitter relative to expected frame interval (positive = late, negative = early).
    pub fn jitter_ns(&self, current_ts_ns: u64) -> i64 {
        if self.last_frame_ts_ns == 0 {
            0
        } else {
            let elapsed = (current_ts_ns as i64) - (self.last_frame_ts_ns as i64);
            elapsed - (self.frame_interval_ns as i64)
        }
    }

    pub fn reset(&mut self) {
        self.last_frame_ts_ns = 0;
    }
}

/// Dumb-buffer video streamer pipeline engine.
pub struct DumbBufferStreamer {
    format: VideoEncoderFormat,
    pacer: FramePacer,
    keyframe_interval: u64,
    frames_since_keyframe: u64,
}

impl DumbBufferStreamer {
    pub fn new(format: VideoEncoderFormat, target_fps: u32, keyframe_interval: u64) -> Self {
        Self {
            format,
            pacer: FramePacer::new(target_fps),
            keyframe_interval: if keyframe_interval == 0 { 30 } else { keyframe_interval },
            frames_since_keyframe: 0,
        }
    }

    #[inline(always)]
    pub fn format(&self) -> VideoEncoderFormat {
        self.format
    }

    #[inline(always)]
    pub fn pacer(&self) -> &FramePacer {
        &self.pacer
    }

    #[inline(always)]
    pub fn pacer_mut(&mut self) -> &mut FramePacer {
        &mut self.pacer
    }

    /// Processes a captured frame through the streamer pipeline.
    /// If pacing dictates emitting, converts to the target planar format and returns `Ok(Some(packet))`.
    pub fn process_frame(&mut self, frame: &CapturedFrame) -> Result<Option<EncodedVideoPacket>, StreamError> {
        if !self.pacer.should_emit_frame(frame.timestamp_ns) {
            return Ok(None);
        }

        let is_keyframe = self.frames_since_keyframe == 0
            || self.frames_since_keyframe >= self.keyframe_interval;

        let payload = match self.format {
            VideoEncoderFormat::Nv12 => {
                VideoFormatConverter::argb_to_nv12(&frame.pixels, frame.width, frame.height)?
            }
            VideoEncoderFormat::I420 => {
                VideoFormatConverter::argb_to_i420(&frame.pixels, frame.width, frame.height)?
            }
            VideoEncoderFormat::Argb8888 => {
                let mut raw = Vec::with_capacity(frame.pixels.len() * 4);
                for p in &frame.pixels {
                    raw.extend_from_slice(&p.to_ne_bytes());
                }
                raw
            }
        };

        self.pacer.advance_pacing(frame.timestamp_ns);
        if is_keyframe {
            self.frames_since_keyframe = 1;
        } else {
            self.frames_since_keyframe += 1;
        }

        Ok(Some(EncodedVideoPacket {
            seq: frame.seq,
            timestamp_ns: frame.timestamp_ns,
            width: frame.width,
            height: frame.height,
            format: self.format,
            payload,
            is_keyframe,
        }))
    }
}

/// Unit test verifying ARGB to NV12/I420 conversion and frame pacing.
pub fn test_video_streamer_and_pacer() -> bool {
    // 1. Test color conversion: 2x2 white pixels (0xFF_FF_FF_FF)
    let white_pixels = [0xFF_FF_FF_FFu32; 4];
    let nv12 = VideoFormatConverter::argb_to_nv12(&white_pixels, 2, 2).expect("nv12 failed");
    // NV12 size: 2*2 + (2*2)/2 = 6 bytes
    if nv12.len() != 6 {
        return false;
    }
    // White in BT.601: Y ~ 235, U ~ 128, V ~ 128
    if nv12[0] < 200 || nv12[4] < 120 || nv12[4] > 136 || nv12[5] < 120 || nv12[5] > 136 {
        return false;
    }

    // 2. Test I420 size: 4 Y + 1 U + 1 V = 6 bytes
    let i420 = VideoFormatConverter::argb_to_i420(&white_pixels, 2, 2).expect("i420 failed");
    if i420.len() != 6 {
        return false;
    }

    // 3. Test FramePacer (60 FPS -> 16.666 ms interval)
    let mut pacer = FramePacer::new(60);
    if pacer.frame_interval_ns() != 16_666_666 {
        return false;
    }
    // First frame always emits
    if !pacer.should_emit_frame(1_000_000_000) {
        return false;
    }
    pacer.advance_pacing(1_000_000_000);

    // After 10 ms (too early) -> false
    if pacer.should_emit_frame(1_010_000_000) {
        return false;
    }

    // After 17 ms -> true
    if !pacer.should_emit_frame(1_017_000_000) {
        return false;
    }
    pacer.advance_pacing(1_017_000_000);

    // 4. Test DumbBufferStreamer end-to-end
    let mut streamer = DumbBufferStreamer::new(VideoEncoderFormat::Nv12, 60, 2);
    let f1 = CapturedFrame::new(1, 1_000_000_000, 2, 2, 2, white_pixels.to_vec(), None);
    let pkt1 = streamer.process_frame(&f1).expect("process f1").expect("emit f1");
    if !pkt1.is_keyframe || pkt1.format != VideoEncoderFormat::Nv12 || pkt1.payload.len() != 6 {
        return false;
    }

    // Second frame too early -> throttled
    let f2 = CapturedFrame::new(2, 1_005_000_000, 2, 2, 2, white_pixels.to_vec(), None);
    let pkt2 = streamer.process_frame(&f2).expect("process f2");
    if pkt2.is_some() {
        return false;
    }

    // Third frame on time -> delta frame emitted
    let f3 = CapturedFrame::new(3, 1_020_000_000, 2, 2, 2, white_pixels.to_vec(), None);
    let pkt3 = streamer.process_frame(&f3).expect("process f3").expect("emit f3");
    if pkt3.is_keyframe {
        return false; // keyframe interval is 2
    }

    true
}
