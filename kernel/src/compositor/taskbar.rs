//! Desktop Taskbar and Window Task Item Renderer for KntonOS Compositor.
//!
//! Provides a 32px bottom taskbar layout, acrylic frosted glass background,
//! top border highlight (0x33FFFFFF), and dynamic window task buttons with
//! active/minimized visual indicators.

use alloc::string::String;
use alloc::vec::Vec;
use crate::display::color::Color;
use crate::display::font::{get_glyph, FONT_HEIGHT, FONT_WIDTH};
use super::acrylic::{AcrylicConfig, AcrylicSurface};
use super::dirty::Rect;
use super::window::Window;

/// Standard height of the bottom desktop taskbar.
pub const TASKBAR_HEIGHT: usize = 32;

/// Top border highlight line color (33% alpha white, 0x33FFFFFF).
pub const TASKBAR_TOP_BORDER_HIGHLIGHT: Color = Color::rgba(255, 255, 255, 0x33);

/// Action generated when clicking on a taskbar control.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TaskbarClickAction {
    /// Clicked Start button to toggle application launcher flyout.
    ToggleStartMenu,
    /// Clicked a window taskbar button to activate or minimize window.
    SelectWindow(u32),
}

/// Represents an active window item displayed on the taskbar.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TaskbarItem {
    pub window_id: u32,
    pub title: String,
    pub is_active: bool,
    pub is_minimized: bool,
    pub bounds: Rect,
}

/// Desktop taskbar managing bottom bar layout, window items, and background rendering.
pub struct DesktopTaskbar {
    pub height: usize,
    pub items: Vec<TaskbarItem>,
    pub top_border_color: Color,
    pub background_color: Color,
    pub acrylic_config: AcrylicConfig,
    pub start_button_bounds: Rect,
}

impl DesktopTaskbar {
    /// Creates a new desktop taskbar with standard 32px height.
    pub fn new(height: usize) -> Self {
        Self {
            height,
            items: Vec::new(),
            top_border_color: TASKBAR_TOP_BORDER_HIGHLIGHT,
            background_color: Color::WIN11_TASKBAR,
            acrylic_config: AcrylicConfig::default_dark(),
            start_button_bounds: Rect::new(4, 0, 36, 24),
        }
    }

    /// Returns the absolute screen bounding rectangle of the taskbar.
    pub fn bounds(&self, screen_width: usize, screen_height: usize) -> Rect {
        let y = screen_height.saturating_sub(self.height);
        Rect::new(0, y, screen_width, self.height)
    }

    /// Synchronizes taskbar buttons with the current list of opened windows.
    pub fn update_windows(&mut self, windows: &[Window], screen_width: usize, screen_height: usize) {
        self.items.clear();
        let taskbar_y = screen_height.saturating_sub(self.height);
        self.start_button_bounds = Rect::new(4, taskbar_y + 4, 36, 24);

        let start_x = 44;
        let item_width = 100;
        let item_height = 24;
        let gap = 4;
        let max_x = screen_width.saturating_sub(140); // reserve space for clock & tray

        for (i, win) in windows.iter().enumerate() {
            let item_x = start_x + i * (item_width + gap);
            if item_x + item_width > max_x {
                break;
            }

            self.items.push(TaskbarItem {
                window_id: win.id,
                title: win.title.clone(),
                is_active: win.is_focused,
                is_minimized: win.is_minimized,
                bounds: Rect::new(item_x, taskbar_y + 4, item_width, item_height),
            });
        }
    }

    /// Determines the action triggered by a mouse click at screen coordinates `(x, y)`.
    pub fn handle_mouse_click(&self, x: usize, y: usize) -> Option<TaskbarClickAction> {
        if self.start_button_bounds.contains_point(x, y) {
            return Some(TaskbarClickAction::ToggleStartMenu);
        }

        for item in &self.items {
            if item.bounds.contains_point(x, y) {
                return Some(TaskbarClickAction::SelectWindow(item.window_id));
            }
        }

        None
    }

    /// Renders the complete taskbar: acrylic background, top border highlight, Start button,
    /// and window task buttons into the compositor backbuffer.
    pub fn render(
        &self,
        buffer: &mut [Color],
        screen_width: usize,
        screen_height: usize,
        clip: Option<&Rect>,
    ) {
        if screen_width == 0 || screen_height == 0 || self.height == 0 {
            return;
        }

        let taskbar_y = screen_height.saturating_sub(self.height);
        let taskbar_rect = Rect::new(0, taskbar_y, screen_width, self.height);

        if let Some(c) = clip {
            if !c.intersects(&taskbar_rect) {
                return;
            }
        }

        // 1. Acrylic blur backdrop & base fill
        let snapshot = buffer.to_vec();
        AcrylicSurface::render_acrylic(
            &snapshot,
            buffer,
            screen_width,
            screen_height,
            &taskbar_rect,
            &self.acrylic_config,
            clip,
        );

        // 2. Top border highlight line (y = taskbar_y): 0x33FFFFFF blended over background
        let top_y = taskbar_y;
        if top_y < screen_height {
            let row_offset = top_y * screen_width;
            for x in 0..screen_width {
                if let Some(c) = clip {
                    if !c.contains_point(x, top_y) {
                        continue;
                    }
                }
                let bg = buffer[row_offset + x];
                buffer[row_offset + x] = self.top_border_color.blend(bg);
            }
        }

        // 3. Start Button at left corner
        self.render_start_button(buffer, screen_width, screen_height, clip);

        // 4. Window task buttons
        for item in &self.items {
            self.render_item(item, buffer, screen_width, screen_height, clip);
        }
    }

    /// Renders the Start button in the taskbar.
    fn render_start_button(
        &self,
        buffer: &mut [Color],
        screen_width: usize,
        screen_height: usize,
        clip: Option<&Rect>,
    ) {
        let b = self.start_button_bounds;
        draw_rect_clipped(buffer, screen_width, screen_height, b.x, b.y, b.width, b.height, Color::WIN11_HOVER, clip);

        // 4 Fluent logo tiles (2x2 grid)
        let tile_size = 5;
        let gap = 2;
        let tile_x = b.x + (b.width.saturating_sub(tile_size * 2 + gap)) / 2;
        let tile_y = b.y + (b.height.saturating_sub(tile_size * 2 + gap)) / 2;

        draw_rect_clipped(buffer, screen_width, screen_height, tile_x, tile_y, tile_size, tile_size, Color::WIN11_ACCENT, clip);
        draw_rect_clipped(buffer, screen_width, screen_height, tile_x + tile_size + gap, tile_y, tile_size, tile_size, Color::WIN11_ACCENT, clip);
        draw_rect_clipped(buffer, screen_width, screen_height, tile_x, tile_y + tile_size + gap, tile_size, tile_size, Color::WIN11_ACCENT, clip);
        draw_rect_clipped(buffer, screen_width, screen_height, tile_x + tile_size + gap, tile_y + tile_size + gap, tile_size, tile_size, Color::WIN11_ACCENT, clip);
    }

    /// Renders an individual window item button.
    fn render_item(
        &self,
        item: &TaskbarItem,
        buffer: &mut [Color],
        screen_width: usize,
        screen_height: usize,
        clip: Option<&Rect>,
    ) {
        let b = item.bounds;

        // Button background color depending on state
        let bg_color = if item.is_active && !item.is_minimized {
            Color::WIN11_HOVER
        } else if item.is_minimized {
            Color::rgba(24, 28, 36, 180)
        } else {
            Color::rgba(38, 44, 56, 160)
        };
        draw_rect_clipped(buffer, screen_width, screen_height, b.x, b.y, b.width, b.height, bg_color, clip);

        // Active/minimized bottom indicator line
        if item.is_active && !item.is_minimized {
            // Bright blue accent underline
            let bar_y = b.y + b.height.saturating_sub(2);
            let bar_w = b.width.saturating_sub(16);
            let bar_x = b.x + 8;
            draw_rect_clipped(buffer, screen_width, screen_height, bar_x, bar_y, bar_w, 2, Color::WIN11_ACCENT, clip);
        } else if !item.is_minimized {
            // Inactive open window: subtle grey indicator line
            let bar_y = b.y + b.height.saturating_sub(2);
            let bar_w = b.width.saturating_sub(24);
            let bar_x = b.x + 12;
            draw_rect_clipped(buffer, screen_width, screen_height, bar_x, bar_y, bar_w, 2, Color::WIN11_CARD_BORDER, clip);
        }

        // Window icon tile (6x6)
        let icon_x = b.x + 6;
        let icon_y = b.y + (b.height.saturating_sub(8)) / 2;
        let icon_color = if item.is_active {
            Color::WIN11_ACCENT
        } else {
            Color::WIN11_TEXT_SECONDARY
        };
        draw_rect_clipped(buffer, screen_width, screen_height, icon_x, icon_y, 8, 8, icon_color, clip);

        // Window title label (truncated to available button width)
        let max_chars = b.width.saturating_sub(20) / FONT_WIDTH;
        if max_chars > 0 {
            let label_x = b.x + 18;
            let label_y = b.y + (b.height.saturating_sub(FONT_HEIGHT)) / 2;
            let label_color = if item.is_active && !item.is_minimized {
                Color::WIN11_TEXT_PRIMARY
            } else {
                Color::WIN11_TEXT_SECONDARY
            };

            let mut char_count = 0;
            for c in item.title.chars() {
                if char_count >= max_chars {
                    break;
                }
                draw_char_clipped(buffer, screen_width, screen_height, label_x + char_count * FONT_WIDTH, label_y, c, label_color, clip);
                char_count += 1;
            }
        }
    }
}

/// Helper to draw a solid rectangle clipped to `clip` and screen boundaries.
pub fn draw_rect_clipped(
    buffer: &mut [Color],
    screen_width: usize,
    screen_height: usize,
    x: usize,
    y: usize,
    w: usize,
    h: usize,
    color: Color,
    clip: Option<&Rect>,
) {
    for row in 0..h {
        let ty = y + row;
        if ty >= screen_height {
            break;
        }
        let row_offset = ty * screen_width;
        for col in 0..w {
            let tx = x + col;
            if tx >= screen_width {
                break;
            }
            if let Some(c) = clip {
                if !c.contains_point(tx, ty) {
                    continue;
                }
            }
            buffer[row_offset + tx] = color;
        }
    }
}

/// Helper to render an 8x16 bitmap character glyph clipped to `clip` and screen boundaries.
pub fn draw_char_clipped(
    buffer: &mut [Color],
    screen_width: usize,
    screen_height: usize,
    x: usize,
    y: usize,
    c: char,
    fg: Color,
    clip: Option<&Rect>,
) {
    if c == '\n' {
        return;
    }
    let glyph = get_glyph(c);
    for row in 0..FONT_HEIGHT {
        let ty = y + row;
        if ty >= screen_height {
            break;
        }
        let bits = glyph[row];
        let row_offset = ty * screen_width;
        for col in 0..FONT_WIDTH {
            let tx = x + col;
            if tx >= screen_width {
                break;
            }
            if (bits & (0x80 >> col)) != 0 {
                if let Some(cl) = clip {
                    if !cl.contains_point(tx, ty) {
                        continue;
                    }
                }
                buffer[row_offset + tx] = fg;
            }
        }
    }
}
