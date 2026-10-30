//! 3D Graphics Mathematics, Geometry, Matrix, and Vertex Types for KntonOS.
//! Provides vector, matrix, vertex, and triangle structures for the software 3D pipeline:

pub const PI: f32 = 3.14159265358979323846;
pub const HALF_PI: f32 = PI / 2.0;
pub const TWO_PI: f32 = PI * 2.0;

// Pure no_std Floating Point Math Helpers (Taylor series & Newton-Raphson)

/// Computes the square root of a floating-point number using Newton-Raphson iteration.
pub fn sqrt_f32(x: f32) -> f32 {
    if x <= 0.0 {
        return 0.0;
    }
    let mut guess = if x > 1.0 { x * 0.5 } else { 1.0 };
    for _ in 0..12 {
        guess = 0.5 * (guess + x / guess);
    }
    guess
}

/// Computes sine in radians using range reduction and 9th-order Taylor polynomial.
pub fn sin_f32(mut rad: f32) -> f32 {
    // Range reduction to [-PI, PI]
    while rad > PI {
        rad -= TWO_PI;
    }
    while rad < -PI {
        rad += TWO_PI;
    }

    // Reduce to [-PI/2, PI/2] via sine symmetry
    if rad > HALF_PI {
        rad = PI - rad;
    } else if rad < -HALF_PI {
        rad = -PI - rad;
    }

    let x = rad;
    let x2 = x * x;
    let x3 = x * x2;
    let x5 = x3 * x2;
    let x7 = x5 * x2;
    let x9 = x7 * x2;

    x - (x3 / 6.0) + (x5 / 120.0) - (x7 / 5040.0) + (x9 / 362880.0)
}

/// Computes cosine in radians using identity cos(x) = sin(x + PI/2).
pub fn cos_f32(rad: f32) -> f32 {
    sin_f32(rad + HALF_PI)
}

/// Computes tangent in radians: sin(rad) / cos(rad).
pub fn tan_f32(rad: f32) -> f32 {
    let c = cos_f32(rad);
    if c.abs() < 1e-7 {
        1e7
    } else {
        sin_f32(rad) / c
    }
}

// Vector3 (3D coordinates & directions)

#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Vec3 {
    pub x: f32,
    pub y: f32,
    pub z: f32,
}

impl Vec3 {
    pub const fn new(x: f32, y: f32, z: f32) -> Self {
        Self { x, y, z }
    }

    pub const fn zero() -> Self {
        Self { x: 0.0, y: 0.0, z: 0.0 }
    }

    pub const fn one() -> Self {
        Self { x: 1.0, y: 1.0, z: 1.0 }
    }

    #[inline(always)]
    pub fn add(self, other: Vec3) -> Vec3 {
        Vec3::new(self.x + other.x, self.y + other.y, self.z + other.z)
    }

    #[inline(always)]
    pub fn sub(self, other: Vec3) -> Vec3 {
        Vec3::new(self.x - other.x, self.y - other.y, self.z - other.z)
    }

    #[inline(always)]
    pub fn scale(self, s: f32) -> Vec3 {
        Vec3::new(self.x * s, self.y * s, self.z * s)
    }

    #[inline(always)]
    pub fn dot(self, other: Vec3) -> f32 {
        self.x * other.x + self.y * other.y + self.z * other.z
    }

    #[inline(always)]
    pub fn cross(self, other: Vec3) -> Vec3 {
        Vec3::new(
            self.y * other.z - self.z * other.y,
            self.z * other.x - self.x * other.z,
            self.x * other.y - self.y * other.x,
        )
    }

    #[inline(always)]
    pub fn length_sq(self) -> f32 {
        self.dot(self)
    }

    #[inline(always)]
    pub fn length(self) -> f32 {
        sqrt_f32(self.length_sq())
    }

    #[inline(always)]
    pub fn normalize(self) -> Vec3 {
        let len = self.length();
        if len > 1e-7 {
            self.scale(1.0 / len)
        } else {
            Vec3::zero()
        }
    }

    #[inline(always)]
    pub const fn to_vec4(self, w: f32) -> Vec4 {
        Vec4::new(self.x, self.y, self.z, w)
    }
}

// Vector4 (Homogeneous coordinates)

#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Vec4 {
    pub x: f32,
    pub y: f32,
    pub z: f32,
    pub w: f32,
}

impl Vec4 {
    pub const fn new(x: f32, y: f32, z: f32, w: f32) -> Self {
        Self { x, y, z, w }
    }

    pub const fn from_vec3(v: Vec3, w: f32) -> Self {
        Self { x: v.x, y: v.y, z: v.z, w }
    }

    #[inline(always)]
    pub const fn to_vec3(self) -> Vec3 {
        Vec3::new(self.x, self.y, self.z)
    }

    #[inline(always)]
    pub fn perspective_divide(self) -> Vec3 {
        let inv_w = if self.w.abs() > 1e-7 { 1.0 / self.w } else { 1.0 };
        Vec3::new(self.x * inv_w, self.y * inv_w, self.z * inv_w)
    }

    #[inline(always)]
    pub fn dot(self, other: Vec4) -> f32 {
        self.x * other.x + self.y * other.y + self.z * other.z + self.w * other.w
    }

    #[inline(always)]
    pub fn add(self, other: Vec4) -> Vec4 {
        Vec4::new(self.x + other.x, self.y + other.y, self.z + other.z, self.w + other.w)
    }

    #[inline(always)]
    pub fn sub(self, other: Vec4) -> Vec4 {
        Vec4::new(self.x - other.x, self.y - other.y, self.z - other.z, self.w - other.w)
    }

    #[inline(always)]
    pub fn scale(self, s: f32) -> Vec4 {
        Vec4::new(self.x * s, self.y * s, self.z * s, self.w * s)
    }
}

// Matrix4x4 (Row-Major 3D Transformation Matrix)

#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Mat4 {
    pub m: [[f32; 4]; 4],
}

impl Default for Mat4 {
    fn default() -> Self {
        Self::identity()
    }
}

impl Mat4 {
    /// Returns 4x4 identity matrix.
    pub const fn identity() -> Self {
        Self {
            m: [
                [1.0, 0.0, 0.0, 0.0],
                [0.0, 1.0, 0.0, 0.0],
                [0.0, 0.0, 1.0, 0.0],
                [0.0, 0.0, 0.0, 1.0],
            ],
        }
    }

    /// Returns zero matrix.
    pub const fn zero() -> Self {
        Self {
            m: [
                [0.0, 0.0, 0.0, 0.0],
                [0.0, 0.0, 0.0, 0.0],
                [0.0, 0.0, 0.0, 0.0],
                [0.0, 0.0, 0.0, 0.0],
            ],
        }
    }

    /// Returns a 3D translation matrix.
    pub const fn translation(tx: f32, ty: f32, tz: f32) -> Self {
        Self {
            m: [
                [1.0, 0.0, 0.0, tx],
                [0.0, 1.0, 0.0, ty],
                [0.0, 0.0, 1.0, tz],
                [0.0, 0.0, 0.0, 1.0],
            ],
        }
    }

    /// Returns a 3D scaling matrix.
    pub const fn scaling(sx: f32, sy: f32, sz: f32) -> Self {
        Self {
            m: [
                [sx, 0.0, 0.0, 0.0],
                [0.0, sy, 0.0, 0.0],
                [0.0, 0.0, sz, 0.0],
                [0.0, 0.0, 0.0, 1.0],
            ],
        }
    }

    /// Returns a 3D rotation matrix around X-axis by angle in radians.
    pub fn rotation_x(rad: f32) -> Self {
        let s = sin_f32(rad);
        let c = cos_f32(rad);
        Self {
            m: [
                [1.0, 0.0, 0.0, 0.0],
                [0.0, c, -s, 0.0],
                [0.0, s, c, 0.0],
                [0.0, 0.0, 0.0, 1.0],
            ],
        }
    }

    /// Returns a 3D rotation matrix around Y-axis by angle in radians.
    pub fn rotation_y(rad: f32) -> Self {
        let s = sin_f32(rad);
        let c = cos_f32(rad);
        Self {
            m: [
                [c, 0.0, s, 0.0],
                [0.0, 1.0, 0.0, 0.0],
                [-s, 0.0, c, 0.0],
                [0.0, 0.0, 0.0, 1.0],
            ],
        }
    }

    /// Returns a 3D rotation matrix around Z-axis by angle in radians.
    pub fn rotation_z(rad: f32) -> Self {
        let s = sin_f32(rad);
        let c = cos_f32(rad);
        Self {
            m: [
                [c, -s, 0.0, 0.0],
                [s, c, 0.0, 0.0],
                [0.0, 0.0, 1.0, 0.0],
                [0.0, 0.0, 0.0, 1.0],
            ],
        }
    }

    /// Constructs a standard perspective projection matrix.
    /// maps frustum into normalized device coordinates (NDC: [-1, 1] on X/Y, [0, 1] on Z).
    pub fn perspective(fov_rad: f32, aspect: f32, near: f32, far: f32) -> Self {
        let f = 1.0 / tan_f32(fov_rad * 0.5);
        let range = near - far;

        Self {
            m: [
                [f / aspect, 0.0, 0.0, 0.0],
                [0.0, f, 0.0, 0.0],
                [0.0, 0.0, (far + near) / range, (2.0 * far * near) / range],
                [0.0, 0.0, -1.0, 0.0],
            ],
        }
    }

    /// Multiplies self with another 4x4 matrix: (self * other).
    pub fn mul_mat4(&self, other: &Mat4) -> Mat4 {
        let mut res = Mat4::zero();
        for r in 0..4 {
            for c in 0..4 {
                res.m[r][c] = self.m[r][0] * other.m[0][c]
                    + self.m[r][1] * other.m[1][c]
                    + self.m[r][2] * other.m[2][c]
                    + self.m[r][3] * other.m[3][c];
            }
        }
        res
    }

    /// Multiplies matrix with a 4-component vector: (self * v).
    #[inline(always)]
    pub fn mul_vec4(&self, v: Vec4) -> Vec4 {
        Vec4::new(
            self.m[0][0] * v.x + self.m[0][1] * v.y + self.m[0][2] * v.z + self.m[0][3] * v.w,
            self.m[1][0] * v.x + self.m[1][1] * v.y + self.m[1][2] * v.z + self.m[1][3] * v.w,
            self.m[2][0] * v.x + self.m[2][1] * v.y + self.m[2][2] * v.z + self.m[2][3] * v.w,
            self.m[3][0] * v.x + self.m[3][1] * v.y + self.m[3][2] * v.z + self.m[3][3] * v.w,
        )
    }

    /// Multiplies matrix with a 3-component vector assuming specified w component (typically 1.0 for points, 0.0 for vectors).
    #[inline(always)]
    pub fn mul_vec3(&self, v: Vec3, w: f32) -> Vec4 {
        self.mul_vec4(Vec4::from_vec3(v, w))
    }
}

// Floating Point Color4 for Smooth Vertex Shading Interpolation

#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Color4 {
    pub r: f32,
    pub g: f32,
    pub b: f32,
    pub a: f32,
}

impl Color4 {
    pub const fn new(r: f32, g: f32, b: f32, a: f32) -> Self {
        Self { r, g, b, a }
    }

    pub const fn rgb(r: f32, g: f32, b: f32) -> Self {
        Self { r, g, b, a: 1.0 }
    }

    pub const fn white() -> Self {
        Self { r: 1.0, g: 1.0, b: 1.0, a: 1.0 }
    }

    pub const fn red() -> Self {
        Self { r: 1.0, g: 0.0, b: 0.0, a: 1.0 }
    }

    pub const fn green() -> Self {
        Self { r: 0.0, g: 1.0, b: 0.0, a: 1.0 }
    }

    pub const fn blue() -> Self {
        Self { r: 0.0, g: 0.0, b: 1.0, a: 1.0 }
    }

    /// Scales color intensity by a lighting factor.
    #[inline(always)]
    pub fn scale(self, s: f32) -> Color4 {
        Color4::new(
            (self.r * s).clamp(0.0, 1.0),
            (self.g * s).clamp(0.0, 1.0),
            (self.b * s).clamp(0.0, 1.0),
            self.a,
        )
    }

    /// Converts to standard packed 32-bit BGR/BGRA integer pixel format.
    #[inline(always)]
    pub fn to_u32_bgr(self) -> u32 {
        let b = (self.b * 255.0).clamp(0.0, 255.0) as u32;
        let g = (self.g * 255.0).clamp(0.0, 255.0) as u32;
        let r = (self.r * 255.0).clamp(0.0, 255.0) as u32;
        let a = (self.a * 255.0).clamp(0.0, 255.0) as u32;
        (a << 24) | (r << 16) | (g << 8) | b
    }

    /// Converts to standard packed 32-bit RGB/RGBA integer pixel format.
    #[inline(always)]
    pub fn to_u32_rgb(self) -> u32 {
        let r = (self.r * 255.0).clamp(0.0, 255.0) as u32;
        let g = (self.g * 255.0).clamp(0.0, 255.0) as u32;
        let b = (self.b * 255.0).clamp(0.0, 255.0) as u32;
        let a = (self.a * 255.0).clamp(0.0, 255.0) as u32;
        (a << 24) | (b << 16) | (g << 8) | r
    }
}

// Vertex3D & Triangle3D Structures

#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Vertex3D {
    pub position: Vec4,
    pub color: Color4,
    pub normal: Vec3,
    pub uv: [f32; 2],
}

impl Vertex3D {
    pub const fn new(position: Vec4, color: Color4, normal: Vec3) -> Self {
        Self {
            position,
            color,
            normal,
            uv: [0.0, 0.0],
        }
    }
}

#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Triangle3D {
    pub vertices: [Vertex3D; 3],
}

impl Triangle3D {
    pub const fn new(v0: Vertex3D, v1: Vertex3D, v2: Vertex3D) -> Self {
        Self {
            vertices: [v0, v1, v2],
        }
    }

    /// Computes the unnormalized or normalized geometric surface normal.
    pub fn face_normal(&self) -> Vec3 {
        let p0 = self.vertices[0].position.to_vec3();
        let p1 = self.vertices[1].position.to_vec3();
        let p2 = self.vertices[2].position.to_vec3();

        let edge1 = p1.sub(p0);
        let edge2 = p2.sub(p0);
        edge1.cross(edge2).normalize()
    }
}

// Unit Tests for 3D Math & Structures

/// Verification unit test for 3D vector, matrix, vertex, and triangle operations.
pub fn test_3d_math_and_structures() -> bool {
    // 1. Vector operations
    let v1 = Vec3::new(1.0, 2.0, 3.0);
    let v2 = Vec3::new(4.0, 5.0, 6.0);
    assert_eq!(v1.add(v2), Vec3::new(5.0, 7.0, 9.0));
    assert_eq!(v2.sub(v1), Vec3::new(3.0, 3.0, 3.0));
    assert_eq!(v1.scale(2.0), Vec3::new(2.0, 4.0, 6.0));
    assert_eq!(v1.dot(v2), 32.0); // 1*4 + 2*5 + 3*6 = 4 + 10 + 18 = 32

    // Cross product: (1, 0, 0) x (0, 1, 0) = (0, 0, 1)
    let vx = Vec3::new(1.0, 0.0, 0.0);
    let vy = Vec3::new(0.0, 1.0, 0.0);
    let vz = vx.cross(vy);
    assert_eq!(vz, Vec3::new(0.0, 0.0, 1.0));
    assert_eq!(vz.length(), 1.0);
    assert_eq!(vz.normalize(), Vec3::new(0.0, 0.0, 1.0));

    // 2. Matrix operations
    let ident = Mat4::identity();
    let p = Vec4::new(10.0, 20.0, 30.0, 1.0);
    assert_eq!(ident.mul_vec4(p), p);

    let trans = Mat4::translation(5.0, -2.0, 10.0);
    let p_trans = trans.mul_vec4(p);
    assert_eq!(p_trans, Vec4::new(15.0, 18.0, 40.0, 1.0));

    // Rotation Z by 90 degrees (HALF_PI): (1, 0, 0, 1) -> (0, 1, 0, 1)
    let rot_z = Mat4::rotation_z(HALF_PI);
    let p_rot = rot_z.mul_vec4(Vec4::new(1.0, 0.0, 0.0, 1.0));
    assert!((p_rot.x - 0.0).abs() < 1e-4);
    assert!((p_rot.y - 1.0).abs() < 1e-4);

    // Matrix multiplication: (trans * ident) == trans
    let comb = trans.mul_mat4(&ident);
    assert_eq!(comb, trans);

    // 3. Perspective projection
    let proj = Mat4::perspective(PI / 3.0, 4.0 / 3.0, 0.1, 100.0);
    assert!(proj.m[0][0] > 0.0);
    assert!(proj.m[1][1] > 0.0);
    assert_eq!(proj.m[3][2], -1.0);

    // 4. Color4 & Shading
    let c = Color4::new(1.0, 0.5, 0.0, 1.0);
    let bgr = c.to_u32_bgr();
    assert_eq!(bgr & 0xFF, 0);          // B
    assert_eq!((bgr >> 8) & 0xFF, 127); // G
    assert_eq!((bgr >> 16) & 0xFF, 255);// R

    // 5. Triangle3D & normal calculation
    let v0 = Vertex3D::new(Vec4::new(0.0, 0.0, 0.0, 1.0), Color4::white(), Vec3::zero());
    let v1 = Vertex3D::new(Vec4::new(1.0, 0.0, 0.0, 1.0), Color4::white(), Vec3::zero());
    let v2 = Vertex3D::new(Vec4::new(0.0, 1.0, 0.0, 1.0), Color4::white(), Vec3::zero());
    let tri = Triangle3D::new(v0, v1, v2);
    let norm = tri.face_normal();
    assert_eq!(norm, Vec3::new(0.0, 0.0, 1.0));

    true
}
