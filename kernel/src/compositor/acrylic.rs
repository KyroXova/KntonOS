//! Acrylic Glass Shader Simulation & Frosted Backdrop Rasterizer for KntonOS.
//! Implements a fast 3x3 box blur kernel, 60% alpha tinted translucent overlay,

use crate::display::color::Color;
use super::dirty::Rect;

/// Configuration parameters for the Acrylic glass rasterizer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AcrylicConfig {
    pub blur_radius: usize,
    pub tint_color: Color,
    pub noise_alpha: u8,
}

impl AcrylicConfig {
    /// Default dark acrylic preset with 60% alpha tint and subtle noise grain.
    pub const fn default_dark() -> Self {
        Self {
            blur_radius: 1,
            tint_color: Color::rgba(32, 38, 48, 153), // 60% alpha
            noise_alpha: 0x20,                         // 32/255 subtle noise
        }
    }

    /// Titlebar acrylic preset for window headers.
    pub const fn titlebar(is_focused: bool) -> Self {
        if is_focused {
            Self {
                blur_radius: 1,
                tint_color: Color::rgba(36, 44, 58, 180),
                noise_alpha: 0x18,
            }
        } else {
            Self {
                blur_radius: 1,
                tint_color: Color::rgba(28, 32, 42, 180),
                noise_alpha: 0x10,
            }
        }
    }

    /// Light acrylic preset.
    pub const fn light() -> Self {
        Self {
            blur_radius: 1,
            tint_color: Color::rgba(240, 243, 249, 160),
            noise_alpha: 0x18,
        }
    }
}

/// Deterministic per-pixel noise generator for acrylic frosted glass texture.
#[inline(always)]
pub fn pseudo_noise(x: usize, y: usize) -> u8 {
    let mut h = (x as u32).wrapping_mul(374761393).wrapping_add((y as u32).wrapping_mul(668265263));
    h = (h ^ (h >> 13)).wrapping_mul(1274126177);
    (h & 0x1F) as u8
}

/// Acrylic glass backdrop surface renderer.
pub struct AcrylicSurface;

impl AcrylicSurface {
    /// Apply a fast 3x3 box blur to the given region of the source buffer into the target.
    pub fn box_blur_3x3(
        source: &[Color],
        target: &mut [Color],
        width: usize,
        height: usize,
        region: &Rect,
        clip: Option<&Rect>,
    ) {
        if region.is_empty() || width == 0 || height == 0 {
            return;
        }

        let start_x = region.x;
        let end_x = (region.x + region.width).min(width);
        let start_y = region.y;
        let end_y = (region.y + region.height).min(height);

        for y in start_y..end_y {
            for x in start_x..end_x {
                if let Some(c) = clip {
                    if !c.contains_point(x, y) {
                        continue;
                    }
                }

                let mut sum_r = 0u32;
                let mut sum_g = 0u32;
                let mut sum_b = 0u32;
                let mut count = 0u32;

                for dy in 0..=2 {
                    let py = (y + dy).saturating_sub(1).min(height - 1);
                    let row_offset = py * width;
                    for dx in 0..=2 {
                        let px = (x + dx).saturating_sub(1).min(width - 1);
                        let p = source[row_offset + px];
                        sum_r += p.r as u32;
                        sum_g += p.g as u32;
                        sum_b += p.b as u32;
                        count += 1;
                    }
                }

                let blurred = Color::rgb(
                    (sum_r / count) as u8,
                    (sum_g / count) as u8,
                    (sum_b / count) as u8,
                );

                target[y * width + x] = blurred;
            }
        }
    }

    /// Render a full Acrylic glass frosted surface over the background into `target`.
    /// Applies 3x3 blur, tinted alpha blend, and subtle noise grain texture.
    pub fn render_acrylic(
        source: &[Color],
        target: &mut [Color],
        width: usize,
        height: usize,
        bounds: &Rect,
        config: &AcrylicConfig,
        clip: Option<&Rect>,
    ) {
        if bounds.is_empty() || width == 0 || height == 0 {
            return;
        }

        let start_x = bounds.x;
        let end_x = (bounds.x + bounds.width).min(width);
        let start_y = bounds.y;
        let end_y = (bounds.y + bounds.height).min(height);

        for y in start_y..end_y {
            for x in start_x..end_x {
                if let Some(c) = clip {
                    if !c.contains_point(x, y) {
                        continue;
                    }
                }

                // 1. Dual 3x3 box blur sampling from source background
                let mut sum_r = 0u32;
                let mut sum_g = 0u32;
                let mut sum_b = 0u32;
                let mut count = 0u32;

                for dy in 0..=2 {
                    let py = (y + dy).saturating_sub(1).min(height - 1);
                    let row_offset = py * width;
                    for dx in 0..=2 {
                        let px = (x + dx).saturating_sub(1).min(width - 1);
                        let p = source[row_offset + px];
                        sum_r += p.r as u32;
                        sum_g += p.g as u32;
                        sum_b += p.b as u32;
                        count += 1;
                    }
                }

                let blurred = Color::rgb(
                    (sum_r / count) as u8,
                    (sum_g / count) as u8,
                    (sum_b / count) as u8,
                );

                // 2. Blend tinted acrylic layer (typically 60% alpha) over blurred background
                let tinted = config.tint_color.blend(blurred);

                // 3. Apply subtle frosted noise texture (0x20FFFFFF modulated by per-pixel grain)
                let grain = pseudo_noise(x, y);
                let grain_alpha = ((config.noise_alpha as u32 * (grain as u32 + 16)) / 48) as u8;
                let noise_color = Color::rgba(255, 255, 255, grain_alpha);
                let final_pixel = noise_color.blend(tinted);

                target[y * width + x] = final_pixel;
            }
        }
    }

    /// In-place variant of `render_acrylic` when reading and writing to the same target buffer.
    pub fn render_acrylic_in_place(
        target: &mut [Color],
        width: usize,
        height: usize,
        bounds: &Rect,
        config: &AcrylicConfig,
        clip: Option<&Rect>,
    ) {
        if bounds.is_empty() || width == 0 || height == 0 {
            return;
        }

        let start_x = bounds.x;
        let end_x = (bounds.x + bounds.width).min(width);
        let start_y = bounds.y;
        let end_y = (bounds.y + bounds.height).min(height);
        let actual_w = end_x.saturating_sub(start_x);
        let actual_h = end_y.saturating_sub(start_y);

        if actual_w == 0 || actual_h == 0 {
            return;
        }

        let mut prev_row = alloc::vec::Vec::with_capacity(actual_w);
        let mut curr_row = alloc::vec::Vec::with_capacity(actual_w);

        let row_offset_0 = start_y * width;
        for x in start_x..end_x {
            let p = target[row_offset_0 + x];
            curr_row.push(p);
            prev_row.push(p);
        }

        for y in start_y..end_y {
            let has_next = y + 1 < end_y;
            let next_row_offset = (y + 1) * width;

            for (col_idx, x) in (start_x..end_x).enumerate() {
                if let Some(c) = clip {
                    if !c.contains_point(x, y) {
                        continue;
                    }
                }

                let mut sum_r = 0u32;
                let mut sum_g = 0u32;
                let mut sum_b = 0u32;
                let mut count = 0u32;

                for dy in 0..=2 {
                    for dx in 0..=2 {
                        let px = (col_idx + dx).saturating_sub(1).min(actual_w - 1);
                        let p = match dy {
                            0 => prev_row[px],
                            1 => curr_row[px],
                            _ => {
                                if has_next {
                                    target[next_row_offset + start_x + px]
                                } else {
                                    curr_row[px]
                                }
                            }
                        };
                        sum_r += p.r as u32;
                        sum_g += p.g as u32;
                        sum_b += p.b as u32;
                        count += 1;
                    }
                }

                let blurred = Color::rgb(
                    (sum_r / count) as u8,
                    (sum_g / count) as u8,
                    (sum_b / count) as u8,
                );

                let tinted = config.tint_color.blend(blurred);
                let grain = pseudo_noise(x, y);
                let grain_alpha = ((config.noise_alpha as u32 * (grain as u32 + 16)) / 48) as u8;
                let noise_color = Color::rgba(255, 255, 255, grain_alpha);
                let final_pixel = noise_color.blend(tinted);

                target[y * width + x] = final_pixel;
            }

            prev_row.clear();
            prev_row.extend_from_slice(&curr_row);

            if has_next {
                curr_row.clear();
                for x in start_x..end_x {
                    curr_row.push(target[next_row_offset + x]);
                }
            }
        }
    }
}
