//! Transparent Huge Pages (THP) - 2MB Large Page Allocator & Page Table Structures.
//!
//! Provides 2MB physical frame allocation, 2MB PML4/PDPT/PD translation mappings,
//! and large page entry management for x86-64 long mode.


/// Standard x86-64 4KB page size.
pub const PAGE_SIZE_4KB: usize = 4096;

/// Standard x86-64 2MB huge page size.
pub const PAGE_SIZE_2MB: usize = 2 * 1024 * 1024; // 2,097,152 bytes (0x200000)

/// Number of 4KB pages in a single 2MB huge page.
pub const PAGES_PER_2MB: usize = 512;

/// Bitmask for extracting the 2MB page offset from a 64-bit virtual address.
pub const OFFSET_MASK_2MB: usize = PAGE_SIZE_2MB - 1; // 0x1F_FFFF (21 bits)

/// Bitmask for 2MB physical address in PDE (bits 51:21).
pub const PHYS_MASK_2MB: u64 = 0x000F_FFFF_FFE0_0000;

/// Standard x86-64 2MB page table entry flags.
pub struct HugePageFlags;

impl HugePageFlags {
    pub const NONE: u64 = 0;
    pub const PRESENT: u64 = 1 << 0;
    pub const WRITABLE: u64 = 1 << 1;
    pub const USER_ACCESSIBLE: u64 = 1 << 2;
    pub const WRITE_THROUGH: u64 = 1 << 3;
    pub const NO_CACHE: u64 = 1 << 4;
    pub const ACCESSED: u64 = 1 << 5;
    pub const DIRTY: u64 = 1 << 6;
    /// Bit 7 in a Page Directory Entry (PDE) indicates a 2MB large page (PS=1).
    pub const HUGE_PAGE: u64 = 1 << 7;
    pub const GLOBAL: u64 = 1 << 8;
    /// Bit 12 in a 2MB PDE acts as the PAT (Page Attribute Table) selector bit.
    pub const PAT_2MB: u64 = 1 << 12;
    /// Bit 63 in AMD64 / Intel 64 disables execution.
    pub const NO_EXECUTE: u64 = 1 << 63;
}

/// A 64-bit Page Directory Entry (PDE) for 2MB large page mappings.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(transparent)]
pub struct HugePageEntry(pub u64);

impl HugePageEntry {
    /// Creates a zeroed (not present) huge page table entry.
    pub const fn empty() -> Self {
        Self(0)
    }

    /// Creates a 2MB page entry pointing to `phys_base` with specified flags.
    /// Automatically ensures `HUGE_PAGE` (bit 7) is set.
    pub const fn new(phys_base: usize, flags: u64) -> Self {
        let addr_masked = (phys_base as u64) & PHYS_MASK_2MB;
        let entry = addr_masked | flags | HugePageFlags::HUGE_PAGE;
        Self(entry)
    }

    #[inline(always)]
    pub const fn raw(self) -> u64 {
        self.0
    }

    #[inline(always)]
    pub const fn is_present(&self) -> bool {
        (self.0 & HugePageFlags::PRESENT) != 0
    }

    #[inline(always)]
    pub const fn is_writable(&self) -> bool {
        (self.0 & HugePageFlags::WRITABLE) != 0
    }

    #[inline(always)]
    pub const fn is_user(&self) -> bool {
        (self.0 & HugePageFlags::USER_ACCESSIBLE) != 0
    }

    #[inline(always)]
    pub const fn is_huge(&self) -> bool {
        (self.0 & HugePageFlags::HUGE_PAGE) != 0
    }

    #[inline(always)]
    pub const fn is_dirty(&self) -> bool {
        (self.0 & HugePageFlags::DIRTY) != 0
    }

    #[inline(always)]
    pub const fn is_accessed(&self) -> bool {
        (self.0 & HugePageFlags::ACCESSED) != 0
    }

    #[inline(always)]
    pub const fn no_execute(&self) -> bool {
        (self.0 & HugePageFlags::NO_EXECUTE) != 0
    }

    /// Extracts the 2MB-aligned base physical address.
    #[inline(always)]
    pub const fn physical_address(&self) -> usize {
        (self.0 & PHYS_MASK_2MB) as usize
    }

    /// Sets or clears the writable flag.
    pub fn set_writable(&mut self, writable: bool) {
        if writable {
            self.0 |= HugePageFlags::WRITABLE;
        } else {
            self.0 &= !HugePageFlags::WRITABLE;
        }
    }

    /// Sets or clears the dirty flag.
    pub fn set_dirty(&mut self, dirty: bool) {
        if dirty {
            self.0 |= HugePageFlags::DIRTY;
        } else {
            self.0 &= !HugePageFlags::DIRTY;
        }
    }
}

/// Decomposed 4-level virtual address components for x86-64.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VirtualAddressComponents {
    pub pml4_index: usize,
    pub pdpt_index: usize,
    pub pd_index: usize,
    pub pt_index: usize,
    pub offset_4kb: usize,
    pub offset_2mb: usize,
}

impl VirtualAddressComponents {
    /// Decomposes a canonical 64-bit virtual address.
    pub const fn decompose(vaddr: usize) -> Self {
        Self {
            pml4_index: (vaddr >> 39) & 0x1FF,
            pdpt_index: (vaddr >> 30) & 0x1FF,
            pd_index: (vaddr >> 21) & 0x1FF,
            pt_index: (vaddr >> 12) & 0x1FF,
            offset_4kb: vaddr & 0xFFF,
            offset_2mb: vaddr & OFFSET_MASK_2MB,
        }
    }
}

/// A 2MB-aligned physical frame handle.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
#[repr(transparent)]
pub struct HugePhysFrame(pub usize);

impl HugePhysFrame {
    /// Creates a huge physical frame from a 2MB-aligned physical address.
    pub const fn from_addr(addr: usize) -> Option<Self> {
        if (addr & OFFSET_MASK_2MB) == 0 {
            Some(Self(addr / PAGE_SIZE_2MB))
        } else {
            None
        }
    }

    /// Creates a huge physical frame directly from frame index.
    pub const fn from_index(index: usize) -> Self {
        Self(index)
    }

    #[inline(always)]
    pub const fn base_address(self) -> usize {
        self.0 * PAGE_SIZE_2MB
    }

    #[inline(always)]
    pub const fn index(self) -> usize {
        self.0
    }

    /// Returns the starting 4KB physical frame index within this 2MB chunk.
    #[inline(always)]
    pub const fn first_4kb_frame(self) -> usize {
        self.0 * PAGES_PER_2MB
    }
}

/// Maximum number of 2MB huge frames tracked (128 huge frames * 2MB = 256 MiB physical memory pool).
pub const MAX_HUGE_PAGES: usize = 128;
const BITMAP_WORDS: usize = (MAX_HUGE_PAGES + 63) / 64; // 2 words (128 bits)

/// Physical 2MB Large Page Allocator.
///
/// Tracks 2MB-aligned physical frames, providing fast $O(1)$ allocation, buddy-like alignment,
/// and usage metrics.
pub struct HugePageAllocator {
    bitmap: [u64; BITMAP_WORDS],
    base_phys_addr: usize,
    total_huge_pages: usize,
    allocated_huge_pages: usize,
}

impl HugePageAllocator {
    /// Creates an uninitialized allocator with all huge frames marked reserved.
    pub const fn new() -> Self {
        Self {
            bitmap: [u64::MAX; BITMAP_WORDS],
            base_phys_addr: 0,
            total_huge_pages: 0,
            allocated_huge_pages: 0,
        }
    }

    /// Initializes the huge page pool over physical memory starting at `base_phys_addr`.
    /// `base_phys_addr` must be 2MB-aligned.
    pub fn init(&mut self, base_phys_addr: usize, count: usize) -> bool {
        if (base_phys_addr & OFFSET_MASK_2MB) != 0 {
            return false;
        }

        let clamped_count = if count > MAX_HUGE_PAGES {
            MAX_HUGE_PAGES
        } else {
            count
        };

        self.base_phys_addr = base_phys_addr;
        self.total_huge_pages = clamped_count;
        self.allocated_huge_pages = 0;

        // Clear all bits to mark as free (0 = free, 1 = allocated)
        self.bitmap = [0u64; BITMAP_WORDS];

        // Mark frames beyond clamped_count as allocated/reserved
        for i in clamped_count..MAX_HUGE_PAGES {
            let word_idx = i / 64;
            let bit_idx = i % 64;
            self.bitmap[word_idx] |= 1u64 << bit_idx;
        }

        true
    }

    /// Allocates a single 2MB-aligned physical huge frame.
    pub fn allocate_2mb_frame(&mut self) -> Option<HugePhysFrame> {
        for word_idx in 0..BITMAP_WORDS {
            let word = self.bitmap[word_idx];
            if word != u64::MAX {
                // Find first free bit (0)
                let bit_idx = (!word).trailing_zeros() as usize;
                let page_idx = word_idx * 64 + bit_idx;

                if page_idx < self.total_huge_pages {
                    self.bitmap[word_idx] |= 1u64 << bit_idx;
                    self.allocated_huge_pages += 1;
                    return Some(HugePhysFrame::from_index(
                        (self.base_phys_addr / PAGE_SIZE_2MB) + page_idx,
                    ));
                }
            }
        }
        None
    }

    /// Deallocates a 2MB physical huge frame.
    pub fn deallocate_2mb_frame(&mut self, frame: HugePhysFrame) -> bool {
        let pool_start_index = self.base_phys_addr / PAGE_SIZE_2MB;
        if frame.index() < pool_start_index {
            return false;
        }

        let relative_idx = frame.index() - pool_start_index;
        if relative_idx >= self.total_huge_pages {
            return false;
        }

        let word_idx = relative_idx / 64;
        let bit_idx = relative_idx % 64;

        if (self.bitmap[word_idx] & (1u64 << bit_idx)) == 0 {
            // Already free (double free)
            return false;
        }

        self.bitmap[word_idx] &= !(1u64 << bit_idx);
        if self.allocated_huge_pages > 0 {
            self.allocated_huge_pages -= 1;
        }
        true
    }

    #[inline(always)]
    pub const fn total_pages(&self) -> usize {
        self.total_huge_pages
    }

    #[inline(always)]
    pub const fn allocated_pages(&self) -> usize {
        self.allocated_huge_pages
    }

    #[inline(always)]
    pub const fn free_pages(&self) -> usize {
        self.total_huge_pages.saturating_sub(self.allocated_huge_pages)
    }

    #[inline(always)]
    pub const fn base_address(&self) -> usize {
        self.base_phys_addr
    }
}

/// Simulated Page Directory (PD) containing 512 entries for 2MB mapping verification.
pub struct PageDirectoryTable {
    pub entries: [HugePageEntry; 512],
}

impl PageDirectoryTable {
    pub const fn new() -> Self {
        Self {
            entries: [HugePageEntry::empty(); 512],
        }
    }

    /// Maps a 2MB virtual address to a 2MB physical base address.
    pub fn map_2mb(
        &mut self,
        vaddr: usize,
        phys_base: usize,
        flags: u64,
    ) -> Result<(), &'static str> {
        let comp = VirtualAddressComponents::decompose(vaddr);
        if (phys_base & OFFSET_MASK_2MB) != 0 {
            return Err("Physical base is not 2MB aligned");
        }

        let entry = HugePageEntry::new(phys_base, flags);
        self.entries[comp.pd_index] = entry;
        Ok(())
    }

    /// Unmaps a 2MB page directory entry.
    pub fn unmap_2mb(&mut self, vaddr: usize) -> Option<HugePhysFrame> {
        let comp = VirtualAddressComponents::decompose(vaddr);
        let entry = self.entries[comp.pd_index];
        if entry.is_present() && entry.is_huge() {
            self.entries[comp.pd_index] = HugePageEntry::empty();
            HugePhysFrame::from_addr(entry.physical_address())
        } else {
            None
        }
    }

    /// Translates a virtual address through this Page Directory if mapped as a 2MB huge page.
    pub fn translate_2mb(&self, vaddr: usize) -> Option<usize> {
        let comp = VirtualAddressComponents::decompose(vaddr);
        let entry = self.entries[comp.pd_index];
        if entry.is_present() && entry.is_huge() {
            Some(entry.physical_address() + comp.offset_2mb)
        } else {
            None
        }
    }
}
