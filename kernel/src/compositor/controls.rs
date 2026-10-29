//! UI Widget Hierarchy and GUI Controls for KntonOS Compositor.
//!
//! Provides interactive GUI widgets (Buttons, Labels, TitleBarButtons) with state feedback
//! (Normal, Hovered, Pressed, Disabled), text centering, border transitions, and click event handling.

use alloc::string::String;
use crate::display::color::Color;
use crate::display::font::{get_glyph, FONT_HEIGHT, FONT_WIDTH};
use super::dirty::Rect;

/// Interactive states for GUI buttons and controls.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ButtonState {
    Normal,
    Hovered,
    Pressed,
    Disabled,
}

impl ButtonState {
    /// Returns background color according to interaction state and accent styling.
    pub fn bg_color(&self, default_bg: Color, is_accent: bool) -> Color {
        match self {
            ButtonState::Normal => default_bg,
            ButtonState::Hovered => {
                if is_accent {
                    Color::rgb(0, 140, 255)
                } else {
                    Color::WIN11_HOVER
                }
            }
            ButtonState::Pressed => {
                if is_accent {
                    Color::rgb(0, 100, 200)
                } else {
                    Color::WIN11_PRESSED
                }
            }
            ButtonState::Disabled => Color::rgb(45, 45, 55),
        }
    }

    /// Returns text foreground color according to state.
    pub fn text_color(&self) -> Color {
        match self {
            ButtonState::Disabled => Color::WIN11_TEXT_SECONDARY,
            _ => Color::WHITE,
        }
    }

    /// Returns border color according to state and focus.
    pub fn border_color(&self, is_focused: bool) -> Color {
        match self {
            ButtonState::Pressed => Color::WIN11_ACCENT,
            ButtonState::Hovered => Color::rgb(120, 130, 150),
            _ => {
                if is_focused {
                    Color::WIN11_ACCENT
                } else {
                    Color::WIN11_CARD_BORDER
                }
            }
        }
    }
}

/// Standard types for window header controls.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TitleBarButtonType {
    Minimize,
    Maximize,
    Close,
}

/// Interactive button control with state feedback and click reporting.
#[derive(Debug, Clone)]
pub struct Button {
    pub id: u32,
    pub bounds: Rect,
    pub label: String,
    pub state: ButtonState,
    pub is_accent: bool,
    pub is_focused: bool,
    pub clicked: bool,
}

impl Button {
    pub fn new(id: u32, bounds: Rect, label: &str) -> Self {
        Self {
            id,
            bounds,
            label: String::from(label),
            state: ButtonState::Normal,
            is_accent: false,
            is_focused: false,
            clicked: false,
        }
    }

    pub fn new_accent(id: u32, bounds: Rect, label: &str) -> Self {
        Self {
            id,
            bounds,
            label: String::from(label),
            state: ButtonState::Normal,
            is_accent: true,
            is_focused: true,
            clicked: false,
        }
    }

    pub fn set_disabled(&mut self, disabled: bool) {
        if disabled {
            self.state = ButtonState::Disabled;
        } else if self.state == ButtonState::Disabled {
            self.state = ButtonState::Normal;
        }
    }

    pub fn take_click(&mut self) -> bool {
        let was_clicked = self.clicked;
        self.clicked = false;
        was_clicked
    }

    /// Process mouse position and button state. Returns true if visual state changed.
    pub fn handle_mouse(&mut self, mx: usize, my: usize, is_down: bool, was_down: bool) -> bool {
        if self.state == ButtonState::Disabled {
            return false;
        }

        let inside = self.bounds.contains_point(mx, my);
        let old_state = self.state;

        if inside {
            if is_down {
                self.state = ButtonState::Pressed;
            } else {
                if was_down && old_state == ButtonState::Pressed {
                    self.clicked = true;
                }
                self.state = ButtonState::Hovered;
            }
        } else {
            self.state = ButtonState::Normal;
        }

        old_state != self.state || self.clicked
    }

    pub fn render(
        &self,
        target: &mut [Color],
        target_w: usize,
        target_h: usize,
        clip: Option<&Rect>,
    ) {
        if self.bounds.is_empty() {
            return;
        }

        let default_bg = if self.is_accent {
            Color::WIN11_ACCENT
        } else {
            Color::WIN11_CARD_BG
        };
        let bg_color = self.state.bg_color(default_bg, self.is_accent);
        let border_color = self.state.border_color(self.is_focused);
        let text_color = self.state.text_color();

        // 1. Fill background
        fill_rect(target, target_w, target_h, &self.bounds, bg_color, clip);

        // 2. Draw 1px border outline
        draw_rect_outline(target, target_w, target_h, &self.bounds, 1, border_color, clip);

        // 3. Render centered text label
        let text_width = self.label.len() * FONT_WIDTH;
        let tx = if self.bounds.width > text_width {
            self.bounds.x + (self.bounds.width - text_width) / 2
        } else {
            self.bounds.x + 2
        };
        let ty = if self.bounds.height > FONT_HEIGHT {
            self.bounds.y + (self.bounds.height - FONT_HEIGHT) / 2
        } else {
            self.bounds.y + 1
        };

        render_text(target, target_w, target_h, tx, ty, &self.label, text_color, clip);
    }
}

/// Standard window titlebar control button (Minimize, Maximize, Close).
#[derive(Debug, Clone)]
pub struct TitleBarButton {
    pub button_type: TitleBarButtonType,
    pub bounds: Rect,
    pub state: ButtonState,
    pub clicked: bool,
}

impl TitleBarButton {
    pub fn new(button_type: TitleBarButtonType, bounds: Rect) -> Self {
        Self {
            button_type,
            bounds,
            state: ButtonState::Normal,
            clicked: false,
        }
    }

    pub fn take_click(&mut self) -> bool {
        let was_clicked = self.clicked;
        self.clicked = false;
        was_clicked
    }

    pub fn handle_mouse(&mut self, mx: usize, my: usize, is_down: bool, was_down: bool) -> bool {
        let inside = self.bounds.contains_point(mx, my);
        let old_state = self.state;

        if inside {
            if is_down {
                self.state = ButtonState::Pressed;
            } else {
                if was_down && old_state == ButtonState::Pressed {
                    self.clicked = true;
                }
                self.state = ButtonState::Hovered;
            }
        } else {
            self.state = ButtonState::Normal;
        }

        old_state != self.state || self.clicked
    }

    pub fn render(
        &self,
        target: &mut [Color],
        target_w: usize,
        target_h: usize,
        clip: Option<&Rect>,
    ) {
        if self.bounds.is_empty() {
            return;
        }

        let bg_color = match self.button_type {
            TitleBarButtonType::Close => match self.state {
                ButtonState::Hovered => Color::rgb(232, 17, 35),
                ButtonState::Pressed => Color::rgb(180, 10, 25),
                _ => Color::WIN11_CLOSE_RED,
            },
            _ => match self.state {
                ButtonState::Hovered => Color::WIN11_HOVER,
                ButtonState::Pressed => Color::WIN11_PRESSED,
                _ => Color::TRANSPARENT,
            },
        };

        if bg_color.a > 0 {
            fill_rect(target, target_w, target_h, &self.bounds, bg_color, clip);
        }

        let text_color = Color::WHITE;
        let cx = self.bounds.x + (self.bounds.width.saturating_sub(FONT_WIDTH)) / 2;
        let cy = self.bounds.y + (self.bounds.height.saturating_sub(FONT_HEIGHT)) / 2;

        match self.button_type {
            TitleBarButtonType::Minimize => {
                // Horizontal line for '_'
                render_text(target, target_w, target_h, cx, cy, "_", text_color, clip);
            }
            TitleBarButtonType::Maximize => {
                // Square outline for '□'
                let box_rect = Rect::new(self.bounds.x + 11, self.bounds.y + 11, 10, 10);
                draw_rect_outline(target, target_w, target_h, &box_rect, 1, text_color, clip);
            }
            TitleBarButtonType::Close => {
                render_text(target, target_w, target_h, cx, cy, "X", text_color, clip);
            }
        }
    }
}

/// Static text label widget.
#[derive(Debug, Clone)]
pub struct Label {
    pub id: u32,
    pub bounds: Rect,
    pub text: String,
    pub color: Color,
}

impl Label {
    pub fn new(id: u32, bounds: Rect, text: &str, color: Color) -> Self {
        Self {
            id,
            bounds,
            text: String::from(text),
            color,
        }
    }

    pub fn render(
        &self,
        target: &mut [Color],
        target_w: usize,
        target_h: usize,
        clip: Option<&Rect>,
    ) {
        if self.bounds.is_empty() {
            return;
        }
        render_text(
            target,
            target_w,
            target_h,
            self.bounds.x,
            self.bounds.y,
            &self.text,
            self.color,
            clip,
        );
    }
}

/// Polymorphic UI widget container.
#[derive(Debug, Clone)]
pub enum UiWidget {
    Button(Button),
    TitleBarButton(TitleBarButton),
    Label(Label),
}

impl UiWidget {
    pub fn bounds(&self) -> Rect {
        match self {
            UiWidget::Button(b) => b.bounds,
            UiWidget::TitleBarButton(tb) => tb.bounds,
            UiWidget::Label(l) => l.bounds,
        }
    }

    pub fn render(
        &self,
        target: &mut [Color],
        target_w: usize,
        target_h: usize,
        clip: Option<&Rect>,
    ) {
        match self {
            UiWidget::Button(b) => b.render(target, target_w, target_h, clip),
            UiWidget::TitleBarButton(tb) => tb.render(target, target_w, target_h, clip),
            UiWidget::Label(l) => l.render(target, target_w, target_h, clip),
        }
    }

    pub fn handle_mouse(&mut self, mx: usize, my: usize, is_down: bool, was_down: bool) -> bool {
        match self {
            UiWidget::Button(b) => b.handle_mouse(mx, my, is_down, was_down),
            UiWidget::TitleBarButton(tb) => tb.handle_mouse(mx, my, is_down, was_down),
            UiWidget::Label(_) => false,
        }
    }
}

/// Helper function to fill a clipped rectangle into a color backbuffer.
pub fn fill_rect(
    target: &mut [Color],
    target_w: usize,
    target_h: usize,
    rect: &Rect,
    color: Color,
    clip: Option<&Rect>,
) {
    if rect.is_empty() || color.a == 0 {
        return;
    }

    let start_x = rect.x;
    let end_x = (rect.x + rect.width).min(target_w);
    let start_y = rect.y;
    let end_y = (rect.y + rect.height).min(target_h);

    for y in start_y..end_y {
        for x in start_x..end_x {
            if let Some(c) = clip {
                if !c.contains_point(x, y) {
                    continue;
                }
            }
            let idx = y * target_w + x;
            if color.a == 255 {
                target[idx] = color;
            } else {
                target[idx] = color.blend(target[idx]);
            }
        }
    }
}

/// Helper function to draw an outlined border rectangle.
pub fn draw_rect_outline(
    target: &mut [Color],
    target_w: usize,
    target_h: usize,
    rect: &Rect,
    border_width: usize,
    color: Color,
    clip: Option<&Rect>,
) {
    if rect.is_empty() || border_width == 0 || color.a == 0 {
        return;
    }

    // Top border
    fill_rect(
        target,
        target_w,
        target_h,
        &Rect::new(rect.x, rect.y, rect.width, border_width),
        color,
        clip,
    );
    // Bottom border
    let bottom_y = rect.y + rect.height.saturating_sub(border_width);
    fill_rect(
        target,
        target_w,
        target_h,
        &Rect::new(rect.x, bottom_y, rect.width, border_width),
        color,
        clip,
    );
    // Left border
    fill_rect(
        target,
        target_w,
        target_h,
        &Rect::new(rect.x, rect.y, border_width, rect.height),
        color,
        clip,
    );
    // Right border
    let right_x = rect.x + rect.width.saturating_sub(border_width);
    fill_rect(
        target,
        target_w,
        target_h,
        &Rect::new(right_x, rect.y, border_width, rect.height),
        color,
        clip,
    );
}

/// Helper function to render a string with font glyphs into a color buffer.
pub fn render_text(
    target: &mut [Color],
    target_w: usize,
    target_h: usize,
    start_x: usize,
    start_y: usize,
    text: &str,
    color: Color,
    clip: Option<&Rect>,
) {
    let mut cur_x = start_x;
    for ch in text.chars() {
        if cur_x + FONT_WIDTH > target_w {
            break;
        }

        let glyph = get_glyph(ch);
        for row in 0..FONT_HEIGHT {
            let py = start_y + row;
            if py >= target_h {
                break;
            }
            let bits = glyph[row];
            for col in 0..FONT_WIDTH {
                let px = cur_x + col;
                if px >= target_w {
                    break;
                }
                if let Some(c) = clip {
                    if !c.contains_point(px, py) {
                        continue;
                    }
                }
                let is_set = (bits & (0x80 >> col)) != 0;
                if is_set {
                    let idx = py * target_w + px;
                    target[idx] = color;
                }
            }
        }
        cur_x += FONT_WIDTH;
    }
}
