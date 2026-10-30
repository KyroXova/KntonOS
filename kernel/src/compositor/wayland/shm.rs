//! Wayland Shared Memory (`wl_shm`, `wl_shm_pool`, `wl_buffer`) & Damage Tracking.
//!
//! Implements zero-copy shared memory buffer pools, pixel format negotiations (ARGB8888, XRGB8888),
//! buffer attachments to surfaces, and damage tracking regions.

use alloc::vec::Vec;
use crate::compositor::dirty::{DamageRegion, Rect};

/// Pixel format supported by `wl_shm`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u32)]
pub enum WlShmFormat {
    Argb8888 = 0,
    Xrgb8888 = 1,
}

/// Representation of an active `wl_buffer`.
#[derive(Debug, Clone)]
pub struct WlBuffer {
    pub id: u32,
    pub pool_id: u32,
    pub offset: usize,
    pub width: i32,
    pub height: i32,
    pub stride: i32,
    pub format: WlShmFormat,
    pub busy: bool,
}

impl WlBuffer {
    pub fn new(
        id: u32,
        pool_id: u32,
        offset: usize,
        width: i32,
        height: i32,
        stride: i32,
        format: WlShmFormat,
    ) -> Self {
        Self {
            id,
            pool_id,
            offset,
            width,
            height,
            stride,
            format,
            busy: false,
        }
    }

    #[inline(always)]
    pub fn byte_size(&self) -> usize {
        (self.height.max(0) as usize) * (self.stride.max(0) as usize)
    }
}

/// Representation of a `wl_shm_pool` backing memory.
pub struct WlShmPool {
    pub id: u32,
    pub size: usize,
    pub memory: Vec<u8>,
}

impl WlShmPool {
    pub fn new(id: u32, size: usize) -> Self {
        let mut memory = Vec::with_capacity(size);
        memory.resize(size, 0);
        Self { id, size, memory }
    }

    pub fn resize(&mut self, new_size: usize) {
        if new_size > self.size {
            self.memory.resize(new_size, 0);
            self.size = new_size;
        }
    }

    pub fn create_buffer(
        &self,
        id: u32,
        offset: usize,
        width: i32,
        height: i32,
        stride: i32,
        format: WlShmFormat,
    ) -> Result<WlBuffer, &'static str> {
        let req_size = (height.max(0) as usize) * (stride.max(0) as usize);
        if offset + req_size > self.size {
            return Err("Buffer exceeds shared memory pool boundary");
        }
        Ok(WlBuffer::new(id, self.id, offset, width, height, stride, format))
    }
}

/// Per-surface pending and committed state for Wayland protocol.
#[derive(Debug, Clone)]
pub struct SurfaceState {
    pub attached_buffer: Option<u32>, // wl_buffer ID
    pub offset_x: i32,
    pub offset_y: i32,
    pub damage: DamageRegion,
    pub opaque_region: Option<Rect>,
    pub input_region: Option<Rect>,
    pub buffer_scale: i32,
}

impl SurfaceState {
    pub fn new() -> Self {
        Self {
            attached_buffer: None,
            offset_x: 0,
            offset_y: 0,
            damage: DamageRegion::new(),
            opaque_region: None,
            input_region: None,
            buffer_scale: 1,
        }
    }
}

impl Default for SurfaceState {
    fn default() -> Self {
        Self::new()
    }
}

/// Wayland surface (`wl_surface`) managing double-buffered state and damage.
pub struct WlSurface {
    pub id: u32,
    pub pending: SurfaceState,
    pub current: SurfaceState,
    pub frame_callbacks: Vec<u32>, // wl_callback object IDs waiting for next VBLANK/frame
}

impl WlSurface {
    pub fn new(id: u32) -> Self {
        Self {
            id,
            pending: SurfaceState::new(),
            current: SurfaceState::new(),
            frame_callbacks: Vec::new(),
        }
    }

    /// Attaches a `wl_buffer` to the surface in pending state.
    pub fn attach(&mut self, buffer_id: Option<u32>, x: i32, y: i32) {
        self.pending.attached_buffer = buffer_id;
        self.pending.offset_x = x;
        self.pending.offset_y = y;
    }

    /// Adds a damaged region to the pending state.
    pub fn damage(&mut self, x: i32, y: i32, width: i32, height: i32) {
        if width > 0 && height > 0 {
            self.pending.damage.add_rect(Rect::new(
                x.max(0) as usize,
                y.max(0) as usize,
                width as usize,
                height as usize,
            ));
        }
    }

    /// Requests a frame callback to be invoked on the next frame display presentation.
    pub fn frame(&mut self, callback_id: u32) {
        self.frame_callbacks.push(callback_id);
    }

    /// Commits the pending state into current state, making buffer and damage visible.
    pub fn commit(&mut self) -> DamageRegion {
        let committed_damage = self.pending.damage.clone();
        self.current = self.pending.clone();
        // Clear pending damage after commit
        self.pending.damage.clear();
        committed_damage
    }
}

/// Unit test verifying `wl_shm_pool`, `wl_buffer` creation, surface attachment, and damage tracking.
pub fn test_shm_and_surface_damage() -> bool {
    // 1. Create a 64KB shared memory pool
    let pool_size = 64 * 1024;
    let pool = WlShmPool::new(10, pool_size);

    // 2. Create a 100x100 ARGB8888 buffer (stride = 400 bytes, size = 40,000 bytes)
    let buf = match pool.create_buffer(20, 0, 100, 100, 400, WlShmFormat::Argb8888) {
        Ok(b) => b,
        Err(_) => return false,
    };

    if buf.id != 20 || buf.width != 100 || buf.height != 100 || buf.stride != 400 {
        return false;
    }
    if buf.byte_size() != 40000 {
        return false;
    }

    // Attempt buffer exceeding pool size (should fail gracefully)
    if pool.create_buffer(21, 30000, 100, 100, 400, WlShmFormat::Argb8888).is_ok() {
        return false;
    }

    // 3. Create a Wayland surface and test double-buffered commit lifecycle
    let mut surface = WlSurface::new(30);
    surface.attach(Some(buf.id), 0, 0);
    surface.damage(10, 10, 50, 50);
    surface.damage(40, 40, 30, 30);
    surface.frame(99);

    // Before commit, current state has no attached buffer
    if surface.current.attached_buffer.is_some() {
        return false;
    }
    if surface.current.damage.bounding_box().is_some() {
        return false;
    }

    // Commit pending state
    let damage_region = surface.commit();
    if surface.current.attached_buffer != Some(buf.id) {
        return false;
    }

    let bounds = match damage_region.bounding_box() {
        Some(b) => b,
        None => return false,
    };
    if bounds.x != 10 || bounds.y != 10 || bounds.width != 60 || bounds.height != 60 {
        return false;
    }

    // Pending damage must be cleared after commit
    if surface.pending.damage.bounding_box().is_some() {
        return false;
    }

    // Frame callback registered
    if surface.frame_callbacks.len() != 1 || surface.frame_callbacks[0] != 99 {
        return false;
    }

    true
}
