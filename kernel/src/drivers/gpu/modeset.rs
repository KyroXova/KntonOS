//! DRM/KMS Modesetting and Hardware Display Pipeline Controller.
//!
//! Provides display modesetting configuration, linking CRTCs, encoders,
//! connectors, and framebuffers into an active scanout pipeline.

use super::drm::{
    DrmDevice, DrmModeInfo, DRM_MODE_CONNECTED,
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
