//! Physical Memory Manager (PMM) - 4 KiB Bitmap Frame Allocator.

pub const PAGE_SIZE: usize = 4096;

/// A 64-bit physical memory address.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
#[repr(transparent)]
pub struct PhysAddr(pub usize);

impl PhysAddr {
    #[inline(always)]
    pub const fn new(addr: usize) -> Self {
        Self(addr)
    }

    #[inline(always)]
    pub const fn as_usize(self) -> usize {
        self.0
    }

    #[inline(always)]
    pub const fn is_aligned_to(self, align: usize) -> bool {
        (self.0 & (align - 1)) == 0
    }

    #[inline(always)]
    pub const fn containing_frame(self) -> PhysFrame {
        PhysFrame(self.0 / PAGE_SIZE)
    }
}

/// A 4 KiB physical memory frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
#[repr(transparent)]
pub struct PhysFrame(pub usize);

impl PhysFrame {
    #[inline(always)]
    pub const fn from_number(number: usize) -> Self {
        Self(number)
    }

    #[inline(always)]
    pub const fn start_address(self) -> PhysAddr {
        PhysAddr(self.0 * PAGE_SIZE)
    }

    #[inline(always)]
    pub const fn number(self) -> usize {
        self.0
    }
}

pub trait FrameAllocator {
    fn allocate_frame(&mut self) -> Option<PhysFrame>;
    fn deallocate_frame(&mut self, frame: PhysFrame);
}

/// Maximum frames tracked: 65,536 frames * 4 KiB = 256 MiB physical memory.
pub const BITMAP_WORDS: usize = 1024;
pub const MAX_FRAMES: usize = BITMAP_WORDS * 64;

/// Bitmap-based frame allocator tracking 4 KiB physical frames.
/// Bit = 0: Frame is FREE.
pub struct BitmapFrameAllocator {
    bitmap: [u64; BITMAP_WORDS],
    total_frames: usize,
    allocated_frames: usize,
    search_hint: usize,
}

impl BitmapFrameAllocator {
    /// Creates an empty allocator with all frames initially marked as reserved (1).
    pub const fn new() -> Self {
        Self {
            bitmap: [u64::MAX; BITMAP_WORDS],
            total_frames: 0,
            allocated_frames: 0,
            search_hint: 0,
        }
    }

    /// Initializes the allocator with a given number of usable frames.
    /// Unusable frames beyond `usable_frames` remain marked as reserved.
    pub fn init(&mut self, usable_frames: usize) {
        let count = if usable_frames > MAX_FRAMES {
            MAX_FRAMES
        } else {
            usable_frames
        };

        self.total_frames = count;
        // Initially mark everything reserved
        self.bitmap = [u64::MAX; BITMAP_WORDS];
        self.allocated_frames = count;

        // Mark usable frames (excluding the first 1 MiB / 256 frames reserved for firmware) as free
        let start_usable_frame = 256; // 1 MiB / 4 KiB
        if count > start_usable_frame {
            self.mark_range_free(start_usable_frame, count - start_usable_frame);
        }
        self.search_hint = start_usable_frame / 64;
    }

    /// Marks a contiguous range of frames as free (bit = 0).
    pub fn mark_range_free(&mut self, start_frame: usize, count: usize) {
        for frame in start_frame..(start_frame + count) {
            if frame >= self.total_frames {
                break;
            }
            let word_idx = frame / 64;
            let bit_idx = frame % 64;
            let mask = 1u64 << bit_idx;

            if (self.bitmap[word_idx] & mask) != 0 {
                self.bitmap[word_idx] &= !mask;
                self.allocated_frames = self.allocated_frames.saturating_sub(1);
            }
        }
        let hint = start_frame / 64;
        if hint < self.search_hint {
            self.search_hint = hint;
        }
    }

    /// Marks a contiguous range of frames as reserved/allocated (bit = 1).
    pub fn mark_range_reserved(&mut self, start_frame: usize, count: usize) {
        for frame in start_frame..(start_frame + count) {
            if frame >= self.total_frames {
                break;
            }
            let word_idx = frame / 64;
            let bit_idx = frame % 64;
            let mask = 1u64 << bit_idx;

            if (self.bitmap[word_idx] & mask) == 0 {
                self.bitmap[word_idx] |= mask;
                self.allocated_frames += 1;
            }
        }
    }

    /// Total number of physical frames managed.
    #[inline(always)]
    pub fn total_frames(&self) -> usize {
        self.total_frames
    }

    /// Total number of currently allocated/reserved physical frames.
    #[inline(always)]
    pub fn allocated_frames(&self) -> usize {
        self.allocated_frames
    }

    /// Number of free 4 KiB physical frames available for allocation.
    #[inline(always)]
    pub fn free_frames(&self) -> usize {
        self.total_frames.saturating_sub(self.allocated_frames)
    }

    /// Allocates a single 4 KiB physical frame.
    pub fn allocate_frame(&mut self) -> Option<PhysFrame> {
        let words = (self.total_frames + 63) / 64;

        for offset in 0..words {
            let idx = (self.search_hint + offset) % words;
            let word = self.bitmap[idx];

            if word != u64::MAX {
                let bit = (!word).trailing_zeros() as usize;
                let frame_num = idx * 64 + bit;

                if frame_num < self.total_frames {
                    self.bitmap[idx] |= 1u64 << bit;
                    self.allocated_frames += 1;
                    self.search_hint = idx;
                    return Some(PhysFrame(frame_num));
                }
            }
        }
        None
    }

    /// Deallocates a previously allocated 4 KiB physical frame.
    pub fn deallocate_frame(&mut self, frame: PhysFrame) {
        if frame.0 >= self.total_frames {
            return;
        }

        let word_idx = frame.0 / 64;
        let bit_idx = frame.0 % 64;
        let mask = 1u64 << bit_idx;

        if (self.bitmap[word_idx] & mask) != 0 {
            self.bitmap[word_idx] &= !mask;
            self.allocated_frames = self.allocated_frames.saturating_sub(1);
            if word_idx < self.search_hint {
                self.search_hint = word_idx;
            }
        }
    }
}

impl FrameAllocator for BitmapFrameAllocator {
    fn allocate_frame(&mut self) -> Option<PhysFrame> {
        self.allocate_frame()
    }

    fn deallocate_frame(&mut self, frame: PhysFrame) {
        self.deallocate_frame(frame);
    }
}
