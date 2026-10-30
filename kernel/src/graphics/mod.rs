//! Software 3D Graphics Pipeline & Barycentric Triangle Rasterizer for KntonOS.
//! Provides an independent, real-time 3D rendering pipeline:

pub mod types;
pub mod pipeline;
pub mod rasterizer;
pub mod cube_test;

pub use types::{
    cos_f32, sin_f32, sqrt_f32, tan_f32, test_3d_math_and_structures, Color4, Mat4, Triangle3D,
    Vec3, Vec4, Vertex3D, HALF_PI, PI, TWO_PI,
};

pub use pipeline::{
    clip_triangle_near_plane, is_front_facing_screen, lerp_vertex, test_3d_pipeline_and_clipping,
    Camera3D, TransformPipeline, Viewport,
};

pub use rasterizer::{
    test_rasterizer_and_depth_buffering, DepthBuffer, Rasterizer3D, ShadingMode,
};

pub use cube_test::{generate_cube_mesh, test_graphics_subsystem};
