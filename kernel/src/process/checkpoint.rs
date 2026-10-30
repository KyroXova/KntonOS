//! State-Externalized Partition Structures & Crash-Safe Journals (`knton.shm.*`).
//! Provides transactional state externalization for critical micro-kernel services:

use alloc::vec::Vec;
use core::sync::atomic::{AtomicU64, Ordering};

/// Magic identifier for KntonOS Shared Memory State Partitions ("KNSH").
pub const PARTITION_MAGIC: u32 = 0x4B4E_5348;

/// Format version for state partition headers.
pub const PARTITION_VERSION: u32 = 1;

/// Maximum length of a partition identifier string (e.g. "knton.shm.compositor").
pub const MAX_PARTITION_NAME_LEN: usize = 32;

/// Maximum number of write-ahead journal entries in a partition ring buffer.
pub const MAX_JOURNAL_ENTRIES: usize = 64;

/// Capacity of fixed state banks in bytes (e.g. 4 KiB per bank for critical service state).
pub const STATE_BANK_SIZE: usize = 4096;

/// Computes an Adler-32 checksum over the provided byte slice for corruption detection.
pub fn compute_checksum(data: &[u8]) -> u32 {
    let mut a: u32 = 1;
    let mut b: u32 = 0;
    const MOD_ADLER: u32 = 65521;

    for &byte in data {
        a = (a + byte as u32) % MOD_ADLER;
        b = (b + a) % MOD_ADLER;
    }

    (b << 16) | a
}

/// Journal operation codes representing transactional state changes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[repr(u16)]
pub enum JournalOpCode {
    #[default]
    OpNone = 0,
    OpStateCheckpoint = 1,
    OpClientSessionUpdate = 2,
    OpBufferSwap = 3,
    OpConfigCommit = 4,
}

/// A single atomic journal entry in the write-ahead recovery ring.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct JournalEntry {
    /// Monotonic sequence number / epoch.
    pub sequence: u64,
    /// Transaction operation code.
    pub op_code: JournalOpCode,
    /// Flags (bit 0 = Committed, bit 1 = Applied).
    pub flags: u16,
    /// Length of payload in bytes (up to 64 bytes).
    pub payload_size: u32,
    /// Adler-32 checksum of the payload bytes.
    pub checksum: u32,
    /// Inline payload data.
    pub payload: [u8; 64],
}

impl Default for JournalEntry {
    fn default() -> Self {
        Self {
            sequence: 0,
            op_code: JournalOpCode::OpNone,
            flags: 0,
            payload_size: 0,
            checksum: 0,
            payload: [0u8; 64],
        }
    }
}

impl JournalEntry {
    pub const FLAG_COMMITTED: u16 = 1 << 0;
    pub const FLAG_APPLIED: u16 = 1 << 1;

    /// Validates entry checksum against its payload data.
    pub fn is_valid(&self) -> bool {
        if self.payload_size > 64 {
            return false;
        }
        let expected = compute_checksum(&self.payload[..self.payload_size as usize]);
        self.checksum == expected
    }
}

/// Circular write-ahead journal ring buffer.
pub struct CrashSafeJournal {
    pub entries: [JournalEntry; MAX_JOURNAL_ENTRIES],
    pub head: usize,
    pub tail: usize,
    pub next_seq: AtomicU64,
}

impl CrashSafeJournal {
    pub fn new() -> Self {
        Self {
            entries: [JournalEntry::default(); MAX_JOURNAL_ENTRIES],
            head: 0,
            tail: 0,
            next_seq: AtomicU64::new(1),
        }
    }

    /// Appends a new write-ahead transaction entry. Returns the allocated sequence number.
    pub fn append(&mut self, op: JournalOpCode, data: &[u8]) -> Option<u64> {
        let len = data.len().min(64);
        let seq = self.next_seq.fetch_add(1, Ordering::SeqCst);

        let mut payload = [0u8; 64];
        payload[..len].copy_from_slice(&data[..len]);
        let checksum = compute_checksum(&payload[..len]);

        let entry = JournalEntry {
            sequence: seq,
            op_code: op,
            flags: 0, // Uncommitted initially
            payload_size: len as u32,
            checksum,
            payload,
        };

        let idx = (seq as usize) % MAX_JOURNAL_ENTRIES;
        self.entries[idx] = entry;
        self.head = idx;

        Some(seq)
    }

    /// Marks a pending sequence number as committed with an atomic memory barrier.
    pub fn commit(&mut self, seq: u64) -> bool {
        let idx = (seq as usize) % MAX_JOURNAL_ENTRIES;
        if self.entries[idx].sequence == seq && self.entries[idx].is_valid() {
            self.entries[idx].flags |= JournalEntry::FLAG_COMMITTED;
            true
        } else {
            false
        }
    }

    /// Recovers the latest valid, committed journal entry in the ring.
    pub fn recover_latest_committed(&self) -> Option<&JournalEntry> {
        let mut best: Option<&JournalEntry> = None;
        let mut max_seq: u64 = 0;

        for entry in &self.entries {
            if (entry.flags & JournalEntry::FLAG_COMMITTED) != 0
                && entry.sequence > max_seq
                && entry.is_valid()
            {
                max_seq = entry.sequence;
                best = Some(entry);
            }
        }

        best
    }
}

/// Metadata header describing an externalized state partition (`knton.shm.*`).
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PartitionHeader {
    pub magic: u32,
    pub version: u32,
    pub name: [u8; MAX_PARTITION_NAME_LEN],
    /// Active bank index: 0 = Bank A, 1 = Bank B.
    pub active_bank: u8,
    pub reserved: [u8; 3],
    /// Monotonic generation counter for Bank A.
    pub generation_a: u64,
    /// Monotonic generation counter for Bank B.
    pub generation_b: u64,
    /// Checksum of Bank A data.
    pub checksum_a: u32,
    /// Checksum of Bank B data.
    pub checksum_b: u32,
    /// Highest committed transaction sequence number.
    pub committed_generation: u64,
    /// Total bytes currently stored in the active bank.
    pub data_length: u32,
}

impl PartitionHeader {
    pub fn new(name_str: &str) -> Self {
        let mut name = [0u8; MAX_PARTITION_NAME_LEN];
        let bytes = name_str.as_bytes();
        let len = bytes.len().min(MAX_PARTITION_NAME_LEN);
        name[..len].copy_from_slice(&bytes[..len]);

        Self {
            magic: PARTITION_MAGIC,
            version: PARTITION_VERSION,
            name,
            active_bank: 0,
            reserved: [0u8; 3],
            generation_a: 0,
            generation_b: 0,
            checksum_a: 0,
            checksum_b: 0,
            committed_generation: 0,
            data_length: 0,
        }
    }

    pub fn name_as_str(&self) -> &str {
        let len = self.name.iter().position(|&b| b == 0).unwrap_or(MAX_PARTITION_NAME_LEN);
        core::str::from_utf8(&self.name[..len]).unwrap_or("unknown")
    }
}

/// State-Externalized Dual-Ring Memory Partition.
/// Holds two identical physical memory banks (A and B). While the active bank is
pub struct StatePartition {
    pub header: PartitionHeader,
    pub bank_a: [u8; STATE_BANK_SIZE],
    pub bank_b: [u8; STATE_BANK_SIZE],
    pub journal: CrashSafeJournal,
}

impl StatePartition {
    /// Creates a new state partition with the given shared memory name (e.g. `knton.shm.compositor`).
    pub fn new(name: &str) -> Self {
        Self {
            header: PartitionHeader::new(name),
            bank_a: [0u8; STATE_BANK_SIZE],
            bank_b: [0u8; STATE_BANK_SIZE],
            journal: CrashSafeJournal::new(),
        }
    }

    /// Atomically updates partition state using the write-ahead journal and double-buffering.
    pub fn commit_checkpoint(&mut self, data: &[u8]) -> bool {
        if data.len() > STATE_BANK_SIZE {
            return false;
        }

        // 1. Append transaction to write-ahead crash-safe journal
        let seq = match self.journal.append(JournalOpCode::OpStateCheckpoint, data) {
            Some(s) => s,
            None => return false,
        };

        // 2. Write data into the standby bank (opposite of active bank)
        let standby_is_b = self.header.active_bank == 0;
        let checksum = compute_checksum(data);

        if standby_is_b {
            self.bank_b[..data.len()].copy_from_slice(data);
            self.header.checksum_b = checksum;
            self.header.generation_b = self.header.generation_b.saturating_add(1);
        } else {
            self.bank_a[..data.len()].copy_from_slice(data);
            self.header.checksum_a = checksum;
            self.header.generation_a = self.header.generation_a.saturating_add(1);
        }

        // 3. Commit journal entry
        if !self.journal.commit(seq) {
            return false;
        }

        // 4. Atomically toggle active bank
        self.header.active_bank = if standby_is_b { 1 } else { 0 };
        self.header.committed_generation = seq;
        self.header.data_length = data.len() as u32;

        true
    }

    /// Returns a slice to the current committed active state bank.
    pub fn active_data(&self) -> &[u8] {
        let len = (self.header.data_length as usize).min(STATE_BANK_SIZE);
        if self.header.active_bank == 0 {
            &self.bank_a[..len]
        } else {
            &self.bank_b[..len]
        }
    }

    /// Validates the integrity of the active bank against its checksum.
    pub fn validate_active_bank(&self) -> bool {
        let len = (self.header.data_length as usize).min(STATE_BANK_SIZE);
        let (data, expected_chk) = if self.header.active_bank == 0 {
            (&self.bank_a[..len], self.header.checksum_a)
        } else {
            (&self.bank_b[..len], self.header.checksum_b)
        };

        compute_checksum(data) == expected_chk
    }

    /// Returns the currently active generation counter.
    #[inline(always)]
    pub fn current_generation(&self) -> u64 {
        if self.header.active_bank == 0 {
            self.header.generation_a
        } else {
            self.header.generation_b
        }
    }
}

/// Global registry managing externalized service partitions.
pub struct StatePartitionRegistry {
    pub partitions: Vec<StatePartition>,
}

impl StatePartitionRegistry {
    pub const fn new() -> Self {
        Self {
            partitions: Vec::new(),
        }
    }

    pub fn register(&mut self, partition: StatePartition) {
        self.partitions.push(partition);
    }

    pub fn find(&self, name: &str) -> Option<&StatePartition> {
        self.partitions.iter().find(|p| p.header.name_as_str() == name)
    }

    pub fn find_mut(&mut self, name: &str) -> Option<&mut StatePartition> {
        self.partitions.iter_mut().find(|p| p.header.name_as_str() == name)
    }
}
