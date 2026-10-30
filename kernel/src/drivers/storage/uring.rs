//! High-Speed NVMe Overlapped Async I/O (io_uring Style) Completion Engine.
//!
//! Provides lockless shared-memory Submission Queue (SQ) and Completion Queue (CQ)
//! ring buffers with atomic head and tail cursors, Submission Queue Entries (SQE),
//! Completion Queue Entries (CQE), async request dispatching directly to NVMe/block drivers,
//! out-of-order completion harvesting, and token correlation.

use core::sync::atomic::{AtomicU32, Ordering};

pub const DEFAULT_RING_ENTRIES: usize = 32;

// --- io_uring Opcodes ---
pub const IORING_OP_NOP: u8 = 0;
pub const IORING_OP_READV: u8 = 1;
pub const IORING_OP_WRITEV: u8 = 2;
pub const IORING_OP_FSYNC: u8 = 3;
pub const IORING_OP_READ_FIXED: u8 = 4;
pub const IORING_OP_WRITE_FIXED: u8 = 5;
pub const IORING_OP_POLL_ADD: u8 = 6;
pub const IORING_OP_POLL_REMOVE: u8 = 7;
pub const IORING_OP_SYNC_FILE_RANGE: u8 = 8;
pub const IORING_OP_SENDMSG: u8 = 9;
pub const IORING_OP_RECVMSG: u8 = 10;
pub const IORING_OP_TIMEOUT: u8 = 11;
pub const IORING_OP_READ: u8 = 22;
pub const IORING_OP_WRITE: u8 = 23;

// --- SQE Flags ---
pub const IOSQE_FIXED_FILE: u8 = 1 << 0;
pub const IOSQE_IO_DRAIN: u8 = 1 << 1;
pub const IOSQE_IO_LINK: u8 = 1 << 2;
pub const IOSQE_IO_HARDLINK: u8 = 1 << 3;
pub const IOSQE_ASYNC: u8 = 1 << 4;
pub const IOSQE_BUFFER_SELECT: u8 = 1 << 5;

// --- CQE Flags ---
pub const IORING_CQE_F_BUFFER: u32 = 1 << 0;
pub const IORING_CQE_F_MORE: u32 = 1 << 1;

/// Submission Queue Entry (SQE) matching io_uring 64-byte layout.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct IoUringSqe {
    pub opcode: u8,
    pub flags: u8,
    pub ioprio: u16,
    pub fd: i32,
    pub off: u64,
    pub addr: u64,
    pub len: u32,
    pub rw_flags: u32,
    pub user_data: u64,
    pub buf_index: u16,
    pub personality: u16,
    pub splice_fd_in: i32,
    pub rsvd: [u8; 16],
}

impl IoUringSqe {
    pub const fn empty() -> Self {
        Self {
            opcode: IORING_OP_NOP,
            flags: 0,
            ioprio: 0,
            fd: -1,
            off: 0,
            addr: 0,
            len: 0,
            rw_flags: 0,
            user_data: 0,
            buf_index: 0,
            personality: 0,
            splice_fd_in: 0,
            rsvd: [0u8; 16],
        }
    }

    pub const fn new_read(fd: i32, addr: u64, len: u32, off: u64, user_data: u64) -> Self {
        let mut sqe = Self::empty();
        sqe.opcode = IORING_OP_READ;
        sqe.fd = fd;
        sqe.addr = addr;
        sqe.len = len;
        sqe.off = off;
        sqe.user_data = user_data;
        sqe
    }

    pub const fn new_write(fd: i32, addr: u64, len: u32, off: u64, user_data: u64) -> Self {
        let mut sqe = Self::empty();
        sqe.opcode = IORING_OP_WRITE;
        sqe.fd = fd;
        sqe.addr = addr;
        sqe.len = len;
        sqe.off = off;
        sqe.user_data = user_data;
        sqe
    }

    pub const fn new_fsync(fd: i32, user_data: u64) -> Self {
        let mut sqe = Self::empty();
        sqe.opcode = IORING_OP_FSYNC;
        sqe.fd = fd;
        sqe.user_data = user_data;
        sqe
    }

    pub const fn new_nop(user_data: u64) -> Self {
        let mut sqe = Self::empty();
        sqe.opcode = IORING_OP_NOP;
        sqe.user_data = user_data;
        sqe
    }
}

/// Completion Queue Entry (CQE) matching io_uring 16-byte layout.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct IoUringCqe {
    pub user_data: u64,
    pub res: i32, // Result (bytes transferred or negative errno)
    pub flags: u32,
}

impl IoUringCqe {
    pub const fn empty() -> Self {
        Self {
            user_data: 0,
            res: 0,
            flags: 0,
        }
    }

    pub const fn new(user_data: u64, res: i32, flags: u32) -> Self {
        Self {
            user_data,
            res,
            flags,
        }
    }

    #[inline(always)]
    pub fn is_success(&self) -> bool {
        self.res >= 0
    }
}

/// Submission Queue (SQ) Ring Buffer with atomic cursors.
pub struct SubmissionQueueRing<const N: usize = DEFAULT_RING_ENTRIES> {
    pub head: AtomicU32,
    pub tail: AtomicU32,
    pub ring_mask: u32,
    pub ring_entries: u32,
    pub flags: AtomicU32,
    pub dropped: AtomicU32,
    pub entries: [IoUringSqe; N],
}

impl<const N: usize> SubmissionQueueRing<N> {
    pub const fn new() -> Self {
        // N must be power of two
        Self {
            head: AtomicU32::new(0),
            tail: AtomicU32::new(0),
            ring_mask: (N as u32) - 1,
            ring_entries: N as u32,
            flags: AtomicU32::new(0),
            dropped: AtomicU32::new(0),
            entries: [IoUringSqe::empty(); N],
        }
    }

    /// Number of ready SQEs awaiting submission.
    #[inline(always)]
    pub fn count(&self) -> u32 {
        let head = self.head.load(Ordering::Acquire);
        let tail = self.tail.load(Ordering::Acquire);
        tail.wrapping_sub(head)
    }

    /// Available space in submission ring.
    #[inline(always)]
    pub fn space(&self) -> u32 {
        self.ring_entries.saturating_sub(self.count())
    }

    /// Userland pushes an SQE to the submission ring.
    pub fn push(&mut self, sqe: IoUringSqe) -> bool {
        let head = self.head.load(Ordering::Acquire);
        let tail = self.tail.load(Ordering::Acquire);

        if tail.wrapping_sub(head) >= self.ring_entries {
            self.dropped.fetch_add(1, Ordering::Relaxed);
            return false;
        }

        let idx = (tail & self.ring_mask) as usize;
        self.entries[idx] = sqe;
        self.tail.store(tail.wrapping_add(1), Ordering::Release);
        true
    }

    /// Kernel consumes an SQE from the submission ring.
    pub fn pop(&mut self) -> Option<IoUringSqe> {
        let head = self.head.load(Ordering::Acquire);
        let tail = self.tail.load(Ordering::Acquire);

        if head == tail {
            return None;
        }

        let idx = (head & self.ring_mask) as usize;
        let sqe = self.entries[idx];
        self.head.store(head.wrapping_add(1), Ordering::Release);
        Some(sqe)
    }
}

/// Completion Queue (CQ) Ring Buffer with atomic cursors.
pub struct CompletionQueueRing<const N: usize = DEFAULT_RING_ENTRIES> {
    pub head: AtomicU32,
    pub tail: AtomicU32,
    pub ring_mask: u32,
    pub ring_entries: u32,
    pub overflow: AtomicU32,
    pub entries: [IoUringCqe; N],
}

impl<const N: usize> CompletionQueueRing<N> {
    pub const fn new() -> Self {
        Self {
            head: AtomicU32::new(0),
            tail: AtomicU32::new(0),
            ring_mask: (N as u32) - 1,
            ring_entries: N as u32,
            overflow: AtomicU32::new(0),
            entries: [IoUringCqe::empty(); N],
        }
    }

    /// Number of available CQEs awaiting harvest.
    #[inline(always)]
    pub fn count(&self) -> u32 {
        let head = self.head.load(Ordering::Acquire);
        let tail = self.tail.load(Ordering::Acquire);
        tail.wrapping_sub(head)
    }

    /// Kernel posts a CQE to the completion ring.
    pub fn push(&mut self, cqe: IoUringCqe) -> bool {
        let head = self.head.load(Ordering::Acquire);
        let tail = self.tail.load(Ordering::Acquire);

        if tail.wrapping_sub(head) >= self.ring_entries {
            self.overflow.fetch_add(1, Ordering::Relaxed);
            return false;
        }

        let idx = (tail & self.ring_mask) as usize;
        self.entries[idx] = cqe;
        self.tail.store(tail.wrapping_add(1), Ordering::Release);
        true
    }

    /// Userland reaps a completed CQE.
    pub fn pop(&mut self) -> Option<IoUringCqe> {
        let head = self.head.load(Ordering::Acquire);
        let tail = self.tail.load(Ordering::Acquire);

        if head == tail {
            return None;
        }

        let idx = (head & self.ring_mask) as usize;
        let cqe = self.entries[idx];
        self.head.store(head.wrapping_add(1), Ordering::Release);
        Some(cqe)
    }
}

/// Combined io_uring Context holding shared SQ and CQ rings.
pub struct IoUringContext<const N: usize = DEFAULT_RING_ENTRIES> {
    pub sq: SubmissionQueueRing<N>,
    pub cq: CompletionQueueRing<N>,
    pub ring_id: u32,
}

impl<const N: usize> IoUringContext<N> {
    pub const fn new(ring_id: u32) -> Self {
        Self {
            sq: SubmissionQueueRing::new(),
            cq: CompletionQueueRing::new(),
            ring_id,
        }
    }
}

/// Unit test verifying SQ/CQ ring buffer operations, wrapping logic, and atomic cursors.
pub fn verify_uring_ring_buffers() -> bool {
    // 1. Verify descriptor sizes
    if core::mem::size_of::<IoUringSqe>() != 64 {
        return false;
    }
    if core::mem::size_of::<IoUringCqe>() != 16 {
        return false;
    }

    // 2. Test SQ ring push/pop and wrapping
    let mut sq = SubmissionQueueRing::<4>::new();
    if sq.count() != 0 || sq.space() != 4 {
        return false;
    }

    // Push 4 entries to fill
    for i in 0..4 {
        let sqe = IoUringSqe::new_read(1, 0x1000 + (i as u64 * 512), 512, i as u64, 100 + i as u64);
        if !sq.push(sqe) {
            return false;
        }
    }
    if sq.count() != 4 || sq.space() != 0 {
        return false;
    }

    // Fifth push should fail (dropped count increments)
    let overflow_sqe = IoUringSqe::new_nop(999);
    if sq.push(overflow_sqe) {
        return false;
    }
    if sq.dropped.load(Ordering::Acquire) != 1 {
        return false;
    }

    // Pop 2 entries
    let sqe0 = sq.pop().expect("pop 0 failed");
    let sqe1 = sq.pop().expect("pop 1 failed");
    if sqe0.user_data != 100 || sqe1.user_data != 101 {
        return false;
    }
    if sq.count() != 2 || sq.space() != 2 {
        return false;
    }

    // Push 2 more to test circular index wrapping past initial capacity
    if !sq.push(IoUringSqe::new_nop(104)) || !sq.push(IoUringSqe::new_nop(105)) {
        return false;
    }

    // Pop remaining 4 entries
    let mut popped_tokens = [0u64; 4];
    for i in 0..4 {
        popped_tokens[i] = sq.pop().expect("pop wrap failed").user_data;
    }
    if popped_tokens != [102, 103, 104, 105] {
        return false;
    }
    if sq.count() != 0 {
        return false;
    }

    // 3. Test CQ ring push/pop and overflow tracking
    let mut cq = CompletionQueueRing::<4>::new();
    if cq.count() != 0 {
        return false;
    }

    for i in 0..4 {
        let cqe = IoUringCqe::new(200 + i as u64, 512, 0);
        if !cq.push(cqe) {
            return false;
        }
    }

    if cq.push(IoUringCqe::new(999, -1, 0)) {
        return false;
    }
    if cq.overflow.load(Ordering::Acquire) != 1 {
        return false;
    }

    for i in 0..4 {
        let c = cq.pop().expect("cq pop failed");
        if c.user_data != 200 + i as u64 || c.res != 512 {
            return false;
        }
    }
    if cq.count() != 0 {
        return false;
    }

    true
}

pub const MAX_INFLIGHT_REQUESTS: usize = 64;

/// In-flight async I/O request tracking structure.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AsyncIoRequest {
    pub sqe_opcode: u8,
    pub fd: i32,
    pub user_data: u64,
    pub buf_addr: u64,
    pub byte_count: u32,
    pub lba_offset: u64,
    pub nvme_cid: u16,
    pub active: bool,
}

impl AsyncIoRequest {
    pub const fn empty() -> Self {
        Self {
            sqe_opcode: IORING_OP_NOP,
            fd: -1,
            user_data: 0,
            buf_addr: 0,
            byte_count: 0,
            lba_offset: 0,
            nvme_cid: 0,
            active: false,
        }
    }
}

/// Overlapped Async I/O Request Dispatcher bridging io_uring SQEs to NVMe hardware queues.
pub struct AsyncIoDispatcher {
    pub inflight_slots: [AsyncIoRequest; MAX_INFLIGHT_REQUESTS],
    pub inflight_count: usize,
    pub submitted_ops: u64,
    pub next_cid: u16,
}

impl AsyncIoDispatcher {
    pub const fn new() -> Self {
        Self {
            inflight_slots: [AsyncIoRequest::empty(); MAX_INFLIGHT_REQUESTS],
            inflight_count: 0,
            submitted_ops: 0,
            next_cid: 1,
        }
    }

    /// Allocates an in-flight tracking slot for an async request.
    pub fn allocate_slot(&mut self, sqe: &IoUringSqe) -> Option<(usize, u16)> {
        if self.inflight_count >= MAX_INFLIGHT_REQUESTS {
            return None;
        }

        for (idx, slot) in self.inflight_slots.iter_mut().enumerate() {
            if !slot.active {
                let cid = self.next_cid;
                self.next_cid = if self.next_cid >= 0xFFFE { 1 } else { self.next_cid + 1 };

                slot.sqe_opcode = sqe.opcode;
                slot.fd = sqe.fd;
                slot.user_data = sqe.user_data;
                slot.buf_addr = sqe.addr;
                slot.byte_count = sqe.len;
                slot.lba_offset = sqe.off;
                slot.nvme_cid = cid;
                slot.active = true;

                self.inflight_count += 1;
                self.submitted_ops += 1;
                return Some((idx, cid));
            }
        }
        None
    }

    /// Translates an SQE into a native 64-byte NVMe Submission Entry.
    pub fn translate_sqe_to_nvme(
        &mut self,
        sqe: &IoUringSqe,
        nsid: u32,
    ) -> Result<(crate::drivers::storage::nvme::NvmeSubmissionEntry, u16), i32> {
        let (_slot_idx, cid) = self.allocate_slot(sqe).ok_or(-28)?; // -ENOSPC

        match sqe.opcode {
            IORING_OP_NOP => {
                let mut cmd = crate::drivers::storage::nvme::NvmeSubmissionEntry::new(0, cid, nsid);
                cmd.cdw0 |= 0; // NOP-like cmd
                Ok((cmd, cid))
            }
            IORING_OP_READ => {
                let lba = sqe.off / 512;
                let sector_count = ((sqe.len + 511) / 512) as u16;
                let (prp1, prp2) = (sqe.addr, 0u64);
                let mut cmd = crate::drivers::storage::nvme::build_nvm_read(nsid, lba, sector_count, prp1, prp2);
                cmd.cdw0 = (cmd.cdw0 & 0x0000_FFFF) | ((cid as u32) << 16);
                Ok((cmd, cid))
            }
            IORING_OP_WRITE => {
                let lba = sqe.off / 512;
                let sector_count = ((sqe.len + 511) / 512) as u16;
                let (prp1, prp2) = (sqe.addr, 0u64);
                let mut cmd = crate::drivers::storage::nvme::build_nvm_write(nsid, lba, sector_count, prp1, prp2);
                cmd.cdw0 = (cmd.cdw0 & 0x0000_FFFF) | ((cid as u32) << 16);
                Ok((cmd, cid))
            }
            IORING_OP_FSYNC => {
                let cmd = crate::drivers::storage::nvme::NvmeSubmissionEntry::new(
                    crate::drivers::storage::nvme::NVME_NVM_FLUSH,
                    cid,
                    nsid,
                );
                Ok((cmd, cid))
            }
            _ => Err(-22), // -EINVAL (Unsupported opcode)
        }
    }

    /// Dispatches all pending SQEs from a SubmissionQueueRing into an NVMe I/O QueuePair.
    pub fn dispatch_sq_ring<const N: usize>(
        &mut self,
        sq: &mut SubmissionQueueRing<N>,
        nvme_qp: &mut crate::drivers::storage::nvme::NvmeQueuePair,
        nsid: u32,
    ) -> usize {
        let mut dispatched = 0;
        while let Some(sqe) = sq.pop() {
            if let Ok((cmd, _cid)) = self.translate_sqe_to_nvme(&sqe, nsid) {
                nvme_qp.submit_command(cmd);
                dispatched += 1;
            } else {
                break;
            }
        }
        dispatched
    }
}

/// Unit test verifying overlapped SQE translation and NVMe command dispatch.
pub fn verify_async_dispatcher() -> bool {
    let mut dispatcher = AsyncIoDispatcher::new();
    let mut sq = SubmissionQueueRing::<8>::new();
    let mut nvme_qp = crate::drivers::storage::nvme::NvmeQueuePair::new(1, 16, 0, 0);

    // Queue 3 different SQEs: Read, Write, and Fsync
    let sqe_read = IoUringSqe::new_read(3, 0x0001_8000, 1024, 2048, 0xCAFE_0001);
    let sqe_write = IoUringSqe::new_write(3, 0x0002_8000, 512, 4096, 0xCAFE_0002);
    let sqe_fsync = IoUringSqe::new_fsync(3, 0xCAFE_0003);

    assert!(sq.push(sqe_read));
    assert!(sq.push(sqe_write));
    assert!(sq.push(sqe_fsync));

    // Dispatch batch to NVMe queue
    let dispatched = dispatcher.dispatch_sq_ring(&mut sq, &mut nvme_qp, 1);
    if dispatched != 3 || dispatcher.inflight_count != 3 {
        return false;
    }

    // Inspect the generated NVMe submission entries
    let cmd0 = nvme_qp.sq_entries[0];
    if cmd0.opcode() != crate::drivers::storage::nvme::NVME_NVM_READ
        || cmd0.prp1 != 0x0001_8000
        || cmd0.cdw10 != 4 // 2048 / 512 = 4
    {
        return false;
    }

    let cmd1 = nvme_qp.sq_entries[1];
    if cmd1.opcode() != crate::drivers::storage::nvme::NVME_NVM_WRITE
        || cmd1.prp1 != 0x0002_8000
        || cmd1.cdw10 != 8 // 4096 / 512 = 8
    {
        return false;
    }

    let cmd2 = nvme_qp.sq_entries[2];
    if cmd2.opcode() != crate::drivers::storage::nvme::NVME_NVM_FLUSH {
        return false;
    }

    true
}
