//! DRM/KMS Modesetting and Hardware Display Pipeline Controller.
//!
//! Provides display modesetting configuration, linking CRTCs, encoders,
//! connectors, and framebuffers into an active scanout pipeline.
//! Implements atomic commit property table transactions (FB_ID, CRTC_ID,
//! coordinates, rotation, scaling filter, alpha, zpos).

use super::drm::{
    DrmDevice, DrmModeInfo, DRM_MODE_ATOMIC_TEST_ONLY, DRM_MODE_CONNECTED, DRM_MODE_REFLECT_MASK,
    DRM_MODE_ROTATE_MASK, DRM_PROP_ACTIVE, DRM_PROP_ALPHA, DRM_PROP_CRTC_H, DRM_PROP_CRTC_ID,
    DRM_PROP_CRTC_W, DRM_PROP_CRTC_X, DRM_PROP_CRTC_Y, DRM_PROP_FB_ID, DRM_PROP_ROTATION,
    DRM_PROP_SCALING_FILTER, DRM_PROP_SRC_H, DRM_PROP_SRC_W, DRM_PROP_SRC_X, DRM_PROP_SRC_Y,
    DRM_PROP_ZPOS, DRM_SCALING_FILTER_LINEAR,
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
