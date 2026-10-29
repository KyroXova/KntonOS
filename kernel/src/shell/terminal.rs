//! Virtual Terminal text grid buffer with ANSI escape code color support, cursor blink, and scrolling.

use alloc::string::String;
use alloc::vec::Vec;
use crate::display::color::Color;
use crate::display::font::{get_glyph, FONT_HEIGHT, FONT_WIDTH};

pub const DEFAULT_COLS: usize = 80;
pub const DEFAULT_ROWS: usize = 25;

pub const ANSI_COLOR_BLACK: Color = Color::rgb(12, 12, 12);
pub const ANSI_COLOR_RED: Color = Color::rgb(197, 15, 31);
pub const ANSI_COLOR_GREEN: Color = Color::rgb(19, 161, 14);
pub const ANSI_COLOR_YELLOW: Color = Color::rgb(193, 156, 0);
pub const ANSI_COLOR_BLUE: Color = Color::rgb(0, 55, 218);
pub const ANSI_COLOR_MAGENTA: Color = Color::rgb(136, 23, 152);
pub const ANSI_COLOR_CYAN: Color = Color::rgb(58, 150, 221);
pub const ANSI_COLOR_WHITE: Color = Color::rgb(204, 204, 204);

pub const ANSI_COLOR_BRIGHT_BLACK: Color = Color::rgb(118, 118, 118);
pub const ANSI_COLOR_BRIGHT_RED: Color = Color::rgb(231, 72, 86);
pub const ANSI_COLOR_BRIGHT_GREEN: Color = Color::rgb(22, 198, 12);
pub const ANSI_COLOR_BRIGHT_YELLOW: Color = Color::rgb(249, 241, 165);
pub const ANSI_COLOR_BRIGHT_BLUE: Color = Color::rgb(59, 120, 255);
pub const ANSI_COLOR_BRIGHT_MAGENTA: Color = Color::rgb(180, 0, 158);
pub const ANSI_COLOR_BRIGHT_CYAN: Color = Color::rgb(97, 214, 214);
pub const ANSI_COLOR_BRIGHT_WHITE: Color = Color::rgb(242, 242, 242);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TerminalCell {
    pub character: char,
    pub fg: Color,
    pub bg: Color,
}

impl TerminalCell {
    pub const fn empty() -> Self {
        Self {
            character: ' ',
            fg: Color::WIN11_TEXT_PRIMARY,
            bg: Color::BLACK,
        }
    }

    pub const fn new(character: char, fg: Color, bg: Color) -> Self {
        Self { character, fg, bg }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AnsiState {
    Normal,
    Esc,
    Csi,
}

/// 2D text grid terminal emulator buffer with ANSI escape parsing.
pub struct TerminalBuffer {
    pub cols: usize,
    pub rows: usize,
    cells: Vec<TerminalCell>,
    pub cursor_col: usize,
    pub cursor_row: usize,
    pub cursor_visible: bool,
    pub cursor_blink_on: bool,
    current_fg: Color,
    current_bg: Color,
    default_fg: Color,
    default_bg: Color,
    ansi_state: AnsiState,
    ansi_param_buf: String,
}

impl TerminalBuffer {
    pub fn new(cols: usize, rows: usize) -> Self {
        let total = cols * rows;
        let mut cells = Vec::with_capacity(total);
        let default_fg = Color::WIN11_TEXT_PRIMARY;
        let default_bg = Color::BLACK;
        cells.resize(total, TerminalCell::new(' ', default_fg, default_bg));

        Self {
            cols,
            rows,
            cells,
            cursor_col: 0,
            cursor_row: 0,
            cursor_visible: true,
            cursor_blink_on: true,
            current_fg: default_fg,
            current_bg: default_bg,
            default_fg,
            default_bg,
            ansi_state: AnsiState::Normal,
            ansi_param_buf: String::new(),
        }
    }

    #[inline(always)]
    pub fn cell_index(&self, col: usize, row: usize) -> usize {
        row * self.cols + col
    }

    pub fn get_cell(&self, col: usize, row: usize) -> Option<TerminalCell> {
        if col < self.cols && row < self.rows {
            Some(self.cells[self.cell_index(col, row)])
        } else {
            None
        }
    }

    pub fn set_cell(&mut self, col: usize, row: usize, cell: TerminalCell) {
        if col < self.cols && row < self.rows {
            let idx = self.cell_index(col, row);
            self.cells[idx] = cell;
        }
    }

    pub fn clear(&mut self) {
        let empty = TerminalCell::new(' ', self.current_fg, self.current_bg);
        for cell in self.cells.iter_mut() {
            *cell = empty;
        }
        self.cursor_col = 0;
        self.cursor_row = 0;
    }

    pub fn toggle_cursor_blink(&mut self) {
        self.cursor_blink_on = !self.cursor_blink_on;
    }

    pub fn scroll_up(&mut self, lines: usize) {
        if lines == 0 {
            return;
        }
        if lines >= self.rows {
            self.clear();
            return;
        }

        let cells_to_move = (self.rows - lines) * self.cols;
        let offset = lines * self.cols;

        for i in 0..cells_to_move {
            self.cells[i] = self.cells[i + offset];
        }

        let empty = TerminalCell::new(' ', self.current_fg, self.current_bg);
        for i in cells_to_move..self.cells.len() {
            self.cells[i] = empty;
        }

        self.cursor_row = self.cursor_row.saturating_sub(lines);
    }

    /// Writes a single character to the terminal grid, interpreting ANSI escape sequences.
    pub fn write_char(&mut self, c: char) {
        match self.ansi_state {
            AnsiState::Normal => {
                if c == '\x1B' {
                    self.ansi_state = AnsiState::Esc;
                } else {
                    self.put_printable_char(c);
                }
            }
            AnsiState::Esc => {
                if c == '[' {
                    self.ansi_state = AnsiState::Csi;
                    self.ansi_param_buf.clear();
                } else {
                    self.ansi_state = AnsiState::Normal;
                    self.put_printable_char(c);
                }
            }
            AnsiState::Csi => {
                if c.is_ascii_digit() || c == ';' {
                    self.ansi_param_buf.push(c);
                } else {
                    self.execute_csi_command(c);
                    self.ansi_state = AnsiState::Normal;
                }
            }
        }
    }

    pub fn write_str(&mut self, s: &str) {
        for c in s.chars() {
            self.write_char(c);
        }
    }

    fn put_printable_char(&mut self, c: char) {
        match c {
            '\n' => {
                self.cursor_col = 0;
                self.cursor_row += 1;
                if self.cursor_row >= self.rows {
                    self.scroll_up(1);
                    self.cursor_row = self.rows - 1;
                }
            }
            '\r' => {
                self.cursor_col = 0;
            }
            '\t' => {
                let next_tab = (self.cursor_col + 4) & !3;
                while self.cursor_col < next_tab && self.cursor_col < self.cols {
                    self.put_char_at_cursor(' ');
                }
            }
            '\x08' => {
                // Backspace
                if self.cursor_col > 0 {
                    self.cursor_col -= 1;
                    let idx = self.cell_index(self.cursor_col, self.cursor_row);
                    self.cells[idx] = TerminalCell::new(' ', self.current_fg, self.current_bg);
                }
            }
            printable => {
                self.put_char_at_cursor(printable);
            }
        }
    }

    fn put_char_at_cursor(&mut self, c: char) {
        if self.cursor_col >= self.cols {
            self.cursor_col = 0;
            self.cursor_row += 1;
            if self.cursor_row >= self.rows {
                self.scroll_up(1);
                self.cursor_row = self.rows - 1;
            }
        }

        let idx = self.cell_index(self.cursor_col, self.cursor_row);
        self.cells[idx] = TerminalCell::new(c, self.current_fg, self.current_bg);
        self.cursor_col += 1;
    }

    fn execute_csi_command(&mut self, cmd: char) {
        match cmd {
            'm' => {
                // SGR (Select Graphic Rendition)
                if self.ansi_param_buf.is_empty() {
                    self.current_fg = self.default_fg;
                    self.current_bg = self.default_bg;
                    return;
                }

                for param in self.ansi_param_buf.split(';') {
                    let code: usize = param.parse().unwrap_or(0);
                    match code {
                        0 => {
                            self.current_fg = self.default_fg;
                            self.current_bg = self.default_bg;
                        }
                        30 => self.current_fg = ANSI_COLOR_BLACK,
                        31 => self.current_fg = ANSI_COLOR_RED,
                        32 => self.current_fg = ANSI_COLOR_GREEN,
                        33 => self.current_fg = ANSI_COLOR_YELLOW,
                        34 => self.current_fg = ANSI_COLOR_BLUE,
                        35 => self.current_fg = ANSI_COLOR_MAGENTA,
                        36 => self.current_fg = ANSI_COLOR_CYAN,
                        37 => self.current_fg = ANSI_COLOR_WHITE,
                        39 => self.current_fg = self.default_fg,
                        40 => self.current_bg = ANSI_COLOR_BLACK,
                        41 => self.current_bg = ANSI_COLOR_RED,
                        42 => self.current_bg = ANSI_COLOR_GREEN,
                        43 => self.current_bg = ANSI_COLOR_YELLOW,
                        44 => self.current_bg = ANSI_COLOR_BLUE,
                        45 => self.current_bg = ANSI_COLOR_MAGENTA,
                        46 => self.current_bg = ANSI_COLOR_CYAN,
                        47 => self.current_bg = ANSI_COLOR_WHITE,
                        49 => self.current_bg = self.default_bg,
                        90 => self.current_fg = ANSI_COLOR_BRIGHT_BLACK,
                        91 => self.current_fg = ANSI_COLOR_BRIGHT_RED,
                        92 => self.current_fg = ANSI_COLOR_BRIGHT_GREEN,
                        93 => self.current_fg = ANSI_COLOR_BRIGHT_YELLOW,
                        94 => self.current_fg = ANSI_COLOR_BRIGHT_BLUE,
                        95 => self.current_fg = ANSI_COLOR_BRIGHT_MAGENTA,
                        96 => self.current_fg = ANSI_COLOR_BRIGHT_CYAN,
                        97 => self.current_fg = ANSI_COLOR_BRIGHT_WHITE,
                        _ => {}
                    }
                }
            }
            'J' => {
                // Clear screen
                self.clear();
            }
            'H' | 'f' => {
                // Cursor position
                let mut parts = self.ansi_param_buf.split(';');
                let r: usize = parts.next().and_then(|p| p.parse().ok()).unwrap_or(1);
                let c: usize = parts.next().and_then(|p| p.parse().ok()).unwrap_or(1);
                self.cursor_row = (r.saturating_sub(1)).min(self.rows.saturating_sub(1));
                self.cursor_col = (c.saturating_sub(1)).min(self.cols.saturating_sub(1));
            }
            _ => {}
        }
    }

    /// Renders the text grid buffer into a pixel surface (client buffer) using bitmap font glyphs.
    pub fn render_to_client_buffer(&self, dest: &mut [Color], dest_w: usize, dest_h: usize) {
        for row in 0..self.rows {
            let py = row * FONT_HEIGHT;
            if py >= dest_h {
                break;
            }

            for col in 0..self.cols {
                let px = col * FONT_WIDTH;
                if px >= dest_w {
                    break;
                }

                let cell = self.cells[self.cell_index(col, row)];
                let is_cursor = self.cursor_visible
                    && self.cursor_blink_on
                    && col == self.cursor_col
                    && row == self.cursor_row;

                let fg = if is_cursor { cell.bg } else { cell.fg };
                let bg = if is_cursor { Color::WIN11_ACCENT } else { cell.bg };

                let glyph = get_glyph(cell.character);

                for gy in 0..FONT_HEIGHT {
                    let ty = py + gy;
                    if ty >= dest_h {
                        break;
                    }
                    let bits = glyph[gy];
                    for gx in 0..FONT_WIDTH {
                        let tx = px + gx;
                        if tx >= dest_w {
                            break;
                        }
                        let is_set = (bits & (0x80 >> gx)) != 0;
                        let pixel = if is_cursor {
                            // Draw block cursor or underline cursor
                            if gy >= FONT_HEIGHT - 3 {
                                Color::WIN11_ACCENT
                            } else if is_set {
                                fg
                            } else {
                                bg
                            }
                        } else if is_set {
                            fg
                        } else {
                            bg
                        };

                        dest[ty * dest_w + tx] = pixel;
                    }
                }
            }
        }
    }
}
