//! Process Control Block (PCB), Address Space references, and Handle/FD tables.

use alloc::string::String;
use alloc::vec::Vec;
use core::sync::atomic::{AtomicU64, Ordering};

static NEXT_PID: AtomicU64 = AtomicU64::new(1);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProcessState {
    Active,
    Sleeping,
    Zombie,
    Terminated,
}

#[derive(Debug, Clone)]
pub struct FileDescriptor {
    pub fd: u32,
    pub path: String,
    pub flags: u32,
}

pub struct ProcessControlBlock {
    pub pid: u64,
    pub name: String,
    pub state: ProcessState,
    pub page_table_cr3: u64,
    pub file_descriptors: Vec<Option<FileDescriptor>>,
    pub thread_ids: Vec<u64>,
    pub parent_pid: Option<u64>,
}

impl ProcessControlBlock {
    /// Creates a new process with an isolated or shared address space.
    pub fn new(name: &str, cr3: u64, parent_pid: Option<u64>) -> Self {
        let pid = NEXT_PID.fetch_add(1, Ordering::Relaxed);
        let mut fd_table = Vec::with_capacity(16);

        // Standard descriptors: 0 = stdin, 1 = stdout, 2 = stderr
        fd_table.push(Some(FileDescriptor {
            fd: 0,
            path: String::from("/dev/stdin"),
            flags: 0,
        }));
        fd_table.push(Some(FileDescriptor {
            fd: 1,
            path: String::from("/dev/stdout"),
            flags: 1,
        }));
        fd_table.push(Some(FileDescriptor {
            fd: 2,
            path: String::from("/dev/stderr"),
            flags: 1,
        }));

        Self {
            pid,
            name: String::from(name),
            state: ProcessState::Active,
            page_table_cr3: cr3,
            file_descriptors: fd_table,
            thread_ids: Vec::new(),
            parent_pid,
        }
    }

    /// Allocates the next available file descriptor entry in the process handle table.
    pub fn allocate_fd(&mut self, path: &str, flags: u32) -> u32 {
        for (idx, slot) in self.file_descriptors.iter_mut().enumerate() {
            if slot.is_none() {
                *slot = Some(FileDescriptor {
                    fd: idx as u32,
                    path: String::from(path),
                    flags,
                });
                return idx as u32;
            }
        }

        let fd = self.file_descriptors.len() as u32;
        self.file_descriptors.push(Some(FileDescriptor {
            fd,
            path: String::from(path),
            flags,
        }));
        fd
    }

    /// Releases a file descriptor from the process handle table.
    pub fn close_fd(&mut self, fd: u32) -> bool {
        if let Some(slot) = self.file_descriptors.get_mut(fd as usize) {
            if slot.is_some() {
                *slot = None;
                return true;
            }
        }
        false
    }

    /// Associates a new thread ID with this process.
    pub fn add_thread(&mut self, tid: u64) {
        if !self.thread_ids.contains(&tid) {
            self.thread_ids.push(tid);
        }
    }

    /// Disassociates a thread ID upon termination.
    pub fn remove_thread(&mut self, tid: u64) {
        self.thread_ids.retain(|&id| id != tid);
    }
}
