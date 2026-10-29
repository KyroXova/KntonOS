//! Software 3D Graphics Pipeline & Barycentric Triangle Rasterizer for KntonOS.
//!
//! Provides an independent, real-time 3D rendering pipeline:
//! - 3D vector, matrix, vertex, and triangle mathematics
//! - Perspective projection, viewport transformation, and frustum clipping
//! - Sub-pixel barycentric triangle rasterizer with 16-bit Z-buffer depth testing
//! - Flat and Gouraud vertex color shading

pub mod types;

pub use types::{
    cos_f32, sin_f32, sqrt_f32, tan_f32, test_3d_math_and_structures, Color4, Mat4, Triangle3D,
    Vec3, Vec4, Vertex3D, HALF_PI, PI, TWO_PI,
};
