//! Fluent window frame styling, borders, titlebars, buttons, and client buffers for KntonOS.

use alloc::string::String;
use alloc::vec::Vec;
use crate::display::color::Color;
use crate::display::font::{get_glyph, FONT_HEIGHT, FONT_WIDTH};
use super::dirty::Rect;

pub const TITLEBAR_HEIGHT: usize = 32;
pub const BORDER_WIDTH: usize = 1;
pub const BUTTON_WIDTH: usize = 32;

/// Fluent window representation with client surface buffer and frame decorations.
#[derive(Clone, Debug)]
pub struct Window {
    pub id: u32,
    pub title: String,
    pub x: usize,
    pub y: usize,
    pub width: usize,
    pub height: usize,
    pub is_focused: bool,
    pub is_minimized: bool,
    pub is_maximized: bool,
    pub z_order: usize,
    pub client_buffer: Vec<Color>,
    pub client_width: usize,
    pub client_height: usize,
}

impl Window {
    pub fn new(id: u32, title: String, x: usize, y: usize, width: usize, height: usize) -> Self {
        let client_width = width.saturating_sub(BORDER_WIDTH * 2);
        let client_height = height.saturating_sub(TITLEBAR_HEIGHT + BORDER_WIDTH * 2);
        let buffer_size = client_width * client_height;
        let mut client_buffer = Vec::with_capacity(buffer_size);
        client_buffer.resize(buffer_size, Color::WIN11_CARD_BG);

        Self {
            id,
            title,
            x,
            y,
            width,
            height,
            is_focused: false,
            is_minimized: false,
            is_maximized: false,
            z_order: 0,
            client_buffer,
            client_width,
            client_height,
        }
    }

    #[inline(always)]
    pub fn frame_bounds(&self) -> Rect {
        if self.is_minimized {
            Rect::empty()
        } else {
            Rect::new(self.x, self.y, self.width, self.height)
        }
    }

    #[inline(always)]
    pub fn client_bounds(&self) -> Rect {
        if self.is_minimized {
            Rect::empty()
        } else {
            Rect::new(
                self.x + BORDER_WIDTH,
                self.y + BORDER_WIDTH + TITLEBAR_HEIGHT,
                self.client_width,
                self.client_height,
            )
        }
    }

    #[inline(always)]
    pub fn titlebar_bounds(&self) -> Rect {
        if self.is_minimized {
            Rect::empty()
        } else {
            Rect::new(
                self.x + BORDER_WIDTH,
                self.y + BORDER_WIDTH,
                self.client_width,
                TITLEBAR_HEIGHT,
            )
        }
    }

    pub fn set_position(&mut self, new_x: usize, new_y: usize) {
        self.x = new_x;
        self.y = new_y;
    }

    pub fn resize(&mut self, new_width: usize, new_height: usize) {
        self.width = new_width;
        self.height = new_height;
        self.client_width = new_width.saturating_sub(BORDER_WIDTH * 2);
        self.client_height = new_height.saturating_sub(TITLEBAR_HEIGHT + BORDER_WIDTH * 2);
        let buffer_size = self.client_width * self.client_height;
        self.client_buffer.clear();
        self.client_buffer.resize(buffer_size, Color::WIN11_CARD_BG);
    }

    pub fn clear_client(&mut self, color: Color) {
        for pixel in self.client_buffer.iter_mut() {
            *pixel = color;
        }
    }

    pub fn put_client_pixel(&mut self, cx: usize, cy: usize, color: Color) {
        if cx >= self.client_width || cy >= self.client_height {
            return;
        }
        self.client_buffer[cy * self.client_width + cx] = color;
    }

    pub fn get_client_pixel(&self, cx: usize, cy: usize) -> Option<Color> {
        if cx >= self.client_width || cy >= self.client_height {
            None
        } else {
            Some(self.client_buffer[cy * self.client_width + cx])
        }
    }

    pub fn draw_client_rect(&mut self, cx: usize, cy: usize, w: usize, h: usize, color: Color) {
        for row in 0..h {
            let py = cy + row;
            if py >= self.client_height {
                break;
            }
            for col in 0..w {
                let px = cx + col;
                if px >= self.client_width {
                    break;
                }
                self.client_buffer[py * self.client_width + px] = color;
            }
        }
    }

    pub fn draw_client_text(
        &mut self,
        cx: usize,
        cy: usize,
        text: &str,
        fg: Color,
        bg: Option<Color>,
    ) {
        let mut cur_x = cx;
        for c in text.chars() {
            if c == '\n' {
                continue;
            }
            let glyph = get_glyph(c);
            for row in 0..FONT_HEIGHT {
                let py = cy + row;
                if py >= self.client_height {
                    break;
                }
                let bits = glyph[row];
                for col in 0..FONT_WIDTH {
                    let px = cur_x + col;
                    if px >= self.client_width {
                        break;
                    }
                    let is_set = (bits & (0x80 >> col)) != 0;
                    if is_set {
                        self.client_buffer[py * self.client_width + px] = fg;
                    } else if let Some(bg_color) = bg {
                        self.client_buffer[py * self.client_width + px] = bg_color;
                    }
                }
            }
            cur_x += FONT_WIDTH;
            if cur_x + FONT_WIDTH > self.client_width {
                break;
            }
        }
    }

    /// Render this window's frame (borders, titlebar, controls) and client surface into the target buffer.
    pub fn render_to_buffer(
        &self,
        target: &mut [Color],
        target_w: usize,
        target_h: usize,
        clip: Option<&Rect>,
    ) {
        if self.is_minimized || self.width == 0 || self.height == 0 {
            return;
        }

        let border_color = if self.is_focused {
            Color::WIN11_ACCENT
        } else {
            Color::WIN11_CARD_BORDER
        };

        let titlebar_color = if self.is_focused {
            Color::WIN11_TITLEBAR
        } else {
            Color::rgb(30, 34, 44)
        };

        // 1. Outer 1px border
        self.render_rect_outline_clipped(
            target,
            target_w,
            target_h,
            self.x,
            self.y,
            self.width,
            self.height,
            BORDER_WIDTH,
            border_color,
            clip,
        );

        // 2. Titlebar background
        let tb_x = self.x + BORDER_WIDTH;
        let tb_y = self.y + BORDER_WIDTH;
        self.render_rect_clipped(
            target,
            target_w,
            target_h,
            tb_x,
            tb_y,
            self.client_width,
            TITLEBAR_HEIGHT,
            titlebar_color,
            clip,
        );

        // 3. Titlebar text
        let title_x = tb_x + 12;
        let title_y = tb_y + 8;
        self.render_string_clipped(
            target,
            target_w,
            target_h,
            title_x,
            title_y,
            &self.title,
            Color::WIN11_TEXT_PRIMARY,
            clip,
        );

        // 4. Window controls (Minimize, Maximize, Close buttons)
        let close_x = self.x + self.width - BORDER_WIDTH - BUTTON_WIDTH;
        // Close button: subtle red accent
        self.render_rect_clipped(
            target,
            target_w,
            target_h,
            close_x,
            tb_y,
            BUTTON_WIDTH,
            TITLEBAR_HEIGHT,
            Color::WIN11_CLOSE_RED,
            clip,
        );
        self.render_string_clipped(
            target,
            target_w,
            target_h,
            close_x + 12,
            tb_y + 8,
            "X",
            Color::WHITE,
            clip,
        );

        // Maximize button
        let max_x = close_x - BUTTON_WIDTH;
        self.render_rect_clipped(
            target,
            target_w,
            target_h,
            max_x,
            tb_y,
            BUTTON_WIDTH,
            TITLEBAR_HEIGHT,
            Color::WIN11_HOVER,
            clip,
        );
        self.render_rect_outline_clipped(
            target,
            target_w,
            target_h,
            max_x + 11,
            tb_y + 11,
            10,
            10,
            1,
            Color::WIN11_TEXT_PRIMARY,
            clip,
        );

        // Minimize button
        let min_x = max_x - BUTTON_WIDTH;
        self.render_rect_clipped(
            target,
            target_w,
            target_h,
            min_x,
            tb_y,
            BUTTON_WIDTH,
            TITLEBAR_HEIGHT,
            Color::WIN11_HOVER,
            clip,
        );
        self.render_rect_clipped(
            target,
            target_w,
            target_h,
            min_x + 11,
            tb_y + 17,
            10,
            2,
            Color::WIN11_TEXT_PRIMARY,
            clip,
        );

        // 5. Client area pixels
        let client_x = self.x + BORDER_WIDTH;
        let client_y = self.y + BORDER_WIDTH + TITLEBAR_HEIGHT;

        for cy in 0..self.client_height {
            let ty = client_y + cy;
            if ty >= target_h {
                break;
            }

            for cx in 0..self.client_width {
                let tx = client_x + cx;
                if tx >= target_w {
                    break;
                }

                if let Some(c) = clip {
                    if !c.contains_point(tx, ty) {
                        continue;
                    }
                }

                let color = self.client_buffer[cy * self.client_width + cx];
                target[ty * target_w + tx] = color;
            }
        }
    }

    fn render_rect_clipped(
        &self,
        target: &mut [Color],
        target_w: usize,
        target_h: usize,
        x: usize,
        y: usize,
        w: usize,
        h: usize,
        color: Color,
        clip: Option<&Rect>,
    ) {
        for row in 0..h {
            let ty = y + row;
            if ty >= target_h {
                break;
            }
            for col in 0..w {
                let tx = x + col;
                if tx >= target_w {
                    break;
                }
                if let Some(c) = clip {
                    if !c.contains_point(tx, ty) {
                        continue;
                    }
                }
                target[ty * target_w + tx] = color;
            }
        }
    }

    fn render_rect_outline_clipped(
        &self,
        target: &mut [Color],
        target_w: usize,
        target_h: usize,
        x: usize,
        y: usize,
        w: usize,
        h: usize,
        border_width: usize,
        color: Color,
        clip: Option<&Rect>,
    ) {
        // Top border
        self.render_rect_clipped(target, target_w, target_h, x, y, w, border_width, color, clip);
        // Bottom border
        if h > border_width {
            self.render_rect_clipped(
                target,
                target_w,
                target_h,
                x,
                y + h - border_width,
                w,
                border_width,
                color,
                clip,
            );
        }
        // Left border
        self.render_rect_clipped(target, target_w, target_h, x, y, border_width, h, color, clip);
        // Right border
        if w > border_width {
            self.render_rect_clipped(
                target,
                target_w,
                target_h,
                x + w - border_width,
                y,
                border_width,
                h,
                color,
                clip,
            );
        }
    }

    fn render_string_clipped(
        &self,
        target: &mut [Color],
        target_w: usize,
        target_h: usize,
        x: usize,
        y: usize,
        s: &str,
        fg: Color,
        clip: Option<&Rect>,
    ) {
        let mut cur_x = x;
        for c in s.chars() {
            if c == '\n' {
                continue;
            }
            let glyph = get_glyph(c);
            for row in 0..FONT_HEIGHT {
                let ty = y + row;
                if ty >= target_h {
                    break;
                }
                let bits = glyph[row];
                for col in 0..FONT_WIDTH {
                    let tx = cur_x + col;
                    if tx >= target_w {
                        break;
                    }
                    if let Some(cl) = clip {
                        if !cl.contains_point(tx, ty) {
                            continue;
                        }
                    }
                    if (bits & (0x80 >> col)) != 0 {
                        target[ty * target_w + tx] = fg;
                    }
                }
            }
            cur_x += FONT_WIDTH;
            if cur_x + FONT_WIDTH > target_w {
                break;
            }
        }
    }
}
