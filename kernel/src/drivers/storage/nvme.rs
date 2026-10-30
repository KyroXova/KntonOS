//! NVMe (Non-Volatile Memory Express) 1.4 Specification Memory Registers & Command Descriptors.
//!
//! Provides memory-mapped register definitions for NVMe host controllers,
//! 64-byte Submission Queue Entries (SQE), 16-byte Completion Queue Entries (CQE),
//! and Controller Capabilities (CAP) / Configuration (CC) bit decoders.

pub const PCI_CLASS_MASS_STORAGE: u8 = 0x01;
pub const PCI_SUBCLASS_NVME: u8 = 0x08;
pub const PCI_PROGIF_NVME: u8 = 0x02;

// --- NVMe Controller MMIO Register Offsets ---
pub const NVME_REG_CAP: usize = 0x0000;    // Controller Capabilities (64-bit)
pub const NVME_REG_VS: usize = 0x0008;     // Version (32-bit)
pub const NVME_REG_INTMS: usize = 0x000C;  // Interrupt Mask Set (32-bit)
pub const NVME_REG_INTMC: usize = 0x0010;  // Interrupt Mask Clear (32-bit)
pub const NVME_REG_CC: usize = 0x0014;     // Controller Configuration (32-bit)
pub const NVME_REG_CSTS: usize = 0x001C;   // Controller Status (32-bit)
pub const NVME_REG_NSSR: usize = 0x0020;   // NVM Subsystem Reset (32-bit)
pub const NVME_REG_AQA: usize = 0x0024;    // Admin Queue Attributes (32-bit)
pub const NVME_REG_ASQ: usize = 0x0028;    // Admin Submission Queue Base Address (64-bit)
pub const NVME_REG_ACQ: usize = 0x0030;    // Admin Completion Queue Base Address (64-bit)
pub const NVME_REG_CMBLOC: usize = 0x0038; // Controller Memory Buffer Location (32-bit)
pub const NVME_REG_CMBSZ: usize = 0x003C;  // Controller Memory Buffer Size (32-bit)
pub const NVME_DOORBELL_BASE: usize = 0x1000; // Doorbell Registers Base Offset

// --- Controller Capabilities (CAP) Bitfields ---
pub const CAP_MQES_MASK: u64 = 0xFFFF;     // Maximum Queue Entries Supported (0-based)
pub const CAP_CQS: u64 = 1 << 16;          // Contiguous Queues Required
pub const CAP_AMS_WRR: u64 = 1 << 17;      // Weighted Round Robin with Urgent Priority
pub const CAP_AMS_VENDOR: u64 = 1 << 18;   // Vendor Specific Arbitration
pub const CAP_TO_SHIFT: usize = 24;        // Timeout Shift
pub const CAP_TO_MASK: u64 = 0xFF << 24;   // Timeout in 500 ms units
pub const CAP_DSTRD_SHIFT: usize = 32;     // Doorbell Stride Shift (2 ^ (2 + DSTRD))
pub const CAP_DSTRD_MASK: u64 = 0x0F << 32;
pub const CAP_NSSRS: u64 = 1 << 36;        // NVM Subsystem Reset Supported
pub const CAP_CSS_NVM: u64 = 1 << 37;      // NVM Command Set Supported
pub const CAP_BPS: u64 = 1 << 45;          // Boot Partition Support
pub const CAP_MPSMIN_SHIFT: usize = 48;    // Memory Page Size Minimum (2 ^ (12 + MPSMIN))
pub const CAP_MPSMIN_MASK: u64 = 0x0F << 48;
pub const CAP_MPSMAX_SHIFT: usize = 52;    // Memory Page Size Maximum (2 ^ (12 + MPSMAX))
pub const CAP_MPSMAX_MASK: u64 = 0x0F << 52;

// --- Controller Configuration (CC) Bitfields ---
pub const CC_EN: u32 = 1 << 0;             // Enable
pub const CC_CSS_NVM: u32 = 0 << 4;        // Command Set Selected (NVM)
pub const CC_MPS_SHIFT: usize = 7;         // Memory Page Size (2 ^ (12 + MPS))
pub const CC_AMS_RR: u32 = 0 << 11;        // Round Robin Arbitration
pub const CC_SHN_NONE: u32 = 0 << 14;      // No shutdown notification
pub const CC_SHN_NORMAL: u32 = 1 << 14;    // Normal shutdown notification
pub const CC_SHN_ABRUPT: u32 = 2 << 14;    // Abrupt shutdown notification
pub const CC_IOSQES_64: u32 = 6 << 16;     // I/O SQ Entry Size: 2^6 = 64 bytes
pub const CC_IOCQES_16: u32 = 4 << 20;     // I/O CQ Entry Size: 2^4 = 16 bytes

// --- Controller Status (CSTS) Bitfields ---
pub const CSTS_RDY: u32 = 1 << 0;          // Ready
pub const CSTS_CFS: u32 = 1 << 1;          // Controller Fatal Status
pub const CSTS_SHST_MASK: u32 = 3 << 2;    // Shutdown Status Mask
pub const CSTS_SHST_NORMAL: u32 = 0 << 2;  // Normal operation
pub const CSTS_SHST_OCCURRING: u32 = 1 << 2;
pub const CSTS_SHST_COMPLETE: u32 = 2 << 2;
pub const CSTS_NSSRO: u32 = 1 << 4;        // NVM Subsystem Reset Occurred

// --- Admin & NVM Command Opcodes ---
pub const NVME_ADMIN_DELETE_SQ: u8 = 0x00;
pub const NVME_ADMIN_CREATE_SQ: u8 = 0x01;
pub const NVME_ADMIN_GET_LOG_PAGE: u8 = 0x02;
pub const NVME_ADMIN_DELETE_CQ: u8 = 0x04;
pub const NVME_ADMIN_CREATE_CQ: u8 = 0x05;
pub const NVME_ADMIN_IDENTIFY: u8 = 0x06;
pub const NVME_ADMIN_ABORT: u8 = 0x08;
pub const NVME_ADMIN_SET_FEATURES: u8 = 0x09;
pub const NVME_ADMIN_GET_FEATURES: u8 = 0x0A;

pub const NVME_NVM_FLUSH: u8 = 0x00;
pub const NVME_NVM_WRITE: u8 = 0x01;
pub const NVME_NVM_READ: u8 = 0x02;

// --- Admin Queue Attributes (AQA) Helper ---
#[inline(always)]
pub const fn make_aqa(asqs: u16, acqs: u16) -> u32 {
    ((acqs as u32) << 16) | (asqs as u32)
}

// --- Doorbell Offset Helper ---
#[inline(always)]
pub const fn nvme_sq_doorbell(qid: u16, dstrd: usize) -> usize {
    NVME_DOORBELL_BASE + (2 * qid as usize) * (4 << dstrd)
}

#[inline(always)]
pub const fn nvme_cq_doorbell(qid: u16, dstrd: usize) -> usize {
    NVME_DOORBELL_BASE + (2 * qid as usize + 1) * (4 << dstrd)
}

/// 64-byte Submission Queue Entry (SQE) according to NVMe specification.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NvmeSubmissionEntry {
    pub cdw0: u32,       // Opcode [7:0], Fused [9:8], PSDT [15:14], CID [31:16]
    pub nsid: u32,       // Namespace Identifier
    pub reserved: [u32; 2],
    pub mptr: u64,       // Metadata Pointer
    pub prp1: u64,       // PRP Entry 1
    pub prp2: u64,       // PRP Entry 2 / PRP List Pointer
    pub cdw10: u32,      // Command Specific Dwords
    pub cdw11: u32,
    pub cdw12: u32,
    pub cdw13: u32,
    pub cdw14: u32,
    pub cdw15: u32,
}

impl NvmeSubmissionEntry {
    pub const fn empty() -> Self {
        Self {
            cdw0: 0,
            nsid: 0,
            reserved: [0; 2],
            mptr: 0,
            prp1: 0,
            prp2: 0,
            cdw10: 0,
            cdw11: 0,
            cdw12: 0,
            cdw13: 0,
            cdw14: 0,
            cdw15: 0,
        }
    }

    #[inline(always)]
    pub fn new(opcode: u8, cid: u16, nsid: u32) -> Self {
        let mut entry = Self::empty();
        entry.cdw0 = (opcode as u32) | ((cid as u32) << 16);
        entry.nsid = nsid;
        entry
    }

    #[inline(always)]
    pub fn opcode(&self) -> u8 {
        (self.cdw0 & 0xFF) as u8
    }

    #[inline(always)]
    pub fn command_id(&self) -> u16 {
        (self.cdw0 >> 16) as u16
    }
}

/// 16-byte Completion Queue Entry (CQE) according to NVMe specification.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NvmeCompletionEntry {
    pub result: u32,     // Command-specific result
    pub reserved: u32,
    pub sq_head: u16,    // SQ Head Pointer
    pub sq_id: u16,      // SQ Identifier
    pub cid: u16,        // Command Identifier
    pub status: u16,     // Phase Tag (bit 0), Status Code (bits 15:1)
}

impl NvmeCompletionEntry {
    pub const fn empty() -> Self {
        Self {
            result: 0,
            reserved: 0,
            sq_head: 0,
            sq_id: 0,
            cid: 0,
            status: 0,
        }
    }

    #[inline(always)]
    pub fn phase(&self) -> bool {
        (self.status & 1) != 0
    }

    #[inline(always)]
    pub fn status_code(&self) -> u16 {
        (self.status >> 1) & 0x7FFF
    }

    #[inline(always)]
    pub fn is_success(&self) -> bool {
        self.status_code() == 0
    }
}

/// Decoded NVMe Controller Capabilities (CAP).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NvmeCapabilities {
    pub max_queue_entries: u16, // MQES + 1
    pub contiguous_queues_required: bool,
    pub timeout_ms: u32,        // TO * 500 ms
    pub doorbell_stride_bytes: usize, // 4 << DSTRD
    pub doorbell_stride_shift: usize, // DSTRD
    pub nvm_subsystem_reset: bool,
    pub nvm_command_set: bool,
    pub min_page_size: usize,   // 1 << (12 + MPSMIN)
    pub max_page_size: usize,   // 1 << (12 + MPSMAX)
}

impl NvmeCapabilities {
    pub fn from_raw(raw: u64) -> Self {
        let mqes = (raw & CAP_MQES_MASK) as u16;
        let cqs = (raw & CAP_CQS) != 0;
        let to = ((raw & CAP_TO_MASK) >> CAP_TO_SHIFT) as u32;
        let dstrd = ((raw & CAP_DSTRD_MASK) >> CAP_DSTRD_SHIFT) as usize;
        let nssrs = (raw & CAP_NSSRS) != 0;
        let css_nvm = (raw & CAP_CSS_NVM) != 0;
        let mpsmin = ((raw & CAP_MPSMIN_MASK) >> CAP_MPSMIN_SHIFT) as usize;
        let mpsmax = ((raw & CAP_MPSMAX_MASK) >> CAP_MPSMAX_SHIFT) as usize;

        Self {
            max_queue_entries: mqes.saturating_add(1),
            contiguous_queues_required: cqs,
            timeout_ms: to * 500,
            doorbell_stride_bytes: 4 << dstrd,
            doorbell_stride_shift: dstrd,
            nvm_subsystem_reset: nssrs,
            nvm_command_set: css_nvm,
            min_page_size: 1 << (12 + mpsmin),
            max_page_size: 1 << (12 + mpsmax),
        }
    }
}

/// Unit test verifying NVMe memory structures, sizing, and capability decoding.
pub fn verify_nvme_structures() -> bool {
    // 1. Verify exact sizes required by NVMe specification
    assert_eq!(core::mem::size_of::<NvmeSubmissionEntry>(), 64);
    assert_eq!(core::mem::size_of::<NvmeCompletionEntry>(), 16);

    // 2. Verify capability decoding with standard QEMU NVMe CAP value
    // QEMU typical CAP: MQES=1023 (1024 entries), CQS=1, TO=120 (60s), DSTRD=0 (4 bytes),
    // CSS=1 (NVM command set), MPSMIN=0 (4 KiB)
    let raw_cap: u64 = 1023
        | CAP_CQS
        | (120 << CAP_TO_SHIFT)
        | (0 << CAP_DSTRD_SHIFT)
        | CAP_CSS_NVM
        | (0 << CAP_MPSMIN_SHIFT)
        | (0 << CAP_MPSMAX_SHIFT);

    let caps = NvmeCapabilities::from_raw(raw_cap);
    assert_eq!(caps.max_queue_entries, 1024);
    assert!(caps.contiguous_queues_required);
    assert_eq!(caps.timeout_ms, 60_000);
    assert_eq!(caps.doorbell_stride_bytes, 4);
    assert_eq!(caps.doorbell_stride_shift, 0);
    assert!(caps.nvm_command_set);
    assert_eq!(caps.min_page_size, 4096);
    assert_eq!(caps.max_page_size, 4096);

    // 3. Verify doorbell offset computation for DSTRD=0
    assert_eq!(nvme_sq_doorbell(0, 0), 0x1000);
    assert_eq!(nvme_cq_doorbell(0, 0), 0x1004);
    assert_eq!(nvme_sq_doorbell(1, 0), 0x1008);
    assert_eq!(nvme_cq_doorbell(1, 0), 0x100C);

    // 4. Verify AQA packing
    assert_eq!(make_aqa(63, 63), (63 << 16) | 63);

    // 5. Verify SQE and CQE field decoding
    let sqe = NvmeSubmissionEntry::new(NVME_NVM_READ, 0x1234, 1);
    assert_eq!(sqe.opcode(), NVME_NVM_READ);
    assert_eq!(sqe.command_id(), 0x1234);
    assert_eq!(sqe.nsid, 1);

    let mut cqe = NvmeCompletionEntry::empty();
    cqe.cid = 0x1234;
    cqe.status = 1; // Phase=1, SC=0 (success)
    assert!(cqe.phase());
    assert_eq!(cqe.status_code(), 0);
    assert!(cqe.is_success());

    true
}

pub const NVME_MAX_QUEUE_SIZE: usize = 64;

/// Errors returned by NVMe controller and command operations.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NvmeError {
    ControllerNotReady,
    QueueCreationFailed,
    Timeout,
    CommandFailed(u16),
    InvalidParameter,
    DmaError,
}

/// Circular ring buffer pair for NVMe Submission Queue (SQ) and Completion Queue (CQ).
pub struct NvmeQueuePair {
    pub qid: u16,
    pub size: u16,
    pub sq_tail: u16,
    pub sq_head: u16,
    pub cq_head: u16,
    pub cq_phase: bool,
    pub next_cid: u16,
    pub sq_phys: u64,
    pub cq_phys: u64,
    pub sq_doorbell_offset: usize,
    pub cq_doorbell_offset: usize,
    pub mmio_base: usize, // 0 if simulated / virtualized
    pub sq_entries: [NvmeSubmissionEntry; NVME_MAX_QUEUE_SIZE],
    pub cq_entries: [NvmeCompletionEntry; NVME_MAX_QUEUE_SIZE],
}

impl NvmeQueuePair {
    pub const fn new(qid: u16, size: u16, dstrd: usize, mmio_base: usize) -> Self {
        let q_size = if size > (NVME_MAX_QUEUE_SIZE as u16) {
            NVME_MAX_QUEUE_SIZE as u16
        } else if size == 0 {
            2
        } else {
            size
        };

        Self {
            qid,
            size: q_size,
            sq_tail: 0,
            sq_head: 0,
            cq_head: 0,
            cq_phase: true, // NVMe spec: CQ phase begins at 1 (true)
            next_cid: 1,
            sq_phys: 0,
            cq_phys: 0,
            sq_doorbell_offset: nvme_sq_doorbell(qid, dstrd),
            cq_doorbell_offset: nvme_cq_doorbell(qid, dstrd),
            mmio_base,
            sq_entries: [NvmeSubmissionEntry::empty(); NVME_MAX_QUEUE_SIZE],
            cq_entries: [NvmeCompletionEntry::empty(); NVME_MAX_QUEUE_SIZE],
        }
    }

    pub fn set_dma_buffers(&mut self, sq_phys: u64, cq_phys: u64) {
        self.sq_phys = sq_phys;
        self.cq_phys = cq_phys;
    }

    /// Submits an SQE to the Submission Queue and rings the SQ doorbell.
    pub fn submit_command(&mut self, mut entry: NvmeSubmissionEntry) -> u16 {
        let cid = self.next_cid;
        self.next_cid = self.next_cid.wrapping_add(1);
        if self.next_cid == 0 {
            self.next_cid = 1;
        }

        // Set CID in cdw0: opcode in bits 7:0, CID in bits 31:16
        entry.cdw0 = (entry.cdw0 & 0x0000_FFFF) | ((cid as u32) << 16);

        self.sq_entries[self.sq_tail as usize] = entry;
        self.sq_tail = (self.sq_tail + 1) % self.size;

        self.ring_sq_doorbell();
        cid
    }

    pub fn ring_sq_doorbell(&self) {
        if self.mmio_base != 0 {
            unsafe {
                let db_ptr = (self.mmio_base + self.sq_doorbell_offset) as *mut u32;
                core::ptr::write_volatile(db_ptr, self.sq_tail as u32);
            }
        }
    }

    pub fn ring_cq_doorbell(&self) {
        if self.mmio_base != 0 {
            unsafe {
                let db_ptr = (self.mmio_base + self.cq_doorbell_offset) as *mut u32;
                core::ptr::write_volatile(db_ptr, self.cq_head as u32);
            }
        }
    }

    /// Polls the Completion Queue for a completion matching `cid`.
    pub fn poll_completion(&mut self, cid: u16, max_spins: usize) -> Result<NvmeCompletionEntry, NvmeError> {
        let mut spins = 0;
        loop {
            let entry = self.cq_entries[self.cq_head as usize];
            if entry.phase() == self.cq_phase {
                // Advance CQ head
                self.cq_head = (self.cq_head + 1) % self.size;
                if self.cq_head == 0 {
                    self.cq_phase = !self.cq_phase; // Invert phase bit on queue wraparound
                }

                self.ring_cq_doorbell();

                if entry.cid == cid {
                    if !entry.is_success() {
                        return Err(NvmeError::CommandFailed(entry.status_code()));
                    }
                    return Ok(entry);
                }
            }

            spins += 1;
            if spins >= max_spins {
                return Err(NvmeError::Timeout);
            }
            core::hint::spin_loop();
        }
    }

    /// Simulates hardware completing a command (for loopback self-tests).
    pub fn simulate_completion(&mut self, cid: u16, result: u32, status_code: u16) {
        let idx = self.cq_head as usize;
        let mut entry = NvmeCompletionEntry::empty();
        entry.result = result;
        entry.cid = cid;
        entry.sq_id = self.qid;
        entry.sq_head = self.sq_tail;
        let phase_bit = if self.cq_phase { 1u16 } else { 0u16 };
        entry.status = (status_code << 1) | phase_bit;

        self.cq_entries[idx] = entry;
    }
}

/// Unit test verifying NVMe Submission and Completion Queue rings, phase toggling, and doorbells.
pub fn verify_nvme_queue_engine() -> bool {
    let mut qp = NvmeQueuePair::new(0, 4, 0, 0); // 4-entry queue, DSTRD=0, simulated
    assert_eq!(qp.size, 4);
    assert_eq!(qp.sq_tail, 0);
    assert_eq!(qp.cq_head, 0);
    assert!(qp.cq_phase);

    // 1. Submit command 1
    let cmd1 = NvmeSubmissionEntry::new(NVME_ADMIN_IDENTIFY, 0, 0);
    let cid1 = qp.submit_command(cmd1);
    assert_eq!(cid1, 1);
    assert_eq!(qp.sq_tail, 1);
    assert_eq!(qp.sq_entries[0].command_id(), 1);

    // 2. Simulate completion for CID 1
    qp.simulate_completion(cid1, 0x55AA, 0);
    let cqe1 = qp.poll_completion(cid1, 10).expect("poll_completion for cmd1 failed");
    assert_eq!(cqe1.cid, 1);
    assert_eq!(cqe1.result, 0x55AA);
    assert_eq!(qp.cq_head, 1);
    assert!(qp.cq_phase); // Still phase 1

    // 3. Fill and drain remaining slots to trigger ring wraparound and phase toggle
    let cid2 = qp.submit_command(NvmeSubmissionEntry::new(NVME_ADMIN_GET_FEATURES, 0, 0));
    let cid3 = qp.submit_command(NvmeSubmissionEntry::new(NVME_ADMIN_GET_FEATURES, 0, 0));
    let cid4 = qp.submit_command(NvmeSubmissionEntry::new(NVME_ADMIN_GET_FEATURES, 0, 0));
    assert_eq!(qp.sq_tail, 0); // Wraps around (1 + 3 = 4 -> 0)

    qp.simulate_completion(cid2, 0, 0);
    assert!(qp.poll_completion(cid2, 10).is_ok());
    assert_eq!(qp.cq_head, 2);

    qp.simulate_completion(cid3, 0, 0);
    assert!(qp.poll_completion(cid3, 10).is_ok());
    assert_eq!(qp.cq_head, 3);

    qp.simulate_completion(cid4, 0, 0);
    assert!(qp.poll_completion(cid4, 10).is_ok());
    assert_eq!(qp.cq_head, 0); // Wrapped around to 0
    assert!(!qp.cq_phase);     // Phase inverted to false (0)!

    // 4. Test next cycle with inverted phase
    let cid5 = qp.submit_command(NvmeSubmissionEntry::new(NVME_NVM_READ, 0, 1));
    qp.simulate_completion(cid5, 0xCAFE, 0);
    let cqe5 = qp.poll_completion(cid5, 10).expect("poll_completion with inverted phase failed");
    assert_eq!(cqe5.cid, cid5);
    assert_eq!(cqe5.result, 0xCAFE);
    assert!(!cqe5.phase()); // CQE phase is 0 matching inverted queue phase

    // 5. Test command failure status
    let cid6 = qp.submit_command(NvmeSubmissionEntry::new(NVME_NVM_WRITE, 0, 1));
    qp.simulate_completion(cid6, 0, 0x02); // Status code 0x02 = Invalid Field
    match qp.poll_completion(cid6, 10) {
        Err(NvmeError::CommandFailed(0x02)) => {}
        _ => panic!("Expected CommandFailed(0x02)"),
    }

    true
}


// --- Admin IDENTIFY Data Structures (4096 bytes each) ---

/// Power State Descriptor (32 bytes).
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NvmePowerStateDesc {
    pub max_power: u16,
    pub rsvd2: u8,
    pub flags: u8,
    pub entry_lat: u32,
    pub exit_lat: u32,
    pub read_tput: u8,
    pub read_lat: u8,
    pub write_tput: u8,
    pub write_lat: u8,
    pub idle_power: u16,
    pub idle_scale: u8,
    pub rsvd19: u8,
    pub active_power: u16,
    pub active_work_scale: u8,
    pub rsvd23: [u8; 9],
}

/// 4096-byte IDENTIFY CONTROLLER Data Structure according to NVMe 1.4 spec.
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct NvmeIdentifyController {
    pub vid: u16,
    pub ssvid: u16,
    pub sn: [u8; 20],        // Serial Number (ASCII)
    pub mn: [u8; 40],        // Model Number (ASCII)
    pub fr: [u8; 8],         // Firmware Revision (ASCII)
    pub rab: u8,
    pub ieee: [u8; 3],
    pub mic: u8,
    pub mdts: u8,
    pub cntlid: u16,
    pub ver: u32,
    pub rtr: u32,
    pub rgr: [u8; 16],
    pub oaes: u32,
    pub ctratt: u32,
    pub rsvd112: [u8; 144],
    pub oacs: u16,
    pub acl: u8,
    pub aerl: u8,
    pub frmw: u8,
    pub lpa: u8,
    pub elpe: u8,
    pub npss: u8,
    pub avscc: u8,
    pub apsta: u8,
    pub wctemp: u16,
    pub cctemp: u16,
    pub mtfa: u16,
    pub hmpre: u32,
    pub hmmin: u32,
    pub tnvmcap: [u8; 16],
    pub unvmcap: [u8; 16],
    pub rsvd320: [u8; 192],
    pub sqes: u8,
    pub cqes: u8,
    pub maxcmd: u16,
    pub nn: u32,             // Number of Namespaces
    pub oncs: u16,
    pub fuses: u16,
    pub fna: u8,
    pub vwc: u8,
    pub awun: u16,
    pub awupf: u16,
    pub nvscc: u8,
    pub nwpc: u8,
    pub acwu: u16,
    pub rsvd534: [u8; 2],
    pub sgls: u32,
    pub rsvd540: [u8; 164],
    pub rsvd704: [u8; 1344],
    pub psd: [NvmePowerStateDesc; 32], // 32 * 32 = 1024 bytes
    pub vs: [u8; 1024],
}

/// LBA Format Descriptor (4 bytes).
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct NvmeLbaFormat {
    pub ms: u16,             // Metadata Size
    pub lbads: u8,           // LBA Data Size (2 ^ LBADS, e.g. 9 = 512, 12 = 4096)
    pub rp: u8,              // Relative Performance
}

/// 4096-byte IDENTIFY NAMESPACE Data Structure according to NVMe 1.4 spec.
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct NvmeIdentifyNamespace {
    pub nsze: u64,           // Namespace Size in logical blocks
    pub ncap: u64,           // Namespace Capacity in logical blocks
    pub nuse: u64,           // Namespace Utilization in logical blocks
    pub nsfeat: u8,
    pub nlbaf: u8,           // Number of LBA Formats (0-based)
    pub flbas: u8,           // Formatted LBA Size (bits 3:0 = format index)
    pub mc: u8,
    pub dpc: u8,
    pub dps: u8,
    pub nmic: u8,
    pub rescap: u8,
    pub fpi: u8,
    pub dlfeat: u8,
    pub nawun: u16,
    pub nawupf: u16,
    pub nacwu: u16,
    pub nabsn: u16,
    pub nabo: u16,
    pub nabspf: u16,
    pub noiob: u16,
    pub nvmcap: [u8; 16],
    pub rsvd64: [u8; 40],
    pub nguid: [u8; 16],
    pub eui64: u64,
    pub lbaf: [NvmeLbaFormat; 16], // 16 * 4 = 64 bytes
    pub rsvd192: [u8; 192],
    pub vs: [u8; 3712],
}

impl NvmeIdentifyNamespace {
    /// Returns the formatted sector size in bytes (e.g. 512 or 4096).
    pub fn sector_size(&self) -> usize {
        let fmt_idx = (self.flbas & 0x0F) as usize;
        let ds = if fmt_idx < 16 {
            self.lbaf[fmt_idx].lbads
        } else {
            9
        };
        if ds == 0 {
            512
        } else {
            1 << ds
        }
    }
}

// --- Admin & NVM Command Builders ---

/// Builds an Admin IDENTIFY CONTROLLER (CNS = 1) command.
pub fn build_identify_controller(prp1: u64) -> NvmeSubmissionEntry {
    let mut sqe = NvmeSubmissionEntry::new(NVME_ADMIN_IDENTIFY, 0, 0);
    sqe.prp1 = prp1;
    sqe.cdw10 = 0x01; // CNS = 1 (Identify Controller)
    sqe
}

/// Builds an Admin IDENTIFY NAMESPACE (CNS = 0) command.
pub fn build_identify_namespace(nsid: u32, prp1: u64) -> NvmeSubmissionEntry {
    let mut sqe = NvmeSubmissionEntry::new(NVME_ADMIN_IDENTIFY, 0, nsid);
    sqe.prp1 = prp1;
    sqe.cdw10 = 0x00; // CNS = 0 (Identify Namespace)
    sqe
}

/// Builds an NVM 64-bit LBA READ command.
pub fn build_nvm_read(nsid: u32, slba: u64, blocks: u16, prp1: u64, prp2: u64) -> NvmeSubmissionEntry {
    let mut sqe = NvmeSubmissionEntry::new(NVME_NVM_READ, 0, nsid);
    sqe.prp1 = prp1;
    sqe.prp2 = prp2;
    sqe.cdw10 = (slba & 0xFFFF_FFFF) as u32;
    sqe.cdw11 = ((slba >> 32) & 0xFFFF_FFFF) as u32;
    sqe.cdw12 = (blocks.saturating_sub(1)) as u32; // 0-based block count
    sqe
}

/// Builds an NVM 64-bit LBA WRITE command.
pub fn build_nvm_write(nsid: u32, slba: u64, blocks: u16, prp1: u64, prp2: u64) -> NvmeSubmissionEntry {
    let mut sqe = NvmeSubmissionEntry::new(NVME_NVM_WRITE, 0, nsid);
    sqe.prp1 = prp1;
    sqe.prp2 = prp2;
    sqe.cdw10 = (slba & 0xFFFF_FFFF) as u32;
    sqe.cdw11 = ((slba >> 32) & 0xFFFF_FFFF) as u32;
    sqe.cdw12 = (blocks.saturating_sub(1)) as u32; // 0-based block count
    sqe
}

/// Constructs PRP1 and PRP2 for DMA buffer transfers according to NVMe page boundaries.
///
/// If buffer spans <= 4096 bytes: PRP1 = buf, PRP2 = 0.
/// If buffer spans <= 8192 bytes: PRP1 = buf, PRP2 = buf + 4096.
/// If buffer spans > 8192 bytes: PRP1 = buf, PRP2 = prp_list_phys (with list filled).
pub fn build_prp(
    buffer_phys: u64,
    byte_len: usize,
    prp_list_phys: u64,
    prp_list_slice: &mut [u64],
) -> (u64, u64) {
    let prp1 = buffer_phys;
    if byte_len <= 4096 {
        (prp1, 0)
    } else if byte_len <= 8192 {
        (prp1, buffer_phys + 4096)
    } else {
        // Populate PRP List for remaining pages
        let remaining_bytes = byte_len - 4096;
        let num_pages = (remaining_bytes + 4095) / 4096;
        let fill_count = num_pages.min(prp_list_slice.len());

        for i in 0..fill_count {
            prp_list_slice[i] = buffer_phys + ((i as u64 + 1) * 4096);
        }

        (prp1, prp_list_phys)
    }
}

/// High-level NVMe Block Device descriptor exposing VFS /dev/nvme0n1.
#[derive(Debug, Clone)]
pub struct NvmeBlockDevice {
    pub nsid: u32,
    pub sector_size: usize,
    pub total_sectors: u64,
    pub model: [u8; 40],
    pub serial: [u8; 20],
}

impl NvmeBlockDevice {
    pub const fn new(
        nsid: u32,
        sector_size: usize,
        total_sectors: u64,
        model: [u8; 40],
        serial: [u8; 20],
    ) -> Self {
        Self {
            nsid,
            sector_size,
            total_sectors,
            model,
            serial,
        }
    }

    pub fn device_name(&self) -> &'static str {
        "/dev/nvme0n1"
    }

    pub fn capacity_bytes(&self) -> u64 {
        self.total_sectors * (self.sector_size as u64)
    }

    pub fn build_read_command(&self, slba: u64, count: u16, prp1: u64, prp2: u64) -> NvmeSubmissionEntry {
        build_nvm_read(self.nsid, slba, count, prp1, prp2)
    }

    pub fn build_write_command(&self, slba: u64, count: u16, prp1: u64, prp2: u64) -> NvmeSubmissionEntry {
        build_nvm_write(self.nsid, slba, count, prp1, prp2)
    }
}

/// Unit test verifying NVMe IDENTIFY structures, PRP scatter-gather generation, and command builders.
pub fn verify_nvme_identify_and_prp() -> bool {
    // 1. Verify exact 4096-byte sizing of IDENTIFY structures
    assert_eq!(core::mem::size_of::<NvmeIdentifyController>(), 4096);
    assert_eq!(core::mem::size_of::<NvmeIdentifyNamespace>(), 4096);
    assert_eq!(core::mem::size_of::<NvmePowerStateDesc>(), 32);
    assert_eq!(core::mem::size_of::<NvmeLbaFormat>(), 4);

    // 2. Test PRP builder
    // Single-page transfer (512 bytes): PRP1 = addr, PRP2 = 0
    let (p1, p2) = build_prp(0x1000, 512, 0, &mut []);
    assert_eq!(p1, 0x1000);
    assert_eq!(p2, 0);

    // Two-page transfer (8192 bytes): PRP1 = 0x1000, PRP2 = 0x2000
    let (p1_2, p2_2) = build_prp(0x1000, 8192, 0, &mut []);
    assert_eq!(p1_2, 0x1000);
    assert_eq!(p2_2, 0x2000);

    // Multi-page transfer (16384 bytes = 4 pages): PRP1 = 0x1000, PRP2 = list_phys
    let mut prp_list = [0u64; 8];
    let (p1_m, p2_m) = build_prp(0x1000, 16384, 0x5000, &mut prp_list);
    assert_eq!(p1_m, 0x1000);
    assert_eq!(p2_m, 0x5000);
    assert_eq!(prp_list[0], 0x2000);
    assert_eq!(prp_list[1], 0x3000);
    assert_eq!(prp_list[2], 0x4000);

    // 3. Test Command Builders
    let id_ctrl = build_identify_controller(0x8000);
    assert_eq!(id_ctrl.opcode(), NVME_ADMIN_IDENTIFY);
    assert_eq!(id_ctrl.prp1, 0x8000);
    assert_eq!(id_ctrl.cdw10, 0x01); // CNS = 1

    let id_ns = build_identify_namespace(1, 0x9000);
    assert_eq!(id_ns.opcode(), NVME_ADMIN_IDENTIFY);
    assert_eq!(id_ns.nsid, 1);
    assert_eq!(id_ns.prp1, 0x9000);
    assert_eq!(id_ns.cdw10, 0x00); // CNS = 0

    let read_cmd = build_nvm_read(1, 0x0001_0000_1234, 8, 0xA000, 0);
    assert_eq!(read_cmd.opcode(), NVME_NVM_READ);
    assert_eq!(read_cmd.nsid, 1);
    assert_eq!(read_cmd.cdw10, 0x0000_1234);
    assert_eq!(read_cmd.cdw11, 0x0001);
    assert_eq!(read_cmd.cdw12, 7); // 8 blocks - 1 = 7

    let write_cmd = build_nvm_write(1, 0x0002_0000_5678, 1, 0xB000, 0);
    assert_eq!(write_cmd.opcode(), NVME_NVM_WRITE);
    assert_eq!(write_cmd.nsid, 1);
    assert_eq!(write_cmd.cdw10, 0x0000_5678);
    assert_eq!(write_cmd.cdw11, 0x0002);
    assert_eq!(write_cmd.cdw12, 0); // 1 block - 1 = 0

    // 4. Test Namespace sector size resolution
    let mut ns_mock: NvmeIdentifyNamespace = unsafe { core::mem::zeroed() };
    ns_mock.nsze = 2097152; // 1 GiB in 512-byte sectors
    ns_mock.ncap = 2097152;
    ns_mock.flbas = 0; // format 0
    ns_mock.lbaf[0].lbads = 9; // 2^9 = 512 bytes
    assert_eq!(ns_mock.sector_size(), 512);

    let dev = NvmeBlockDevice::new(1, ns_mock.sector_size(), ns_mock.nsze, [b' '; 40], [b' '; 20]);
    assert_eq!(dev.device_name(), "/dev/nvme0n1");
    assert_eq!(dev.capacity_bytes(), 2097152 * 512);

    true
}
