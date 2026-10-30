//! Fast User-Space Locking (Futex) Engine with Hash Buckets and Wait Queues.
//! Provides lockless fast-path and kernel-mediated sleep/wake synchronization

use alloc::vec::Vec;
use core::sync::atomic::{AtomicU32, Ordering};
use crate::memory::spinlock::Spinlock;

pub const FUTEX_BUCKETS: usize = 64;

/// Unique physical futex identifier ensuring threads in different address spaces
/// sharing the same underlying page frame synchronize on the exact same wait queue.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct FutexKey {
    pub phys_page: u64,
    pub page_offset: u32,
}

impl FutexKey {
    /// Resolves a user or kernel virtual pointer to its underlying physical frame and offset.
    pub fn from_virtual(val_addr: *const u32) -> Result<Self, FutexError> {
        let vaddr = val_addr as usize;
        if vaddr == 0 || (vaddr & 0x03) != 0 {
            return Err(FutexError::InvalidAddress);
        }

        // Query active CR3 page tables to locate the backing physical frame
        let cr3: u64;
        unsafe {
            core::arch::asm!("mov {}, cr3", out(reg) cr3, options(nomem, nostack, preserves_flags));
        }
        let root_pml4 = (cr3 & !0xFFF) as *mut crate::memory::vmm::PageTable;
        let ptm = unsafe { crate::memory::vmm::PageTableManager::new(root_pml4) };

        let phys_opt = unsafe { ptm.translate(crate::memory::vmm::VirtAddr::new(vaddr)) };

        let phys_addr = match phys_opt {
            Some(phys) => phys.0 as u64,
            None => {
                // In early identity-mapped memory or kernel identity space
                vaddr as u64
            }
        };

        Ok(Self {
            phys_page: phys_addr & !0xFFF,
            page_offset: (phys_addr & 0xFFF) as u32,
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FutexError {
    ValueMismatch,
    InvalidAddress,
    Timeout,
    QueueUnavailable,
}

/// Entry representing a thread currently waiting on a futex variable.
#[derive(Debug, Clone)]
pub struct FutexWaiter {
    pub tid: u64,
    pub key: FutexKey,
    pub val_addr: usize,
    pub awakened: bool,
}

pub struct FutexBucket {
    pub waiters: Spinlock<Vec<FutexWaiter>>,
}

impl FutexBucket {
    pub const fn new() -> Self {
        Self {
            waiters: Spinlock::new(Vec::new()),
        }
    }
}

pub struct FutexTable {
    pub buckets: [FutexBucket; FUTEX_BUCKETS],
}

impl FutexTable {
    pub const fn new() -> Self {
        const INIT_BUCKET: FutexBucket = FutexBucket::new();
        Self {
            buckets: [INIT_BUCKET; FUTEX_BUCKETS],
        }
    }

    #[inline(always)]
    pub fn bucket_index(&self, key: &FutexKey) -> usize {
        let h = key.phys_page.wrapping_mul(0x9E3779B97F4A7C15) ^ (key.page_offset as u64);
        (h as usize) % FUTEX_BUCKETS
    }
}

pub static FUTEX_TABLE: FutexTable = FutexTable::new();

/// Futex Wait: Atomically verifies `*val_addr == expected_val`. If matching,
/// places current thread onto the futex wait queue and yields execution.
pub fn futex_wait(
    val_addr: *const u32,
    expected_val: u32,
    _timeout_ticks: Option<u64>,
) -> Result<(), FutexError> {
    let key = FutexKey::from_virtual(val_addr)?;
    let b_idx = FUTEX_TABLE.bucket_index(&key);
    let tid = crate::process::scheduler::current_thread_id();

    {
        let mut bucket = FUTEX_TABLE.buckets[b_idx].waiters.lock();

        // 1. Atomically verify memory word at val_addr matches expected_val
        let current_val = unsafe {
            let atomic_ref = &*(val_addr as *const AtomicU32);
            atomic_ref.load(Ordering::SeqCst)
        };

        if current_val != expected_val {
            return Err(FutexError::ValueMismatch);
        }

        // 2. Register thread in futex wait queue while holding bucket spinlock
        bucket.push(FutexWaiter {
            tid,
            key,
            val_addr: val_addr as usize,
            awakened: false,
        });
    }

    // 3. Mark current thread as Blocked and context-switch away
    crate::process::scheduler::block_current_thread();

    Ok(())
}

/// Futex Wake: Wakes up to `count` threads waiting on the futex key at `val_addr`.
/// Returns the number of threads actually woken.
pub fn futex_wake(val_addr: *const u32, count: u32) -> u32 {
    let key = match FutexKey::from_virtual(val_addr) {
        Ok(k) => k,
        Err(_) => return 0,
    };
    let b_idx = FUTEX_TABLE.bucket_index(&key);

    let mut to_wake = Vec::new();

    {
        let mut bucket = FUTEX_TABLE.buckets[b_idx].waiters.lock();
        let mut i = 0;
        let mut woken = 0;

        while i < bucket.len() && woken < count {
            if bucket[i].key == key {
                let waiter = bucket.remove(i);
                to_wake.push(waiter.tid);
                woken += 1;
            } else {
                i += 1;
            }
        }
    }

    let count_woken = to_wake.len() as u32;
    for tid in to_wake {
        crate::process::scheduler::wake_thread(tid);
    }

    count_woken
}

/// Wakes a thread directly by its thread ID if it is currently waiting in any futex bucket.
pub fn futex_wake_tid(tid: u64) -> bool {
    for bucket in FUTEX_TABLE.buckets.iter() {
        let mut waiters = bucket.waiters.lock();
        if let Some(pos) = waiters.iter().position(|w| w.tid == tid) {
            waiters.remove(pos);
            drop(waiters);
            crate::process::scheduler::wake_thread(tid);
            return true;
        }
    }
    // Also wake directly from scheduler blocked list if not in futex queue
    crate::process::scheduler::wake_thread(tid)
}

/// Standalone verification self-test for fast futex synchronization engine.
pub fn test_futex_engine() -> bool {
    crate::serial_write_str("[KNT-IPC-FUTEX-TEST] Testing fast futex synchronization engine...\r\n");

    static TEST_WORD: AtomicU32 = AtomicU32::new(42);
    let word_ptr = TEST_WORD.as_ptr();

    // 1. Value mismatch check: if expected != current, futex_wait must return ValueMismatch
    if futex_wait(word_ptr, 999, None) != Err(FutexError::ValueMismatch) {
        crate::serial_write_str("[KNT-PANIC] Futex wait did not reject mismatched expected value!\r\n");
        return false;
    }

    // 2. Futex key computation check
    let key = match FutexKey::from_virtual(word_ptr) {
        Ok(k) => k,
        Err(_) => {
            crate::serial_write_str("[KNT-PANIC] Failed to generate FutexKey from virtual address!\r\n");
            return false;
        }
    };

    if (key.page_offset as usize) != (word_ptr as usize & 0xFFF) {
        crate::serial_write_str("[KNT-PANIC] FutexKey page offset calculation mismatch!\r\n");
        return false;
    }

    // 3. Waking on an empty wait queue returns 0 without crashing
    let woken = futex_wake(word_ptr, 1);
    if woken != 0 {
        crate::serial_write_str("[KNT-PANIC] Futex wake returned non-zero for empty queue!\r\n");
        return false;
    }

    crate::serial_write_str("[KNT-IPC-FUTEX-PASS] Fast futex key resolution and atomic validation verified\r\n");
    true
}
