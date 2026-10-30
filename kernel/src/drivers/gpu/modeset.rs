//! DRM/KMS Modesetting and Hardware Display Pipeline Controller.
//!
//! Provides display modesetting configuration, linking CRTCs, encoders,
//! connectors, and framebuffers into an active scanout pipeline.
//! Implements atomic commit property table transactions (FB_ID, CRTC_ID,
//! coordinates, rotation, scaling filter, alpha, zpos) and direct-to-display
//! multi-plane overlay scanout presentation (primary, overlay, cursor).

extern crate alloc;
use alloc::vec;
use alloc::vec::Vec;

use super::drm::{
    DrmDevice, DrmModeInfo, DRM_MODE_ATOMIC_TEST_ONLY, DRM_MODE_CONNECTED, DRM_MODE_REFLECT_MASK,
    DRM_MODE_REFLECT_X, DRM_MODE_REFLECT_Y, DRM_MODE_ROTATE_MASK, DRM_PLANE_TYPE_CURSOR,
    DRM_PLANE_TYPE_OVERLAY, DRM_PLANE_TYPE_PRIMARY, DRM_PROP_ACTIVE, DRM_PROP_ALPHA,
    DRM_PROP_CRTC_H, DRM_PROP_CRTC_ID, DRM_PROP_CRTC_W, DRM_PROP_CRTC_X, DRM_PROP_CRTC_Y,
    DRM_PROP_FB_ID, DRM_PROP_ROTATION, DRM_PROP_SCALING_FILTER, DRM_PROP_SRC_H, DRM_PROP_SRC_W,
    DRM_PROP_SRC_X, DRM_PROP_SRC_Y, DRM_PROP_ZPOS, DRM_SCALING_FILTER_LINEAR,
};

/// Modesetting status and error codes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModesetError {
    InvalidCrtc,
    InvalidConnector,
    InvalidEncoder,
    InvalidFramebuffer,
    ConnectorDisconnected,
    IncompatibleCrtc,
    UnsupportedMode,
    AtomicCheckFailed,
}

/// Legacy / baseline modeset configuration state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DrmModeSet {
    pub crtc_id: u32,
    pub fb_id: u32,
    pub connector_ids: [u32; 2],
    pub connector_count: usize,
    pub mode: DrmModeInfo,
    pub x: u32,
    pub y: u32,
}

impl DrmModeSet {
    pub const fn new(crtc_id: u32, fb_id: u32, mode: DrmModeInfo) -> Self {
        Self {
            crtc_id,
            fb_id,
            connector_ids: [0, 0],
            connector_count: 0,
            mode,
            x: 0,
            y: 0,
        }
    }

    pub fn add_connector(&mut self, connector_id: u32) -> bool {
        if self.connector_count < self.connector_ids.len() {
            self.connector_ids[self.connector_count] = connector_id;
            self.connector_count += 1;
            true
        } else {
            false
        }
    }

    /// Validates and applies this modeset configuration onto the DRM device.
    pub fn apply(&self, dev: &mut DrmDevice) -> Result<(), ModesetError> {
        let _crtc_index = {
            let crtc = dev.find_crtc(self.crtc_id).ok_or(ModesetError::InvalidCrtc)?;
            crtc.index
        };

        if self.fb_id != 0 {
            let _fb = dev.find_framebuffer(self.fb_id).ok_or(ModesetError::InvalidFramebuffer)?;
        }

        // Validate all target connectors
        for i in 0..self.connector_count {
            let conn_id = self.connector_ids[i];
            let mut found = false;
            for slot in dev.connectors.iter() {
                if let Some(ref conn) = slot {
                    if conn.id == conn_id {
                        found = true;
                        if conn.connection_status != DRM_MODE_CONNECTED {
                            return Err(ModesetError::ConnectorDisconnected);
                        }
                    }
                }
            }
            if !found {
                return Err(ModesetError::InvalidConnector);
            }
        }

        // Configure CRTC with target mode and active framebuffer
        if let Some(crtc) = dev.find_crtc_mut(self.crtc_id) {
            crtc.set_mode(self.mode);
            crtc.current_fb_id = self.fb_id;
            crtc.x = self.x;
            crtc.y = self.y;
        }

        Ok(())
    }
}

// -----------------------------------------------------------------------------
// DRM Atomic Modesetting Property Table & Transaction Engine
// -----------------------------------------------------------------------------

pub const MAX_ATOMIC_PROPERTIES: usize = 32;

/// A single key-value property modification target in an atomic transaction.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DrmPropertyValue {
    pub obj_id: u32,
    pub prop_id: u32,
    pub value: u64,
}

/// Represents an atomic batch of property modifications across KMS objects.
#[derive(Debug, Clone, Copy)]
pub struct DrmAtomicReq {
    pub props: [DrmPropertyValue; MAX_ATOMIC_PROPERTIES],
    pub prop_count: usize,
    pub flags: u32,
}

impl DrmAtomicReq {
    pub const fn new() -> Self {
        Self {
            props: [DrmPropertyValue { obj_id: 0, prop_id: 0, value: 0 }; MAX_ATOMIC_PROPERTIES],
            prop_count: 0,
            flags: 0,
        }
    }

    pub fn add_property(&mut self, obj_id: u32, prop_id: u32, value: u64) -> Result<(), ModesetError> {
        if self.prop_count < MAX_ATOMIC_PROPERTIES {
            self.props[self.prop_count] = DrmPropertyValue { obj_id, prop_id, value };
            self.prop_count += 1;
            Ok(())
        } else {
            Err(ModesetError::AtomicCheckFailed)
        }
    }

    pub fn set_plane_fb(&mut self, plane_id: u32, fb_id: u32) -> Result<(), ModesetError> {
        self.add_property(plane_id, DRM_PROP_FB_ID, fb_id as u64)
    }

    pub fn set_plane_crtc(&mut self, plane_id: u32, crtc_id: u32) -> Result<(), ModesetError> {
        self.add_property(plane_id, DRM_PROP_CRTC_ID, crtc_id as u64)
    }

    pub fn set_plane_dest(&mut self, plane_id: u32, x: i32, y: i32, w: u32, h: u32) -> Result<(), ModesetError> {
        self.add_property(plane_id, DRM_PROP_CRTC_X, x as u32 as u64)?;
        self.add_property(plane_id, DRM_PROP_CRTC_Y, y as u32 as u64)?;
        self.add_property(plane_id, DRM_PROP_CRTC_W, w as u64)?;
        self.add_property(plane_id, DRM_PROP_CRTC_H, h as u64)?;
        Ok(())
    }

    pub fn set_plane_src(&mut self, plane_id: u32, x: u32, y: u32, w: u32, h: u32) -> Result<(), ModesetError> {
        // DRM specification: source coordinates in 16.16 fixed point format
        self.add_property(plane_id, DRM_PROP_SRC_X, (x as u64) << 16)?;
        self.add_property(plane_id, DRM_PROP_SRC_Y, (y as u64) << 16)?;
        self.add_property(plane_id, DRM_PROP_SRC_W, (w as u64) << 16)?;
        self.add_property(plane_id, DRM_PROP_SRC_H, (h as u64) << 16)?;
        Ok(())
    }

    pub fn set_plane_rotation(&mut self, plane_id: u32, rotation: u32) -> Result<(), ModesetError> {
        self.add_property(plane_id, DRM_PROP_ROTATION, rotation as u64)
    }

    pub fn set_plane_scaling_filter(&mut self, plane_id: u32, filter: u32) -> Result<(), ModesetError> {
        self.add_property(plane_id, DRM_PROP_SCALING_FILTER, filter as u64)
    }

    pub fn set_plane_alpha(&mut self, plane_id: u32, alpha: u16) -> Result<(), ModesetError> {
        self.add_property(plane_id, DRM_PROP_ALPHA, alpha as u64)
    }

    pub fn set_plane_zpos(&mut self, plane_id: u32, zpos: u32) -> Result<(), ModesetError> {
        self.add_property(plane_id, DRM_PROP_ZPOS, zpos as u64)
    }

    pub fn set_crtc_active(&mut self, crtc_id: u32, active: bool) -> Result<(), ModesetError> {
        self.add_property(crtc_id, DRM_PROP_ACTIVE, if active { 1 } else { 0 })
    }

    /// Evaluates and atomically applies (or tests) all property assignments across KMS objects.
    pub fn commit(&self, dev: &mut DrmDevice, flags: u32) -> Result<(), ModesetError> {
        // 1. Identify and stage plane state updates
        for slot in dev.planes.iter() {
            if let Some(ref plane) = slot {
                let plane_id = plane.id;

                // Check if this plane is addressed in the atomic request
                let mut has_updates = false;
                let mut target_fb_id = plane.fb_id;
                let mut target_crtc_id = plane.crtc_id;
                let mut target_crtc_x = plane.crtc_x;
                let mut target_crtc_y = plane.crtc_y;
                let mut target_crtc_w = plane.crtc_w;
                let mut target_crtc_h = plane.crtc_h;
                let mut target_src_x = plane.src_x;
                let mut target_src_y = plane.src_y;
                let mut target_src_w = plane.src_w;
                let mut target_src_h = plane.src_h;
                let mut target_rotation = plane.rotation;
                let mut target_filter = plane.scaling_filter;
                let mut target_alpha = plane.alpha;
                let mut target_zpos = plane.zpos;

                for i in 0..self.prop_count {
                    let p = &self.props[i];
                    if p.obj_id == plane_id {
                        has_updates = true;
                        match p.prop_id {
                            DRM_PROP_FB_ID => target_fb_id = p.value as u32,
                            DRM_PROP_CRTC_ID => target_crtc_id = p.value as u32,
                            DRM_PROP_CRTC_X => target_crtc_x = p.value as i32,
                            DRM_PROP_CRTC_Y => target_crtc_y = p.value as i32,
                            DRM_PROP_CRTC_W => target_crtc_w = p.value as u32,
                            DRM_PROP_CRTC_H => target_crtc_h = p.value as u32,
                            DRM_PROP_SRC_X => target_src_x = p.value as u32,
                            DRM_PROP_SRC_Y => target_src_y = p.value as u32,
                            DRM_PROP_SRC_W => target_src_w = p.value as u32,
                            DRM_PROP_SRC_H => target_src_h = p.value as u32,
                            DRM_PROP_ROTATION => target_rotation = p.value as u32,
                            DRM_PROP_SCALING_FILTER => target_filter = p.value as u32,
                            DRM_PROP_ALPHA => target_alpha = p.value as u16,
                            DRM_PROP_ZPOS => target_zpos = p.value as u32,
                            _ => {}
                        }
                    }
                }

                if has_updates && target_fb_id != 0 {
                    // Validate CRTC
                    let crtc = dev.find_crtc(target_crtc_id).ok_or(ModesetError::InvalidCrtc)?;
                    if !plane.is_compatible_crtc(crtc.index) {
                        return Err(ModesetError::IncompatibleCrtc);
                    }

                    // Validate Framebuffer
                    let fb = dev.find_framebuffer(target_fb_id).ok_or(ModesetError::InvalidFramebuffer)?;
                    if !plane.supports_format(fb.pixel_format) {
                        return Err(ModesetError::AtomicCheckFailed);
                    }

                    // Validate Source Coordinates (16.16 fixed-point)
                    if target_src_w == 0 || target_src_h == 0 {
                        return Err(ModesetError::AtomicCheckFailed);
                    }
                    let src_x_px = target_src_x >> 16;
                    let src_y_px = target_src_y >> 16;
                    let src_w_px = target_src_w >> 16;
                    let src_h_px = target_src_h >> 16;

                    if src_x_px + src_w_px > fb.width || src_y_px + src_h_px > fb.height {
                        return Err(ModesetError::AtomicCheckFailed);
                    }

                    // Validate Destination Coordinates
                    if target_crtc_w == 0 || target_crtc_h == 0 {
                        return Err(ModesetError::AtomicCheckFailed);
                    }

                    // If CRTC has an active mode, check that destination rect intersects the screen
                    if let Some(ref mode) = crtc.mode {
                        if target_crtc_x + (target_crtc_w as i32) <= 0
                            || target_crtc_x >= (mode.hdisplay as i32)
                            || target_crtc_y + (target_crtc_h as i32) <= 0
                            || target_crtc_y >= (mode.vdisplay as i32)
                        {
                            return Err(ModesetError::AtomicCheckFailed);
                        }
                    }

                    // Validate Scaling bounds (up to 8x downscale or 8x upscale)
                    if src_w_px > target_crtc_w * 8 || target_crtc_w > src_w_px * 8 {
                        return Err(ModesetError::AtomicCheckFailed);
                    }
                    if src_h_px > target_crtc_h * 8 || target_crtc_h > src_h_px * 8 {
                        return Err(ModesetError::AtomicCheckFailed);
                    }

                    // Validate Rotation Flags
                    let rot_angle = target_rotation & DRM_MODE_ROTATE_MASK;
                    if rot_angle == 0 || (target_rotation & !(DRM_MODE_ROTATE_MASK | DRM_MODE_REFLECT_MASK)) != 0 {
                        return Err(ModesetError::AtomicCheckFailed);
                    }

                    // Validate Scaling Filter
                    if target_filter > DRM_SCALING_FILTER_LINEAR {
                        return Err(ModesetError::AtomicCheckFailed);
                    }

                    // Validate ZPOS & Alpha
                    if target_zpos > 16 {
                        return Err(ModesetError::AtomicCheckFailed);
                    }
                    let _ = target_alpha;
                }
            }
        }

        // 2. Validate CRTC state updates
        for slot in dev.crtcs.iter() {
            if let Some(ref crtc) = slot {
                for i in 0..self.prop_count {
                    let p = &self.props[i];
                    if p.obj_id == crtc.id {
                        match p.prop_id {
                            DRM_PROP_ACTIVE => {
                                // If activating CRTC, ensure it has a valid mode configured
                                if p.value != 0 && crtc.mode.is_none() {
                                    return Err(ModesetError::UnsupportedMode);
                                }
                            }
                            _ => {}
                        }
                    }
                }
            }
        }

        // If TEST_ONLY, exit immediately without modifying device state
        if (flags & DRM_MODE_ATOMIC_TEST_ONLY) != 0 {
            return Ok(());
        }

        // 3. Atomically apply changes to device state
        for i in 0..self.prop_count {
            let p = &self.props[i];

            // Try plane first
            if let Some(plane) = dev.find_plane_mut(p.obj_id) {
                match p.prop_id {
                    DRM_PROP_FB_ID => plane.fb_id = p.value as u32,
                    DRM_PROP_CRTC_ID => plane.crtc_id = p.value as u32,
                    DRM_PROP_CRTC_X => plane.crtc_x = p.value as i32,
                    DRM_PROP_CRTC_Y => plane.crtc_y = p.value as i32,
                    DRM_PROP_CRTC_W => plane.crtc_w = p.value as u32,
                    DRM_PROP_CRTC_H => plane.crtc_h = p.value as u32,
                    DRM_PROP_SRC_X => plane.src_x = p.value as u32,
                    DRM_PROP_SRC_Y => plane.src_y = p.value as u32,
                    DRM_PROP_SRC_W => plane.src_w = p.value as u32,
                    DRM_PROP_SRC_H => plane.src_h = p.value as u32,
                    DRM_PROP_ROTATION => plane.rotation = p.value as u32,
                    DRM_PROP_SCALING_FILTER => plane.scaling_filter = p.value as u32,
                    DRM_PROP_ALPHA => plane.alpha = p.value as u16,
                    DRM_PROP_ZPOS => plane.zpos = p.value as u32,
                    _ => {}
                }
            } else if let Some(crtc) = dev.find_crtc_mut(p.obj_id) {
                match p.prop_id {
                    DRM_PROP_ACTIVE => {
                        crtc.active = p.value != 0;
                        crtc.enabled = p.value != 0;
                    }
                    _ => {}
                }
            }
        }

        Ok(())
    }
}

// -----------------------------------------------------------------------------
// Direct-to-Display Multi-Plane Overlay Scanout Engine
// -----------------------------------------------------------------------------

/// Output hardware scanout surface (CRTC target scanout memory).
pub struct DrmScanoutSurface {
    pub width: usize,
    pub height: usize,
    pub pitch: usize,
    pub buffer: Vec<u32>,
}

impl DrmScanoutSurface {
    pub fn new(width: usize, height: usize) -> Self {
        let size = width * height;
        Self {
            width,
            height,
            pitch: width * 4,
            buffer: vec![0u32; size],
        }
    }

    #[inline(always)]
    pub fn clear(&mut self, color: u32) {
        self.buffer.fill(color);
    }

    #[inline(always)]
    pub fn get_pixel(&self, x: usize, y: usize) -> u32 {
        if x < self.width && y < self.height {
            self.buffer[y * self.width + x]
        } else {
            0
        }
    }

    #[inline(always)]
    pub fn set_pixel(&mut self, x: usize, y: usize, color: u32) {
        if x < self.width && y < self.height {
            self.buffer[y * self.width + x] = color;
        }
    }
}

/// Dirty rectangle bounding box tracker for scanout flushes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DrmDirtyTracker {
    pub min_x: usize,
    pub min_y: usize,
    pub max_x: usize,
    pub max_y: usize,
    pub is_dirty: bool,
}

impl DrmDirtyTracker {
    pub const fn new() -> Self {
        Self {
            min_x: usize::MAX,
            min_y: usize::MAX,
            max_x: 0,
            max_y: 0,
            is_dirty: false,
        }
    }

    pub fn reset(&mut self) {
        self.min_x = usize::MAX;
        self.min_y = usize::MAX;
        self.max_x = 0;
        self.max_y = 0;
        self.is_dirty = false;
    }

    pub fn mark_rect(&mut self, x0: usize, y0: usize, x1: usize, y1: usize) {
        if x0 < self.min_x { self.min_x = x0; }
        if y0 < self.min_y { self.min_y = y0; }
        if x1 > self.max_x { self.max_x = x1; }
        if y1 > self.max_y { self.max_y = y1; }
        self.is_dirty = true;
    }

    pub fn dirty_rect(&self) -> Option<(usize, usize, usize, usize)> {
        if self.is_dirty && self.min_x <= self.max_x && self.min_y <= self.max_y {
            Some((self.min_x, self.min_y, self.max_x, self.max_y))
        } else {
            None
        }
    }
}

/// In-memory descriptor of a single plane submitted for hardware presentation.
pub struct DrmPlanePresentation {
    pub plane_id: u32,
    pub plane_type: u32,
    pub zpos: u32,
    pub alpha: u16, // 0xFFFF = 100% opaque, 0 = 0%
    pub rotation: u32,
    pub scaling_filter: u32,
    pub crtc_x: i32,
    pub crtc_y: i32,
    pub crtc_w: u32,
    pub crtc_h: u32,
    pub src_x: u32, // 16.16 fixed-point
    pub src_y: u32,
    pub src_w: u32,
    pub src_h: u32,
    pub fb_width: u32,
    pub fb_height: u32,
    pub pixels: Vec<u32>,
}

impl DrmPlanePresentation {
    pub fn new(
        plane_id: u32,
        plane_type: u32,
        zpos: u32,
        alpha: u16,
        rotation: u32,
        scaling_filter: u32,
        crtc_x: i32,
        crtc_y: i32,
        crtc_w: u32,
        crtc_h: u32,
        src_w: u32,
        src_h: u32,
        fb_width: u32,
        fb_height: u32,
        pixels: Vec<u32>,
    ) -> Self {
        Self {
            plane_id,
            plane_type,
            zpos,
            alpha,
            rotation,
            scaling_filter,
            crtc_x,
            crtc_y,
            crtc_w,
            crtc_h,
            src_x: 0,
            src_y: 0,
            src_w: src_w << 16,
            src_h: src_h << 16,
            fb_width,
            fb_height,
            pixels,
        }
    }
}

/// Direct-to-Display Multi-Plane Scanout Compositor.
pub struct DrmScanoutCompositor;

impl DrmScanoutCompositor {
    /// Composes multiple hardware planes into a target scanout surface in z-order.
    ///
    /// Evaluates plane coordinates, clipping against the scanout display boundaries,
    /// applies scaling and rotation/reflection, performs alpha compositing,
    /// and tracks the dirty bounding rectangle for low-latency hardware flush.
    pub fn compose_planes(
        scanout: &mut DrmScanoutSurface,
        planes: &mut [DrmPlanePresentation],
        tracker: &mut DrmDirtyTracker,
    ) -> usize {
        // 1. Sort active planes by z-order ascending (Primary z=0 -> Overlay z=1 -> Cursor z=2)
        let plane_len = planes.len();
        for i in 0..plane_len {
            for j in (i + 1)..plane_len {
                if planes[j].zpos < planes[i].zpos {
                    planes.swap(i, j);
                }
            }
        }

        let mut rendered_planes = 0;

        // 2. Render planes onto scanout surface
        for plane in planes.iter() {
            if plane.crtc_w == 0 || plane.crtc_h == 0 || plane.pixels.is_empty() {
                continue;
            }

            let crtc_x = plane.crtc_x;
            let crtc_y = plane.crtc_y;
            let crtc_w = plane.crtc_w as i32;
            let crtc_h = plane.crtc_h as i32;

            // Clip destination rectangle against scanout boundaries
            let clip_x0 = crtc_x.max(0) as usize;
            let clip_y0 = crtc_y.max(0) as usize;
            let clip_x1 = (crtc_x + crtc_w).max(0).min(scanout.width as i32) as usize;
            let clip_y1 = (crtc_y + crtc_h).max(0).min(scanout.height as i32) as usize;

            if clip_x0 >= clip_x1 || clip_y0 >= clip_y1 {
                continue;
            }

            tracker.mark_rect(clip_x0, clip_y0, clip_x1, clip_y1);

            let src_x_px = (plane.src_x >> 16) as usize;
            let src_y_px = (plane.src_y >> 16) as usize;
            let src_w_px = ((plane.src_w >> 16) as usize).max(1);
            let src_h_px = ((plane.src_h >> 16) as usize).max(1);

            let global_alpha = (plane.alpha >> 8) as u32; // 0..255

            for dy in clip_y0..clip_y1 {
                let rel_y = (dy as i32 - crtc_y) as usize;
                let mut sy = (rel_y * src_h_px) / (plane.crtc_h as usize) + src_y_px;
                if (plane.rotation & DRM_MODE_REFLECT_Y) != 0 && sy < src_h_px {
                    sy = (src_h_px - 1).saturating_sub(sy);
                }
                sy = sy.min(plane.fb_height as usize - 1);

                for dx in clip_x0..clip_x1 {
                    let rel_x = (dx as i32 - crtc_x) as usize;
                    let mut sx = (rel_x * src_w_px) / (plane.crtc_w as usize) + src_x_px;
                    if (plane.rotation & DRM_MODE_REFLECT_X) != 0 && sx < src_w_px {
                        sx = (src_w_px - 1).saturating_sub(sx);
                    }
                    sx = sx.min(plane.fb_width as usize - 1);

                    let src_idx = sy * (plane.fb_width as usize) + sx;
                    if src_idx >= plane.pixels.len() {
                        continue;
                    }
                    let src_pixel = plane.pixels[src_idx];

                    // Primary plane: opaque write
                    if plane.plane_type == DRM_PLANE_TYPE_PRIMARY && plane.alpha == 0xFFFF {
                        scanout.set_pixel(dx, dy, src_pixel);
                    } else {
                        // Per-pixel alpha compositing for Overlay & Cursor planes
                        let sa = (src_pixel >> 24) & 0xFF;
                        let eff_alpha = (sa * global_alpha) / 255;

                        if eff_alpha == 0 {
                            // Transparent, keep background
                        } else if eff_alpha == 255 {
                            // Fully opaque
                            scanout.set_pixel(dx, dy, (0xFF << 24) | (src_pixel & 0x00FF_FFFF));
                        } else {
                            // Blend with existing scanout pixel
                            let dst_pixel = scanout.get_pixel(dx, dy);
                            let sr = (src_pixel >> 16) & 0xFF;
                            let sg = (src_pixel >> 8) & 0xFF;
                            let sb = src_pixel & 0xFF;

                            let dr = (dst_pixel >> 16) & 0xFF;
                            let dg = (dst_pixel >> 8) & 0xFF;
                            let db = dst_pixel & 0xFF;

                            let inv_a = 255 - eff_alpha;
                            let out_r = (sr * eff_alpha + dr * inv_a) / 255;
                            let out_g = (sg * eff_alpha + dg * inv_a) / 255;
                            let out_b = (sb * eff_alpha + db * inv_a) / 255;

                            scanout.set_pixel(dx, dy, (0xFF << 24) | (out_r << 16) | (out_g << 8) | out_b);
                        }
                    }
                }
            }

            rendered_planes += 1;
        }

        rendered_planes
    }
}

// -----------------------------------------------------------------------------
// Unit Verification for Multi-Plane Presentation
// -----------------------------------------------------------------------------

/// Unit test verifying multi-plane overlay presentation (primary surface, overlay plane, cursor plane).
pub fn verify_multi_plane_overlay() -> bool {
    let mut scanout = DrmScanoutSurface::new(128, 128);
    let mut tracker = DrmDirtyTracker::new();

    // 1. Primary Plane: 128x128 solid dark blue (0xFF000088)
    let primary_pixels = vec![0xFF00_0088u32; 128 * 128];
    let primary = DrmPlanePresentation::new(
        1,
        DRM_PLANE_TYPE_PRIMARY,
        0, // zpos 0
        0xFFFF,
        0,
        0,
        0,
        0,
        128,
        128,
        128,
        128,
        128,
        128,
        primary_pixels,
    );

    // 2. Hardware Overlay Plane: 32x32 video surface (green 0xFF00FF00 with 50% alpha 0x8000),
    // positioned at (20, 20) with 2x hardware upscaling to 64x64
    let overlay_pixels = vec![0xFF00_FF00u32; 32 * 32];
    let overlay = DrmPlanePresentation::new(
        2,
        DRM_PLANE_TYPE_OVERLAY,
        1, // zpos 1
        0x8000, // 50% alpha
        0,
        DRM_SCALING_FILTER_LINEAR,
        20,
        20,
        64,
        64,
        32,
        32,
        32,
        32,
        overlay_pixels,
    );

    // 3. Hardware Cursor Plane: 16x16 cursor with crosshair (white 0xFFFFFFFF),
    // positioned at (30, 30) on top (zpos 2)
    let mut cursor_pixels = vec![0x0000_0000u32; 16 * 16]; // Default transparent
    for i in 0..16 {
        cursor_pixels[8 * 16 + i] = 0xFFFF_FFFF; // Horizontal bar
        cursor_pixels[i * 16 + 8] = 0xFFFF_FFFF; // Vertical bar
    }
    let cursor = DrmPlanePresentation::new(
        3,
        DRM_PLANE_TYPE_CURSOR,
        2, // zpos 2
        0xFFFF,
        0,
        0,
        30,
        30,
        16,
        16,
        16,
        16,
        16,
        16,
        cursor_pixels,
    );

    let mut planes = [primary, overlay, cursor];

    // 4. Compose multi-plane overlay presentation
    let count = DrmScanoutCompositor::compose_planes(&mut scanout, &mut planes, &mut tracker);
    assert_eq!(count, 3);

    // 5. Verify pixel output and z-ordering
    // A) Background area (5, 5) -> pure primary dark blue (0xFF000088)
    let bg_pixel = scanout.get_pixel(5, 5);
    assert_eq!(bg_pixel, 0xFF00_0088);

    // B) Overlay area outside cursor (25, 25) -> blend of green and dark blue:
    // Green (0, 255, 0) * 0.5 + Dark Blue (0, 0, 136) * 0.5 = (0, 127, 68)
    let overlay_pixel = scanout.get_pixel(25, 25);
    let r = (overlay_pixel >> 16) & 0xFF;
    let g = (overlay_pixel >> 8) & 0xFF;
    let b = overlay_pixel & 0xFF;
    assert_eq!(r, 0);
    assert!(g >= 120 && g <= 135);
    assert!(b >= 60 && b <= 75);

    // C) Cursor center crosshair at (30 + 8 = 38, 30 + 8 = 38) -> opaque white (0xFFFFFFFF)
    let cursor_pixel = scanout.get_pixel(38, 38);
    assert_eq!(cursor_pixel, 0xFFFF_FFFF);

    // D) Dirty tracker covers the full composited area
    assert!(tracker.is_dirty);
    let dirty = tracker.dirty_rect().expect("Dirty rect not recorded");
    assert_eq!(dirty.0, 0);
    assert_eq!(dirty.1, 0);
    assert_eq!(dirty.2, 128);
    assert_eq!(dirty.3, 128);

    true
}

// -----------------------------------------------------------------------------
// Unit Verification for Atomic Modesetting Engine
// -----------------------------------------------------------------------------

/// Unit test verifying DRM atomic commit property tables, dry-run checks, and atomic state updates.
pub fn verify_atomic_commit_engine() -> bool {
    use super::drm::{
        DrmCrtc, DrmFramebuffer, DrmPlane, DRM_FORMAT_ARGB8888, DRM_FORMAT_XRGB8888,
        DRM_MODE_REFLECT_X, DRM_MODE_ROTATE_0, DRM_PLANE_TYPE_OVERLAY, DRM_PLANE_TYPE_PRIMARY,
        MODE_1920X1080_60,
    };

    let mut dev = DrmDevice::new();

    // 1. Setup CRTC
    let mut crtc = DrmCrtc::new(10, 0);
    crtc.set_mode(MODE_1920X1080_60);
    assert!(dev.register_crtc(crtc));

    // 2. Setup Primary Plane & Overlay Plane
    let mut primary_plane = DrmPlane::new(20, DRM_PLANE_TYPE_PRIMARY, 1 << 0);
    assert!(primary_plane.add_format(DRM_FORMAT_XRGB8888));
    assert!(dev.register_plane(primary_plane));

    let mut overlay_plane = DrmPlane::new(21, DRM_PLANE_TYPE_OVERLAY, 1 << 0);
    assert!(overlay_plane.add_format(DRM_FORMAT_ARGB8888));
    assert!(dev.register_plane(overlay_plane));

    // 3. Setup Framebuffers
    let fb_primary = DrmFramebuffer::new(50, 1920, 1080, DRM_FORMAT_XRGB8888, 1920 * 4, 0x1000_0000, 0);
    assert!(dev.register_framebuffer(fb_primary));

    let fb_video = DrmFramebuffer::new(51, 640, 480, DRM_FORMAT_ARGB8888, 640 * 4, 0x2000_0000, 0);
    assert!(dev.register_framebuffer(fb_video));

    // 4. Construct Atomic Request configuring primary and overlay planes
    let mut req = DrmAtomicReq::new();
    assert!(req.set_plane_fb(20, 50).is_ok());
    assert!(req.set_plane_crtc(20, 10).is_ok());
    assert!(req.set_plane_dest(20, 0, 0, 1920, 1080).is_ok());
    assert!(req.set_plane_src(20, 0, 0, 1920, 1080).is_ok());
    assert!(req.set_plane_rotation(20, DRM_MODE_ROTATE_0).is_ok());

    // Configure video overlay plane with 2x hardware upscaling (640x480 -> 1280x960) at (100, 50)
    assert!(req.set_plane_fb(21, 51).is_ok());
    assert!(req.set_plane_crtc(21, 10).is_ok());
    assert!(req.set_plane_dest(21, 100, 50, 1280, 960).is_ok());
    assert!(req.set_plane_src(21, 0, 0, 640, 480).is_ok());
    assert!(req.set_plane_rotation(21, DRM_MODE_ROTATE_0 | DRM_MODE_REFLECT_X).is_ok());
    assert!(req.set_plane_scaling_filter(21, DRM_SCALING_FILTER_LINEAR).is_ok());
    assert!(req.set_plane_alpha(21, 0xE000).is_ok());
    assert!(req.set_plane_zpos(21, 1).is_ok());

    // 5. Test Dry-Run (DRM_MODE_ATOMIC_TEST_ONLY) -> Must succeed without modifying plane state
    let test_res = req.commit(&mut dev, DRM_MODE_ATOMIC_TEST_ONLY);
    assert!(test_res.is_ok());
    let p20_unmodified = dev.find_plane(20).unwrap();
    assert_eq!(p20_unmodified.fb_id, 0); // Not modified!

    // 6. Test Error Cases
    // A) Source rect exceeding framebuffer bounds
    let mut bad_req = DrmAtomicReq::new();
    assert!(bad_req.set_plane_fb(20, 50).is_ok());
    assert!(bad_req.set_plane_crtc(20, 10).is_ok());
    assert!(bad_req.set_plane_dest(20, 0, 0, 1920, 1080).is_ok());
    assert!(bad_req.set_plane_src(20, 0, 0, 2000, 1080).is_ok()); // Exceeds 1920 width!
    assert_eq!(bad_req.commit(&mut dev, DRM_MODE_ATOMIC_TEST_ONLY), Err(ModesetError::AtomicCheckFailed));

    // B) Incompatible CRTC
    let mut bad_req_crtc = DrmAtomicReq::new();
    assert!(bad_req_crtc.set_plane_fb(20, 50).is_ok());
    assert!(bad_req_crtc.set_plane_crtc(20, 99).is_ok()); // Nonexistent CRTC!
    assert_eq!(bad_req_crtc.commit(&mut dev, DRM_MODE_ATOMIC_TEST_ONLY), Err(ModesetError::InvalidCrtc));

    // C) Excessive downscaling (> 8x)
    let mut bad_req_scale = DrmAtomicReq::new();
    assert!(bad_req_scale.set_plane_fb(20, 50).is_ok());
    assert!(bad_req_scale.set_plane_crtc(20, 10).is_ok());
    assert!(bad_req_scale.set_plane_src(20, 0, 0, 1920, 1080).is_ok());
    assert!(bad_req_scale.set_plane_dest(20, 0, 0, 100, 1080).is_ok()); // 1920 -> 100 is 19.2x downscale (> 8x)
    assert_eq!(bad_req_scale.commit(&mut dev, DRM_MODE_ATOMIC_TEST_ONLY), Err(ModesetError::AtomicCheckFailed));

    // 7. Perform real Atomic Commit
    let commit_res = req.commit(&mut dev, 0);
    assert!(commit_res.is_ok());

    // Verify Primary Plane state updated
    let p20 = dev.find_plane(20).unwrap();
    assert_eq!(p20.fb_id, 50);
    assert_eq!(p20.crtc_id, 10);
    assert_eq!(p20.crtc_w, 1920);
    assert_eq!(p20.crtc_h, 1080);
    assert_eq!(p20.src_w, 1920 << 16);

    // Verify Video Overlay Plane state updated
    let p21 = dev.find_plane(21).unwrap();
    assert_eq!(p21.fb_id, 51);
    assert_eq!(p21.crtc_id, 10);
    assert_eq!(p21.crtc_x, 100);
    assert_eq!(p21.crtc_y, 50);
    assert_eq!(p21.crtc_w, 1280);
    assert_eq!(p21.crtc_h, 960);
    assert_eq!(p21.src_w, 640 << 16);
    assert_eq!(p21.src_h, 480 << 16);
    assert_eq!(p21.rotation, DRM_MODE_ROTATE_0 | DRM_MODE_REFLECT_X);
    assert_eq!(p21.scaling_filter, DRM_SCALING_FILTER_LINEAR);
    assert_eq!(p21.alpha, 0xE000);
    assert_eq!(p21.zpos, 1);

    true
}


/// Comprehensive verification of the DRM/KMS modesetting and multi-plane scanout presentation layer.
///
/// Tests:
/// 1. DRM object abstractions, video mode timings, and device topology.
/// 2. Atomic commit property table engine, test-only validation, and atomic state updates.
/// 3. Direct-to-display multi-plane overlay presentation (primary surface, video overlay plane, cursor plane)
///    with scaling, rotation, alpha blending, and dirty rectangle tracking.
pub fn test_drm_kms_subsystem() -> bool {
    // 1. Verify DRM object definitions, CRTC, plane, connector, and encoder abstractions
    if !super::drm::verify_drm_abstractions() {
        return false;
    }

    // 2. Verify atomic commit property table transactions and dry-run validation
    if !verify_atomic_commit_engine() {
        return false;
    }

    // 3. Verify direct-to-display multi-plane overlay presentation and compositor
    if !verify_multi_plane_overlay() {
        return false;
    }

    // 4. Emit deterministic boot sentinel
    crate::serial_write_str("[KNT-GFX-DRM-PASS] DRM/KMS atomic modesetting, CRTC scanout, and multi-plane overlay verified\r\n");

    true
}
