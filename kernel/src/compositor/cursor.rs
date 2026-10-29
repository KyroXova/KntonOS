//! Mouse cursor bitmap sprite overlay and software blitter for KntonOS compositor.

use crate::display::color::Color;
use super::dirty::Rect;

pub const CURSOR_WIDTH: usize = 12;
pub const CURSOR_HEIGHT: usize = 19;

/// 12x19 Windows Fluent-style arrow pointer sprite.
/// 0: Transparent, 1: Black outline, 2: White body.
const CURSOR_SPRITE_DATA: [u8; CURSOR_WIDTH * CURSOR_HEIGHT] = [
    // Row 0
    1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
    // Row 1
    1, 1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
    // Row 2
    1, 2, 1, 0, 0, 0, 0, 0, 0, 0, 0, 0,
    // Row 3
    1, 2, 2, 1, 0, 0, 0, 0, 0, 0, 0, 0,
    // Row 4
    1, 2, 2, 2, 1, 0, 0, 0, 0, 0, 0, 0,
    // Row 5
    1, 2, 2, 2, 2, 1, 0, 0, 0, 0, 0, 0,
    // Row 6
    1, 2, 2, 2, 2, 2, 1, 0, 0, 0, 0, 0,
    // Row 7
    1, 2, 2, 2, 2, 2, 2, 1, 0, 0, 0, 0,
    // Row 8
    1, 2, 2, 2, 2, 2, 2, 2, 1, 0, 0, 0,
    // Row 9
    1, 2, 2, 2, 2, 2, 2, 2, 2, 1, 0, 0,
    // Row 10
    1, 2, 2, 2, 2, 2, 1, 1, 1, 1, 1, 0,
    // Row 11
    1, 2, 2, 2, 1, 2, 1, 0, 0, 0, 0, 0,
    // Row 12
    1, 2, 2, 1, 0, 1, 2, 1, 0, 0, 0, 0,
    // Row 13
    1, 2, 1, 0, 0, 1, 2, 1, 0, 0, 0, 0,
    // Row 14
    1, 1, 0, 0, 0, 0, 1, 2, 1, 0, 0, 0,
    // Row 15
    1, 0, 0, 0, 0, 0, 1, 2, 1, 0, 0, 0,
    // Row 16
    0, 0, 0, 0, 0, 0, 0, 1, 2, 1, 0, 0,
    // Row 17
    0, 0, 0, 0, 0, 0, 0, 1, 2, 1, 0, 0,
    // Row 18
    0, 0, 0, 0, 0, 0, 0, 0, 1, 1, 0, 0,
];

/// Returns the Color for a sprite pixel at (x, y).
#[inline(always)]
pub fn cursor_sprite_pixel(x: usize, y: usize) -> Color {
    if x >= CURSOR_WIDTH || y >= CURSOR_HEIGHT {
        return Color::TRANSPARENT;
    }
    match CURSOR_SPRITE_DATA[y * CURSOR_WIDTH + x] {
        1 => Color::BLACK,
        2 => Color::WHITE,
        _ => Color::TRANSPARENT,
    }
}

/// Represents the software mouse cursor state.
#[derive(Clone, Debug)]
pub struct CursorState {
    pub x: usize,
    pub y: usize,
    pub visible: bool,
    pub hotspot_x: usize,
    pub hotspot_y: usize,
}

impl CursorState {
    pub const fn new(x: usize, y: usize) -> Self {
        Self {
            x,
            y,
            visible: true,
            hotspot_x: 0,
            hotspot_y: 0,
        }
    }

    #[inline(always)]
    pub fn position(&self) -> (usize, usize) {
        (self.x, self.y)
    }

    #[inline(always)]
    pub fn bounds(&self) -> Rect {
        if !self.visible {
            Rect::empty()
        } else {
            Rect::new(self.x, self.y, CURSOR_WIDTH, CURSOR_HEIGHT)
        }
    }

    /// Update cursor position and return the bounding rectangle of the damaged area
    /// (the union of the previous and new cursor bounding boxes).
    pub fn set_position(&mut self, new_x: usize, new_y: usize) -> Rect {
        let old_bounds = self.bounds();
        self.x = new_x;
        self.y = new_y;
        let new_bounds = self.bounds();
        old_bounds.union(&new_bounds)
    }

    /// Set cursor visibility and return the damaged area if visibility changed.
    pub fn set_visible(&mut self, visible: bool) -> Rect {
        if self.visible == visible {
            return Rect::empty();
        }
        let bounds = self.bounds();
        self.visible = visible;
        if visible {
            self.bounds()
        } else {
            bounds
        }
    }

    /// Blit the cursor sprite into a target linear backbuffer slice with alpha blending.
    pub fn render(
        &self,
        target: &mut [Color],
        target_w: usize,
        target_h: usize,
        clip: Option<&Rect>,
    ) {
        if !self.visible {
            return;
        }

        for cy in 0..CURSOR_HEIGHT {
            let ty = self.y + cy;
            if ty >= target_h {
                break;
            }

            for cx in 0..CURSOR_WIDTH {
                let tx = self.x + cx;
                if tx >= target_w {
                    break;
                }

                if let Some(c) = clip {
                    if !c.contains_point(tx, ty) {
                        continue;
                    }
                }

                let color = cursor_sprite_pixel(cx, cy);
                if color.a == 0 {
                    continue;
                }

                let idx = ty * target_w + tx;
                target[idx] = color.blend(target[idx]);
            }
        }
    }
}
