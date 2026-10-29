//! 2D primitive rendering operations on LinearFramebuffer.

use super::color::Color;
use super::font::{get_glyph, FONT_HEIGHT, FONT_WIDTH};
use super::framebuffer::LinearFramebuffer;

pub fn draw_rect(fb: &mut LinearFramebuffer, x: usize, y: usize, w: usize, h: usize, color: Color) {
    for row in 0..h {
        let cur_y = y + row;
        if cur_y >= fb.height() {
            break;
        }
        for col in 0..w {
            let cur_x = x + col;
            if cur_x >= fb.width() {
                break;
            }
            fb.put_pixel(cur_x, cur_y, color);
        }
    }
}

pub fn draw_rect_outline(
    fb: &mut LinearFramebuffer,
    x: usize,
    y: usize,
    w: usize,
    h: usize,
    border_width: usize,
    color: Color,
) {
    // Top and bottom borders
    draw_rect(fb, x, y, w, border_width, color);
    if h > border_width {
        draw_rect(fb, x, y + h - border_width, w, border_width, color);
    }
    // Left and right borders
    draw_rect(fb, x, y, border_width, h, color);
    if w > border_width {
        draw_rect(fb, x + w - border_width, y, border_width, h, color);
    }
}

pub fn draw_char(fb: &mut LinearFramebuffer, x: usize, y: usize, c: char, fg: Color, bg: Option<Color>) {
    let glyph = get_glyph(c);
    for row in 0..FONT_HEIGHT {
        let cur_y = y + row;
        if cur_y >= fb.height() {
            break;
        }
        let row_bits = glyph[row];
        for col in 0..FONT_WIDTH {
            let cur_x = x + col;
            if cur_x >= fb.width() {
                break;
            }
            let is_set = (row_bits & (0x80 >> col)) != 0;
            if is_set {
                fb.put_pixel(cur_x, cur_y, fg);
            } else if let Some(bg_color) = bg {
                fb.put_pixel(cur_x, cur_y, bg_color);
            }
        }
    }
}

pub fn draw_string(fb: &mut LinearFramebuffer, x: usize, y: usize, s: &str, fg: Color, bg: Option<Color>) {
    let mut cur_x = x;
    for c in s.chars() {
        if c == '\n' {
            continue;
        }
        draw_char(fb, cur_x, y, c, fg, bg);
        cur_x += FONT_WIDTH;
        if cur_x + FONT_WIDTH > fb.width() {
            break;
        }
    }
}
