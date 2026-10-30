//! ANSI / VT100 / xterm Terminal Escape Sequence Parser & 24-bit TrueColor SGR Renderer.
//!
//! Implements:
//! - ANSI parser state machine (Ground, Escape, CSI, OSC)
//! - Cursor navigation: up, down, forward, backward, absolute positioning, save/restore
//! - SGR graphic rendition: bold, dim, underline, reverse, 16-color, 256-color palette,
//!   and full 24-bit RGB TrueColor (`CSI 38;2;r;g;b m` / `CSI 48;2;r;g;b m`)
//! - Text grid buffer management, line clearing, and scrolling

use alloc::vec;
use alloc::vec::Vec;
use crate::display::color::Color;

/// Standard 16 ANSI colors (0..7 normal, 8..15 bright).
pub const ANSI_16_COLORS: [Color; 16] = [
    Color::rgb(0, 0, 0),       // 0: Black
    Color::rgb(205, 0, 0),     // 1: Red
    Color::rgb(0, 205, 0),     // 2: Green
    Color::rgb(205, 205, 0),   // 3: Yellow
    Color::rgb(0, 0, 238),     // 4: Blue
    Color::rgb(205, 0, 205),   // 5: Magenta
    Color::rgb(0, 205, 205),   // 6: Cyan
    Color::rgb(229, 229, 229), // 7: White
    Color::rgb(127, 127, 127), // 8: Bright Black
    Color::rgb(255, 0, 0),     // 9: Bright Red
    Color::rgb(0, 255, 0),     // 10: Bright Green
    Color::rgb(255, 255, 0),   // 11: Bright Yellow
    Color::rgb(92, 92, 255),   // 12: Bright Blue
    Color::rgb(255, 0, 255),   // 13: Bright Magenta
    Color::rgb(0, 255, 255),   // 14: Bright Cyan
    Color::rgb(255, 255, 255), // 15: Bright White
];

/// Converts an xterm 256-color palette index to 24-bit RGB Color.
pub fn xterm_256_to_color(idx: u8) -> Color {
    if (idx as usize) < 16 {
        return ANSI_16_COLORS[idx as usize];
    }
    if idx >= 16 && idx <= 231 {
        let n = idx - 16;
        let r = (n / 36) * 51;
        let g = ((n % 36) / 6) * 51;
        let b = (n % 6) * 51;
        return Color::rgb(r, g, b);
    }
    // 232..=255: 24 grayscale steps
    let gray = 8 + (idx - 232) * 10;
    Color::rgb(gray, gray, gray)
}

/// Active text rendition style.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AnsiStyle {
    pub fg: Color,
    pub bg: Color,
    pub bold: bool,
    pub dim: bool,
    pub underline: bool,
    pub reverse: bool,
    pub blink: bool,
    pub hidden: bool,
}

impl AnsiStyle {
    pub const fn default_style(fg: Color, bg: Color) -> Self {
        Self {
            fg,
            bg,
            bold: false,
            dim: false,
            underline: false,
            reverse: false,
            blink: false,
            hidden: false,
        }
    }

    /// Returns effective foreground and background colors accounting for reverse video.
    pub fn effective_colors(&self) -> (Color, Color) {
        if self.reverse {
            (self.bg, self.fg)
        } else {
            (self.fg, self.bg)
        }
    }
}

/// Character cell in the virtual terminal 2D grid.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AnsiCell {
    pub ch: char,
    pub style: AnsiStyle,
}

impl AnsiCell {
    pub const fn empty(style: AnsiStyle) -> Self {
        Self { ch: ' ', style }
    }
}

/// Terminal cursor state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AnsiCursor {
    pub col: usize, // 0-indexed column
    pub row: usize, // 0-indexed row
    pub visible: bool,
}

impl AnsiCursor {
    pub const fn new(col: usize, row: usize) -> Self {
        Self {
            col,
            row,
            visible: true,
        }
    }
}

/// Internal states of the ANSI escape parser.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ParserState {
    Ground,
    Escape,
    Csi,
    Osc,
}

/// High-performance 2D terminal grid emulator with full ANSI/VT100/xterm and 24-bit TrueColor support.
pub struct AnsiTerminal {
    pub cols: usize,
    pub rows: usize,
    cells: Vec<AnsiCell>,
    pub cursor: AnsiCursor,
    saved_cursor: Option<AnsiCursor>,
    pub current_style: AnsiStyle,
    pub default_fg: Color,
    pub default_bg: Color,
    state: ParserState,
    param_buf: Vec<u8>,
    is_private_csi: bool,
}

impl AnsiTerminal {
    /// Creates a new terminal emulator grid with default dimensions (e.g. 80x25).
    pub fn new(cols: usize, rows: usize) -> Self {
        let default_fg = Color::rgb(240, 244, 250);
        let default_bg = Color::rgb(12, 12, 12);
        let current_style = AnsiStyle::default_style(default_fg, default_bg);
        let total = cols * rows;
        let cells = vec![AnsiCell::empty(current_style); total];

        Self {
            cols,
            rows,
            cells,
            cursor: AnsiCursor::new(0, 0),
            saved_cursor: None,
            current_style,
            default_fg,
            default_bg,
            state: ParserState::Ground,
            param_buf: Vec::with_capacity(64),
            is_private_csi: false,
        }
    }

    #[inline(always)]
    fn index_of(&self, col: usize, row: usize) -> usize {
        row * self.cols + col
    }

    /// Retrieves the cell at the given column and row.
    pub fn cell(&self, col: usize, row: usize) -> Option<&AnsiCell> {
        if col < self.cols && row < self.rows {
            Some(&self.cells[self.index_of(col, row)])
        } else {
            None
        }
    }

    /// Resizes the terminal grid to new dimensions.
    pub fn resize(&mut self, new_cols: usize, new_rows: usize) {
        if new_cols == self.cols && new_rows == self.rows {
            return;
        }
        let total = new_cols * new_rows;
        let mut new_cells = vec![AnsiCell::empty(self.current_style); total];

        let min_cols = core::cmp::min(self.cols, new_cols);
        let min_rows = core::cmp::min(self.rows, new_rows);

        for r in 0..min_rows {
            for c in 0..min_cols {
                new_cells[r * new_cols + c] = self.cells[self.index_of(c, r)];
            }
        }

        self.cols = new_cols;
        self.rows = new_rows;
        self.cells = new_cells;

        if self.cursor.col >= self.cols {
            self.cursor.col = self.cols.saturating_sub(1);
        }
        if self.cursor.row >= self.rows {
            self.cursor.row = self.rows.saturating_sub(1);
        }
    }

    /// Processes a stream of input bytes through the ANSI state machine.
    pub fn process_bytes(&mut self, data: &[u8]) {
        for &byte in data {
            match self.state {
                ParserState::Ground => self.handle_ground(byte),
                ParserState::Escape => self.handle_escape(byte),
                ParserState::Csi => self.handle_csi(byte),
                ParserState::Osc => self.handle_osc(byte),
            }
        }
    }

    fn handle_ground(&mut self, byte: u8) {
        match byte {
            0x1B => {
                // ESC
                self.state = ParserState::Escape;
            }
            b'\r' => {
                self.cursor.col = 0;
            }
            b'\n' => {
                self.new_line();
            }
            0x08 => {
                // Backspace
                if self.cursor.col > 0 {
                    self.cursor.col -= 1;
                }
            }
            b'\t' => {
                // Tab stops at multiples of 8
                self.cursor.col = (self.cursor.col + 8) & !7;
                if self.cursor.col >= self.cols {
                    self.cursor.col = 0;
                    self.new_line();
                }
            }
            0x07 => {
                // BEL (no-op or bell)
            }
            0x20..=0x7E => {
                // Printable ASCII
                let ch = byte as char;
                self.put_char(ch);
            }
            _ => {
                // Ignore other control codes
            }
        }
    }

    fn handle_escape(&mut self, byte: u8) {
        match byte {
            b'[' => {
                // CSI
                self.state = ParserState::Csi;
                self.param_buf.clear();
                self.is_private_csi = false;
            }
            b']' => {
                // OSC
                self.state = ParserState::Osc;
            }
            b'7' => {
                // Save cursor (DECSC)
                self.saved_cursor = Some(self.cursor);
                self.state = ParserState::Ground;
            }
            b'8' => {
                // Restore cursor (DECRC)
                if let Some(sc) = self.saved_cursor {
                    self.cursor = sc;
                }
                self.state = ParserState::Ground;
            }
            b'M' => {
                // Reverse Index (scroll down if at top)
                if self.cursor.row > 0 {
                    self.cursor.row -= 1;
                }
                self.state = ParserState::Ground;
            }
            b'c' => {
                // Full reset (RIS)
                self.reset_all();
                self.state = ParserState::Ground;
            }
            _ => {
                self.state = ParserState::Ground;
            }
        }
    }

    fn handle_csi(&mut self, byte: u8) {
        if self.param_buf.is_empty() && byte == b'?' {
            self.is_private_csi = true;
            return;
        }

        match byte {
            b'0'..=b'9' | b';' => {
                if self.param_buf.len() < 64 {
                    self.param_buf.push(byte);
                }
            }
            cmd @ (b'A'..=b'Z' | b'a'..=b'z') => {
                let params = self.parse_csi_params();
                self.execute_csi(cmd, &params);
                self.state = ParserState::Ground;
            }
            0x1B => {
                // Interrupting ESC
                self.state = ParserState::Escape;
            }
            _ => {
                self.state = ParserState::Ground;
            }
        }
    }

    fn handle_osc(&mut self, byte: u8) {
        // OSC strings terminate on BEL (0x07) or ST (ESC \)
        if byte == 0x07 || byte == 0x1B {
            self.state = ParserState::Ground;
        }
    }

    fn parse_csi_params(&self) -> Vec<u32> {
        let mut params = Vec::new();
        if self.param_buf.is_empty() {
            return params;
        }

        let mut current: u32 = 0;
        let mut has_digit = false;

        for &b in &self.param_buf {
            if b >= b'0' && b <= b'9' {
                current = current.saturating_mul(10).saturating_add((b - b'0') as u32);
                has_digit = true;
            } else if b == b';' {
                params.push(if has_digit { current } else { 0 });
                current = 0;
                has_digit = false;
            }
        }
        if has_digit || !params.is_empty() {
            params.push(current);
        }

        params
    }

    fn execute_csi(&mut self, cmd: u8, params: &[u32]) {
        let p1 = params.get(0).copied().unwrap_or(0);
        let p2 = params.get(1).copied().unwrap_or(0);

        if self.is_private_csi {
            // Private CSI (e.g. ?25h / ?25l cursor visibility)
            if p1 == 25 {
                if cmd == b'h' {
                    self.cursor.visible = true;
                } else if cmd == b'l' {
                    self.cursor.visible = false;
                }
            }
            return;
        }

        match cmd {
            b'A' => {
                // Cursor Up
                let count = if p1 == 0 { 1 } else { p1 as usize };
                self.cursor.row = self.cursor.row.saturating_sub(count);
            }
            b'B' => {
                // Cursor Down
                let count = if p1 == 0 { 1 } else { p1 as usize };
                self.cursor.row = core::cmp::min(self.rows - 1, self.cursor.row + count);
            }
            b'C' => {
                // Cursor Forward
                let count = if p1 == 0 { 1 } else { p1 as usize };
                self.cursor.col = core::cmp::min(self.cols - 1, self.cursor.col + count);
            }
            b'D' => {
                // Cursor Backward
                let count = if p1 == 0 { 1 } else { p1 as usize };
                self.cursor.col = self.cursor.col.saturating_sub(count);
            }
            b'E' => {
                // Cursor Next Line
                let count = if p1 == 0 { 1 } else { p1 as usize };
                self.cursor.col = 0;
                self.cursor.row = core::cmp::min(self.rows - 1, self.cursor.row + count);
            }
            b'F' => {
                // Cursor Previous Line
                let count = if p1 == 0 { 1 } else { p1 as usize };
                self.cursor.col = 0;
                self.cursor.row = self.cursor.row.saturating_sub(count);
            }
            b'G' => {
                // Cursor Horizontal Absolute (1-indexed)
                let target = if p1 == 0 { 0 } else { (p1 - 1) as usize };
                self.cursor.col = core::cmp::min(self.cols - 1, target);
            }
            b'H' | b'f' => {
                // Cursor Position (row, col - 1-indexed)
                let r = if p1 == 0 { 0 } else { (p1 - 1) as usize };
                let c = if p2 == 0 { 0 } else { (p2 - 1) as usize };
                self.cursor.row = core::cmp::min(self.rows - 1, r);
                self.cursor.col = core::cmp::min(self.cols - 1, c);
            }
            b'J' => {
                // Erase in Display
                self.erase_in_display(p1);
            }
            b'K' => {
                // Erase in Line
                self.erase_in_line(p1);
            }
            b'm' => {
                // Select Graphic Rendition (SGR)
                self.execute_sgr(params);
            }
            b's' => {
                // Save Cursor
                self.saved_cursor = Some(self.cursor);
            }
            b'u' => {
                // Restore Cursor
                if let Some(sc) = self.saved_cursor {
                    self.cursor = sc;
                }
            }
            _ => {}
        }
    }

    /// Select Graphic Rendition (SGR) state decoder supporting 16-color, 256-color, and 24-bit TrueColor.
    fn execute_sgr(&mut self, params: &[u32]) {
        if params.is_empty() {
            self.current_style = AnsiStyle::default_style(self.default_fg, self.default_bg);
            return;
        }

        let mut idx = 0;
        while idx < params.len() {
            let code = params[idx];
            match code {
                0 => {
                    self.current_style = AnsiStyle::default_style(self.default_fg, self.default_bg);
                }
                1 => self.current_style.bold = true,
                2 => self.current_style.dim = true,
                4 => self.current_style.underline = true,
                5 => self.current_style.blink = true,
                7 => self.current_style.reverse = true,
                8 => self.current_style.hidden = true,
                22 => {
                    self.current_style.bold = false;
                    self.current_style.dim = false;
                }
                24 => self.current_style.underline = false,
                25 => self.current_style.blink = false,
                27 => self.current_style.reverse = false,
                28 => self.current_style.hidden = false,

                // Standard 16 colors: Foreground
                30..=37 => {
                    self.current_style.fg = ANSI_16_COLORS[(code - 30) as usize];
                }
                39 => {
                    self.current_style.fg = self.default_fg;
                }

                // Standard 16 colors: Background
                40..=47 => {
                    self.current_style.bg = ANSI_16_COLORS[(code - 40) as usize];
                }
                49 => {
                    self.current_style.bg = self.default_bg;
                }

                // Bright 16 colors: Foreground
                90..=97 => {
                    self.current_style.fg = ANSI_16_COLORS[(code - 90 + 8) as usize];
                }

                // Bright 16 colors: Background
                100..=107 => {
                    self.current_style.bg = ANSI_16_COLORS[(code - 100 + 8) as usize];
                }

                // Extended Foreground Color (256-color or 24-bit TrueColor)
                38 => {
                    if idx + 1 < params.len() {
                        let mode = params[idx + 1];
                        if mode == 5 && idx + 2 < params.len() {
                            // 256-color: CSI 38;5;n m
                            let color_idx = params[idx + 2] as u8;
                            self.current_style.fg = xterm_256_to_color(color_idx);
                            idx += 2;
                        } else if mode == 2 && idx + 4 < params.len() {
                            // 24-bit TrueColor: CSI 38;2;r;g;b m
                            let r = (params[idx + 2] & 0xFF) as u8;
                            let g = (params[idx + 3] & 0xFF) as u8;
                            let b = (params[idx + 4] & 0xFF) as u8;
                            self.current_style.fg = Color::rgb(r, g, b);
                            idx += 4;
                        }
                    }
                }

                // Extended Background Color (256-color or 24-bit TrueColor)
                48 => {
                    if idx + 1 < params.len() {
                        let mode = params[idx + 1];
                        if mode == 5 && idx + 2 < params.len() {
                            // 256-color: CSI 48;5;n m
                            let color_idx = params[idx + 2] as u8;
                            self.current_style.bg = xterm_256_to_color(color_idx);
                            idx += 2;
                        } else if mode == 2 && idx + 4 < params.len() {
                            // 24-bit TrueColor: CSI 48;2;r;g;b m
                            let r = (params[idx + 2] & 0xFF) as u8;
                            let g = (params[idx + 3] & 0xFF) as u8;
                            let b = (params[idx + 4] & 0xFF) as u8;
                            self.current_style.bg = Color::rgb(r, g, b);
                            idx += 4;
                        }
                    }
                }

                _ => {}
            }
            idx += 1;
        }
    }

    fn erase_in_display(&mut self, mode: u32) {
        let empty = AnsiCell::empty(self.current_style);
        match mode {
            0 => {
                // Cursor to end of screen
                let start = self.index_of(self.cursor.col, self.cursor.row);
                for cell in &mut self.cells[start..] {
                    *cell = empty;
                }
            }
            1 => {
                // Start of screen to cursor
                let end = self.index_of(self.cursor.col, self.cursor.row);
                for cell in &mut self.cells[..=end] {
                    *cell = empty;
                }
            }
            2 | 3 => {
                // Clear entire screen
                for cell in &mut self.cells {
                    *cell = empty;
                }
            }
            _ => {}
        }
    }

    fn erase_in_line(&mut self, mode: u32) {
        let empty = AnsiCell::empty(self.current_style);
        let row_start = self.index_of(0, self.cursor.row);
        let row_end = self.index_of(self.cols - 1, self.cursor.row);

        match mode {
            0 => {
                // Cursor to end of line
                let start = self.index_of(self.cursor.col, self.cursor.row);
                for cell in &mut self.cells[start..=row_end] {
                    *cell = empty;
                }
            }
            1 => {
                // Start of line to cursor
                let end = self.index_of(self.cursor.col, self.cursor.row);
                for cell in &mut self.cells[row_start..=end] {
                    *cell = empty;
                }
            }
            2 => {
                // Entire line
                for cell in &mut self.cells[row_start..=row_end] {
                    *cell = empty;
                }
            }
            _ => {}
        }
    }

    fn put_char(&mut self, ch: char) {
        if self.cursor.col >= self.cols {
            self.cursor.col = 0;
            self.new_line();
        }

        let idx = self.index_of(self.cursor.col, self.cursor.row);
        self.cells[idx] = AnsiCell {
            ch,
            style: self.current_style,
        };
        self.cursor.col += 1;
    }

    fn new_line(&mut self) {
        if self.cursor.row + 1 < self.rows {
            self.cursor.row += 1;
        } else {
            self.scroll_up();
        }
    }

    pub fn scroll_up(&mut self) {
        let empty = AnsiCell::empty(self.current_style);
        // Shift rows up by 1
        for r in 0..(self.rows - 1) {
            let dst_start = r * self.cols;
            let src_start = (r + 1) * self.cols;
            for c in 0..self.cols {
                self.cells[dst_start + c] = self.cells[src_start + c];
            }
        }
        // Blank last row
        let last_start = (self.rows - 1) * self.cols;
        for c in 0..self.cols {
            self.cells[last_start + c] = empty;
        }
    }

    pub fn reset_all(&mut self) {
        self.cursor = AnsiCursor::new(0, 0);
        self.saved_cursor = None;
        self.current_style = AnsiStyle::default_style(self.default_fg, self.default_bg);
        let empty = AnsiCell::empty(self.current_style);
        for cell in &mut self.cells {
            *cell = empty;
        }
    }
}
