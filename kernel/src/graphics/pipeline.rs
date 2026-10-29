//! 3D Transformation Pipeline, Perspective Projection, Viewport Mapping, and Near-Plane Clipping.
//!
//! Implements standard 3D pipeline stages:
//! - Model-View-Projection (MVP) matrix transformation
//! - Sutherland-Hodgman near-plane clipping preventing W <= 0 projection singularity
//! - Perspective divide (x/w, y/w, z/w) into Normalized Device Coordinates (NDC)
//! - Screen-space viewport mapping and 2D backface culling

use alloc::vec::Vec;
use super::types::{Color4, Mat4, Triangle3D, Vec3, Vec4, Vertex3D};

/// 3D Camera specifying eye position, look-at target, and up vector.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Camera3D {
    pub position: Vec3,
    pub target: Vec3,
    pub up: Vec3,
}

impl Camera3D {
    pub const fn new(position: Vec3, target: Vec3, up: Vec3) -> Self {
        Self { position, target, up }
    }

    /// Computes the 4x4 View Matrix transforming world coordinates into camera-relative view space.
    pub fn view_matrix(&self) -> Mat4 {
        // Forward/view direction from target to eye (standard RHS)
        let z_axis = self.position.sub(self.target).normalize();
        let x_axis = self.up.cross(z_axis).normalize();
        let y_axis = z_axis.cross(x_axis);

        let rot = Mat4 {
            m: [
                [x_axis.x, x_axis.y, x_axis.z, 0.0],
                [y_axis.x, y_axis.y, y_axis.z, 0.0],
                [z_axis.x, z_axis.y, z_axis.z, 0.0],
                [0.0, 0.0, 0.0, 1.0],
            ],
        };

        let trans = Mat4::translation(-self.position.x, -self.position.y, -self.position.z);
        rot.mul_mat4(&trans)
    }
}

/// 2D Screen Viewport describing the display target region.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Viewport {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
    pub min_z: f32,
    pub max_z: f32,
}

impl Viewport {
    pub const fn new(x: f32, y: f32, width: f32, height: f32) -> Self {
        Self {
            x,
            y,
            width,
            height,
            min_z: 0.0,
            max_z: 1.0,
        }
    }

    /// Maps Normalized Device Coordinates (NDC: [-1, 1]) to screen pixel coordinates.
    #[inline(always)]
    pub fn map_ndc_to_screen(&self, ndc: Vec3) -> Vec3 {
        let sx = self.x + (ndc.x + 1.0) * 0.5 * self.width;
        // Invert Y axis so +1 NDC is at top of screen and -1 NDC is at bottom
        let sy = self.y + (1.0 - ndc.y) * 0.5 * self.height;
        let sz = self.min_z + (ndc.z + 1.0) * 0.5 * (self.max_z - self.min_z);
        Vec3::new(sx, sy, sz)
    }
}

/// Linearly interpolates all vertex attributes (position, color, normal, uv) at factor t in [0, 1].
pub fn lerp_vertex(v0: &Vertex3D, v1: &Vertex3D, t: f32) -> Vertex3D {
    let pos = Vec4::new(
        v0.position.x + (v1.position.x - v0.position.x) * t,
        v0.position.y + (v1.position.y - v0.position.y) * t,
        v0.position.z + (v1.position.z - v0.position.z) * t,
        v0.position.w + (v1.position.w - v0.position.w) * t,
    );

    let col = Color4::new(
        v0.color.r + (v1.color.r - v0.color.r) * t,
        v0.color.g + (v1.color.g - v0.color.g) * t,
        v0.color.b + (v1.color.b - v0.color.b) * t,
        v0.color.a + (v1.color.a - v0.color.a) * t,
    );

    let norm = Vec3::new(
        v0.normal.x + (v1.normal.x - v0.normal.x) * t,
        v0.normal.y + (v1.normal.y - v0.normal.y) * t,
        v0.normal.z + (v1.normal.z - v0.normal.z) * t,
    );

    let uv = [
        v0.uv[0] + (v1.uv[0] - v0.uv[0]) * t,
        v0.uv[1] + (v1.uv[1] - v0.uv[1]) * t,
    ];

    Vertex3D {
        position: pos,
        color: col,
        normal: norm,
        uv,
    }
}

/// Clips a triangle in clip-space against the camera near-plane (W >= near_plane).
/// Returns 0, 1, or 2 output triangles.
pub fn clip_triangle_near_plane(tri: &Triangle3D, near_plane: f32, out: &mut Vec<Triangle3D>) {
    let d0 = tri.vertices[0].position.w - near_plane;
    let d1 = tri.vertices[1].position.w - near_plane;
    let d2 = tri.vertices[2].position.w - near_plane;

    let in0 = d0 >= 0.0;
    let in1 = d1 >= 0.0;
    let in2 = d2 >= 0.0;

    let inside_count = (in0 as u8) + (in1 as u8) + (in2 as u8);

    match inside_count {
        0 => {
            // Completely behind near plane: discard triangle
        }
        3 => {
            // Completely in front of near plane: preserve triangle unmodified
            out.push(*tri);
        }
        1 => {
            // Exactly 1 vertex inside: clips into 1 smaller triangle
            let (v_in, v_out1, v_out2, d_in, d_out1, d_out2) = if in0 {
                (&tri.vertices[0], &tri.vertices[1], &tri.vertices[2], d0, d1, d2)
            } else if in1 {
                (&tri.vertices[1], &tri.vertices[2], &tri.vertices[0], d1, d2, d0)
            } else {
                (&tri.vertices[2], &tri.vertices[0], &tri.vertices[1], d2, d0, d1)
            };

            let t1 = d_in / (d_in - d_out1);
            let t2 = d_in / (d_in - d_out2);

            let new_v1 = lerp_vertex(v_in, v_out1, t1);
            let new_v2 = lerp_vertex(v_in, v_out2, t2);

            out.push(Triangle3D::new(*v_in, new_v1, new_v2));
        }
        2 => {
            // Exactly 2 vertices inside: clips quad into 2 triangles
            let (v_in1, v_in2, v_out, d_in1, d_in2, d_out) = if !in0 {
                (&tri.vertices[1], &tri.vertices[2], &tri.vertices[0], d1, d2, d0)
            } else if !in1 {
                (&tri.vertices[2], &tri.vertices[0], &tri.vertices[1], d2, d0, d1)
            } else {
                (&tri.vertices[0], &tri.vertices[1], &tri.vertices[2], d0, d1, d2)
            };

            let t1 = d_in1 / (d_in1 - d_out);
            let t2 = d_in2 / (d_out - d_in2).abs();

            let new_v1 = lerp_vertex(v_in1, v_out, t1);
            let new_v2 = lerp_vertex(v_in2, v_out, t2);

            out.push(Triangle3D::new(*v_in1, *v_in2, new_v1));
            out.push(Triangle3D::new(*v_in2, new_v2, new_v1));
        }
        _ => {}
    }
}

/// Tests whether a triangle in 2D screen coordinates is front-facing (counter-clockwise winding).
#[inline(always)]
pub fn is_front_facing_screen(p0: Vec3, p1: Vec3, p2: Vec3) -> bool {
    let cross = (p1.x - p0.x) * (p2.y - p0.y) - (p1.y - p0.y) * (p2.x - p0.x);
    cross > 0.0
}

/// Central 3D Transformation Pipeline.
#[derive(Debug, Clone, Copy)]
pub struct TransformPipeline {
    pub model: Mat4,
    pub view: Mat4,
    pub projection: Mat4,
    pub viewport: Viewport,
    pub near_plane: f32,
    pub cull_backfaces: bool,
}

impl TransformPipeline {
    pub const fn new(viewport: Viewport) -> Self {
        Self {
            model: Mat4::identity(),
            view: Mat4::identity(),
            projection: Mat4::identity(),
            viewport,
            near_plane: 0.1,
            cull_backfaces: true,
        }
    }

    /// Transforms a triangle from Model Space -> View Space -> Clip Space -> Screen Space.
    /// Performs Sutherland-Hodgman near-plane clipping, perspective divide, and backface culling.
    pub fn process_triangle(&self, tri: &Triangle3D, screen_triangles: &mut Vec<Triangle3D>) {
        let mvp = self.projection.mul_mat4(&self.view.mul_mat4(&self.model));

        // 1. Transform vertices to homogeneous clip space
        let mut clip_tri = *tri;
        for i in 0..3 {
            clip_tri.vertices[i].position = mvp.mul_vec4(tri.vertices[i].position);
        }

        // 2. Near-plane clipping in clip space
        let mut clipped_triangles = Vec::with_capacity(2);
        clip_triangle_near_plane(&clip_tri, self.near_plane, &mut clipped_triangles);

        // 3. For each clipped triangle: perspective divide and viewport mapping
        for t in &clipped_triangles {
            let mut screen_tri = *t;
            for i in 0..3 {
                let ndc = screen_tri.vertices[i].position.perspective_divide();
                let screen_pos = self.viewport.map_ndc_to_screen(ndc);
                screen_tri.vertices[i].position = Vec4::from_vec3(screen_pos, 1.0);
            }

            // 4. Backface culling in screen space
            if self.cull_backfaces {
                let p0 = screen_tri.vertices[0].position.to_vec3();
                let p1 = screen_tri.vertices[1].position.to_vec3();
                let p2 = screen_tri.vertices[2].position.to_vec3();
                if !is_front_facing_screen(p0, p1, p2) {
                    continue;
                }
            }

            screen_triangles.push(screen_tri);
        }
    }
}

/// Verification unit test ensuring camera view matrices, near-plane clipping, and viewport mapping.
pub fn test_3d_pipeline_and_clipping() -> bool {
    // 1. Camera LookAt View matrix verification
    let camera = Camera3D::new(
        Vec3::new(0.0, 0.0, 5.0),
        Vec3::new(0.0, 0.0, 0.0),
        Vec3::new(0.0, 1.0, 0.0),
    );
    let view = camera.view_matrix();
    let eye_view = view.mul_vec3(camera.position, 1.0);
    // Camera position transformed by view matrix should be at origin (0, 0, 0)
    assert!(eye_view.x.abs() < 1e-4);
    assert!(eye_view.y.abs() < 1e-4);
    assert!(eye_view.z.abs() < 1e-4);

    // 2. Viewport NDC mapping verification
    let vp = Viewport::new(0.0, 0.0, 320.0, 240.0);
    // Center of NDC (0, 0, 0) -> screen center (160, 120, 0.5)
    let center = vp.map_ndc_to_screen(Vec3::new(0.0, 0.0, 0.0));
    assert_eq!(center, Vec3::new(160.0, 120.0, 0.5));

    // Top-left NDC (-1, 1, -1) -> screen (0, 0, 0)
    let top_left = vp.map_ndc_to_screen(Vec3::new(-1.0, 1.0, -1.0));
    assert_eq!(top_left, Vec3::new(0.0, 0.0, 0.0));

    // Bottom-right NDC (1, -1, 1) -> screen (320, 240, 1.0)
    let bottom_right = vp.map_ndc_to_screen(Vec3::new(1.0, -1.0, 1.0));
    assert_eq!(bottom_right, Vec3::new(320.0, 240.0, 1.0));

    // 3. Near-plane clipping: completely outside (behind camera)
    let v_out0 = Vertex3D::new(Vec4::new(0.0, 0.0, -5.0, -1.0), Color4::white(), Vec3::zero());
    let v_out1 = Vertex3D::new(Vec4::new(1.0, 0.0, -5.0, -1.0), Color4::white(), Vec3::zero());
    let v_out2 = Vertex3D::new(Vec4::new(0.0, 1.0, -5.0, -1.0), Color4::white(), Vec3::zero());
    let tri_behind = Triangle3D::new(v_out0, v_out1, v_out2);

    let mut out_behind = Vec::new();
    clip_triangle_near_plane(&tri_behind, 0.1, &mut out_behind);
    assert_eq!(out_behind.len(), 0);

    // 4. Near-plane clipping: completely inside
    let v_in0 = Vertex3D::new(Vec4::new(0.0, 0.0, 2.0, 2.0), Color4::white(), Vec3::zero());
    let v_in1 = Vertex3D::new(Vec4::new(1.0, 0.0, 2.0, 2.0), Color4::white(), Vec3::zero());
    let v_in2 = Vertex3D::new(Vec4::new(0.0, 1.0, 2.0, 2.0), Color4::white(), Vec3::zero());
    let tri_inside = Triangle3D::new(v_in0, v_in1, v_in2);

    let mut out_inside = Vec::new();
    clip_triangle_near_plane(&tri_inside, 0.1, &mut out_inside);
    assert_eq!(out_inside.len(), 1);

    // 5. Backface culling: front-facing vs back-facing
    let s0 = Vec3::new(10.0, 10.0, 0.5);
    let s1 = Vec3::new(20.0, 30.0, 0.5);
    let s2 = Vec3::new(30.0, 10.0, 0.5);
    assert!(is_front_facing_screen(s0, s1, s2));
    // Reverse winding order
    assert!(!is_front_facing_screen(s0, s2, s1));

    true
}
