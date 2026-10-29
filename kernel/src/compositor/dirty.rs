//! Dirty rectangle tracking and damaged region calculation for KntonOS compositor.

use alloc::vec::Vec;
use core::cmp::{max, min};

/// Axis-aligned rectangle defining screen or window bounds.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub struct Rect {
    pub x: usize,
    pub y: usize,
    pub width: usize,
    pub height: usize,
}

impl Rect {
    pub const fn new(x: usize, y: usize, width: usize, height: usize) -> Self {
        Self { x, y, width, height }
    }

    pub const fn empty() -> Self {
        Self {
            x: 0,
            y: 0,
            width: 0,
            height: 0,
        }
    }

    #[inline(always)]
    pub const fn is_empty(&self) -> bool {
        self.width == 0 || self.height == 0
    }

    #[inline(always)]
    pub const fn right(&self) -> usize {
        self.x.saturating_add(self.width)
    }

    #[inline(always)]
    pub const fn bottom(&self) -> usize {
        self.y.saturating_add(self.height)
    }

    /// Check if this rectangle intersects with another rectangle.
    pub fn intersects(&self, other: &Rect) -> bool {
        if self.is_empty() || other.is_empty() {
            return false;
        }
        self.x < other.right()
            && self.right() > other.x
            && self.y < other.bottom()
            && self.bottom() > other.y
    }

    /// Compute the intersection of two rectangles.
    pub fn intersection(&self, other: &Rect) -> Option<Rect> {
        if !self.intersects(other) {
            return None;
        }
        let nx = max(self.x, other.x);
        let ny = max(self.y, other.y);
        let nr = min(self.right(), other.right());
        let nb = min(self.bottom(), other.bottom());

        if nr > nx && nb > ny {
            Some(Rect {
                x: nx,
                y: ny,
                width: nr - nx,
                height: nb - ny,
            })
        } else {
            None
        }
    }

    /// Compute the minimal bounding rectangle enclosing both rectangles.
    pub fn union(&self, other: &Rect) -> Rect {
        if self.is_empty() {
            return *other;
        }
        if other.is_empty() {
            return *self;
        }
        let nx = min(self.x, other.x);
        let ny = min(self.y, other.y);
        let nr = max(self.right(), other.right());
        let nb = max(self.bottom(), other.bottom());

        Rect {
            x: nx,
            y: ny,
            width: nr.saturating_sub(nx),
            height: nb.saturating_sub(ny),
        }
    }

    /// Check if this rectangle contains a point (px, py).
    #[inline(always)]
    pub fn contains_point(&self, px: usize, py: usize) -> bool {
        px >= self.x && px < self.right() && py >= self.y && py < self.bottom()
    }

    /// Clamp this rectangle to fit entirely within screen dimensions (max_w, max_h).
    pub fn clamp(&self, max_w: usize, max_h: usize) -> Rect {
        if self.is_empty() || self.x >= max_w || self.y >= max_h {
            return Rect::empty();
        }
        let r = min(self.right(), max_w);
        let b = min(self.bottom(), max_h);
        Rect {
            x: self.x,
            y: self.y,
            width: r.saturating_sub(self.x),
            height: b.saturating_sub(self.y),
        }
    }
}

/// Damaged region tracker with coalescing support.
/// Accumulates dirty rectangles and coalesces overlapping regions to minimize blit overhead.
#[derive(Clone, Debug)]
pub struct DamageRegion {
    rects: Vec<Rect>,
    max_rects: usize,
}

impl DamageRegion {
    pub const DEFAULT_MAX_RECTS: usize = 16;

    pub fn new() -> Self {
        Self {
            rects: Vec::new(),
            max_rects: Self::DEFAULT_MAX_RECTS,
        }
    }

    pub fn with_capacity(max_rects: usize) -> Self {
        Self {
            rects: Vec::with_capacity(max_rects),
            max_rects,
        }
    }

    #[inline(always)]
    pub fn is_empty(&self) -> bool {
        self.rects.is_empty()
    }

    #[inline(always)]
    pub fn rects(&self) -> &[Rect] {
        &self.rects
    }

    pub fn clear(&mut self) {
        self.rects.clear();
    }

    /// Add a damaged rectangle. If it overlaps with an existing rectangle, coalesce them.
    /// If capacity is exceeded, coalesce all into a single bounding box.
    pub fn add_rect(&mut self, rect: Rect) {
        if rect.is_empty() {
            return;
        }

        // Try to merge with any intersecting rectangle
        let mut merged = false;
        for existing in self.rects.iter_mut() {
            if existing.intersects(&rect) {
                *existing = existing.union(&rect);
                merged = true;
                break;
            }
        }

        if !merged {
            self.rects.push(rect);
        }

        // If list exceeded max_rects, coalesce overlapping or collapse to bounding box
        if self.rects.len() > self.max_rects {
            self.coalesce();
        }
    }

    /// Coalesce overlapping rectangles in the list, or merge to bounding box if still too large.
    pub fn coalesce(&mut self) {
        let mut i = 0;
        while i < self.rects.len() {
            let mut j = i + 1;
            while j < self.rects.len() {
                if self.rects[i].intersects(&self.rects[j]) {
                    self.rects[i] = self.rects[i].union(&self.rects[j]);
                    self.rects.swap_remove(j);
                } else {
                    j += 1;
                }
            }
            i += 1;
        }

        if self.rects.len() > self.max_rects {
            if let Some(bbox) = self.bounding_box() {
                self.rects.clear();
                self.rects.push(bbox);
            }
        }
    }

    /// Compute the overall bounding box enclosing all damaged rectangles.
    pub fn bounding_box(&self) -> Option<Rect> {
        if self.rects.is_empty() {
            return None;
        }
        let mut bbox = self.rects[0];
        for r in &self.rects[1..] {
            bbox = bbox.union(r);
        }
        Some(bbox)
    }
}
