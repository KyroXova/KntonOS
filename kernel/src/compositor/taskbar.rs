//! Desktop Taskbar and Window Task Item Renderer for KntonOS Compositor.
//! Provides a 32px bottom taskbar layout, acrylic frosted glass background,

use alloc::format;
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
    /// Clicked on system tray clock / calendar.
    ToggleClockFlyout,
}

/// Network connectivity indicator state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NetworkStatus {
    Disconnected,
    Connected,
    Active,
}

/// System audio volume indicator state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VolumeStatus {
    Muted,
    Low,
    Medium,
    High,
}

/// System battery and power supply state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PowerStatus {
    Battery(u8),
    Charging(u8),
    AcPower,
}

/// Reads the current hardware time (hours, minutes, seconds) from x86 CMOS RTC registers.
pub fn read_cmos_rtc() -> (u8, u8, u8) {
    unsafe {
        // Read Status Register B to determine if values are binary or BCD
        core::arch::asm!("out 0x70, al", in("al") 0x0Bu8, options(nomem, nostack, preserves_flags));
        let mut status_b: u8;
        core::arch::asm!("in al, 0x71", out("al") status_b, options(nomem, nostack, preserves_flags));

        // Read seconds (reg 0x00)
        core::arch::asm!("out 0x70, al", in("al") 0x00u8, options(nomem, nostack, preserves_flags));
        let mut sec: u8;
        core::arch::asm!("in al, 0x71", out("al") sec, options(nomem, nostack, preserves_flags));

        // Read minutes (reg 0x02)
        core::arch::asm!("out 0x70, al", in("al") 0x02u8, options(nomem, nostack, preserves_flags));
        let mut min: u8;
        core::arch::asm!("in al, 0x71", out("al") min, options(nomem, nostack, preserves_flags));

        // Read hours (reg 0x04)
        core::arch::asm!("out 0x70, al", in("al") 0x04u8, options(nomem, nostack, preserves_flags));
        let mut hr: u8;
        core::arch::asm!("in al, 0x71", out("al") hr, options(nomem, nostack, preserves_flags));

        // If BCD bit (bit 2) is not set, convert BCD to binary
        let is_binary = (status_b & 0x04) != 0;
        if !is_binary {
            sec = (sec & 0x0F) + ((sec / 16) * 10);
            min = (min & 0x0F) + ((min / 16) * 10);
            hr = ((hr & 0x0F) + (((hr & 0x70) / 16) * 10)) | (hr & 0x80);
        }

        // Convert 12-hour format to 24-hour if bit 1 is not set
        let is_24hr = (status_b & 0x02) != 0;
        if !is_24hr && (hr & 0x80) != 0 {
            hr = ((hr & 0x7F) + 12) % 24;
        }

        (hr % 24, min % 60, sec % 60)
    }
}

/// System tray managing digital clock timekeeping and status indicator icons.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SystemTray {
    pub network: NetworkStatus,
    pub volume: VolumeStatus,
    pub power: PowerStatus,
    pub hours: u8,
    pub minutes: u8,
    pub seconds: u8,
}

impl SystemTray {
    /// Constructs a new system tray initialized to noon (12:00:00) on AC power.
    pub fn new() -> Self {
        Self {
            network: NetworkStatus::Connected,
            volume: VolumeStatus::High,
            power: PowerStatus::AcPower,
            hours: 12,
            minutes: 0,
            seconds: 0,
        }
    }

    /// Sets the digital clock time components.
    pub fn set_time(&mut self, hours: u8, minutes: u8, seconds: u8) {
        self.hours = hours % 24;
        self.minutes = minutes % 60;
        self.seconds = seconds % 60;
    }

    /// Advances the clock by one second with minute and hour overflow rollover.
    pub fn tick_second(&mut self) {
        self.seconds += 1;
        if self.seconds >= 60 {
            self.seconds = 0;
            self.minutes += 1;
            if self.minutes >= 60 {
                self.minutes = 0;
                self.hours = (self.hours + 1) % 24;
            }
        }
    }

    /// Synchronizes the digital clock with the hardware CMOS RTC.
    pub fn update_from_rtc(&mut self) {
        let (h, m, s) = read_cmos_rtc();
        self.set_time(h, m, s);
    }

    /// Formats the current time into `HH:MM:SS`.
    pub fn format_time(&self) -> String {
        format!("{:02}:{:02}:{:02}", self.hours, self.minutes, self.seconds)
    }

    /// Renders the system tray status icons and digital clock string into `buffer`.
    pub fn render(
        &self,
        buffer: &mut [Color],
        screen_width: usize,
        screen_height: usize,
        taskbar_y: usize,
        clip: Option<&Rect>,
    ) {
        // 1. Digital Clock at far right
        let clock_str = self.format_time();
        let clock_w = clock_str.len() * FONT_WIDTH;
        let clock_x = screen_width.saturating_sub(clock_w + 12);
        let text_y = taskbar_y + (TASKBAR_HEIGHT.saturating_sub(FONT_HEIGHT)) / 2;

        draw_string_clipped(
            buffer,
            screen_width,
            screen_height,
            clock_x,
            text_y,
            &clock_str,
            Color::WIN11_TEXT_PRIMARY,
            clip,
        );

        // 2. Battery / Power indicator (to the left of clock)
        let power_x = clock_x.saturating_sub(24);
        let icon_y = taskbar_y + 11;
        self.render_power_icon(buffer, screen_width, screen_height, power_x, icon_y, clip);

        // 3. Volume indicator (to the left of power)
        let volume_x = power_x.saturating_sub(22);
        self.render_volume_icon(buffer, screen_width, screen_height, volume_x, icon_y, clip);

        // 4. Network indicator (to the left of volume)
        let net_x = volume_x.saturating_sub(22);
        self.render_network_icon(buffer, screen_width, screen_height, net_x, icon_y, clip);
    }

    fn render_network_icon(
        &self,
        buffer: &mut [Color],
        screen_width: usize,
        screen_height: usize,
        x: usize,
        y: usize,
        clip: Option<&Rect>,
    ) {
        let fg = match self.network {
            NetworkStatus::Active => Color::WIN11_ACCENT,
            NetworkStatus::Connected => Color::WIN11_TEXT_PRIMARY,
            NetworkStatus::Disconnected => Color::WIN11_TEXT_SECONDARY,
        };

        // 3 ascending signal bars (heights 4, 7, 10)
        draw_rect_clipped(buffer, screen_width, screen_height, x, y + 6, 2, 4, fg, clip);
        draw_rect_clipped(buffer, screen_width, screen_height, x + 4, y + 3, 2, 7, fg, clip);
        draw_rect_clipped(buffer, screen_width, screen_height, x + 8, y, 2, 10, fg, clip);

        if self.network == NetworkStatus::Disconnected {
            // Draw small diagonal red slash
            draw_rect_clipped(buffer, screen_width, screen_height, x, y, 10, 1, Color::WIN11_CLOSE_RED, clip);
        }
    }

    fn render_volume_icon(
        &self,
        buffer: &mut [Color],
        screen_width: usize,
        screen_height: usize,
        x: usize,
        y: usize,
        clip: Option<&Rect>,
    ) {
        let fg = if self.volume == VolumeStatus::Muted {
            Color::WIN11_TEXT_SECONDARY
        } else {
            Color::WIN11_TEXT_PRIMARY
        };

        // Speaker rectangle box (3x6)
        draw_rect_clipped(buffer, screen_width, screen_height, x, y + 2, 3, 6, fg, clip);
        // Cone trapezoid
        draw_rect_clipped(buffer, screen_width, screen_height, x + 3, y + 1, 2, 8, fg, clip);
        draw_rect_clipped(buffer, screen_width, screen_height, x + 5, y, 2, 10, fg, clip);

        // Sound arcs
        match self.volume {
            VolumeStatus::High => {
                draw_rect_clipped(buffer, screen_width, screen_height, x + 8, y + 2, 1, 6, Color::WIN11_ACCENT, clip);
                draw_rect_clipped(buffer, screen_width, screen_height, x + 10, y + 1, 1, 8, Color::WIN11_ACCENT, clip);
            }
            VolumeStatus::Medium => {
                draw_rect_clipped(buffer, screen_width, screen_height, x + 8, y + 2, 1, 6, Color::WIN11_ACCENT, clip);
            }
            VolumeStatus::Low => {
                draw_rect_clipped(buffer, screen_width, screen_height, x + 8, y + 3, 1, 4, Color::WIN11_ACCENT, clip);
            }
            VolumeStatus::Muted => {
                // Red 'x'
                draw_rect_clipped(buffer, screen_width, screen_height, x + 8, y + 2, 1, 2, Color::WIN11_CLOSE_RED, clip);
                draw_rect_clipped(buffer, screen_width, screen_height, x + 10, y + 2, 1, 2, Color::WIN11_CLOSE_RED, clip);
                draw_rect_clipped(buffer, screen_width, screen_height, x + 9, y + 4, 1, 2, Color::WIN11_CLOSE_RED, clip);
                draw_rect_clipped(buffer, screen_width, screen_height, x + 8, y + 6, 1, 2, Color::WIN11_CLOSE_RED, clip);
                draw_rect_clipped(buffer, screen_width, screen_height, x + 10, y + 6, 1, 2, Color::WIN11_CLOSE_RED, clip);
            }
        }
    }

    fn render_power_icon(
        &self,
        buffer: &mut [Color],
        screen_width: usize,
        screen_height: usize,
        x: usize,
        y: usize,
        clip: Option<&Rect>,
    ) {
        // Battery body outline (12x7)
        draw_rect_clipped(buffer, screen_width, screen_height, x, y + 1, 12, 1, Color::WIN11_TEXT_SECONDARY, clip);
        draw_rect_clipped(buffer, screen_width, screen_height, x, y + 7, 12, 1, Color::WIN11_TEXT_SECONDARY, clip);
        draw_rect_clipped(buffer, screen_width, screen_height, x, y + 1, 1, 7, Color::WIN11_TEXT_SECONDARY, clip);
        draw_rect_clipped(buffer, screen_width, screen_height, x + 11, y + 1, 1, 7, Color::WIN11_TEXT_SECONDARY, clip);

        // Terminal notch (2x3)
        draw_rect_clipped(buffer, screen_width, screen_height, x + 12, y + 3, 2, 3, Color::WIN11_TEXT_SECONDARY, clip);

        // Fill indicator
        match self.power {
            PowerStatus::AcPower => {
                // Solid green / full
                draw_rect_clipped(buffer, screen_width, screen_height, x + 2, y + 3, 8, 3, Color::rgb(16, 124, 65), clip);
            }
            PowerStatus::Charging(pct) => {
                let fill_w = ((pct as usize * 8) / 100).max(1);
                draw_rect_clipped(buffer, screen_width, screen_height, x + 2, y + 3, fill_w, 3, Color::WIN11_ACCENT, clip);
            }
            PowerStatus::Battery(pct) => {
                let fill_w = ((pct as usize * 8) / 100).max(1);
                let col = if pct < 20 {
                    Color::WIN11_CLOSE_RED
                } else if pct < 50 {
                    Color::rgb(255, 185, 0)
                } else {
                    Color::rgb(16, 124, 65)
                };
                draw_rect_clipped(buffer, screen_width, screen_height, x + 2, y + 3, fill_w, 3, col, clip);
            }
        }
    }
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

/// Desktop taskbar managing bottom bar layout, window items, system tray, and background rendering.
pub struct DesktopTaskbar {
    pub height: usize,
    pub items: Vec<TaskbarItem>,
    pub tray: SystemTray,
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
            tray: SystemTray::new(),
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

        if windows.is_empty() {
            return;
        }

        let start_x = 44;
        let item_height = 24;
        let gap = 4;
        let max_x = screen_width.saturating_sub(140); // reserve space for clock & tray
        let available_w = max_x.saturating_sub(start_x);
        let item_width = ((available_w / windows.len()).saturating_sub(gap)).min(100).max(36);

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
    /// window task buttons, and system tray (indicators + clock) into the compositor backbuffer.
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

        // 1. Acrylic blur backdrop & base fill in-place
        AcrylicSurface::render_acrylic_in_place(
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

        // 5. System Tray (indicators + RTC digital clock)
        self.tray.render(buffer, screen_width, screen_height, taskbar_y, clip);
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

/// Helper to render a text string clipped to `clip` and screen boundaries.
pub fn draw_string_clipped(
    buffer: &mut [Color],
    screen_width: usize,
    screen_height: usize,
    x: usize,
    y: usize,
    s: &str,
    fg: Color,
    clip: Option<&Rect>,
) {
    let mut cur_x = x;
    for c in s.chars() {
        if cur_x + FONT_WIDTH > screen_width {
            break;
        }
        draw_char_clipped(buffer, screen_width, screen_height, cur_x, y, c, fg, clip);
        cur_x += FONT_WIDTH;
    }
}
