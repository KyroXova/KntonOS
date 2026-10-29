//! Windows 11-inspired 2D Desktop Shell & Window Manager for KntonOS.

use super::color::Color;
use super::draw::{draw_rect, draw_rect_outline, draw_string};
use super::font::FONT_WIDTH;
use super::framebuffer::LinearFramebuffer;

pub const TASKBAR_HEIGHT: usize = 48;

#[derive(Clone, Debug)]
pub struct WindowRect {
    pub x: usize,
    pub y: usize,
    pub width: usize,
    pub height: usize,
    pub title: &'static str,
}

pub struct DesktopShell {
    width: usize,
    height: usize,
}

impl DesktopShell {
    pub fn new(width: usize, height: usize) -> Self {
        Self { width, height }
    }

    /// Renders the complete modern Windows 11-style desktop shell
    pub fn render_base_desktop(&self, fb: &mut LinearFramebuffer) {
        // 1. Wallpaper background (Deep slate Fluent dark mode)
        fb.clear(Color::WIN11_BG);

        // Subtle gradient / accent lines across the center
        let center_y = self.height / 3;
        draw_rect(fb, 0, center_y, self.width, 2, Color::WIN11_CARD_BORDER);

        // 2. Taskbar (Bottom 48px, Mica translucent appearance)
        let taskbar_y = self.height - TASKBAR_HEIGHT;
        draw_rect(fb, 0, taskbar_y, self.width, TASKBAR_HEIGHT, Color::WIN11_TASKBAR);
        draw_rect(fb, 0, taskbar_y, self.width, 1, Color::WIN11_CARD_BORDER);

        // 3. Centered Windows 11-style Start Icon & App Pinned Icons
        let center_x = self.width / 2;
        let start_x = center_x - 120;

        // Windows 11 4-tile Start button
        self.render_start_button(fb, start_x, taskbar_y + 10);

        // Pinned Application Icons (Terminal, Files, Settings, Games)
        self.render_taskbar_icon(fb, start_x + 40, taskbar_y + 10, "Term", Color::WIN11_ACCENT);
        self.render_taskbar_icon(fb, start_x + 90, taskbar_y + 10, "Files", Color::rgb(255, 185, 0));
        self.render_taskbar_icon(fb, start_x + 140, taskbar_y + 10, "Game", Color::rgb(16, 124, 65));
        self.render_taskbar_icon(fb, start_x + 190, taskbar_y + 10, "Set", Color::WIN11_TEXT_SECONDARY);

        // 4. System Tray (Clock & status indicators on right)
        let tray_x = self.width - 150;
        draw_string(fb, tray_x, taskbar_y + 16, "11:40 AM", Color::WIN11_TEXT_PRIMARY, None);
        draw_string(fb, tray_x + 80, taskbar_y + 16, "KNT 64", Color::WIN11_ACCENT, None);

        // 5. System Brand on Left
        draw_string(fb, 24, taskbar_y + 16, "KntonOS 1.0 (Snatch)", Color::WIN11_TEXT_SECONDARY, None);
    }

    /// Renders a window with Windows 11 rounded appearance, titlebar, and control buttons
    pub fn render_window(&self, fb: &mut LinearFramebuffer, win: &WindowRect) {
        // Outer subtle shadow/border
        draw_rect_outline(fb, win.x, win.y, win.width, win.height, 1, Color::WIN11_CARD_BORDER);

        // Title bar (height 32px)
        let titlebar_h = 32;
        draw_rect(fb, win.x + 1, win.y + 1, win.width - 2, titlebar_h, Color::WIN11_TITLEBAR);

        // Title text
        draw_string(
            fb,
            win.x + 16,
            win.y + 8,
            win.title,
            Color::WIN11_TEXT_PRIMARY,
            None,
        );

        // Window Controls (Minimize, Maximize, Close on top right)
        let close_btn_x = win.x + win.width - 32;
        draw_rect(fb, close_btn_x, win.y + 1, 31, titlebar_h, Color::WIN11_CLOSE_RED);
        draw_string(fb, close_btn_x + 11, win.y + 8, "X", Color::WHITE, None);

        // Maximize button
        let max_btn_x = close_btn_x - 32;
        draw_rect(fb, max_btn_x, win.y + 1, 31, titlebar_h, Color::WIN11_HOVER);
        draw_rect_outline(fb, max_btn_x + 10, win.y + 10, 11, 11, 1, Color::WIN11_TEXT_PRIMARY);

        // Minimize button
        let min_btn_x = max_btn_x - 32;
        draw_rect(fb, min_btn_x, win.y + 1, 31, titlebar_h, Color::WIN11_HOVER);
        draw_rect(fb, min_btn_x + 10, win.y + 17, 11, 2, Color::WIN11_TEXT_PRIMARY);

        // Client Area
        let client_y = win.y + titlebar_h + 1;
        let client_h = win.height - titlebar_h - 2;
        draw_rect(fb, win.x + 1, client_y, win.width - 2, client_h, Color::WIN11_CARD_BG);
    }

    fn render_start_button(&self, fb: &mut LinearFramebuffer, x: usize, y: usize) {
        // 4 blue square tiles of Windows 11 logo
        let tile_size = 12;
        let gap = 3;
        draw_rect(fb, x, y, tile_size, tile_size, Color::WIN11_ACCENT);
        draw_rect(fb, x + tile_size + gap, y, tile_size, tile_size, Color::WIN11_ACCENT);
        draw_rect(fb, x, y + tile_size + gap, tile_size, tile_size, Color::WIN11_ACCENT);
        draw_rect(fb, x + tile_size + gap, y + tile_size + gap, tile_size, tile_size, Color::WIN11_ACCENT);
    }

    fn render_taskbar_icon(&self, fb: &mut LinearFramebuffer, x: usize, y: usize, label: &str, accent: Color) {
        draw_rect(fb, x, y, 36, 28, Color::WIN11_HOVER);
        draw_rect(fb, x, y + 26, 36, 2, accent);
        let text_offset = (36 - (label.len() * FONT_WIDTH)) / 2;
        draw_string(fb, x + text_offset, y + 6, label, Color::WIN11_TEXT_PRIMARY, None);
    }
}
