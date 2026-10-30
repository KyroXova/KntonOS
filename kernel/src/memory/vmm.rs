//! Virtual Memory Manager (VMM) - 4-Level x86-64 PML4 Paging.

use super::pmm::{FrameAllocator, PhysAddr, PAGE_SIZE};

/// A 64-bit canonical virtual memory address.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
#[repr(transparent)]
pub struct VirtAddr(pub usize);

impl VirtAddr {
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
    pub const fn pml4_index(self) -> usize {
        (self.0 >> 39) & 0x1FF
    }

    #[inline(always)]
    pub const fn pdpt_index(self) -> usize {
        (self.0 >> 30) & 0x1FF
    }

    #[inline(always)]
    pub const fn pd_index(self) -> usize {
        (self.0 >> 21) & 0x1FF
    }

    #[inline(always)]
    pub const fn pt_index(self) -> usize {
        (self.0 >> 12) & 0x1FF
    }

    #[inline(always)]
    pub const fn page_offset(self) -> usize {
        self.0 & 0xFFF
    }
}

/// Standard x86-64 page table entry attributes and flags.
pub struct PageTableFlags;

impl PageTableFlags {
    pub const NONE: u64 = 0;
    pub const PRESENT: u64 = 1 << 0;
    pub const WRITABLE: u64 = 1 << 1;
    pub const USER_ACCESSIBLE: u64 = 1 << 2;
    pub const WRITE_THROUGH: u64 = 1 << 3;
    pub const NO_CACHE: u64 = 1 << 4;
    pub const ACCESSED: u64 = 1 << 5;
    pub const DIRTY: u64 = 1 << 6;
    pub const HUGE_PAGE: u64 = 1 << 7;
    pub const GLOBAL: u64 = 1 << 8;
    pub const COW: u64 = 1 << 9; // Available for software Copy-on-Write tracking
    pub const NO_EXECUTE: u64 = 1 << 63;
}

const ADDR_MASK: u64 = 0x000F_FFFF_FFFF_F000;

/// An individual 64-bit page table entry (PML4E, PDPTE, PDE, or PTE).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(transparent)]
pub struct PageTableEntry(pub u64);

impl PageTableEntry {
    pub const fn new() -> Self {
        Self(0)
    }

    #[inline(always)]
    pub const fn is_present(&self) -> bool {
        (self.0 & PageTableFlags::PRESENT) != 0
    }

    #[inline(always)]
    pub const fn is_writable(&self) -> bool {
        (self.0 & PageTableFlags::WRITABLE) != 0
    }

    #[inline(always)]
    pub const fn is_user_accessible(&self) -> bool {
        (self.0 & PageTableFlags::USER_ACCESSIBLE) != 0
    }

    #[inline(always)]
    pub const fn is_cow(&self) -> bool {
        (self.0 & PageTableFlags::COW) != 0
    }

    #[inline(always)]
    pub fn set_cow(&mut self, cow: bool) {
        if cow {
            self.0 |= PageTableFlags::COW;
        } else {
            self.0 &= !PageTableFlags::COW;
        }
    }

    #[inline(always)]
    pub fn set_writable(&mut self, writable: bool) {
        if writable {
            self.0 |= PageTableFlags::WRITABLE;
        } else {
            self.0 &= !PageTableFlags::WRITABLE;
        }
    }

    #[inline(always)]
    pub const fn is_no_execute(&self) -> bool {
        (self.0 & PageTableFlags::NO_EXECUTE) != 0
    }

    #[inline(always)]
    pub const fn addr(&self) -> PhysAddr {
        PhysAddr((self.0 & ADDR_MASK) as usize)
    }

    #[inline(always)]
    pub const fn flags(&self) -> u64 {
        self.0 & !ADDR_MASK
    }

    #[inline(always)]
    pub fn set(&mut self, addr: PhysAddr, flags: u64) {
        self.0 = (addr.0 as u64 & ADDR_MASK) | flags;
    }

    #[inline(always)]
    pub fn clear(&mut self) {
        self.0 = 0;
    }
}

/// A 4 KiB aligned page table containing 512 entries.
#[repr(C, align(4096))]
pub struct PageTable {
    pub entries: [PageTableEntry; 512],
}

impl PageTable {
    pub const fn new() -> Self {
        Self {
            entries: [PageTableEntry(0); 512],
        }
    }

    pub fn zero(&mut self) {
        for entry in self.entries.iter_mut() {
            entry.clear();
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VmmError {
    AlreadyMapped,
    NotMapped,
    OutOfMemory,
    MisalignedAddress,
}

/// Invalidate Translation Lookaside Buffer (TLB) entry for a given virtual address.
#[inline(always)]
pub unsafe fn invlpg(vaddr: VirtAddr) {
    core::arch::asm!(
        "invlpg [{}]",
        in(reg) vaddr.0,
        options(nostack, preserves_flags)
    );
}

/// Manager for 4-level PML4 paging operations.
pub struct PageTableManager {
    root_pml4: *mut PageTable,
}

impl PageTableManager {
    /// Creates a PageTableManager bound to an active PML4 root table.
    pub unsafe fn new(root_pml4: *mut PageTable) -> Self {
        Self { root_pml4 }
    }

    /// Returns a raw pointer to the root PML4 table.
    pub fn root_pml4(&self) -> *mut PageTable {
        self.root_pml4
    }

    /// Maps a 4 KiB virtual page to a 4 KiB physical frame with specified flags.
    /// Allocates intermediate page tables (PDPT, PD, PT) via `allocator` as needed.
    pub unsafe fn map_page<A: FrameAllocator>(
        &mut self,
        virt: VirtAddr,
        phys: PhysAddr,
        flags: u64,
        allocator: &mut A,
    ) -> Result<(), VmmError> {
        if !virt.is_aligned_to(PAGE_SIZE) || !phys.is_aligned_to(PAGE_SIZE) {
            return Err(VmmError::MisalignedAddress);
        }

        let pml4 = &mut *self.root_pml4;

        // Level 4 -> Level 3 (PML4 to PDPT)
        let pml4_idx = virt.pml4_index();
        let pdpt = Self::get_or_create_table(&mut pml4.entries[pml4_idx], flags, allocator)?;

        // Level 3 -> Level 2 (PDPT to PD)
        let pdpt_idx = virt.pdpt_index();
        let pd = Self::get_or_create_table(&mut (*pdpt).entries[pdpt_idx], flags, allocator)?;

        // Level 2 -> Level 1 (PD to PT)
        let pd_idx = virt.pd_index();
        let pt = Self::get_or_create_table(&mut (*pd).entries[pd_idx], flags, allocator)?;

        // Level 1: Page Table Entry
        let pt_idx = virt.pt_index();
        let pte = &mut (*pt).entries[pt_idx];

        if pte.is_present() {
            return Err(VmmError::AlreadyMapped);
        }

        pte.set(phys, flags | PageTableFlags::PRESENT);
        invlpg(virt);

        Ok(())
    }

    /// Unmaps a previously mapped 4 KiB virtual page, returning its physical frame address.
    pub unsafe fn unmap_page(&mut self, virt: VirtAddr) -> Result<PhysAddr, VmmError> {
        if !virt.is_aligned_to(PAGE_SIZE) {
            return Err(VmmError::MisalignedAddress);
        }

        let pml4 = &mut *self.root_pml4;
        let pml4_entry = &pml4.entries[virt.pml4_index()];
        if !pml4_entry.is_present() {
            return Err(VmmError::NotMapped);
        }

        let pdpt = pml4_entry.addr().0 as *mut PageTable;
        let pdpt_entry = &(*pdpt).entries[virt.pdpt_index()];
        if !pdpt_entry.is_present() {
            return Err(VmmError::NotMapped);
        }

        let pd = pdpt_entry.addr().0 as *mut PageTable;
        let pd_entry = &(*pd).entries[virt.pd_index()];
        if !pd_entry.is_present() {
            return Err(VmmError::NotMapped);
        }

        let pt = pd_entry.addr().0 as *mut PageTable;
        let pte = &mut (*pt).entries[virt.pt_index()];
        if !pte.is_present() {
            return Err(VmmError::NotMapped);
        }

        let phys = pte.addr();
        pte.clear();
        invlpg(virt);

        Ok(phys)
    }

    /// Translates a virtual address to its mapped physical address if present.
    /// Retrieves a mutable reference to the PageTableEntry for the given virtual address if mapped.
    pub unsafe fn get_pte_mut(&mut self, virt: VirtAddr) -> Option<&mut PageTableEntry> {
        let pml4 = &mut *self.root_pml4;
        let pml4_entry = &mut pml4.entries[virt.pml4_index()];
        if !pml4_entry.is_present() {
            return None;
        }

        let pdpt = pml4_entry.addr().0 as *mut PageTable;
        let pdpt_entry = &mut (*pdpt).entries[virt.pdpt_index()];
        if !pdpt_entry.is_present() {
            return None;
        }

        let pd = pdpt_entry.addr().0 as *mut PageTable;
        let pd_entry = &mut (*pd).entries[virt.pd_index()];
        if !pd_entry.is_present() || (pd_entry.flags() & PageTableFlags::HUGE_PAGE) != 0 {
            return None;
        }

        let pt = pd_entry.addr().0 as *mut PageTable;
        let pte = &mut (*pt).entries[virt.pt_index()];
        Some(pte)
    }

    /// Retrieves a copy of the PageTableEntry for the given virtual address if mapped.
    pub unsafe fn get_pte(&self, virt: VirtAddr) -> Option<PageTableEntry> {
        let pml4 = &*self.root_pml4;
        let pml4_entry = &pml4.entries[virt.pml4_index()];
        if !pml4_entry.is_present() {
            return None;
        }

        let pdpt = pml4_entry.addr().0 as *const PageTable;
        let pdpt_entry = &(*pdpt).entries[virt.pdpt_index()];
        if !pdpt_entry.is_present() {
            return None;
        }

        let pd = pdpt_entry.addr().0 as *const PageTable;
        let pd_entry = &(*pd).entries[virt.pd_index()];
        if !pd_entry.is_present() || (pd_entry.flags() & PageTableFlags::HUGE_PAGE) != 0 {
            return None;
        }

        let pt = pd_entry.addr().0 as *const PageTable;
        let pte = (*pt).entries[virt.pt_index()];
        Some(pte)
    }

    pub unsafe fn translate(&self, virt: VirtAddr) -> Option<PhysAddr> {
        let pml4 = &*self.root_pml4;
        let pml4_entry = &pml4.entries[virt.pml4_index()];
        if !pml4_entry.is_present() {
            return None;
        }

        let pdpt = pml4_entry.addr().0 as *const PageTable;
        let pdpt_entry = &(*pdpt).entries[virt.pdpt_index()];
        if !pdpt_entry.is_present() {
            return None;
        }

        let pd = pdpt_entry.addr().0 as *const PageTable;
        let pd_entry = &(*pd).entries[virt.pd_index()];
        if !pd_entry.is_present() {
            return None;
        }

        // Support for 2 MiB large pages
        if (pd_entry.flags() & PageTableFlags::HUGE_PAGE) != 0 {
            let base = pd_entry.addr().0 & 0x000F_FFFF_FFE0_0000;
            return Some(PhysAddr(base + (virt.0 & 0x1F_FFFF)));
        }

        let pt = pd_entry.addr().0 as *const PageTable;
        let pte = &(*pt).entries[virt.pt_index()];
        if !pte.is_present() {
            return None;
        }

        Some(PhysAddr(pte.addr().0 + virt.page_offset()))
    }

    unsafe fn get_or_create_table<A: FrameAllocator>(
        entry: &mut PageTableEntry,
        flags: u64,
        allocator: &mut A,
    ) -> Result<*mut PageTable, VmmError> {
        if entry.is_present() {
            Ok(entry.addr().0 as *mut PageTable)
        } else {
            let frame = allocator.allocate_frame().ok_or(VmmError::OutOfMemory)?;
            let table_ptr = frame.start_address().0 as *mut PageTable;
            (*table_ptr).zero();

            // Intermediate tables require PRESENT, WRITABLE, and USER if target is USER
            let intermediate_flags = PageTableFlags::PRESENT
                | PageTableFlags::WRITABLE
                | (flags & PageTableFlags::USER_ACCESSIBLE);

            entry.set(frame.start_address(), intermediate_flags);
            Ok(table_ptr)
        }
    }
}
