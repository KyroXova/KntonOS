//! Windows 11 Fluent Start Menu Application Launcher Flyout for KntonOS.
//! Provides a 200x260px frosted acrylic glass flyout panel positioned above the bottom

use alloc::string::String;
use alloc::vec::Vec;
use crate::display::color::Color;
use crate::display::font::FONT_WIDTH;
use super::acrylic::{AcrylicConfig, AcrylicSurface};
use super::dirty::Rect;
use super::taskbar::{draw_char_clipped, draw_rect_clipped, draw_string_clipped, TASKBAR_HEIGHT};

/// Width of the Start menu application launcher panel.
pub const LAUNCHER_WIDTH: usize = 200;

/// Height of the Start menu application launcher panel.
pub const LAUNCHER_HEIGHT: usize = 260;

/// Identifier of standard applications available in the launcher.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LauncherAppId {
    Terminal,
    ProcessManager,
    FileExplorer,
    TelemetryMonitor,
}

/// Action produced by mouse interaction with the application launcher.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LauncherClickAction {
    /// Launch the specified system application and dismiss flyout.
    LaunchApp(LauncherAppId),
    /// Clicked outside the launcher bounds; dismiss flyout.
    Dismiss,
}

/// A single application entry in the Start menu list.
#[derive(Debug, Clone)]
pub struct LauncherAppEntry {
    pub id: LauncherAppId,
    pub title: String,
    pub subtitle: String,
    pub icon_color: Color,
    pub bounds: Rect,
}

/// Application launcher flyout state, items, and acrylic renderer.
pub struct StartMenuLauncher {
    pub is_open: bool,
    pub entries: Vec<LauncherAppEntry>,
    pub acrylic_config: AcrylicConfig,
    pub border_color: Color,
    pub background_color: Color,
}

impl StartMenuLauncher {
    /// Creates a new Start menu launcher populated with default applications.
    pub fn new() -> Self {
        let mut launcher = Self {
            is_open: false,
            entries: Vec::new(),
            acrylic_config: AcrylicConfig::default_dark(),
            border_color: Color::WIN11_CARD_BORDER,
            background_color: Color::rgba(32, 38, 48, 230),
        };
        launcher.populate_default_entries(0, 0);
        launcher
    }

    /// Computes the bounding rectangle of the floating launcher panel.
    pub fn bounds(&self, screen_width: usize, screen_height: usize) -> Rect {
        let x = 6.min(screen_width.saturating_sub(LAUNCHER_WIDTH));
        let y = screen_height.saturating_sub(TASKBAR_HEIGHT + LAUNCHER_HEIGHT + 6);
        Rect::new(x, y, LAUNCHER_WIDTH.min(screen_width), LAUNCHER_HEIGHT.min(screen_height))
    }

    /// Toggles the open/closed state of the launcher.
    pub fn toggle(&mut self) {
        self.is_open = !self.is_open;
    }

    /// Opens the launcher flyout.
    pub fn open(&mut self) {
        self.is_open = true;
    }

    /// Closes the launcher flyout.
    pub fn close(&mut self) {
        self.is_open = false;
    }

    /// Populates app entries positioned relative to the panel coordinates.
    pub fn populate_default_entries(&mut self, panel_x: usize, panel_y: usize) {
        self.entries.clear();
        let app_defs = [
            (
                LauncherAppId::Terminal,
                "Terminal",
                "Virtual Terminal Shell (VT)",
                Color::WIN11_ACCENT,
            ),
            (
                LauncherAppId::ProcessManager,
                "Process Manager",
                "NT Tasks & Threads",
                Color::rgb(16, 124, 65),
            ),
            (
                LauncherAppId::FileExplorer,
                "File Explorer",
                "FAT32 & Initramfs VFS",
                Color::rgb(255, 185, 0),
            ),
            (
                LauncherAppId::TelemetryMonitor,
                "Telemetry Monitor",
                "SMP Cores & Memory",
                Color::rgb(180, 0, 180),
            ),
        ];

        let start_y = panel_y + 44;
        let item_height = 42;
        let item_gap = 4;
        let item_width = LAUNCHER_WIDTH.saturating_sub(16);
        let item_x = panel_x + 8;

        for (i, &(id, title, subtitle, icon_color)) in app_defs.iter().enumerate() {
            let item_y = start_y + i * (item_height + item_gap);
            self.entries.push(LauncherAppEntry {
                id,
                title: String::from(title),
                subtitle: String::from(subtitle),
                icon_color,
                bounds: Rect::new(item_x, item_y, item_width, item_height),
            });
        }
    }

    /// Dispatches mouse click events: launches app if clicked inside an item, or dismisses if clicked outside.
    pub fn handle_mouse_click(
        &mut self,
        mx: usize,
        my: usize,
        screen_width: usize,
        screen_height: usize,
    ) -> Option<LauncherClickAction> {
        if !self.is_open {
            return None;
        }

        let panel_bounds = self.bounds(screen_width, screen_height);
        if !panel_bounds.contains_point(mx, my) {
            self.close();
            return Some(LauncherClickAction::Dismiss);
        }

        // Check if an app entry was clicked
        self.populate_default_entries(panel_bounds.x, panel_bounds.y);
        for entry in &self.entries {
            if entry.bounds.contains_point(mx, my) {
                let id = entry.id;
                self.close();
                return Some(LauncherClickAction::LaunchApp(id));
            }
        }

        None
    }

    /// Renders the floating frosted acrylic Start menu panel.
    pub fn render(
        &mut self,
        buffer: &mut [Color],
        screen_width: usize,
        screen_height: usize,
        clip: Option<&Rect>,
    ) {
        if !self.is_open || screen_width == 0 || screen_height == 0 {
            return;
        }

        let panel_bounds = self.bounds(screen_width, screen_height);
        if let Some(c) = clip {
            if !c.intersects(&panel_bounds) {
                return;
            }
        }

        self.populate_default_entries(panel_bounds.x, panel_bounds.y);

        // 1. Acrylic blur backdrop & translucent fill in-place
        AcrylicSurface::render_acrylic_in_place(
            buffer,
            screen_width,
            screen_height,
            &panel_bounds,
            &self.acrylic_config,
            clip,
        );

        // 2. Panel 1px subtle border outline
        let bx = panel_bounds.x;
        let by = panel_bounds.y;
        let bw = panel_bounds.width;
        let bh = panel_bounds.height;
        let border_col = self.border_color;

        draw_rect_clipped(buffer, screen_width, screen_height, bx, by, bw, 1, border_col, clip);
        draw_rect_clipped(buffer, screen_width, screen_height, bx, by + bh - 1, bw, 1, border_col, clip);
        draw_rect_clipped(buffer, screen_width, screen_height, bx, by, 1, bh, border_col, clip);
        draw_rect_clipped(buffer, screen_width, screen_height, bx + bw - 1, by, 1, bh, border_col, clip);

        // 3. Header title
        draw_string_clipped(
            buffer,
            screen_width,
            screen_height,
            bx + 12,
            by + 12,
            "Pinned Apps",
            Color::WIN11_TEXT_PRIMARY,
            clip,
        );

        // Header separator line
        draw_rect_clipped(
            buffer,
            screen_width,
            screen_height,
            bx + 8,
            by + 34,
            bw.saturating_sub(16),
            1,
            Color::rgba(255, 255, 255, 0x1A),
            clip,
        );

        // 4. Application items list
        for entry in &self.entries {
            self.render_entry(entry, buffer, screen_width, screen_height, clip);
        }

        // 5. Footer user profile & power status line
        let footer_y = by + bh.saturating_sub(26);
        draw_rect_clipped(
            buffer,
            screen_width,
            screen_height,
            bx + 8,
            footer_y.saturating_sub(4),
            bw.saturating_sub(16),
            1,
            Color::rgba(255, 255, 255, 0x1A),
            clip,
        );

        // User avatar icon (8x8)
        draw_rect_clipped(buffer, screen_width, screen_height, bx + 12, footer_y + 2, 8, 8, Color::WIN11_ACCENT, clip);
        draw_string_clipped(buffer, screen_width, screen_height, bx + 26, footer_y, "Kyro", Color::WIN11_TEXT_SECONDARY, clip);

        // Power symbol on right side of footer
        let power_x = bx + bw.saturating_sub(24);
        draw_rect_clipped(buffer, screen_width, screen_height, power_x, footer_y + 2, 8, 8, Color::WIN11_CLOSE_RED, clip);
    }

    fn render_entry(
        &self,
        entry: &LauncherAppEntry,
        buffer: &mut [Color],
        screen_width: usize,
        screen_height: usize,
        clip: Option<&Rect>,
    ) {
        let b = entry.bounds;

        // Card hover styling
        draw_rect_clipped(
            buffer,
            screen_width,
            screen_height,
            b.x,
            b.y,
            b.width,
            b.height,
            Color::rgba(45, 52, 64, 140),
            clip,
        );

        // Application icon badge (12x12)
        let icon_x = b.x + 8;
        let icon_y = b.y + (b.height.saturating_sub(12)) / 2;
        draw_rect_clipped(buffer, screen_width, screen_height, icon_x, icon_y, 12, 12, entry.icon_color, clip);

        // App title
        let text_x = b.x + 28;
        draw_string_clipped(
            buffer,
            screen_width,
            screen_height,
            text_x,
            b.y + 4,
            &entry.title,
            Color::WIN11_TEXT_PRIMARY,
            clip,
        );

        // Subtitle (truncated)
        let max_sub_chars = b.width.saturating_sub(32) / FONT_WIDTH;
        if max_sub_chars > 0 {
            let mut char_count = 0;
            for c in entry.subtitle.chars() {
                if char_count >= max_sub_chars {
                    break;
                }
                draw_char_clipped(
                    buffer,
                    screen_width,
                    screen_height,
                    text_x + char_count * FONT_WIDTH,
                    b.y + 22,
                    c,
                    Color::WIN11_TEXT_SECONDARY,
                    clip,
                );
                char_count += 1;
            }
        }
    }
}
