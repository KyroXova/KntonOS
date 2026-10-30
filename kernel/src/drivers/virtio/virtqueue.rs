//! Split Virtqueue DMA Memory Structures and Ring Management.

use alloc::boxed::Box;
use alloc::vec::Vec;

pub const VIRTQ_DESC_F_NEXT: u16 = 1;
pub const VIRTQ_DESC_F_WRITE: u16 = 2;
pub const VIRTQ_DESC_F_INDIRECT: u16 = 4;

pub const VIRTQ_AVAIL_F_NO_INTERRUPT: u16 = 1;
pub const VIRTQ_USED_F_NO_NOTIFY: u16 = 1;

/// Single 16-byte memory descriptor in the Virtqueue descriptor table.
#[derive(Debug, Clone, Copy, Default)]
#[repr(C, align(16))]
pub struct VirtqDesc {
    pub addr: u64,
    pub len: u32,
    pub flags: u16,
    pub next: u16,
}

/// Element in the used ring returned by the VirtIO device upon completion.
#[derive(Debug, Clone, Copy, Default)]
#[repr(C)]
pub struct VirtqUsedElem {
    pub id: u32,
    pub len: u32,
}

/// Split Virtqueue structure managing DMA rings and descriptor chains.
pub struct Virtqueue {
    queue_size: u16,
    descriptors: Box<[VirtqDesc]>,
    avail_ring: Box<[u16]>,
    pub avail_flags: u16,
    avail_idx: u16,
    used_ring: Box<[VirtqUsedElem]>,
    pub used_flags: u16,
    used_idx: u16,
    last_used_idx: u16,
    free_head: u16,
    free_count: u16,
}

impl Virtqueue {
    /// Allocates and initializes a new Virtqueue of `queue_size` (must be power of two).
    pub fn new(queue_size: u16) -> Self {
        assert!(queue_size > 0 && (queue_size & (queue_size - 1)) == 0);

        let mut desc_vec = Vec::with_capacity(queue_size as usize);
        for i in 0..queue_size {
            desc_vec.push(VirtqDesc {
                addr: 0,
                len: 0,
                flags: 0,
                next: (i + 1) % queue_size,
            });
        }

        let avail_vec = alloc::vec![0u16; queue_size as usize];
        let mut used_vec = Vec::with_capacity(queue_size as usize);
        for _ in 0..queue_size {
            used_vec.push(VirtqUsedElem { id: 0, len: 0 });
        }

        Self {
            queue_size,
            descriptors: desc_vec.into_boxed_slice(),
            avail_ring: avail_vec.into_boxed_slice(),
            avail_flags: 0,
            avail_idx: 0,
            used_ring: used_vec.into_boxed_slice(),
            used_flags: 0,
            used_idx: 0,
            last_used_idx: 0,
            free_head: 0,
            free_count: queue_size,
        }
    }

    #[inline(always)]
    pub fn queue_size(&self) -> u16 {
        self.queue_size
    }

    #[inline(always)]
    pub fn free_descriptors(&self) -> u16 {
        self.free_count
    }

    /// Allocates an unused descriptor index from the free list.
    pub fn alloc_descriptor(&mut self) -> Option<u16> {
        if self.free_count == 0 {
            return None;
        }

        let idx = self.free_head;
        self.free_head = self.descriptors[idx as usize].next;
        self.free_count -= 1;

        // Reset descriptor fields
        self.descriptors[idx as usize].flags = 0;
        self.descriptors[idx as usize].next = 0;

        Some(idx)
    }

    /// Sets descriptor parameters.
    pub fn set_descriptor(&mut self, idx: u16, addr: u64, len: u32, flags: u16, next: u16) {
        let desc = &mut self.descriptors[idx as usize];
        desc.addr = addr;
        desc.len = len;
        desc.flags = flags;
        desc.next = next;
    }

    /// Releases a descriptor chain back to the free list.
    pub fn free_descriptor_chain(&mut self, head: u16) {
        let mut curr = head;
        loop {
            let next = self.descriptors[curr as usize].next;
            let has_next = (self.descriptors[curr as usize].flags & VIRTQ_DESC_F_NEXT) != 0;

            self.descriptors[curr as usize].next = self.free_head;
            self.free_head = curr;
            self.free_count += 1;

            if !has_next {
                break;
            }
            curr = next;
        }
    }

    /// Submits a descriptor chain head to the Available Ring for device consumption.
    pub fn submit_chain(&mut self, head: u16) {
        let ring_idx = (self.avail_idx % self.queue_size) as usize;
        self.avail_ring[ring_idx] = head;

        // Memory barrier ensuring ring entry is visible before incrementing index
        core::sync::atomic::fence(core::sync::atomic::Ordering::Release);
        self.avail_idx = self.avail_idx.wrapping_add(1);
    }

    /// Checks if the device has returned any completed requests in the Used Ring.
    pub fn poll_used(&mut self) -> Option<VirtqUsedElem> {
        if self.last_used_idx == self.used_idx {
            return None;
        }

        let ring_idx = (self.last_used_idx % self.queue_size) as usize;
        let elem = self.used_ring[ring_idx];

        self.last_used_idx = self.last_used_idx.wrapping_add(1);
        Some(elem)
    }

    /// Returns raw 64-bit physical address pointers for device configuration registers.
    pub fn desc_addr(&self) -> u64 {
        self.descriptors.as_ptr() as usize as u64
    }

    pub fn avail_addr(&self) -> u64 {
        self.avail_ring.as_ptr() as usize as u64
    }

    pub fn used_addr(&self) -> u64 {
        self.used_ring.as_ptr() as usize as u64
    }

    /// Internal helper allowing device simulation/mocking to populate the used ring.
    pub fn mock_device_complete(&mut self, head_id: u32, len_written: u32) {
        let ring_idx = (self.used_idx % self.queue_size) as usize;
        self.used_ring[ring_idx] = VirtqUsedElem {
            id: head_id,
            len: len_written,
        };
        self.used_idx = self.used_idx.wrapping_add(1);
    }
}
