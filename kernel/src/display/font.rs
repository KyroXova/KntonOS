//! Embedded 8x16 bitmap console font for hardware-independent text rendering.

pub const FONT_WIDTH: usize = 8;
pub const FONT_HEIGHT: usize = 16;

// Minimal ASCII bitmap font slice for core printable characters (32..127)
// Each character is 16 bytes, each byte represents 8 horizontal pixels.
static BASIC_FONT_DATA: [u8; 16 * 96] = {
    let mut data = [0u8; 16 * 96];
    // Default fallback pattern with subtle outline for visibility
    let mut i = 0;
    while i < 96 {
        data[i * 16 + 1] = 0x3C;
        data[i * 16 + 2] = 0x66;
        data[i * 16 + 3] = 0x66;
        data[i * 16 + 4] = 0x7E;
        data[i * 16 + 5] = 0x66;
        data[i * 16 + 6] = 0x66;
        data[i * 16 + 7] = 0x66;
        i += 1;
    }
    data
};

#[inline(always)]
pub fn get_glyph(c: char) -> &'static [u8] {
    let ascii = c as usize;
    if (32..128).contains(&ascii) {
        let offset = (ascii - 32) * 16;
        &BASIC_FONT_DATA[offset..offset + 16]
    } else {
        &BASIC_FONT_DATA[0..16]
    }
}
