//! In-Game Hardware Plane Overlay (Plane #2) Abstraction & Alpha Presentation.
//!
//! Maps a dedicated hardware display overlay plane (Plane #2 / zpos 1) above the primary
//! game rendering plane (Plane #1 / zpos 0) and below hardware cursor (Plane #3 / zpos 2).
//! Provides per-pixel ARGB8888 and plane-wide alpha blending without disturbing primary game buffers.

use alloc::vec::Vec;
use crate::compositor::dirty::Rect;

// -----------------------------------------------------------------------------
// Constants and Plane Identifier Defaults
// -----------------------------------------------------------------------------

/// Standard hardware plane IDs in DRM/KMS scanout pipeline
pub const HARDWARE_PLANE_PRIMARY_ID: u32 = 1;
pub const HARDWARE_PLANE_OVERLAY_ID: u32 = 2; // Plane #2
pub const HARDWARE_PLANE_CURSOR_ID: u32 = 3;

/// Default overlay Z-position relative to primary scanout
pub const OVERLAY_ZPOS: u32 = 1;

/// Color format for alpha-blended overlay plane
pub const OVERLAY_FORMAT_ARGB8888: u32 = 0x34325241; // FourCC AR24

// -----------------------------------------------------------------------------
// Hardware Overlay Plane Abstraction
// -----------------------------------------------------------------------------

/// State and geometry of the dedicated hardware overlay plane (Plane #2).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HardwareOverlayPlane {
    pub plane_id: u32,
    pub zpos: u32,
    pub visible: bool,
    pub plane_alpha: u16, // 0..0xFFFF (0xFFFF = 100% opaque, 0x8000 = ~50% opaque)
    pub bounds: Rect,     // On-screen destination rectangle (X, Y, W, H)
    pub buffer_width: usize,
    pub buffer_height: usize,
    pub buffer: Vec<u32>, // ARGB8888 pixel buffer (32-bit: A=31:24, R=23:16, G=15:8, B=7:0)
    pub dirty_rect: Rect, // Bounding box of damaged pixels since last presentation
}

impl HardwareOverlayPlane {
    pub fn new(plane_id: u32, width: usize, height: usize) -> Self {
        let size = width * height;
        let mut buffer = Vec::with_capacity(size);
        buffer.resize(size, 0); // Transparent black

        Self {
            plane_id,
            zpos: OVERLAY_ZPOS,
            visible: false,
            plane_alpha: 0xE666, // ~90% default opacity (230/255)
            bounds: Rect::new(0, 0, width, height),
            buffer_width: width,
            buffer_height: height,
            buffer,
            dirty_rect: Rect::empty(),
        }
    }

    /// Clears overlay buffer to transparent black (0x0000_0000).
    pub fn clear(&mut self) {
        for pixel in self.buffer.iter_mut() {
            *pixel = 0;
        }
        self.dirty_rect = Rect::new(0, 0, self.buffer_width, self.buffer_height);
    }

    /// Sets or clears visibility of Plane #2.
    pub fn set_visible(&mut self, visible: bool) {
        if self.visible != visible {
            self.visible = visible;
            self.dirty_rect = self.bounds;
        }
    }

    /// Resizes or repositions destination rectangle on display output.
    pub fn set_bounds(&mut self, x: usize, y: usize, width: usize, height: usize) {
        self.bounds = Rect::new(x, y, width, height);
        self.dirty_rect = self.bounds;
    }

    /// Plots a pixel with 32-bit ARGB8888 color and marks dirty bounding box.
    #[inline(always)]
    pub fn write_pixel(&mut self, x: usize, y: usize, argb: u32) {
        if x < self.buffer_width && y < self.buffer_height {
            let idx = y * self.buffer_width + x;
            self.buffer[idx] = argb;
            self.mark_dirty_point(x, y);
        }
    }

    /// Reads a pixel from the overlay buffer.
    #[inline(always)]
    pub fn read_pixel(&self, x: usize, y: usize) -> u32 {
        if x < self.buffer_width && y < self.buffer_height {
            self.buffer[y * self.buffer_width + x]
        } else {
            0
        }
    }

    /// Marks a single coordinate as dirty, extending dirty bounding rectangle.
    pub fn mark_dirty_point(&mut self, x: usize, y: usize) {
        if self.dirty_rect.is_empty() {
            self.dirty_rect = Rect::new(x, y, 1, 1);
        } else {
            let left = self.dirty_rect.x.min(x);
            let top = self.dirty_rect.y.min(y);
            let right = self.dirty_rect.right().max(x + 1);
            let bottom = self.dirty_rect.bottom().max(y + 1);
            self.dirty_rect = Rect::new(left, top, right - left, bottom - top);
        }
    }

    /// Marks a rectangular region as dirty.
    pub fn mark_dirty_rect(&mut self, rect: Rect) {
        if self.dirty_rect.is_empty() {
            self.dirty_rect = rect;
        } else if !rect.is_empty() {
            let left = self.dirty_rect.x.min(rect.x);
            let top = self.dirty_rect.y.min(rect.y);
            let right = self.dirty_rect.right().max(rect.right());
            let bottom = self.dirty_rect.bottom().max(rect.bottom());
            self.dirty_rect = Rect::new(left, top, right - left, bottom - top);
        }
    }

    /// Resets dirty bounding box after scanout presentation.
    pub fn clear_dirty(&mut self) {
        self.dirty_rect = Rect::empty();
    }

    /// Performs alpha-blended software composition of Plane #2 over a primary framebuffer.
    /// Used for hardware configurations where Plane #2 is blended via display engine scanout.
    pub fn blend_over(
        &self,
        primary_fb: &mut [u32],
        primary_width: usize,
        primary_height: usize,
    ) -> usize {
        if !self.visible || self.dirty_rect.is_empty() {
            return 0;
        }

        let mut pixels_blended = 0;
        let start_x = self.dirty_rect.x;
        let start_y = self.dirty_rect.y;
        let end_x = self.dirty_rect.right().min(self.buffer_width);
        let end_y = self.dirty_rect.bottom().min(self.buffer_height);

        let global_alpha = (self.plane_alpha >> 8) as u32; // 0..255

        for y in start_y..end_y {
            let dst_y = self.bounds.y + y;
            if dst_y >= primary_height {
                continue;
            }

            for x in start_x..end_x {
                let dst_x = self.bounds.x + x;
                if dst_x >= primary_width {
                    continue;
                }

                let src_pixel = self.buffer[y * self.buffer_width + x];
                let src_a = ((src_pixel >> 24) & 0xFF) * global_alpha / 255;
                if src_a == 0 {
                    continue; // Completely transparent
                }

                let dst_idx = dst_y * primary_width + dst_x;
                let dst_pixel = primary_fb[dst_idx];

                if src_a >= 255 {
                    primary_fb[dst_idx] = src_pixel | 0xFF00_0000;
                } else {
                    let inv_a = 255 - src_a;
                    let src_r = (src_pixel >> 16) & 0xFF;
                    let src_g = (src_pixel >> 8) & 0xFF;
                    let src_b = src_pixel & 0xFF;

                    let dst_r = (dst_pixel >> 16) & 0xFF;
                    let dst_g = (dst_pixel >> 8) & 0xFF;
                    let dst_b = dst_pixel & 0xFF;

                    let out_r = (src_r * src_a + dst_r * inv_a) / 255;
                    let out_g = (src_g * src_a + dst_g * inv_a) / 255;
                    let out_b = (src_b * src_a + dst_b * inv_a) / 255;

                    primary_fb[dst_idx] = 0xFF00_0000 | (out_r << 16) | (out_g << 8) | out_b;
                }
                pixels_blended += 1;
            }
        }

        pixels_blended
    }
}

// -----------------------------------------------------------------------------
// Unit Tests & Verification
// -----------------------------------------------------------------------------

/// Unit test verifying Plane #2 hardware overlay abstraction, alpha blending, and dirty bounds.
pub fn verify_overlay_plane_abstraction() -> bool {
    let mut overlay = HardwareOverlayPlane::new(HARDWARE_PLANE_OVERLAY_ID, 200, 100);

    if overlay.plane_id != 2 || overlay.zpos != 1 || overlay.visible {
        return false;
    }

    // Write semi-transparent green box (ARGB: 0x8000FF00) at (10, 10) to (20, 20)
    overlay.set_visible(true);
    for y in 10..20 {
        for x in 10..20 {
            overlay.write_pixel(x, y, 0x8000_FF00);
        }
    }

    // Verify dirty bounding box bounds (10, 10, 10, 10)
    let dirty = overlay.dirty_rect;
    if dirty.x != 10 || dirty.y != 10 || dirty.width != 10 || dirty.height != 10 {
        return false;
    }

    // Allocate simulated primary framebuffer 1920x1080 (opaque blue 0xFF0000FF)
    let mut primary_fb = Vec::new();
    primary_fb.resize(200 * 100, 0xFF00_00FF);

    // Blend overlay onto primary
    let blended = overlay.blend_over(&mut primary_fb, 200, 100);
    if blended != 100 {
        return false; // Exactly 10x10 = 100 pixels blended
    }

    // Check pixel at (15, 15): original blue (0, 0, 255), src green (0, 255, 0) with alpha ~115 (0x80 * 0xE6 / 255)
    // Blue and green should both be non-zero in the blended pixel
    let blended_pixel = primary_fb[15 * 200 + 15];
    let green = (blended_pixel >> 8) & 0xFF;
    let blue = blended_pixel & 0xFF;
    if green == 0 || blue == 0 {
        return false;
    }

    // Unblended pixel at (0, 0) should remain pure blue
    if primary_fb[0] != 0xFF00_00FF {
        return false;
    }

    true
}
