//! Fluent window frame styling, borders, titlebars, buttons, and client buffers for KntonOS.

use alloc::string::String;
use alloc::vec::Vec;
use crate::display::color::Color;
use crate::display::font::{get_glyph, FONT_HEIGHT, FONT_WIDTH};
use super::acrylic::{AcrylicConfig, AcrylicSurface};
use super::controls::{Button, TitleBarButton, TitleBarButtonType};
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
    pub close_button: TitleBarButton,
    pub max_button: TitleBarButton,
    pub min_button: TitleBarButton,
    pub widgets: Vec<Button>,
    pub use_acrylic: bool,
}

impl Window {
    pub fn new(id: u32, title: String, x: usize, y: usize, width: usize, height: usize) -> Self {
        let client_width = width.saturating_sub(BORDER_WIDTH * 2);
        let client_height = height.saturating_sub(TITLEBAR_HEIGHT + BORDER_WIDTH * 2);
        let buffer_size = client_width * client_height;
        let mut client_buffer = Vec::with_capacity(buffer_size);
        client_buffer.resize(buffer_size, Color::WIN11_CARD_BG);

        let tb_y = y + BORDER_WIDTH;
        let close_x = x + width.saturating_sub(BORDER_WIDTH + BUTTON_WIDTH);
        let max_x = close_x.saturating_sub(BUTTON_WIDTH);
        let min_x = max_x.saturating_sub(BUTTON_WIDTH);

        let close_button = TitleBarButton::new(
            TitleBarButtonType::Close,
            Rect::new(close_x, tb_y, BUTTON_WIDTH, TITLEBAR_HEIGHT),
        );
        let max_button = TitleBarButton::new(
            TitleBarButtonType::Maximize,
            Rect::new(max_x, tb_y, BUTTON_WIDTH, TITLEBAR_HEIGHT),
        );
        let min_button = TitleBarButton::new(
            TitleBarButtonType::Minimize,
            Rect::new(min_x, tb_y, BUTTON_WIDTH, TITLEBAR_HEIGHT),
        );

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
            close_button,
            max_button,
            min_button,
            widgets: Vec::new(),
            use_acrylic: false,
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

    pub fn update_control_bounds(&mut self) {
        let tb_y = self.y + BORDER_WIDTH;
        let close_x = self.x + self.width.saturating_sub(BORDER_WIDTH + BUTTON_WIDTH);
        let max_x = close_x.saturating_sub(BUTTON_WIDTH);
        let min_x = max_x.saturating_sub(BUTTON_WIDTH);

        self.close_button.bounds = Rect::new(close_x, tb_y, BUTTON_WIDTH, TITLEBAR_HEIGHT);
        self.max_button.bounds = Rect::new(max_x, tb_y, BUTTON_WIDTH, TITLEBAR_HEIGHT);
        self.min_button.bounds = Rect::new(min_x, tb_y, BUTTON_WIDTH, TITLEBAR_HEIGHT);
    }

    pub fn set_acrylic(&mut self, enable: bool) {
        self.use_acrylic = enable;
    }

    pub fn add_button(&mut self, id: u32, rel_x: usize, rel_y: usize, width: usize, height: usize, label: &str) {
        let abs_x = self.x + BORDER_WIDTH + rel_x;
        let abs_y = self.y + BORDER_WIDTH + TITLEBAR_HEIGHT + rel_y;
        let bounds = Rect::new(abs_x, abs_y, width, height);
        self.widgets.push(Button::new(id, bounds, label));
    }

    pub fn add_accent_button(&mut self, id: u32, rel_x: usize, rel_y: usize, width: usize, height: usize, label: &str) {
        let abs_x = self.x + BORDER_WIDTH + rel_x;
        let abs_y = self.y + BORDER_WIDTH + TITLEBAR_HEIGHT + rel_y;
        let bounds = Rect::new(abs_x, abs_y, width, height);
        self.widgets.push(Button::new_accent(id, bounds, label));
    }

    pub fn dispatch_mouse(
        &mut self,
        mx: usize,
        my: usize,
        is_down: bool,
        was_down: bool,
    ) -> (Option<Rect>, bool, bool, bool) {
        if self.is_minimized {
            return (None, false, false, false);
        }

        let mut dirty: Option<Rect> = None;

        let close_changed = self.close_button.handle_mouse(mx, my, is_down, was_down);
        if close_changed {
            dirty = Some(dirty.map_or(self.close_button.bounds, |d| d.union(&self.close_button.bounds)));
        }

        let max_changed = self.max_button.handle_mouse(mx, my, is_down, was_down);
        if max_changed {
            dirty = Some(dirty.map_or(self.max_button.bounds, |d| d.union(&self.max_button.bounds)));
        }

        let min_changed = self.min_button.handle_mouse(mx, my, is_down, was_down);
        if min_changed {
            dirty = Some(dirty.map_or(self.min_button.bounds, |d| d.union(&self.min_button.bounds)));
        }

        for btn in self.widgets.iter_mut() {
            if btn.handle_mouse(mx, my, is_down, was_down) {
                dirty = Some(dirty.map_or(btn.bounds, |d| d.union(&btn.bounds)));
            }
        }

        let close_req = self.close_button.take_click();
        let max_req = self.max_button.take_click();
        let min_req = self.min_button.take_click();

        (dirty, close_req, min_req, max_req)
    }

    pub fn set_position(&mut self, new_x: usize, new_y: usize) {
        let dx = new_x as isize - self.x as isize;
        let dy = new_y as isize - self.y as isize;
        self.x = new_x;
        self.y = new_y;
        self.update_control_bounds();

        for btn in self.widgets.iter_mut() {
            let new_bx = (btn.bounds.x as isize + dx).max(0) as usize;
            let new_by = (btn.bounds.y as isize + dy).max(0) as usize;
            btn.bounds.x = new_bx;
            btn.bounds.y = new_by;
        }
    }

    pub fn resize(&mut self, new_width: usize, new_height: usize) {
        self.width = new_width;
        self.height = new_height;
        self.client_width = new_width.saturating_sub(BORDER_WIDTH * 2);
        self.client_height = new_height.saturating_sub(TITLEBAR_HEIGHT + BORDER_WIDTH * 2);
        let buffer_size = self.client_width * self.client_height;
        self.client_buffer.clear();
        self.client_buffer.resize(buffer_size, Color::WIN11_CARD_BG);
        self.update_control_bounds();
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
        let tb_rect = Rect::new(tb_x, tb_y, self.client_width, TITLEBAR_HEIGHT);

        if self.use_acrylic {
            let cfg = AcrylicConfig::titlebar(self.is_focused);
            AcrylicSurface::render_acrylic_in_place(
                target,
                target_w,
                target_h,
                &tb_rect,
                &cfg,
                clip,
            );
        } else {
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
        }

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
        self.min_button.render(target, target_w, target_h, clip);
        self.max_button.render(target, target_w, target_h, clip);
        self.close_button.render(target, target_w, target_h, clip);

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

        // 6. Interactive client widgets
        for btn in &self.widgets {
            btn.render(target, target_w, target_h, clip);
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
