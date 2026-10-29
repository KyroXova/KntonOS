//! Sub-pixel Barycentric Triangle Rasterizer with 16-bit Depth Buffering and Flat/Gouraud Shading.
//!
//! Implements fixed-point 28.4 sub-pixel edge functions, edge equation setup,
//! 16-bit Z-buffer depth testing, perspective-correct depth testing, and
//! vertex color attribute interpolation (Flat and Gouraud shading modes).

use alloc::vec::Vec;
use super::types::{Color4, Triangle3D, Vec3};

/// Shading interpolation mode for 3D triangle rasterization.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShadingMode {
    Flat,
    Gouraud,
}

/// 16-bit Depth Buffer (Z-Buffer) for depth testing and hidden surface removal.
/// Values range from 0 (nearest, at near plane) to 0xFFFF (farthest, at far plane).
#[derive(Debug, Clone)]
pub struct DepthBuffer {
    pub width: usize,
    pub height: usize,
    pub buffer: Vec<u16>,
}

impl DepthBuffer {
    /// Allocates a new 16-bit depth buffer initialized to maximum depth (0xFFFF).
    pub fn new(width: usize, height: usize) -> Self {
        let size = width * height;
        let mut buffer = Vec::with_capacity(size);
        buffer.resize(size, 0xFFFF);
        Self { width, height, buffer }
    }

    /// Resets all depth buffer values to maximum depth (0xFFFF).
    pub fn clear(&mut self) {
        self.buffer.fill(0xFFFF);
    }

    /// Tests whether the candidate depth is closer than the current depth buffer value.
    /// If closer, updates the depth buffer and returns `true` (pass); otherwise returns `false` (fail).
    #[inline(always)]
    pub fn test_and_set(&mut self, x: usize, y: usize, depth: u16) -> bool {
        if x < self.width && y < self.height {
            let idx = y * self.width + x;
            if depth < self.buffer[idx] {
                self.buffer[idx] = depth;
                return true;
            }
        }
        false
    }

    /// Returns the current depth value at (x, y).
    #[inline(always)]
    pub fn get(&self, x: usize, y: usize) -> u16 {
        if x < self.width && y < self.height {
            self.buffer[y * self.width + x]
        } else {
            0xFFFF
        }
    }
}

/// 28.4 Fixed-point coordinate conversion helper (16 sub-pixel increments per pixel).
#[inline(always)]
fn to_fixed_28_4(v: f32) -> i32 {
    (v * 16.0) as i32
}

/// Evaluates the 2D cross-product edge function for fixed-point 28.4 points.
#[inline(always)]
fn edge_func(ax: i32, ay: i32, bx: i32, by: i32, cx: i32, cy: i32) -> i64 {
    let acx = (cx - ax) as i64;
    let acy = (cy - ay) as i64;
    let abx = (bx - ax) as i64;
    let aby = (by - ay) as i64;
    acx * aby - acy * abx
}

/// Top-left edge tie-breaking bias rule (Direct3D/OpenGL standard rasterization).
#[inline(always)]
fn is_top_left(ax: i32, ay: i32, bx: i32, by: i32) -> bool {
    let dy = by - ay;
    let dx = bx - ax;
    (dy > 0) || (dy == 0 && dx < 0)
}

/// Software 3D Barycentric Triangle Rasterizer.
pub struct Rasterizer3D {
    pub width: usize,
    pub height: usize,
    pub depth_buffer: DepthBuffer,
    pub shading_mode: ShadingMode,
    pub rasterized_pixel_count: u64,
}

impl Rasterizer3D {
    /// Creates a new rasterizer with an assigned width, height, and depth buffer.
    pub fn new(width: usize, height: usize) -> Self {
        Self {
            width,
            height,
            depth_buffer: DepthBuffer::new(width, height),
            shading_mode: ShadingMode::Gouraud,
            rasterized_pixel_count: 0,
        }
    }

    /// Resets the depth buffer for a new frame.
    pub fn clear_depth(&mut self) {
        self.depth_buffer.clear();
        self.rasterized_pixel_count = 0;
    }

    /// Sets the active shading mode (Flat or Gouraud).
    pub fn set_shading_mode(&mut self, mode: ShadingMode) {
        self.shading_mode = mode;
    }

    /// Rasterizes a 3D screen-space triangle into a 32-bit BGRA linear framebuffer with 16-bit depth testing.
    /// Returns the number of fragments successfully shaded and written.
    pub fn draw_triangle(&mut self, tri: &Triangle3D, framebuffer: &mut [u8]) -> usize {
        if self.width == 0 || self.height == 0 {
            return 0;
        }
        let required_bytes = self.width.checked_mul(self.height).and_then(|pixels| pixels.checked_mul(4));
        if required_bytes.map_or(true, |bytes| framebuffer.len() < bytes) {
            return 0;
        }
        let p0 = tri.vertices[0].position;
        let p1 = tri.vertices[1].position;
        let p2 = tri.vertices[2].position;

        // Floating-point screen triangle area
        let area = (p1.x - p0.x) * (p2.y - p0.y) - (p1.y - p0.y) * (p2.x - p0.x);
        if area.abs() < 1e-4 {
            return 0; // Degenerate zero-area triangle
        }
        let inv_area = 1.0 / area;

        // Compute 2D axis-aligned bounding box clamped to framebuffer dimensions
        let min_xf = p0.x.min(p1.x).min(p2.x).max(0.0);
        let max_xf = p0.x.max(p1.x).max(p2.x).min((self.width - 1) as f32);
        let min_yf = p0.y.min(p1.y).min(p2.y).max(0.0);
        let max_yf = p0.y.max(p1.y).max(p2.y).min((self.height - 1) as f32);

        let min_x = min_xf as usize;
        let max_x = max_xf as usize;
        let min_y = min_yf as usize;
        let max_y = max_yf as usize;

        if min_x > max_x || min_y > max_y {
            return 0;
        }

        // Convert vertices to 28.4 fixed-point coordinates for sub-pixel precision
        let x0 = to_fixed_28_4(p0.x);
        let y0 = to_fixed_28_4(p0.y);
        let x1 = to_fixed_28_4(p1.x);
        let y1 = to_fixed_28_4(p1.y);
        let x2 = to_fixed_28_4(p2.x);
        let y2 = to_fixed_28_4(p2.y);

        // Normalize fixed-point winding before applying the half-open edge rule.
        let fixed_area = edge_func(x0, y0, x1, y1, x2, y2);
        if fixed_area == 0 {
            return 0;
        }
        let winding = if fixed_area > 0 { 1 } else { -1 };
        let top_left = |ax, ay, bx, by| {
            if winding > 0 { is_top_left(ax, ay, bx, by) }
            else { is_top_left(bx, by, ax, ay) }
        };
        let bias0 = if top_left(x1, y1, x2, y2) { 0 } else { -1 };
        let bias1 = if top_left(x2, y2, x0, y0) { 0 } else { -1 };
        let bias2 = if top_left(x0, y0, x1, y1) { 0 } else { -1 };

        let mut drawn_pixels = 0;

        // Flat shading pre-computation (uses vertex 0 color or average face color)
        let flat_pixel_value = tri.vertices[0].color.to_u32_bgr();

        for py in min_y..=max_y {
            // Pixel sample center at +0.5 sub-pixel offset (fixed-point +8)
            let sample_y = to_fixed_28_4(py as f32) + 8;
            let row_offset = py * self.width;

            for px in min_x..=max_x {
                let sample_x = to_fixed_28_4(px as f32) + 8;

                // Evaluate fixed-point edge functions
                let w0 = winding * edge_func(x1, y1, x2, y2, sample_x, sample_y) + bias0;
                let w1 = winding * edge_func(x2, y2, x0, y0, sample_x, sample_y) + bias1;
                let w2 = winding * edge_func(x0, y0, x1, y1, sample_x, sample_y) + bias2;

                // Inside test: all edge weights must have consistent sign
                if w0 >= 0 && w1 >= 0 && w2 >= 0 {
                    let fx = px as f32 + 0.5;
                    let fy = py as f32 + 0.5;

                    // Barycentric weight coordinates lambda 0, 1, 2
                    let l0 = ((p1.x - fx) * (p2.y - fy) - (p1.y - fy) * (p2.x - fx)) * inv_area;
                    let l1 = ((p2.x - fx) * (p0.y - fy) - (p2.y - fy) * (p0.x - fx)) * inv_area;
                    let l2 = 1.0 - l0 - l1;

                    // Interpolate Z depth
                    let z = l0 * p0.z + l1 * p1.z + l2 * p2.z;
                    let depth_u16 = (z.clamp(0.0, 1.0) * 65535.0) as u16;

                    // Perform 16-bit depth test against Z-Buffer
                    if self.depth_buffer.test_and_set(px, py, depth_u16) {
                        let pixel_val = match self.shading_mode {
                            ShadingMode::Flat => flat_pixel_value,
                            ShadingMode::Gouraud => {
                                let c0 = tri.vertices[0].color;
                                let c1 = tri.vertices[1].color;
                                let c2 = tri.vertices[2].color;

                                let r = l0 * c0.r + l1 * c1.r + l2 * c2.r;
                                let g = l0 * c0.g + l1 * c1.g + l2 * c2.g;
                                let b = l0 * c0.b + l1 * c1.b + l2 * c2.b;
                                let a = l0 * c0.a + l1 * c1.a + l2 * c2.a;

                                Color4::new(r, g, b, a).to_u32_bgr()
                            }
                        };

                        let fb_idx = (row_offset + px) * 4;
                        if fb_idx + 3 < framebuffer.len() {
                            framebuffer[fb_idx] = (pixel_val & 0xFF) as u8;             // B
                            framebuffer[fb_idx + 1] = ((pixel_val >> 8) & 0xFF) as u8;  // G
                            framebuffer[fb_idx + 2] = ((pixel_val >> 16) & 0xFF) as u8; // R
                            framebuffer[fb_idx + 3] = ((pixel_val >> 24) & 0xFF) as u8; // A
                            drawn_pixels += 1;
                        }
                    }
                }
            }
        }

        self.rasterized_pixel_count += drawn_pixels as u64;
        drawn_pixels
    }
}

/// Verification unit test ensuring fixed-point sub-pixel rasterization and 16-bit depth testing.
pub fn test_rasterizer_and_depth_buffering() -> bool {
    test_shared_edges_and_empty_targets();
    let mut rasterizer = Rasterizer3D::new(100, 100);
    let mut fb = alloc::vec![0u8; 100 * 100 * 4];

    // 1. Draw a front red triangle at Z = 0.5 (Flat Shading)
    rasterizer.set_shading_mode(ShadingMode::Flat);
    let v0 = super::types::Vertex3D::new(
        super::types::Vec4::new(10.0, 10.0, 0.5, 1.0),
        Color4::red(),
        Vec3::zero(),
    );
    let v1 = super::types::Vertex3D::new(
        super::types::Vec4::new(80.0, 10.0, 0.5, 1.0),
        Color4::red(),
        Vec3::zero(),
    );
    let v2 = super::types::Vertex3D::new(
        super::types::Vec4::new(45.0, 80.0, 0.5, 1.0),
        Color4::red(),
        Vec3::zero(),
    );
    let red_tri = Triangle3D::new(v0, v1, v2);

    let pixels_red = rasterizer.draw_triangle(&red_tri, &mut fb);
    assert!(pixels_red > 100, "Red triangle should rasterize significant pixels");

    // Center pixel (45, 30) should be red (B=0, G=0, R=255)
    let center_idx = (30 * 100 + 45) * 4;
    assert_eq!(fb[center_idx + 2], 255); // Red
    assert_eq!(fb[center_idx], 0);       // Blue

    // Depth buffer at center pixel should be approximately 0.5 * 65535 = 32767
    let center_depth = rasterizer.depth_buffer.get(45, 30);
    assert!((center_depth as i32 - 32767).abs() < 500);

    // 2. Draw a blue triangle BEHIND at Z = 0.8 covering the same area
    let vb0 = super::types::Vertex3D::new(
        super::types::Vec4::new(10.0, 10.0, 0.8, 1.0),
        Color4::blue(),
        Vec3::zero(),
    );
    let vb1 = super::types::Vertex3D::new(
        super::types::Vec4::new(80.0, 10.0, 0.8, 1.0),
        Color4::blue(),
        Vec3::zero(),
    );
    let vb2 = super::types::Vertex3D::new(
        super::types::Vec4::new(45.0, 80.0, 0.8, 1.0),
        Color4::blue(),
        Vec3::zero(),
    );
    let blue_tri = Triangle3D::new(vb0, vb1, vb2);

    let pixels_blue = rasterizer.draw_triangle(&blue_tri, &mut fb);
    assert_eq!(pixels_blue, 0, "Distant triangle should be occluded by depth test");

    // Pixel at center should STILL be red!
    assert_eq!(fb[center_idx + 2], 255);
    assert_eq!(fb[center_idx], 0);

    // 3. Draw a green triangle IN FRONT at Z = 0.2
    let vg0 = super::types::Vertex3D::new(
        super::types::Vec4::new(10.0, 10.0, 0.2, 1.0),
        Color4::green(),
        Vec3::zero(),
    );
    let vg1 = super::types::Vertex3D::new(
        super::types::Vec4::new(80.0, 10.0, 0.2, 1.0),
        Color4::green(),
        Vec3::zero(),
    );
    let vg2 = super::types::Vertex3D::new(
        super::types::Vec4::new(45.0, 80.0, 0.2, 1.0),
        Color4::green(),
        Vec3::zero(),
    );
    let green_tri = Triangle3D::new(vg0, vg1, vg2);

    let pixels_green = rasterizer.draw_triangle(&green_tri, &mut fb);
    assert!(pixels_green > 100, "Closer triangle should overwrite via depth test");

    // Pixel at center should now be green! (B=0, G=255, R=0)
    assert_eq!(fb[center_idx + 1], 255); // Green
    assert_eq!(fb[center_idx + 2], 0);   // Red

    // 4. Test Gouraud shading interpolation
    rasterizer.set_shading_mode(ShadingMode::Gouraud);
    rasterizer.clear_depth();
    fb.fill(0);

    let v_g0 = super::types::Vertex3D::new(
        super::types::Vec4::new(10.0, 10.0, 0.5, 1.0),
        Color4::red(), // Red at (10, 10)
        Vec3::zero(),
    );
    let v_g1 = super::types::Vertex3D::new(
        super::types::Vec4::new(80.0, 10.0, 0.5, 1.0),
        Color4::blue(), // Blue at (80, 10)
        Vec3::zero(),
    );
    let v_g2 = super::types::Vertex3D::new(
        super::types::Vec4::new(45.0, 80.0, 0.5, 1.0),
        Color4::green(), // Green at (45, 80)
        Vec3::zero(),
    );
    let gouraud_tri = Triangle3D::new(v_g0, v_g1, v_g2);
    let pixels_gouraud = rasterizer.draw_triangle(&gouraud_tri, &mut fb);
    assert!(pixels_gouraud > 100);

    // Near v_g0 (15, 15), Red should dominate
    let idx_near_red = (15 * 100 + 15) * 4;
    assert!(fb[idx_near_red + 2] > fb[idx_near_red]); // R > B

    // Near v_g1 (75, 15), Blue should dominate
    let idx_near_blue = (15 * 100 + 75) * 4;
    assert!(fb[idx_near_blue] > fb[idx_near_blue + 2]); // B > R

    true
}

/// Checks half-open coverage independently of depth-test draw ordering.
fn test_shared_edges_and_empty_targets() {
    use super::types::{Vec4, Vertex3D};
    let vertex = |x, y, color| Vertex3D::new(Vec4::new(x, y, 0.5, 1.0), color, Vec3::zero());
    let a = Triangle3D::new(vertex(0.0, 0.0, Color4::red()), vertex(2.0, 0.0, Color4::red()), vertex(0.0, 2.0, Color4::red()));
    let b = Triangle3D::new(vertex(2.0, 0.0, Color4::blue()), vertex(2.0, 2.0, Color4::blue()), vertex(0.0, 2.0, Color4::blue()));
    for reverse_a in [false, true] {
        for reverse_b in [false, true] {
            let mut triangles = [a, b];
            if reverse_a { triangles[0].vertices.swap(1, 2); }
            if reverse_b { triangles[1].vertices.swap(1, 2); }
            let mut masks = [[0u8; 16]; 2];
            for i in 0..2 {
                Rasterizer3D::new(2, 2).draw_triangle(&triangles[i], &mut masks[i]);
            }
            for pixel in 0..4 {
                let owners = (masks[0][pixel * 4 + 3] != 0) as u8
                    + (masks[1][pixel * 4 + 3] != 0) as u8;
                assert_eq!(owners, 1, "Each square sample must have exactly one owner");
            }
            let mut forward = [0u8; 16];
            let mut backward = [0u8; 16];
            let mut rasterizer = Rasterizer3D::new(2, 2);
            for triangle in &triangles { rasterizer.draw_triangle(triangle, &mut forward); }
            rasterizer.clear_depth();
            for triangle in triangles.iter().rev() { rasterizer.draw_triangle(triangle, &mut backward); }
            assert_eq!(forward, backward, "Shared-edge colors must not depend on draw order");
        }
    }
    for (width, height) in [(0, 0), (0, 2), (2, 0)] {
        let mut rasterizer = Rasterizer3D::new(width, height);
        assert_eq!(rasterizer.draw_triangle(&a, &mut []), 0);
        assert_eq!(rasterizer.rasterized_pixel_count, 0);
    }
    let mut rasterizer = Rasterizer3D::new(2, 2);
    let mut short_buffer = [0u8; 15];
    assert_eq!(rasterizer.draw_triangle(&a, &mut short_buffer), 0);
    assert_eq!(short_buffer, [0; 15]);
    assert!(rasterizer.depth_buffer.buffer.iter().all(|&depth| depth == u16::MAX));
    assert_eq!(rasterizer.rasterized_pixel_count, 0);
    let mut framebuffer = [0u8; 16];
    assert!(rasterizer.draw_triangle(&a, &mut framebuffer) > 0, "A rejected draw must not occlude a valid retry");
}

#[cfg(test)]
mod tests {
    #[test]
    fn coverage_and_depth_regressions() {
        assert!(super::test_rasterizer_and_depth_buffering());
    }
}
