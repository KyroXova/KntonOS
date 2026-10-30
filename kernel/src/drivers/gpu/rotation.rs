//! Hardware display controller scanout matrix rotation subsystem.

use super::panel::PanelOrientation;

/// 2x3 Affine Coordinate Transformation Matrix for display scanouts.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ScanoutTransformMatrix {
    pub m00: f32,
    pub m01: f32,
    pub tx:  f32,
    pub m10: f32,
    pub m11: f32,
    pub ty:  f32,
}

impl Default for ScanoutTransformMatrix {
    fn default() -> Self {
        Self::IDENTITY
    }
}

impl ScanoutTransformMatrix {
    /// Identity matrix (0 degrees rotation).
    pub const IDENTITY: Self = Self {
        m00: 1.0, m01: 0.0, tx: 0.0,
        m10: 0.0, m11: 1.0, ty: 0.0,
    };

    /// Constructs scanout affine matrix given panel orientation and dimensions.
    pub fn for_orientation(orientation: PanelOrientation, log_w: u32, log_h: u32) -> Self {
        let w = log_w as f32;
        let h = log_h as f32;

        match orientation {
            PanelOrientation::Normal => Self::IDENTITY,
            PanelOrientation::RightUp => {
                // 90 deg clockwise rotation:
                // x_l = y_p
                Self {
                    m00:  0.0, m01: 1.0, tx: 0.0,
                    m10: -1.0, m11: 0.0, ty: (h - 1.0).max(0.0),
                }
            }
            PanelOrientation::BottomUp => {
                // 180 deg upside-down: x_l = (w - 1) - x_p, y_l = (h - 1) - y_p
                Self {
                    m00: -1.0, m01:  0.0, tx: (w - 1.0).max(0.0),
                    m10:  0.0, m11: -1.0, ty: (h - 1.0).max(0.0),
                }
            }
            PanelOrientation::LeftUp => {
                // 270 deg (90 counter-clockwise) rotation:
                // x_l = (w - 1) - y_p
                Self {
                    m00: 0.0, m01: -1.0, tx: (w - 1.0).max(0.0),
                    m10: 1.0, m11:  0.0, ty: 0.0,
                }
            }
        }
    }

    /// Maps physical panel coordinate (phys_x, phys_y) to logical framebuffer coordinate (log_x, log_y).
    #[inline(always)]
    pub fn map_panel_to_logical(&self, phys_x: f32, phys_y: f32) -> (f32, f32) {
        let x_l = self.m00 * phys_x + self.m01 * phys_y + self.tx;
        let y_l = self.m10 * phys_x + self.m11 * phys_y + self.ty;
        (x_l, y_l)
    }

    /// Maps integer physical panel coordinates to integer logical framebuffer coordinates.
    #[inline(always)]
    pub fn map_panel_to_logical_u32(&self, phys_x: u32, phys_y: u32) -> (u32, u32) {
        let (lx, ly) = self.map_panel_to_logical(phys_x as f32, phys_y as f32);
        let ux = if lx < 0.0 { 0 } else { (lx + 0.5) as u32 };
        let uy = if ly < 0.0 { 0 } else { (ly + 0.5) as u32 };
        (ux, uy)
    }
}

/// Hardware display controller scanout DMA configuration.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HardwareScanoutConfig {
    /// Base memory address of the framebuffer.
    pub base_address: u64,
    /// Starting scanout byte offset within the framebuffer for the first pixel (0, 0).
    pub start_offset_bytes: u64,
    /// Logical width of the framebuffer in pixels (e.g. 1280).
    pub logical_width: u32,
    /// Logical height of the framebuffer in pixels (e.g. 800).
    pub logical_height: u32,
    /// Physical width of the panel in pixels (e.g. 800).
    pub physical_width: u32,
    /// Physical height of the panel in pixels (e.g. 1280).
    pub physical_height: u32,
    /// Memory step in bytes per horizontal pixel advance along physical panel row.
    pub pixel_stride_bytes: i32,
    /// Memory step in bytes per vertical line advance along physical panel column.
    pub line_stride_bytes: i32,
    /// Row pitch of the logical framebuffer in bytes.
    pub pitch_bytes: u32,
    /// Bytes per pixel (e.g. 4 for XRGB8888).
    pub bytes_per_pixel: u32,
    /// Display rotation orientation.
    pub orientation: PanelOrientation,
}

impl HardwareScanoutConfig {
    /// Computes the hardware scanout DMA parameters for zero-copy scanout rotation.
    pub fn new(
        base_address: u64,
        log_width: u32,
        log_height: u32,
        pitch_bytes: u32,
        bytes_per_pixel: u32,
        orientation: PanelOrientation,
    ) -> Self {
        let bpp = bytes_per_pixel as i32;
        let pitch = pitch_bytes as i32;

        let (phys_w, phys_h) = if orientation.is_swapped_dimensions() {
            (log_height, log_width)
        } else {
            (log_width, log_height)
        };

        let (start_offset, pixel_stride, line_stride) = match orientation {
            PanelOrientation::Normal => {
                // Top-left origin: advance right (+bpp), advance down (+pitch)
                (0u64, bpp, pitch)
            }
            PanelOrientation::RightUp => {
                // 90 deg clockwise:
                // First physical pixel (0, 0) is at logical (0, log_h - 1)
                let offset = ((log_height.saturating_sub(1)) as u64) * (pitch_bytes as u64);
                (offset, -pitch, bpp)
            }
            PanelOrientation::BottomUp => {
                // 180 deg upside-down:
                // First physical pixel (0, 0) is at logical (log_w - 1, log_h - 1)
                let offset = ((log_height.saturating_sub(1)) as u64) * (pitch_bytes as u64)
                    + ((log_width.saturating_sub(1)) as u64) * (bytes_per_pixel as u64);
                (offset, -bpp, -pitch)
            }
            PanelOrientation::LeftUp => {
                // 270 deg (90 counter-clockwise):
                // First physical pixel (0, 0) is at logical (log_w - 1, 0)
                let offset = ((log_width.saturating_sub(1)) as u64) * (bytes_per_pixel as u64);
                (offset, pitch, -bpp)
            }
        };

        Self {
            base_address,
            start_offset_bytes: start_offset,
            logical_width: log_width,
            logical_height: log_height,
            physical_width: phys_w,
            physical_height: phys_h,
            pixel_stride_bytes: pixel_stride,
            line_stride_bytes: line_stride,
            pitch_bytes,
            bytes_per_pixel,
            orientation,
        }
    }

    /// Computes the exact framebuffer memory offset for a physical display raster coordinate.
    #[inline(always)]
    pub fn calculate_pixel_offset(&self, phys_x: u32, phys_y: u32) -> u64 {
        let x_delta = (phys_x as i64) * (self.pixel_stride_bytes as i64);
        let y_delta = (phys_y as i64) * (self.line_stride_bytes as i64);
        let net_offset = (self.start_offset_bytes as i64) + x_delta + y_delta;
        net_offset as u64
    }

    /// Verifies that all valid physical panel coordinates stay strictly within the allocated framebuffer buffer.
    pub fn verify_bounds(&self, total_buffer_bytes: usize) -> bool {
        if self.physical_width == 0 || self.physical_height == 0 || self.bytes_per_pixel == 0 {
            return false;
        }

        // Test the four corners of the physical panel raster
        let corners = [
            (0, 0),
            (self.physical_width - 1, 0),
            (0, self.physical_height - 1),
            (self.physical_width - 1, self.physical_height - 1),
        ];

        for (px, py) in corners {
            let offset = self.calculate_pixel_offset(px, py);
            if (offset as usize) + (self.bytes_per_pixel as usize) > total_buffer_bytes {
                return false;
            }
        }

        true
    }
}

/// Zero-overhead scanout controller driving hardware scanout DMA with matrix rotation.
pub struct ScanoutRotationController {
    pub config: HardwareScanoutConfig,
    pub matrix: ScanoutTransformMatrix,
}

impl ScanoutRotationController {
    pub fn new(
        base_address: u64,
        log_width: u32,
        log_height: u32,
        pitch_bytes: u32,
        bpp: u32,
        orientation: PanelOrientation,
    ) -> Self {
        let config = HardwareScanoutConfig::new(
            base_address,
            log_width,
            log_height,
            pitch_bytes,
            bpp,
            orientation,
        );
        let matrix = ScanoutTransformMatrix::for_orientation(orientation, log_width, log_height);

        Self { config, matrix }
    }

    /// Reads a 32-bit pixel value from logical framebuffer at physical panel coordinate.
    pub fn fetch_pixel_u32(&self, fb_slice: &[u8], phys_x: u32, phys_y: u32) -> Option<u32> {
        let offset = self.config.calculate_pixel_offset(phys_x, phys_y) as usize;
        if offset + 4 <= fb_slice.len() {
            Some(u32::from_ne_bytes([
                fb_slice[offset],
                fb_slice[offset + 1],
                fb_slice[offset + 2],
                fb_slice[offset + 3],
            ]))
        } else {
            None
        }
    }
}
