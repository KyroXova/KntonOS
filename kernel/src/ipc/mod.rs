//! Inter-Process Communication (IPC) Subsystem for KntonOS.
//!
//! Provides zero-copy SPSC shared memory rings, fast user-space futex synchronization,
//! and NT section mapping primitives.

pub mod ring;
pub mod futex;
pub mod section;
pub mod umwait;

pub use futex::{
    futex_wait, futex_wake, futex_wake_tid, test_futex_engine, FutexBucket, FutexError, FutexKey,
    FutexTable, FutexWaiter, FUTEX_BUCKETS, FUTEX_TABLE,
};
pub use ring::{
    test_spsc_ring, PacketHeader, RingBufferError, SharedMemoryRing, SharedRingHeader,
    SpscRingBuffer, CACHELINE_SIZE, PACKET_HEADER_SIZE, PACKET_MAGIC,
    test_dual_ring_channel, DualRingChannel, DualRingChannelHeader, DualRingEndpoint,
    DualRingPointers, DUAL_RING_MAGIC, DUAL_RING_VERSION,
    test_descriptor_ring_batch, DescriptorRing, PacketDescriptor,
    DESC_FLAG_EOP, DESC_FLAG_ZERO_COPY_REF, DESC_OWNERSHIP_CONSUMER, DESC_OWNERSHIP_PRODUCER,
};
pub use section::{SectionManager, SharedSection, SECTION_MANAGER};
pub use umwait::{
    cpu_tpause, cpu_umwait, cpu_umonitor, detect_waitpkg_support, read_tsc,
    set_waitpkg_override, test_umwait_primitives, AdaptiveWaitBarrier, UmwaitCState, WaitResult,
};

use core::sync::atomic::{AtomicBool, AtomicU32, Ordering};

static FUTEX_SYNC_WORD: AtomicU32 = AtomicU32::new(1); // 1 = locked
static FUTEX_WORKER_FINISHED: AtomicBool = AtomicBool::new(false);

fn ipc_futex_waiter_thread() {
    let word_ptr = FUTEX_SYNC_WORD.as_ptr();
    // Block until FUTEX_SYNC_WORD is unlocked and woken
    let _ = futex_wait(word_ptr, 1, None);
    FUTEX_WORKER_FINISHED.store(true, Ordering::SeqCst);
}

/// Initializes IPC shared memory ring and futex subsystems.
pub fn init() {
    crate::serial_write_str("[KNT-IPC] Initializing IPC shared memory rings & futex subsystem...\r\n");
}

/// Verification test suite exercising SPSC ring transfer, zero-copy section mapping,
/// and cross-thread futex sleep/wake contention.
pub fn test_ipc_subsystem() -> bool {
    crate::serial_write_str("[KNT-IPC-TEST] Running IPC rings, futex synchronization, and section tests...\r\n");

    // 1. SPSC lockless ring buffer verification
    if !test_spsc_ring() {
        crate::serial_write_str("[KNT-PANIC] SPSC ring unit test failed!\r\n");
        return false;
    }

    // 2. Futex engine verification
    if !test_futex_engine() {
        crate::serial_write_str("[KNT-PANIC] Futex engine unit test failed!\r\n");
        return false;
    }

    // 3. Shared memory section zero-copy aliasing test
    {
        let handle = {
            let mut sm = SECTION_MANAGER.lock();
            match sm.create_section("BenchSection", 8192) {
                Ok(h) => h,
                Err(_) => {
                    crate::serial_write_str("[KNT-PANIC] Failed to create IPC section!\r\n");
                    return false;
                }
            }
        };

        let sm = SECTION_MANAGER.lock();
        let vaddr1 = match sm.map_view(handle, 0x0000_7FFF_8000_0000) {
            Ok(v) => v,
            Err(_) => {
                crate::serial_write_str("[KNT-PANIC] Failed to map view 1 of section!\r\n");
                return false;
            }
        };

        let vaddr2 = match sm.map_view(handle, 0x0000_7FFF_8000_2000) {
            Ok(v) => v,
            Err(_) => {
                crate::serial_write_str("[KNT-PANIC] Failed to map view 2 of section!\r\n");
                return false;
            }
        };

        // Write through View 1 and read back through View 2 (aliasing the same physical frame)
        unsafe {
            let p1 = vaddr1 as *mut u64;
            *p1 = 0xFEED_FACE_CAFE_BABE;
            let p2 = vaddr2 as *const u64;
            if *p2 != 0xFEED_FACE_CAFE_BABE {
                crate::serial_write_str("[KNT-PANIC] Zero-copy section mapping failed! View 2 mismatch!\r\n");
                return false;
            }
        }
        crate::serial_write_str("[KNT-IPC-SECTION-PASS] Zero-copy physical frame sharing across views verified\r\n");
    }

    // 4. Cross-thread Futex Contention & Wake test
    {
        FUTEX_SYNC_WORD.store(1, Ordering::SeqCst);
        FUTEX_WORKER_FINISHED.store(false, Ordering::SeqCst);

        let tid = crate::process::scheduler::spawn(
            "ipc_futex_waiter",
            crate::process::Priority::Normal,
            ipc_futex_waiter_thread,
        );
        if tid == 0 {
            crate::serial_write_str("[KNT-PANIC] Failed to spawn futex waiter thread!\r\n");
            return false;
        }

        // Yield control to let the waiter run and enter futex_wait
        crate::process::scheduler::yield_now();

        // Verify waiter is still waiting (not finished)
        if FUTEX_WORKER_FINISHED.load(Ordering::SeqCst) {
            crate::serial_write_str("[KNT-PANIC] Futex waiter completed prematurely without being woken!\r\n");
            return false;
        }

        // Release the lock and wake waiter thread
        FUTEX_SYNC_WORD.store(0, Ordering::SeqCst);
        let woken = futex_wake(FUTEX_SYNC_WORD.as_ptr(), 1);
        if woken != 1 {
            crate::serial_write_str("[KNT-PANIC] Futex wake did not wake exactly 1 thread!\r\n");
            return false;
        }

        // Yield control to allow waiter to resume and complete
        crate::process::scheduler::yield_now();

        if !FUTEX_WORKER_FINISHED.load(Ordering::SeqCst) {
            crate::serial_write_str("[KNT-PANIC] Futex waiter failed to resume after wake!\r\n");
            return false;
        }
        crate::serial_write_str("[KNT-IPC-THREAD-PASS] Cross-thread futex sleep and wake synchronization verified\r\n");
    }

    // 5. Dual-Ring Bidirectional Channel Verification
    if !test_dual_ring_channel() {
        crate::serial_write_str("[KNT-PANIC] Dual-ring channel verification failed!\r\n");
        return false;
    }

    // 6. Zero-Copy Descriptor Ring & Batch API Verification
    if !test_descriptor_ring_batch() {
        crate::serial_write_str("[KNT-PANIC] Descriptor ring batch verification failed!\r\n");
        return false;
    }

    // 7. UMWAIT, UMONITOR, TPAUSE, and Adaptive Synchronization Verification
    if !test_umwait_primitives() {
        crate::serial_write_str("[KNT-PANIC] UMWAIT synchronization primitives test failed!\r\n");
        return false;
    }

    // 8. End-to-end Dual-Ring Throughput & Adaptive Barrier Synchronization
    {
        static mut E2E_CHANNEL: DualRingChannel<4096> = DualRingChannel::new(99);
        let channel = unsafe { &mut *core::ptr::addr_of_mut!(E2E_CHANNEL) };
        let p0 = channel.endpoint_peer0();
        let p1 = channel.endpoint_peer1();

        let barrier = AdaptiveWaitBarrier::default_ipc();

        // Peer 0 sends high-frequency bursts
        for i in 0..16u32 {
            let mut payload = [0u8; 64];
            payload[0] = (i & 0xFF) as u8;
            payload[63] = 0xAA;
            if p0.send(0x01, &payload).is_err() {
                crate::serial_write_str("[KNT-PANIC] E2E dual-ring burst send failed!\r\n");
                return false;
            }
        }

        // Wait until Peer 1 sees packets ready
        let wait_res = barrier.wait_until(|| p1.available_rx() >= PACKET_HEADER_SIZE);
        if wait_res != WaitResult::Success {
            crate::serial_write_str("[KNT-PANIC] E2E adaptive barrier failed waiting for RX!\r\n");
            return false;
        }

        // Peer 1 drains all 16 packets
        let mut rx_buf = [0u8; 128];
        for i in 0..16u32 {
            match p1.recv(&mut rx_buf) {
                Ok((len, flags)) => {
                    if len != 64 || flags != 0x01 || rx_buf[0] != (i & 0xFF) as u8 || rx_buf[63] != 0xAA {
                        crate::serial_write_str("[KNT-PANIC] E2E packet content mismatch!\r\n");
                        return false;
                    }
                }
                Err(_) => {
                    crate::serial_write_str("[KNT-PANIC] E2E recv failed!\r\n");
                    return false;
                }
            }
        }
    }

    crate::serial_write_str("[KNT-IPC-RING-PASS] High-throughput zero-copy IPC ring and UMWAIT synchronization operational\r\n");
    crate::serial_write_str("[KNT-IPC-FUTEX-PASS] All IPC shared memory rings & futex benchmarks passed\r\n");
    true
}
