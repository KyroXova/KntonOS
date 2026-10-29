//! 3D Rotating Shaded Cube Rendering Verification Suite for KntonOS.
//!
//! Generates a 12-triangle shaded 3D mesh cube, applies Model-View-Projection (MVP)
//! transformations, near-plane frustum clipping, backface culling, sub-pixel barycentric
//! rasterization, and 16-bit Z-buffering into a 320x240 backbuffer. Emits boot sentinel
//! [KNT-GFX-3D-PASS].

use alloc::vec::Vec;
use super::pipeline::{Camera3D, TransformPipeline, Viewport};
use super::rasterizer::{Rasterizer3D, ShadingMode};
use super::types::{Color4, Mat4, Triangle3D, Vec3, Vec4, Vertex3D, PI};

/// Constructs the 12 triangles of a 3D unit cube centered at origin [-1, 1] on each axis.
pub fn generate_cube_mesh() -> [Triangle3D; 12] {
    // 8 cube corner vertices
    let p000 = Vec4::new(-1.0, -1.0, -1.0, 1.0);
    let p100 = Vec4::new( 1.0, -1.0, -1.0, 1.0);
    let p110 = Vec4::new( 1.0,  1.0, -1.0, 1.0);
    let p010 = Vec4::new(-1.0,  1.0, -1.0, 1.0);
    let p001 = Vec4::new(-1.0, -1.0,  1.0, 1.0);
    let p101 = Vec4::new( 1.0, -1.0,  1.0, 1.0);
    let p111 = Vec4::new( 1.0,  1.0,  1.0, 1.0);
    let p011 = Vec4::new(-1.0,  1.0,  1.0, 1.0);

    // Distinct vibrant colors per face
    let c_front = Color4::new(1.0, 0.1, 0.1, 1.0);  // Red
    let c_back = Color4::new(0.1, 1.0, 1.0, 1.0);   // Cyan
    let c_top = Color4::new(0.1, 1.0, 0.1, 1.0);    // Green
    let c_bottom = Color4::new(1.0, 0.1, 1.0, 1.0); // Magenta
    let c_left = Color4::new(0.1, 0.1, 1.0, 1.0);   // Blue
    let c_right = Color4::new(1.0, 1.0, 0.1, 1.0);  // Yellow

    let n_front = Vec3::new(0.0, 0.0, 1.0);
    let n_back = Vec3::new(0.0, 0.0, -1.0);
    let n_top = Vec3::new(0.0, 1.0, 0.0);
    let n_bottom = Vec3::new(0.0, -1.0, 0.0);
    let n_left = Vec3::new(-1.0, 0.0, 0.0);
    let n_right = Vec3::new(1.0, 0.0, 0.0);

    let v = |pos: Vec4, col: Color4, norm: Vec3| -> Vertex3D {
        Vertex3D::new(pos, col, norm)
    };

    [
        // Front face (+Z)
        Triangle3D::new(v(p001, c_front, n_front), v(p101, c_front, n_front), v(p111, c_front, n_front)),
        Triangle3D::new(v(p001, c_front, n_front), v(p111, c_front, n_front), v(p011, c_front, n_front)),

        // Back face (-Z)
        Triangle3D::new(v(p100, c_back, n_back), v(p000, c_back, n_back), v(p010, c_back, n_back)),
        Triangle3D::new(v(p100, c_back, n_back), v(p010, c_back, n_back), v(p110, c_back, n_back)),

        // Top face (+Y)
        Triangle3D::new(v(p011, c_top, n_top), v(p111, c_top, n_top), v(p110, c_top, n_top)),
        Triangle3D::new(v(p011, c_top, n_top), v(p110, c_top, n_top), v(p010, c_top, n_top)),

        // Bottom face (-Y)
        Triangle3D::new(v(p000, c_bottom, n_bottom), v(p100, c_bottom, n_bottom), v(p101, c_bottom, n_bottom)),
        Triangle3D::new(v(p000, c_bottom, n_bottom), v(p101, c_bottom, n_bottom), v(p001, c_bottom, n_bottom)),

        // Left face (-X)
        Triangle3D::new(v(p000, c_left, n_left), v(p001, c_left, n_left), v(p011, c_left, n_left)),
        Triangle3D::new(v(p000, c_left, n_left), v(p011, c_left, n_left), v(p010, c_left, n_left)),

        // Right face (+X)
        Triangle3D::new(v(p101, c_right, n_right), v(p100, c_right, n_right), v(p110, c_right, n_right)),
        Triangle3D::new(v(p101, c_right, n_right), v(p110, c_right, n_right), v(p111, c_right, n_right)),
    ]
}

/// Comprehensive 3D graphics verification suite emitting boot sentinel [KNT-GFX-3D-PASS].
pub fn test_graphics_subsystem() -> bool {
    crate::serial_write_str("[KNT-GFX-TEST] Executing 3D graphics pipeline & rasterizer verification...\r\n");

    // 1. Math and vector/matrix unit tests
    if !super::types::test_3d_math_and_structures() {
        crate::serial_write_str("[KNT-TEST-FAIL] 3D vector, matrix, and geometry check failed!\r\n");
        return false;
    }
    crate::serial_write_str("[KNT-GFX-MATH-PASS] 3D vector math, Mat4 transforms, and geometry verified\r\n");

    // 2. Transformation pipeline, perspective projection, and near-plane clipping
    if !super::pipeline::test_3d_pipeline_and_clipping() {
        crate::serial_write_str("[KNT-TEST-FAIL] 3D transformation and clipping check failed!\r\n");
        return false;
    }
    crate::serial_write_str("[KNT-GFX-PIPE-PASS] Camera view matrix, near-plane clipping, and viewport mapping verified\r\n");

    // 3. Sub-pixel barycentric rasterizer and 16-bit depth buffering
    if !super::rasterizer::test_rasterizer_and_depth_buffering() {
        crate::serial_write_str("[KNT-TEST-FAIL] Barycentric rasterizer and Z-buffering check failed!\r\n");
        return false;
    }
    crate::serial_write_str("[KNT-GFX-RASTER-PASS] Fixed-point 28.4 edge functions, Z-buffering, and Gouraud shading verified\r\n");

    // 4. End-to-end rendering test: Render rotating shaded cube into 320x240 backbuffer
    let width = 320;
    let height = 240;
    let mut fb = alloc::vec![0u8; width * height * 4];
    let mut rasterizer = Rasterizer3D::new(width, height);
    rasterizer.set_shading_mode(ShadingMode::Gouraud);
    rasterizer.clear_depth();

    let viewport = Viewport::new(0.0, 0.0, width as f32, height as f32);
    let mut pipeline = TransformPipeline::new(viewport);
    pipeline.cull_backfaces = true;

    // Rotate cube along X and Y axes
    let rot_y = Mat4::rotation_y(0.65);
    let rot_x = Mat4::rotation_x(0.45);
    pipeline.model = rot_y.mul_mat4(&rot_x);

    // Camera at (0, 0, 3.5) looking at origin
    let camera = Camera3D::new(
        Vec3::new(0.0, 0.0, 3.5),
        Vec3::new(0.0, 0.0, 0.0),
        Vec3::new(0.0, 1.0, 0.0),
    );
    pipeline.view = camera.view_matrix();

    // 60-degree field of view perspective projection
    pipeline.projection = Mat4::perspective(PI / 3.0, (width as f32) / (height as f32), 0.1, 100.0);

    let cube_mesh = generate_cube_mesh();
    let mut screen_triangles = Vec::new();

    for tri in &cube_mesh {
        pipeline.process_triangle(tri, &mut screen_triangles);
    }

    // With 3D backface culling active, exactly 3 visible faces (6 triangles) should survive
    assert!(screen_triangles.len() >= 4 && screen_triangles.len() <= 8, "Front-facing cube faces must survive culling");

    // Rasterize all visible triangles with depth testing
    let mut total_pixels = 0;
    for tri in &screen_triangles {
        total_pixels += rasterizer.draw_triangle(tri, &mut fb);
    }

    assert!(total_pixels > 2000, "Rendered cube must shade thousands of fragments");
    assert!(rasterizer.rasterized_pixel_count > 2000);

    // Verify center pixel (160, 120) has valid depth and shaded color
    let center_depth = rasterizer.depth_buffer.get(160, 120);
    assert!(center_depth < 0xFFFF, "Center pixel must be covered by front cube face");

    let center_idx = (120 * width + 160) * 4;
    let b = fb[center_idx];
    let g = fb[center_idx + 1];
    let r = fb[center_idx + 2];
    assert!(r > 0 || g > 0 || b > 0, "Center pixel must have shaded color");

    crate::serial_write_str("[KNT-GFX-3D-PASS] 3D software graphics pipeline & rasterizer operational\r\n");
    true
}
