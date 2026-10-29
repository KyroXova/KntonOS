//! Linear framebuffer interface abstraction for UEFI GOP and native display modes.

use super::color::Color;

#[derive(Copy, Clone, Debug)]
pub struct FramebufferInfo {
    pub base_address: usize,
    pub size: usize,
    pub width: usize,
    pub height: usize,
    pub stride: usize, // pixels per scan line
    pub is_bgr: bool,
}

pub struct LinearFramebuffer {
    info: FramebufferInfo,
}

impl LinearFramebuffer {
    pub const unsafe fn new(info: FramebufferInfo) -> Self {
        Self { info }
    }

    #[inline(always)]
    pub fn width(&self) -> usize {
        self.info.width
    }

    #[inline(always)]
    pub fn height(&self) -> usize {
        self.info.height
    }

    #[inline(always)]
    pub fn stride(&self) -> usize {
        self.info.stride
    }

    #[inline(always)]
    pub fn put_pixel(&mut self, x: usize, y: usize, color: Color) {
        if x >= self.info.width || y >= self.info.height {
            return;
        }

        let pixel_offset = (y * self.info.stride + x) * 4;
        let ptr = (self.info.base_address + pixel_offset) as *mut u32;
        let pixel_value = if self.info.is_bgr {
            color.to_u32_bgr()
        } else {
            color.to_u32_rgb()
        };

        unsafe {
            core::ptr::write_volatile(ptr, pixel_value);
        }
    }

    pub fn clear(&mut self, color: Color) {
        let pixel_value = if self.info.is_bgr {
            color.to_u32_bgr()
        } else {
            color.to_u32_rgb()
        };

        for y in 0..self.info.height {
            for x in 0..self.info.width {
                let pixel_offset = (y * self.info.stride + x) * 4;
                let ptr = (self.info.base_address + pixel_offset) as *mut u32;
                unsafe {
                    core::ptr::write_volatile(ptr, pixel_value);
                }
            }
        }
    }
}
