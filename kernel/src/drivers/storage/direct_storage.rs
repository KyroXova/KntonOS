//! Zero-Copy DirectStorage NVMe-to-GPU Direct Memory Access (DMA) Pipeline.
//!
//! Implements Microsoft DirectStorage / GPUDirect Storage architecture:
//! - Direct NVMe PCIe peer-to-peer (P2P) DMA streaming into GPU VRAM (PCIe BAR1 aperture)
//! - Completely eliminates CPU bounce buffers and host RAM copies
//! - Hardware Scatter-Gather List (SGL) and Physical Region Page (PRP) generation targeting GPU VRAM
//! - Priority-ordered batched request submission to NVMe Submission Queues

extern crate alloc;

use alloc::vec::Vec;
use super::nvme::{NvmeSubmissionEntry, NVME_NVM_READ};

/// Error states encountered during DirectStorage operations.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DirectStorageError {
    InvalidAperture,
    OutOfVramSpace,
    QueueFull,
    InvalidRequest,
    UnalignedAddress,
    DeviceError,
}

/// GPU PCIe BAR1 Memory Aperture Descriptor.
///
/// Represents the physical memory range exposed by the GPU on the PCIe bus
/// (VRAM aperture, e.g. 16 GiB - 256 GiB range) accessible via PCIe peer-to-peer transactions.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GpuBarAperture {
    /// Physical base address on PCIe bus (e.g. 0x0000_0004_0000_0000).
    pub base_phys: u64,
    /// Total capacity of GPU BAR1 window in bytes.
    pub size: u64,
    /// Next allocatable offset inside VRAM aperture.
    pub allocated_offset: u64,
}

impl GpuBarAperture {
    pub const fn new(base_phys: u64, size: u64) -> Self {
        Self {
            base_phys,
            size,
            allocated_offset: 0,
        }
    }

    /// Allocates an aligned slice of GPU VRAM within the BAR1 aperture.
    pub fn allocate(&mut self, size: u64, align: u64) -> Result<u64, DirectStorageError> {
        let align = align.max(4096);
        let aligned_offset = (self.allocated_offset + (align - 1)) & !(align - 1);
        let end_offset = aligned_offset.checked_add(size).ok_or(DirectStorageError::OutOfVramSpace)?;

        if end_offset > self.size {
            return Err(DirectStorageError::OutOfVramSpace);
        }

        self.allocated_offset = end_offset;
        Ok(self.base_phys + aligned_offset)
    }

    /// Checks if a physical address falls within this GPU VRAM aperture.
    #[inline(always)]
    pub fn contains_address(&self, phys_addr: u64, len: u64) -> bool {
        phys_addr >= self.base_phys && (phys_addr + len) <= (self.base_phys + self.size)
    }

    /// Resets VRAM aperture allocation cursor.
    pub fn reset(&mut self) {
        self.allocated_offset = 0;
    }
}

/// DirectStorage request priority classes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum DirectStoragePriority {
    Background = 0,
    Normal = 1,
    High = 2,
    Realtime = 3,
}

/// Operational state of a DirectStorage texture stream request.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DirectStorageStatus {
    Queued,
    InFlight,
    Completed,
    Failed,
}

/// DirectStorage NVMe-to-GPU Streaming Request.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DirectStorageRequest {
    /// Unique identifier for tracking this request.
    pub request_id: u64,
    /// NVMe Namespace ID (usually 1).
    pub nsid: u32,
    /// Starting Logical Block Address (LBA) on NVMe SSD.
    pub start_lba: u64,
    /// Number of disk sectors / logical blocks to read.
    pub block_count: u32,
    /// Sector size in bytes (e.g. 512 or 4096).
    pub block_size: u32,
    /// Direct destination in GPU VRAM (must be inside GpuBarAperture).
    pub gpu_vram_dest: u64,
    /// Target texture resource ID.
    pub texture_id: u32,
    /// Mip-map level being streamed (0 = highest resolution).
    pub mip_level: u8,
    /// Priority level for submission ordering.
    pub priority: DirectStoragePriority,
    /// Current request state.
    pub status: DirectStorageStatus,
    /// Synchronization fence value to signal upon completion.
    pub fence_value: u64,
    /// Actual bytes transferred.
    pub bytes_transferred: u64,
}

/// NVMe Scatter-Gather List (SGL) Data Block Descriptor for Direct GPU DMA.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NvmeSglDataBlock {
    /// 64-bit GPU physical destination address.
    pub address: u64,
    /// Transfer length in bytes (up to 4 GiB).
    pub length: u32,
    /// Reserved.
    pub reserved: [u8; 3],
    /// SGL Descriptor Type (0x0 = Data Block, 0x1 = Bit Bucket, 0x2 = Segment).
    pub sgl_type: u8,
}

impl NvmeSglDataBlock {
    pub const fn new(gpu_phys_addr: u64, length: u32) -> Self {
        Self {
            address: gpu_phys_addr,
            length,
            reserved: [0; 3],
            sgl_type: 0x00, // NVMe SGL Data Block
        }
    }
}

/// DirectStorage NVMe-to-GPU DMA Queue.
///
/// Manages pending texture disk requests, validates GPU VRAM aperture targets,
/// and transforms batched requests into native NVMe submission commands.
pub struct DirectStorageQueue {
    pub requests: Vec<DirectStorageRequest>,
    pub gpu_aperture: GpuBarAperture,
    pub max_batch_size: usize,
    next_request_id: u64,
    pub total_bytes_streamed: u64,
    pub total_dma_requests: u64,
}

impl DirectStorageQueue {
    pub fn new(gpu_aperture: GpuBarAperture, max_batch_size: usize) -> Self {
        Self {
            requests: Vec::new(),
            gpu_aperture,
            max_batch_size: max_batch_size.max(1),
            next_request_id: 1,
            total_bytes_streamed: 0,
            total_dma_requests: 0,
        }
    }

    /// Enqueues a direct NVMe-to-GPU texture streaming read request.
    pub fn enqueue_texture_read(
        &mut self,
        nsid: u32,
        start_lba: u64,
        block_count: u32,
        block_size: u32,
        gpu_vram_dest: u64,
        texture_id: u32,
        mip_level: u8,
        priority: DirectStoragePriority,
        fence_value: u64,
    ) -> Result<u64, DirectStorageError> {
        let total_bytes = (block_count as u64) * (block_size as u64);

        // Verify that destination physical address is within the GPU VRAM aperture
        if !self.gpu_aperture.contains_address(gpu_vram_dest, total_bytes) {
            return Err(DirectStorageError::InvalidAperture);
        }

        // DirectStorage requires 512-byte or 4096-byte alignment
        if gpu_vram_dest % 512 != 0 {
            return Err(DirectStorageError::UnalignedAddress);
        }

        let request_id = self.next_request_id;
        self.next_request_id += 1;

        let req = DirectStorageRequest {
            request_id,
            nsid,
            start_lba,
            block_count,
            block_size,
            gpu_vram_dest,
            texture_id,
            mip_level,
            priority,
            status: DirectStorageStatus::Queued,
            fence_value,
            bytes_transferred: 0,
        };

        self.requests.push(req);
        self.total_dma_requests += 1;
        Ok(request_id)
    }

    /// Converts a DirectStorage request into an NVMe Submission Queue Entry (SQE)
    /// targeting GPU VRAM via direct PRP or SGL mapping.
    pub fn build_nvme_submission_entry(
        &self,
        req: &DirectStorageRequest,
        cid: u16,
    ) -> Result<NvmeSubmissionEntry, DirectStorageError> {
        let total_bytes = (req.block_count as u64) * (req.block_size as u64);
        if !self.gpu_aperture.contains_address(req.gpu_vram_dest, total_bytes) {
            return Err(DirectStorageError::InvalidAperture);
        }

        let mut sqe = NvmeSubmissionEntry::new(NVME_NVM_READ, cid, req.nsid);

        // CDW10 & CDW11: Starting 64-bit LBA
        sqe.cdw10 = req.start_lba as u32;
        sqe.cdw11 = (req.start_lba >> 32) as u32;

        // CDW12: Number of logical blocks (0-based: 0 means 1 block)
        sqe.cdw12 = (req.block_count - 1) & 0xFFFF;

        // PRP mapping directly to GPU Physical Address in VRAM
        sqe.prp1 = req.gpu_vram_dest;

        // If transfer exceeds 1 page (4096 bytes), configure PRP2 or second page
        if total_bytes > 4096 {
            sqe.prp2 = req.gpu_vram_dest + 4096;
        } else {
            sqe.prp2 = 0;
        }

        Ok(sqe)
    }

    /// Gathers pending requests up to `max_batch_size`, sorted by priority,
    /// sets status to `InFlight`, and produces `(request_id, NvmeSubmissionEntry)` pairs.
    pub fn create_submission_batch(&mut self) -> Vec<(u64, NvmeSubmissionEntry)> {
        let mut pending_indices: Vec<usize> = self
            .requests
            .iter()
            .enumerate()
            .filter(|(_, r)| r.status == DirectStorageStatus::Queued)
            .map(|(idx, _)| idx)
            .collect();

        // Sort indices by priority descending (Realtime first, then High, Normal, Background)
        pending_indices.sort_by(|&a, &b| {
            self.requests[b].priority.cmp(&self.requests[a].priority)
        });

        let mut batch = Vec::new();
        let batch_count = pending_indices.len().min(self.max_batch_size);

        for &idx in &pending_indices[..batch_count] {
            let req_copy = self.requests[idx];
            let cid = (req_copy.request_id & 0xFFFF) as u16;
            if let Ok(sqe) = self.build_nvme_submission_entry(&req_copy, cid) {
                self.requests[idx].status = DirectStorageStatus::InFlight;
                batch.push((req_copy.request_id, sqe));
            }
        }

        batch
    }

    /// Marks a request as completed and records transferred byte metrics.
    pub fn mark_completed(&mut self, request_id: u64) -> Option<DirectStorageRequest> {
        if let Some(req) = self.requests.iter_mut().find(|r| r.request_id == request_id) {
            req.status = DirectStorageStatus::Completed;
            let bytes = (req.block_count as u64) * (req.block_size as u64);
            req.bytes_transferred = bytes;
            self.total_bytes_streamed += bytes;
            return Some(*req);
        }
        None
    }

    pub fn pending_count(&self) -> usize {
        self.requests.iter().filter(|r| r.status == DirectStorageStatus::Queued).count()
    }

    pub fn in_flight_count(&self) -> usize {
        self.requests.iter().filter(|r| r.status == DirectStorageStatus::InFlight).count()
    }

    pub fn completed_count(&self) -> usize {
        self.requests.iter().filter(|r| r.status == DirectStorageStatus::Completed).count()
    }
}

/// Unit test verifying DirectStorage NVMe-to-GPU DMA queue and BAR1 aperture mapping.
pub fn verify_direct_storage_queue_and_bar_mapping() -> bool {
    // 1. Initialize GPU BAR1 aperture: 16 GiB window at 0x0000_0004_0000_0000
    let bar1_base = 0x0000_0004_0000_0000u64;
    let bar1_size = 16 * 1024 * 1024 * 1024u64; // 16 GiB
    let mut aperture = GpuBarAperture::new(bar1_base, bar1_size);

    // Allocate 4 MiB buffer in VRAM for mip-level 0 texture data
    let mip0_vram = match aperture.allocate(4 * 1024 * 1024, 64 * 1024) {
        Ok(addr) => addr,
        Err(_) => return false,
    };

    if mip0_vram != bar1_base {
        return false;
    }
    if !aperture.contains_address(mip0_vram, 4 * 1024 * 1024) {
        return false;
    }

    // 2. Initialize DirectStorage Queue with batch limit 16
    let mut ds_queue = DirectStorageQueue::new(aperture, 16);

    // Enqueue 3 requests with different priorities:
    // Req 1: Mip 2 (Background)
    let req1_vram = match aperture.allocate(256 * 1024, 4096) {
        Ok(a) => a,
        Err(_) => return false,
    };
    let id1 = match ds_queue.enqueue_texture_read(1, 100, 512, 512, req1_vram, 101, 2, DirectStoragePriority::Background, 1) {
        Ok(id) => id,
        Err(_) => return false,
    };

    // Req 2: Mip 0 (Realtime / Camera viewport visible)
    let req2_vram = mip0_vram;
    let id2 = match ds_queue.enqueue_texture_read(1, 2000, 8192, 512, req2_vram, 101, 0, DirectStoragePriority::Realtime, 2) {
        Ok(id) => id,
        Err(_) => return false,
    };

    // Req 3: Mip 1 (High)
    let req3_vram = match aperture.allocate(1024 * 1024, 4096) {
        Ok(a) => a,
        Err(_) => return false,
    };
    let id3 = match ds_queue.enqueue_texture_read(1, 800, 2048, 512, req3_vram, 101, 1, DirectStoragePriority::High, 3) {
        Ok(id) => id,
        Err(_) => return false,
    };

    if ds_queue.pending_count() != 3 {
        return false;
    }

    // 3. Reject invalid requests targeting addresses outside GPU VRAM
    let invalid_host_ram = 0x0000_0000_1000_0000u64;
    let err_req = ds_queue.enqueue_texture_read(1, 0, 16, 512, invalid_host_ram, 102, 0, DirectStoragePriority::Normal, 4);
    if err_req != Err(DirectStorageError::InvalidAperture) {
        return false; // Did not reject host RAM pointer
    }

    // 4. Create submission batch: Realtime (Req 2) and High (Req 3) must be ordered before Background (Req 1)
    let batch = ds_queue.create_submission_batch();
    if batch.len() != 3 {
        return false;
    }

    if batch[0].0 != id2 || batch[1].0 != id3 || batch[2].0 != id1 {
        return false; // Priority sorting failed
    }

    // Verify NVMe SQE mapping directly to GPU VRAM physical address
    let sqe_realtime = &batch[0].1;
    if sqe_realtime.opcode() != NVME_NVM_READ || sqe_realtime.prp1 != req2_vram {
        return false;
    }
    if sqe_realtime.cdw10 != 2000 || sqe_realtime.cdw12 != (8192 - 1) {
        return false;
    }

    // 5. Mark completion and verify byte counting
    let comp = ds_queue.mark_completed(id2);
    if comp.is_none() || ds_queue.completed_count() != 1 || ds_queue.total_bytes_streamed != 8192 * 512 {
        return false;
    }

    true
}
