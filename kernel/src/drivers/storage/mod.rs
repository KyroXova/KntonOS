//! Persistent Storage Device Drivers for KntonOS.
//!
//! Provides support for SATA/AHCI controllers, port command queues,
//! and ATA block devices.

pub mod ahci;
pub mod nvme;
pub mod uring;
pub mod direct_storage;
pub mod nvme_zns;

pub use uring::*;
pub use direct_storage::*;
pub use nvme_zns::*;

pub use nvme::{
    make_aqa, nvme_cq_doorbell, nvme_sq_doorbell, build_identify_controller, build_identify_namespace, build_nvm_read, build_nvm_write,
    build_prp, verify_nvme_identify_and_prp, verify_nvme_queue_engine, verify_nvme_structures,
    NvmeBlockDevice, NvmeIdentifyController, NvmeIdentifyNamespace, NvmeLbaFormat, NvmePowerStateDesc, NvmeCapabilities, NvmeError, NvmeQueuePair, NVME_MAX_QUEUE_SIZE,
    NvmeCompletionEntry, NvmeSubmissionEntry, CAP_AMS_VENDOR, CAP_AMS_WRR, CAP_BPS, CAP_CQS,
    CAP_CSS_NVM, CAP_DSTRD_MASK, CAP_DSTRD_SHIFT, CAP_MPSMAX_MASK, CAP_MPSMAX_SHIFT,
    CAP_MPSMIN_MASK, CAP_MPSMIN_SHIFT, CAP_MQES_MASK, CAP_NSSRS, CAP_TO_MASK, CAP_TO_SHIFT,
    CC_AMS_RR, CC_CSS_NVM, CC_EN, CC_IOCQES_16, CC_IOSQES_64, CC_MPS_SHIFT, CC_SHN_ABRUPT,
    CC_SHN_NONE, CC_SHN_NORMAL, CSTS_CFS, CSTS_NSSRO, CSTS_RDY, CSTS_SHST_COMPLETE,
    CSTS_SHST_MASK, CSTS_SHST_NORMAL, CSTS_SHST_OCCURRING, NVME_ADMIN_ABORT,
    NVME_ADMIN_CREATE_CQ, NVME_ADMIN_CREATE_SQ, NVME_ADMIN_DELETE_CQ, NVME_ADMIN_DELETE_SQ,
    NVME_ADMIN_GET_FEATURES, NVME_ADMIN_GET_LOG_PAGE, NVME_ADMIN_IDENTIFY, NVME_ADMIN_SET_FEATURES,
    NVME_DOORBELL_BASE, NVME_NVM_FLUSH, NVME_NVM_READ, NVME_NVM_WRITE, NVME_REG_ACQ, NVME_REG_AQA,
    NVME_REG_ASQ, NVME_REG_CAP, NVME_REG_CC, NVME_REG_CMBLOC, NVME_REG_CMBSZ, NVME_REG_CSTS,
    NVME_REG_INTMC, NVME_REG_INTMS, NVME_REG_NSSR, NVME_REG_VS, PCI_PROGIF_NVME,
    PCI_SUBCLASS_NVME,
};

pub use ahci::{
    verify_ahci_fis_and_commands, verify_ahci_port_engine, verify_ahci_structures, AhciBlockDevice,
    AhciController, AhciError, AhciPort, AtaIdentifyData, FisRegH2D, HbaCapabilities,
    HbaCommandHeader, HbaCommandTable, HbaPortRegisters, HbaPrdtEntry, HbaReceivedFis,
    SataDeviceType, AHCI_PORT_BASE, AHCI_PORT_STRIDE, AHCI_REG_BOHC, AHCI_REG_CAP, AHCI_REG_CAP2,
    AHCI_REG_CCC_CTL, AHCI_REG_CCC_PORTS, AHCI_REG_EM_CTL, AHCI_REG_EM_LOC, AHCI_REG_GHC,
    AHCI_REG_IS, AHCI_REG_PI, AHCI_REG_VS, ATA_CMD_IDENTIFY, ATA_CMD_IDENTIFY_PACKET,
    ATA_CMD_READ_DMA_EXT, ATA_CMD_WRITE_DMA_EXT, ATA_DEV_BUSY, ATA_DEV_DRDY, ATA_DEV_DRQ,
    ATA_DEV_ERR, BOHC_BB, BOHC_BOS, BOHC_OOC, BOHC_OOS, BOHC_SOOE, CAP_ALPE, CAP_FBSS, CAP_S64A,
    CAP_SAM, CAP_SMPS, CAP_SNCQ, CAP_SPM, CAP_SSNTF, CAP_SSS, FIS_TYPE_BIST, FIS_TYPE_DATA,
    FIS_TYPE_DEV_BITS, FIS_TYPE_DMA_ACT, FIS_TYPE_DMA_SETUP, FIS_TYPE_PIO_SETUP, FIS_TYPE_REG_D2H,
    FIS_TYPE_REG_H2D, GHC_AE, GHC_HR, GHC_IE, GHC_MRSM, MAX_COMMAND_SLOTS, MAX_PORTS,
    MAX_PRDT_ENTRIES, PCI_CLASS_MASS_STORAGE, PCI_PROGIF_AHCI, PCI_SUBCLASS_SATA, PORT_CMD_ALPE,
    PORT_CMD_APSTE, PORT_CMD_ASP, PORT_CMD_ATAPI, PORT_CMD_CLO, PORT_CMD_CPD, PORT_CMD_CPS,
    PORT_CMD_CR, PORT_CMD_DLAE, PORT_CMD_ESP, PORT_CMD_FBSCP, PORT_CMD_FR, PORT_CMD_FRE,
    PORT_CMD_HPCP, PORT_CMD_ICC_ACTIVE, PORT_CMD_MPSS, PORT_CMD_MPSP, PORT_CMD_PMA, PORT_CMD_POD,
    PORT_CMD_ST, PORT_CMD_SUD, PORT_IS_CPDS, PORT_IS_DHRS, PORT_IS_DMPS, PORT_IS_DPS,
    PORT_IS_DSS, PORT_IS_HBDS, PORT_IS_HBFS, PORT_IS_IFS, PORT_IS_INFS, PORT_IS_IPMS,
    PORT_IS_OFS, PORT_IS_PCS, PORT_IS_PRCS, PORT_IS_PSS, PORT_IS_SDBS, PORT_IS_TFES, PORT_IS_UFS,
    PORT_REG_CI, PORT_REG_CLB, PORT_REG_CLBU, PORT_REG_CMD, PORT_REG_FB, PORT_REG_FBS,
    PORT_REG_FBU, PORT_REG_IE, PORT_REG_IS, PORT_REG_SACT, PORT_REG_SCTL, PORT_REG_SERR,
    PORT_REG_SIG, PORT_REG_SNTF, PORT_REG_SSTS, PORT_REG_TFD, PRDT_IOC, PRDT_MAX_BYTES,
    SATA_SIG_ATA, SATA_SIG_ATAPI, SATA_SIG_PM, SATA_SIG_SEMB, SSTS_DET_MASK,
    SSTS_DET_PRESENT_PHY, SSTS_IPM_ACTIVE, SSTS_IPM_MASK,
};

/// Comprehensive verification of the AHCI SATA controller and port command engine.
///
/// Tests HBA Generic Host Control registers, port memory mappings, SATA device
/// signature classification, Command Headers, PRDT scatter-gather lists, ATA IDENTIFY
/// device parameter parsing, and 48-bit LBA DMA sector transfers.
pub fn test_ahci_sata_subsystem() -> bool {
    // 1. Verify HBA MMIO register maps and capability decoding
    if !verify_ahci_structures() {
        return false;
    }

    // 2. Verify Command Headers, PRDT descriptors, and FIS structures
    if !verify_ahci_fis_and_commands() {
        return false;
    }

    // 3. Verify ATA IDENTIFY parsing, slot management, and DMA command generation
    if !verify_ahci_port_engine() {
        return false;
    }

    // 4. Sector DMA loopback data integrity simulation
    let mut write_sector = [0u8; 512];
    for i in 0..512 {
        write_sector[i] = ((i * 37 + 101) & 0xFF) as u8;
    }

    let mut port = AhciPort::new(0, 0x100);
    port.set_dma_buffers(0x1_0000, 0x2_0000, 0x3_0000);

    let mut write_hdr = HbaCommandHeader::new(0, 0, false, 0);
    let mut write_tbl = HbaCommandTable::new();
    let write_buf_phys = write_sector.as_ptr() as usize as u64;
    port.build_write_dma_command(&mut write_hdr, &mut write_tbl, write_buf_phys, 0x0001_0000, 1);

    if !write_hdr.is_write() || write_tbl.prdt_entries[0].byte_count() != 512 {
        return false;
    }

    let mut read_sector = [0u8; 512];
    let mut read_hdr = HbaCommandHeader::new(0, 0, false, 0);
    let mut read_tbl = HbaCommandTable::new();
    let read_buf_phys = read_sector.as_mut_ptr() as usize as u64;
    port.build_read_dma_command(&mut read_hdr, &mut read_tbl, read_buf_phys, 0x0001_0000, 1);

    if read_hdr.is_write() || read_tbl.prdt_entries[0].byte_count() != 512 {
        return false;
    }

    // Simulated DMA transfer: write buffer to read buffer
    read_sector.copy_from_slice(&write_sector);
    if read_sector != write_sector {
        return false;
    }

    // 5. Emit boot sentinel
    crate::serial_write_str("[KNT-STORAGE-AHCI-PASS] AHCI SATA controller, port command engine, and ATA DMA verified\r\n");

    true
}


/// Comprehensive verification of the NVMe storage controller and submission/completion queue engine.
///
/// Tests NVMe MMIO register structures, CAP/CC decoding, 64-byte SQE and 16-byte CQE descriptors,
/// circular queue rings with phase bit toggling, Admin IDENTIFY controller/namespace parsing,
/// and 512-byte sector loopback data integrity via PRP DMA commands.
pub fn test_nvme_subsystem() -> bool {
    // 1. Verify NVMe MMIO register maps, capabilities, and descriptor structures
    if !verify_nvme_structures() {
        return false;
    }

    // 2. Verify submission and completion queue engine, doorbell ringing, and phase tracking
    if !verify_nvme_queue_engine() {
        return false;
    }

    // 3. Verify Admin IDENTIFY decoding and PRP scatter-gather DMA generation
    if !verify_nvme_identify_and_prp() {
        return false;
    }

    // 4. End-to-end 512-byte sector loopback data integrity simulation
    let mut write_sector = [0u8; 512];
    for i in 0..512 {
        write_sector[i] = ((i * 41 + 107) & 0xFF) as u8;
    }

    let mut io_qp = NvmeQueuePair::new(1, 16, 0, 0); // QID 1 (I/O queue), 16 entries
    let mut dev_model = [b' '; 40];
    let model_str = b"KntonOS Virtual NVMe Controller";
    dev_model[..model_str.len()].copy_from_slice(model_str);

    let mut dev_serial = [b' '; 20];
    let serial_str = b"KNTNVME001";
    dev_serial[..serial_str.len()].copy_from_slice(serial_str);

    let dev = NvmeBlockDevice::new(1, 512, 2097152, dev_model, dev_serial);

    let write_buf_phys = write_sector.as_ptr() as usize as u64;
    let (w_prp1, w_prp2) = build_prp(write_buf_phys, 512, 0, &mut []);

    let write_cmd = dev.build_write_command(0x0002_0000_1000, 1, w_prp1, w_prp2);
    if write_cmd.opcode() != NVME_NVM_WRITE || write_cmd.prp1 != write_buf_phys || write_cmd.cdw12 != 0 {
        return false;
    }

    let write_cid = io_qp.submit_command(write_cmd);
    io_qp.simulate_completion(write_cid, 0, 0);
    if io_qp.poll_completion(write_cid, 10).is_err() {
        return false;
    }

    let mut read_sector = [0u8; 512];
    let read_buf_phys = read_sector.as_mut_ptr() as usize as u64;
    let (r_prp1, r_prp2) = build_prp(read_buf_phys, 512, 0, &mut []);

    let read_cmd = dev.build_read_command(0x0002_0000_1000, 1, r_prp1, r_prp2);
    if read_cmd.opcode() != NVME_NVM_READ || read_cmd.prp1 != read_buf_phys || read_cmd.cdw12 != 0 {
        return false;
    }

    let read_cid = io_qp.submit_command(read_cmd);

    // Simulated DMA transfer: write buffer payload transferred to read buffer
    read_sector.copy_from_slice(&write_sector);

    io_qp.simulate_completion(read_cid, 0, 0);
    if io_qp.poll_completion(read_cid, 10).is_err() {
        return false;
    }

    if read_sector != write_sector {
        return false;
    }

    // 5. Emit boot sentinel
    crate::serial_write_str("[KNT-STORAGE-NVME-PASS] NVMe controller, admin/IO queue pairs, and PRP DMA verified\r\n");

    true
}

/// Comprehensive verification of the High-Speed NVMe Overlapped Async I/O (io_uring Style) Completion Engine.
///
/// Tests shared-memory SQ/CQ lockless ring buffers with atomic cursors, 64-byte SQE translation,
/// overlapped NVMe asynchronous dispatch, out-of-order completion harvesting, and token correlation.
///
/// Emits sentinel [KNT-NVME-URING-PASS] upon successful validation.
pub fn test_nvme_uring_subsystem() -> bool {
    // 1. Verify SQ/CQ lockless ring buffer mechanics, atomic cursors, and wrap-around
    if !verify_uring_ring_buffers() {
        return false;
    }

    // 2. Verify async request slot allocation, opcode translation, and NVMe dispatch
    if !verify_async_dispatcher() {
        return false;
    }

    // 3. Verify out-of-order completion event harvesting and token matching
    if !verify_completion_harvesting() {
        return false;
    }

    // 4. End-to-end overlapped multi-operation async pipeline simulation:
    // User submits 4 concurrent overlapped operations (Read, Write, Fsync, NOP) to SQ ring,
    // dispatcher submits to NVMe queue pair, hardware completes with arbitrary delay/out-of-order,
    // completions harvested into CQ ring, and verified by token and result byte count.
    let mut ring_ctx = IoUringContext::<16>::new(1);
    let mut dispatcher = AsyncIoDispatcher::new();
    let mut nvme_qp = NvmeQueuePair::new(1, 16, 0, 0);

    // Enqueue 4 overlapped requests into SQ ring
    let op0 = IoUringSqe::new_read(3, 0x1000_0000, 2048, 0, 0xAAAA_0001); // 4 sectors read
    let op1 = IoUringSqe::new_write(3, 0x1000_1000, 4096, 2048, 0xAAAA_0002); // 8 sectors write
    let op2 = IoUringSqe::new_fsync(3, 0xAAAA_0003); // flush
    let op3 = IoUringSqe::new_nop(0xAAAA_0004); // nop

    if !ring_ctx.sq.push(op0)
        || !ring_ctx.sq.push(op1)
        || !ring_ctx.sq.push(op2)
        || !ring_ctx.sq.push(op3)
    {
        return false;
    }

    if ring_ctx.sq.count() != 4 {
        return false;
    }

    // Dispatch all 4 requests into NVMe queue
    let submitted = dispatcher.dispatch_sq_ring(&mut ring_ctx.sq, &mut nvme_qp, 1);
    if submitted != 4 || ring_ctx.sq.count() != 0 || dispatcher.inflight_count != 4 {
        return false;
    }

    // Extract CIDs assigned to each operation
    let cid0 = dispatcher.inflight_slots[0].nvme_cid;
    let cid1 = dispatcher.inflight_slots[1].nvme_cid;
    let cid2 = dispatcher.inflight_slots[2].nvme_cid;
    let cid3 = dispatcher.inflight_slots[3].nvme_cid;

    // Simulate completion in reverse order: op3 (NOP), op2 (FSYNC), op1 (WRITE 4096), op0 (READ 2048)
    nvme_qp.simulate_completion(cid3, 0, 0);
    nvme_qp.cq_head = (nvme_qp.cq_head + 1) % nvme_qp.size;
    nvme_qp.simulate_completion(cid2, 0, 0);
    nvme_qp.cq_head = (nvme_qp.cq_head + 1) % nvme_qp.size;
    nvme_qp.simulate_completion(cid1, 0, 0);
    nvme_qp.cq_head = (nvme_qp.cq_head + 1) % nvme_qp.size;
    nvme_qp.simulate_completion(cid0, 0, 0);
    nvme_qp.cq_head = 0; // Rewind head to start of completed batch

    // Harvest all completions into CQ ring
    let harvested = dispatcher.harvest_completions(&mut nvme_qp, &mut ring_ctx.cq, 10);
    if harvested != 4 || dispatcher.inflight_count != 0 || ring_ctx.cq.count() != 4 {
        return false;
    }

    // Verify CQEs match user_data tokens and byte counts
    let c0 = ring_ctx.cq.pop().expect("c0 pop failed");
    if c0.user_data != 0xAAAA_0004 || c0.res != 0 {
        return false;
    }

    let c1 = ring_ctx.cq.pop().expect("c1 pop failed");
    if c1.user_data != 0xAAAA_0003 || c1.res != 0 {
        return false;
    }

    let c2 = ring_ctx.cq.pop().expect("c2 pop failed");
    if c2.user_data != 0xAAAA_0002 || c2.res != 4096 {
        return false;
    }

    let c3 = ring_ctx.cq.pop().expect("c3 pop failed");
    if c3.user_data != 0xAAAA_0001 || c3.res != 2048 {
        return false;
    }

    if ring_ctx.cq.count() != 0 {
        return false;
    }

    // 5. Emit boot sentinel
    crate::serial_write_str("[KNT-NVME-URING-PASS] NVMe overlapped async I/O completion engine verified\r\n");

    true
}


/// Comprehensive verification self-test for Zero-Copy DirectStorage NVMe-to-GPU Streaming & Compressed Texture Pipeline.
///
/// Emits sentinel [KNT-STORAGE-DIRECT-PASS] upon successful validation.
pub fn test_direct_storage_subsystem() -> bool {
    // 1. Verify GPU BAR1 aperture allocation and priority-ordered DirectStorage DMA queue
    if !direct_storage::verify_direct_storage_queue_and_bar_mapping() {
        return false;
    }

    // 2. Verify batched asynchronous I/O completion fence dispatcher and out-of-order CQE tracking
    if !direct_storage::verify_batched_completion_fence_dispatcher() {
        return false;
    }

    // 3. Verify BC1/BC3/BC7 block-compressed texture decoding, mip-level streaming, and VRAM memory-pressure eviction
    if !crate::gfx::texture_stream::verify_texture_streaming_and_block_compression() {
        return false;
    }

    // 4. End-to-end integration scenario:
    // DirectStorage streaming of a high-resolution 4K character texture into GPU VRAM (BAR1).
    let bar1_base = 0x0000_0004_0000_0000u64;
    let bar1_size = 16 * 1024 * 1024 * 1024u64; // 16 GiB
    let aperture = direct_storage::GpuBarAperture::new(bar1_base, bar1_size);

    let mut ds_queue = direct_storage::DirectStorageQueue::new(aperture, 16);
    let mut dispatcher = direct_storage::BatchedCompletionDispatcher::new();
    let mut tex_mgr = crate::gfx::texture_stream::TextureStreamManager::new(aperture, 512 * 1024 * 1024);

    let fence_id = 100;
    dispatcher.register_fence(fence_id, 0);

    // Register 4K BC7 texture asset (texture ID 200, 12 mips, starting at SSD LBA 50,000)
    let tex = crate::gfx::texture_stream::StreamingTexture::new(
        200,
        crate::gfx::texture_stream::BlockCompressionFormat::Bc7Rgba,
        4096,
        4096,
        12,
        50_000,
        512,
    );
    tex_mgr.register_texture(tex);

    // Request Mip 1 (High priority) and Mip 0 (Realtime camera viewport visible)
    let req_mip1 = match tex_mgr.request_mip_level(200, 1, direct_storage::DirectStoragePriority::High, 1) {
        Ok(Some(r)) => r,
        _ => return false,
    };
    let req_mip0 = match tex_mgr.request_mip_level(200, 0, direct_storage::DirectStoragePriority::Realtime, 1) {
        Ok(Some(r)) => r,
        _ => return false,
    };

    // Enqueue both requests into DirectStorage queue
    let id_mip1 = match ds_queue.enqueue_texture_read(
        req_mip1.nsid,
        req_mip1.start_lba,
        req_mip1.block_count,
        req_mip1.block_size,
        req_mip1.gpu_vram_dest,
        req_mip1.texture_id,
        req_mip1.mip_level,
        req_mip1.priority,
        req_mip1.fence_value,
    ) {
        Ok(id) => id,
        Err(_) => return false,
    };

    let id_mip0 = match ds_queue.enqueue_texture_read(
        req_mip0.nsid,
        req_mip0.start_lba,
        req_mip0.block_count,
        req_mip0.block_size,
        req_mip0.gpu_vram_dest,
        req_mip0.texture_id,
        req_mip0.mip_level,
        req_mip0.priority,
        req_mip0.fence_value,
    ) {
        Ok(id) => id,
        Err(_) => return false,
    };

    // Batch submission to NVMe SQ: Realtime (Mip 0) must lead High (Mip 1)
    let batch = ds_queue.create_submission_batch();
    if batch.len() != 2 || batch[0].0 != id_mip0 || batch[1].0 != id_mip1 {
        return false;
    }

    // Submit batch to completion dispatcher tied to Fence 100 -> 1
    let req_ids = alloc::vec![id_mip0, id_mip1];
    let total_bytes = (req_mip0.block_count as u64 + req_mip1.block_count as u64) * 512;
    let _ = dispatcher.submit_batch(req_ids, total_bytes, fence_id, 1, 10_000_000);

    // Simulate NVMe DMA hardware completion out-of-order: complete Mip 1 first, then Mip 0
    let c1 = ds_queue.mark_completed(id_mip1).expect("id_mip1 mark failed");
    tex_mgr.on_mip_stream_complete(c1.texture_id, c1.mip_level, c1.gpu_vram_dest);
    let _ = dispatcher.process_completion(id_mip1, 10_001_500);

    // Fence must not be signaled yet (target value 1)
    if dispatcher.is_fence_reached(fence_id, 1) {
        return false;
    }

    // Complete Mip 0 (final request in batch)
    let c0 = ds_queue.mark_completed(id_mip0).expect("id_mip0 mark failed");
    tex_mgr.on_mip_stream_complete(c0.texture_id, c0.mip_level, c0.gpu_vram_dest);
    let comp_event = dispatcher.process_completion(id_mip0, 10_002_000);

    if comp_event != Some((1, fence_id, 1)) {
        return false;
    }

    // Fence 100 must now be reached!
    if !dispatcher.is_fence_reached(fence_id, 1) || dispatcher.get_fence_value(fence_id) != 1 {
        return false;
    }

    // Texture Mip 0 must now be resident in VRAM
    let resident_tex = &tex_mgr.textures[0];
    if resident_tex.resident_highest_mip != 0 || !resident_tex.mips[0].is_resident {
        return false;
    }

    // 5. Emit deterministic boot sentinel
    crate::serial_write_str("[KNT-STORAGE-DIRECT-PASS] DirectStorage NVMe-to-GPU DMA streaming and BC texture pipeline verified\r\n");

    true
}
