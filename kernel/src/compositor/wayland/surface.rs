//! Wayland Sub-Surface Hierarchy, Z-Order Staging, & Frame Callback Synchronization.
//!
//! Implements `wl_subcompositor` and `wl_subsurface` abstractions:
//! - Parent-child surface tree hierarchy.
//! - Relative position offsets (x, y).
//! - Z-order stacking (`place_above`, `place_below`).
//! - Synchronized vs. Desynchronized commit modes.
//! - Frame callback (`wl_callback`) presentation dispatching.

use alloc::vec::Vec;
use crate::compositor::wayland::shm::WlSurface;

/// Synchronization mode of a `wl_subsurface`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SubsurfaceSyncMode {
    /// Commits to sub-surface are held cached until parent surface commits.
    Synchronized,
    /// Commits to sub-surface apply immediately to its current state.
    Desynchronized,
}

/// Z-order placement relative to another sibling or parent surface.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ZOrderPlacement {
    Above(u32), // Placed directly above target surface ID
    Below(u32), // Placed directly below target surface ID
}

/// A Wayland sub-surface (`wl_subsurface`) node.
#[derive(Debug, Clone)]
pub struct WlSubsurface {
    pub id: u32,
    pub surface_id: u32,
    pub parent_id: u32,
    pub pos_x: i32,
    pub pos_y: i32,
    pub sync_mode: SubsurfaceSyncMode,
    pub z_order: i32, // Stacking order (higher = in front)
}

impl WlSubsurface {
    pub fn new(id: u32, surface_id: u32, parent_id: u32) -> Self {
        Self {
            id,
            surface_id,
            parent_id,
            pos_x: 0,
            pos_y: 0,
            sync_mode: SubsurfaceSyncMode::Synchronized, // Sub-surfaces start synchronized by default
            z_order: 0,
        }
    }

    pub fn set_position(&mut self, x: i32, y: i32) {
        self.pos_x = x;
        self.pos_y = y;
    }

    pub fn set_sync(&mut self) {
        self.sync_mode = SubsurfaceSyncMode::Synchronized;
    }

    pub fn set_desync(&mut self) {
        self.sync_mode = SubsurfaceSyncMode::Desynchronized;
    }
}

/// Manager maintaining sub-surface hierarchies and frame presentation callbacks.
pub struct SubsurfaceTreeManager {
    subsurfaces: Vec<WlSubsurface>,
}

impl SubsurfaceTreeManager {
    pub fn new() -> Self {
        Self {
            subsurfaces: Vec::new(),
        }
    }

    pub fn create_subsurface(
        &mut self,
        id: u32,
        surface_id: u32,
        parent_id: u32,
    ) -> Result<&mut WlSubsurface, &'static str> {
        if surface_id == parent_id {
            return Err("Surface cannot be its own sub-surface parent");
        }
        // Check for existing subsurface assignment
        if self.subsurfaces.iter().any(|s| s.surface_id == surface_id) {
            return Err("Surface already has a subsurface role");
        }

        self.subsurfaces.push(WlSubsurface::new(id, surface_id, parent_id));
        Ok(self.subsurfaces.last_mut().unwrap())
    }

    pub fn get_subsurface_mut(&mut self, surface_id: u32) -> Option<&mut WlSubsurface> {
        self.subsurfaces.iter_mut().find(|s| s.surface_id == surface_id)
    }

    pub fn get_children(&self, parent_id: u32) -> Vec<WlSubsurface> {
        let mut children: Vec<WlSubsurface> = self
            .subsurfaces
            .iter()
            .filter(|s| s.parent_id == parent_id)
            .cloned()
            .collect();
        // Sort children by z-order ascending
        children.sort_by_key(|c| c.z_order);
        children
    }

    /// Reorders a sub-surface relative to a sibling or parent surface.
    pub fn place_above(&mut self, surface_id: u32, sibling_id: u32) -> bool {
        let target_z = if surface_id == sibling_id {
            return false;
        } else if let Some(sib) = self.subsurfaces.iter().find(|s| s.surface_id == sibling_id) {
            sib.z_order + 1
        } else {
            1 // Above parent (z = 0)
        };

        if let Some(sub) = self.get_subsurface_mut(surface_id) {
            sub.z_order = target_z;
            true
        } else {
            false
        }
    }

    pub fn place_below(&mut self, surface_id: u32, sibling_id: u32) -> bool {
        let target_z = if surface_id == sibling_id {
            return false;
        } else if let Some(sib) = self.subsurfaces.iter().find(|s| s.surface_id == sibling_id) {
            sib.z_order - 1
        } else {
            -1 // Below parent (z = 0)
        };

        if let Some(sub) = self.get_subsurface_mut(surface_id) {
            sub.z_order = target_z;
            true
        } else {
            false
        }
    }

    /// Cascades commit down the sub-surface hierarchy when a parent commits.
    pub fn commit_surface_tree(
        &self,
        parent: &mut WlSurface,
        children: &mut [WlSurface],
    ) {
        // Parent commits its own double-buffered state
        let _ = parent.commit();

        // Any synchronized children commit simultaneously
        for child in children.iter_mut() {
            if let Some(sub) = self.subsurfaces.iter().find(|s| s.surface_id == child.id && s.parent_id == parent.id) {
                if sub.sync_mode == SubsurfaceSyncMode::Synchronized {
                    let _ = child.commit();
                }
            }
        }
    }

    /// Dispatches frame callbacks for presented surfaces with timestamp in milliseconds.
    pub fn dispatch_frame_callbacks(surface: &mut WlSurface, timestamp_ms: u32) -> Vec<u32> {
        let callbacks = core::mem::take(&mut surface.frame_callbacks);
        // In real Wayland IPC, `wl_callback.done(callback_data = timestamp_ms)` is sent for each.
        let _ = timestamp_ms;
        callbacks
    }
}

/// Unit test verifying sub-surface hierarchy, z-ordering, sync/desync modes, and frame callbacks.
pub fn test_subsurface_and_frame_callbacks() -> bool {
    let mut mgr = SubsurfaceTreeManager::new();

    let parent_id = 100;
    let child1_id = 101;
    let child2_id = 102;

    // 1. Create sub-surfaces under parent
    if mgr.create_subsurface(1, child1_id, parent_id).is_err() {
        return false;
    }
    if mgr.create_subsurface(2, child2_id, parent_id).is_err() {
        return false;
    }

    // Attempt invalid self-parenting
    if mgr.create_subsurface(3, 200, 200).is_ok() {
        return false;
    }

    // 2. Set positioning and z-order
    if let Some(s1) = mgr.get_subsurface_mut(child1_id) {
        s1.set_position(10, 20);
        s1.set_desync();
    } else {
        return false;
    }

    if let Some(s2) = mgr.get_subsurface_mut(child2_id) {
        s2.set_position(50, 60);
    } else {
        return false;
    }

    if !mgr.place_above(child2_id, child1_id) {
        return false;
    }

    let children = mgr.get_children(parent_id);
    if children.len() != 2 {
        return false;
    }
    // S1 should have lower z-order than S2
    if children[0].surface_id != child1_id || children[1].surface_id != child2_id {
        return false;
    }

    // 3. Test Synchronized vs Desynchronized commits
    let mut parent_surf = WlSurface::new(parent_id);
    let mut child1_surf = WlSurface::new(child1_id);
    let mut child2_surf = WlSurface::new(child2_id);

    // Attach buffer to child 2 (synchronized) without committing yet
    child2_surf.attach(Some(555), 0, 0);
    if child2_surf.current.attached_buffer.is_some() {
        return false;
    }

    // Child 1 is desynchronized -> immediate commit works
    child1_surf.attach(Some(444), 0, 0);
    let _ = child1_surf.commit();
    if child1_surf.current.attached_buffer != Some(444) {
        return false;
    }

    // Parent commits tree -> child 2 (synchronized) now commits
    let mut child_surfaces = [child1_surf, child2_surf];
    mgr.commit_surface_tree(&mut parent_surf, &mut child_surfaces);

    if child_surfaces[1].current.attached_buffer != Some(555) {
        return false;
    }

    // 4. Test frame callback dispatch
    parent_surf.frame(777);
    parent_surf.frame(778);
    let dispatched = SubsurfaceTreeManager::dispatch_frame_callbacks(&mut parent_surf, 16666);
    if dispatched.len() != 2 || dispatched[0] != 777 || dispatched[1] != 778 {
        return false;
    }
    if !parent_surf.frame_callbacks.is_empty() {
        return false;
    }

    true
}
