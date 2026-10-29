//! Multi-window scene compositor blending double-buffered backbuffer to linear framebuffer.

use alloc::string::ToString;
use alloc::vec::Vec;
use crate::display::color::Color;
use crate::display::font::{get_glyph, FONT_HEIGHT, FONT_WIDTH};
use crate::display::framebuffer::LinearFramebuffer;
use super::cursor::CursorState;
use super::dirty::{DamageRegion, Rect};
use super::window::Window;

pub const DEFAULT_TASKBAR_HEIGHT: usize = 48;

/// Manages multi-window Z-order hierarchy, desktop background, taskbar, cursor overlay,
/// and dirty-region composited double-buffering.
pub struct SceneCompositor {
    pub width: usize,
    pub height: usize,
    pub taskbar_height: usize,
    pub backbuffer: Vec<Color>,
    pub windows: Vec<Window>,
    pub cursor: CursorState,
    pub damage: DamageRegion,
    next_window_id: u32,
}

impl SceneCompositor {
    pub fn new(width: usize, height: usize) -> Self {
        let total_pixels = width * height;
        let mut backbuffer = Vec::with_capacity(total_pixels);
        backbuffer.resize(total_pixels, Color::WIN11_BG);

        let mut comp = Self {
            width,
            height,
            taskbar_height: DEFAULT_TASKBAR_HEIGHT,
            backbuffer,
            windows: Vec::new(),
            cursor: CursorState::new(width / 2, height / 2),
            damage: DamageRegion::new(),
            next_window_id: 1,
        };

        // Initially damage the entire screen to guarantee full first frame composition
        comp.damage_all();
        comp
    }

    #[inline(always)]
    pub fn width(&self) -> usize {
        self.width
    }

    #[inline(always)]
    pub fn height(&self) -> usize {
        self.height
    }

    #[inline(always)]
    pub fn backbuffer(&self) -> &[Color] {
        &self.backbuffer
    }

    #[inline(always)]
    pub fn backbuffer_pixel(&self, x: usize, y: usize) -> Option<Color> {
        if x >= self.width || y >= self.height {
            None
        } else {
            Some(self.backbuffer[y * self.width + x])
        }
    }

    /// Mark the entire screen surface as damaged.
    pub fn damage_all(&mut self) {
        self.damage.add_rect(Rect::new(0, 0, self.width, self.height));
    }

    /// Mark a specific rectangular screen region as damaged.
    pub fn damage_rect(&mut self, rect: Rect) {
        let clamped = rect.clamp(self.width, self.height);
        if !clamped.is_empty() {
            self.damage.add_rect(clamped);
        }
    }

    /// Create and register a new window. It is placed on top of the Z-order and focused.
    pub fn create_window(
        &mut self,
        title: &str,
        x: usize,
        y: usize,
        width: usize,
        height: usize,
    ) -> u32 {
        let id = self.next_window_id;
        self.next_window_id = self.next_window_id.wrapping_add(1);

        for w in self.windows.iter_mut() {
            w.is_focused = false;
        }

        let mut win = Window::new(id, title.to_string(), x, y, width, height);
        win.is_focused = true;
        win.z_order = self.windows.len();

        let bounds = win.frame_bounds();
        self.windows.push(win);
        self.damage_rect(bounds);

        id
    }

    pub fn find_window(&self, id: u32) -> Option<&Window> {
        self.windows.iter().find(|w| w.id == id)
    }

    pub fn find_window_mut(&mut self, id: u32) -> Option<&mut Window> {
        self.windows.iter_mut().find(|w| w.id == id)
    }

    /// Bring the specified window to the front of the Z-order and focus it.
    pub fn bring_to_front(&mut self, id: u32) {
        if let Some(pos) = self.windows.iter().position(|w| w.id == id) {
            for w in self.windows.iter_mut() {
                w.is_focused = false;
            }
            let mut win = self.windows.remove(pos);
            win.is_focused = true;
            let bounds = win.frame_bounds();
            self.windows.push(win);

            // Re-index Z-orders
            for (idx, w) in self.windows.iter_mut().enumerate() {
                w.z_order = idx;
            }
            self.damage_rect(bounds);
        }
    }

    /// Move a window to new screen coordinates.
    pub fn move_window(&mut self, id: u32, new_x: usize, new_y: usize) {
        if let Some(win) = self.find_window_mut(id) {
            let old_bounds = win.frame_bounds();
            win.set_position(new_x, new_y);
            let new_bounds = win.frame_bounds();
            let dirty = old_bounds.union(&new_bounds);
            self.damage_rect(dirty);
        }
    }

    /// Close and remove a window from the compositor.
    pub fn close_window(&mut self, id: u32) {
        if let Some(pos) = self.windows.iter().position(|w| w.id == id) {
            let win = self.windows.remove(pos);
            self.damage_rect(win.frame_bounds());

            // Focus the new top window if present
            let top_bounds = if let Some(last) = self.windows.last_mut() {
                last.is_focused = true;
                Some(last.frame_bounds())
            } else {
                None
            };
            if let Some(bounds) = top_bounds {
                self.damage_rect(bounds);
            }

            for (idx, w) in self.windows.iter_mut().enumerate() {
                w.z_order = idx;
            }
        }
    }

    /// Update mouse cursor position and damage both old and new cursor regions.
    pub fn move_cursor(&mut self, new_x: usize, new_y: usize) {
        let cursor_damage = self.cursor.set_position(new_x, new_y);
        self.damage_rect(cursor_damage);
    }

    /// Render wallpaper and taskbar within a damaged rectangle.
    fn render_background_clipped(&mut self, clip: &Rect) {
        let center_y = self.height / 3;
        let taskbar_y = self.height.saturating_sub(self.taskbar_height);

        // 1. Wallpaper fill for clipped region
        for row in 0..clip.height {
            let y = clip.y + row;
            if y >= self.height {
                break;
            }

            let is_accent_line = y == center_y || y == center_y + 1;
            let is_taskbar = y >= taskbar_y;

            let row_color = if is_taskbar {
                if y == taskbar_y {
                    Color::WIN11_CARD_BORDER
                } else {
                    Color::WIN11_TASKBAR
                }
            } else if is_accent_line {
                Color::WIN11_CARD_BORDER
            } else {
                Color::WIN11_BG
            };

            let row_offset = y * self.width;
            for col in 0..clip.width {
                let x = clip.x + col;
                if x >= self.width {
                    break;
                }
                self.backbuffer[row_offset + x] = row_color;
            }
        }

        // 2. Taskbar decorations if clip intersects taskbar
        let taskbar_rect = Rect::new(0, taskbar_y, self.width, self.taskbar_height);
        if clip.intersects(&taskbar_rect) {
            self.render_taskbar_decorations(taskbar_y, clip);
        }
    }

    fn render_taskbar_decorations(&mut self, taskbar_y: usize, clip: &Rect) {
        let center_x = self.width / 2;
        let start_x = center_x.saturating_sub(120);

        // Centered Start button: 4 Fluent tiles
        let tile_size = 12;
        let gap = 3;
        let btn_y = taskbar_y + 10;
        self.draw_rect_clipped(start_x, btn_y, tile_size, tile_size, Color::WIN11_ACCENT, clip);
        self.draw_rect_clipped(start_x + tile_size + gap, btn_y, tile_size, tile_size, Color::WIN11_ACCENT, clip);
        self.draw_rect_clipped(start_x, btn_y + tile_size + gap, tile_size, tile_size, Color::WIN11_ACCENT, clip);
        self.draw_rect_clipped(start_x + tile_size + gap, btn_y + tile_size + gap, tile_size, tile_size, Color::WIN11_ACCENT, clip);

        // Pinned app icons
        self.render_taskbar_icon(start_x + 40, btn_y, "Term", Color::WIN11_ACCENT, clip);
        self.render_taskbar_icon(start_x + 90, btn_y, "Files", Color::rgb(255, 185, 0), clip);
        self.render_taskbar_icon(start_x + 140, btn_y, "Game", Color::rgb(16, 124, 65), clip);
        self.render_taskbar_icon(start_x + 190, btn_y, "Set", Color::WIN11_TEXT_SECONDARY, clip);

        // System Tray (right side)
        let tray_x = self.width.saturating_sub(150);
        self.draw_string_clipped(tray_x, taskbar_y + 16, "11:40 AM", Color::WIN11_TEXT_PRIMARY, clip);
        self.draw_string_clipped(tray_x + 80, taskbar_y + 16, "KNT 64", Color::WIN11_ACCENT, clip);

        // Left Branding
        self.draw_string_clipped(24, taskbar_y + 16, "KntonOS 1.0 (Snatch)", Color::WIN11_TEXT_SECONDARY, clip);
    }

    fn render_taskbar_icon(
        &mut self,
        x: usize,
        y: usize,
        label: &str,
        accent: Color,
        clip: &Rect,
    ) {
        self.draw_rect_clipped(x, y, 36, 28, Color::WIN11_HOVER, clip);
        self.draw_rect_clipped(x, y + 26, 36, 2, accent, clip);
        let text_offset = (36usize.saturating_sub(label.len() * FONT_WIDTH)) / 2;
        self.draw_string_clipped(x + text_offset, y + 6, label, Color::WIN11_TEXT_PRIMARY, clip);
    }

    fn draw_rect_clipped(
        &mut self,
        x: usize,
        y: usize,
        w: usize,
        h: usize,
        color: Color,
        clip: &Rect,
    ) {
        for row in 0..h {
            let ty = y + row;
            if ty >= self.height {
                break;
            }
            for col in 0..w {
                let tx = x + col;
                if tx >= self.width {
                    break;
                }
                if clip.contains_point(tx, ty) {
                    self.backbuffer[ty * self.width + tx] = color;
                }
            }
        }
    }

    fn draw_string_clipped(
        &mut self,
        x: usize,
        y: usize,
        s: &str,
        fg: Color,
        clip: &Rect,
    ) {
        let mut cur_x = x;
        for c in s.chars() {
            if c == '\n' {
                continue;
            }
            let glyph = get_glyph(c);
            for row in 0..FONT_HEIGHT {
                let ty = y + row;
                if ty >= self.height {
                    break;
                }
                let bits = glyph[row];
                for col in 0..FONT_WIDTH {
                    let tx = cur_x + col;
                    if tx >= self.width {
                        break;
                    }
                    if clip.contains_point(tx, ty) && (bits & (0x80 >> col)) != 0 {
                        self.backbuffer[ty * self.width + tx] = fg;
                    }
                }
            }
            cur_x += FONT_WIDTH;
            if cur_x + FONT_WIDTH > self.width {
                break;
            }
        }
    }

    /// Compose the scene for all damaged regions into the internal backbuffer.
    pub fn compose_scene(&mut self) {
        if self.damage.is_empty() {
            return;
        }

        // Clone damage rects to decouple borrowing
        let dirty_rects: Vec<Rect> = self.damage.rects().to_vec();

        for damage_rect in &dirty_rects {
            // 1. Redraw background (wallpaper & taskbar)
            self.render_background_clipped(damage_rect);

            // 2. Render windows in Z-order (back to front)
            for win in self.windows.iter() {
                if win.frame_bounds().intersects(damage_rect) {
                    win.render_to_buffer(
                        &mut self.backbuffer,
                        self.width,
                        self.height,
                        Some(damage_rect),
                    );
                }
            }

            // 3. Render mouse cursor on top
            if self.cursor.bounds().intersects(damage_rect) {
                self.cursor.render(
                    &mut self.backbuffer,
                    self.width,
                    self.height,
                    Some(damage_rect),
                );
            }
        }
    }

    /// Full composition cycle: renders damaged regions into backbuffer, blits to LinearFramebuffer,
    /// and resets damage tracking.
    pub fn composite(&mut self, fb: &mut LinearFramebuffer) {
        if self.damage.is_empty() {
            return;
        }

        self.compose_scene();

        // Blit damaged rectangles from backbuffer to the hardware linear framebuffer
        for rect in self.damage.rects() {
            let clamped = rect.clamp(fb.width(), fb.height());
            for row in 0..clamped.height {
                let y = clamped.y + row;
                for col in 0..clamped.width {
                    let x = clamped.x + col;
                    let color = self.backbuffer[y * self.width + x];
                    fb.put_pixel(x, y, color);
                }
            }
        }

        self.damage.clear();
    }

    /// Headless composition helper: blits damaged backbuffer pixels into a destination buffer.
    pub fn composite_to_slice(&mut self, dest: &mut [Color], dest_w: usize, dest_h: usize) {
        if self.damage.is_empty() {
            return;
        }

        self.compose_scene();

        for rect in self.damage.rects() {
            let clamped = rect.clamp(dest_w, dest_h);
            for row in 0..clamped.height {
                let y = clamped.y + row;
                for col in 0..clamped.width {
                    let x = clamped.x + col;
                    let color = self.backbuffer[y * self.width + x];
                    dest[y * dest_w + x] = color;
                }
            }
        }

        self.damage.clear();
    }
}
