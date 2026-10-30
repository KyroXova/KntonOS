//! Anonymous pipe IPC channel and FIFO byte ring for shell pipelines.

use alloc::collections::VecDeque;
use alloc::vec::Vec;

pub const DEFAULT_PIPE_CAPACITY: usize = 4096;

/// Unidirectional byte-stream anonymous pipe connecting producer and consumer pipeline stages.
#[derive(Debug, Clone)]
pub struct AnonymousPipe {
    buffer: VecDeque<u8>,
    capacity: usize,
    eof: bool,
    writer_closed: bool,
    reader_closed: bool,
}

impl AnonymousPipe {
    pub fn new() -> Self {
        Self::with_capacity(DEFAULT_PIPE_CAPACITY)
    }

    pub fn with_capacity(capacity: usize) -> Self {
        Self {
            buffer: VecDeque::with_capacity(capacity),
            capacity,
            eof: false,
            writer_closed: false,
            reader_closed: false,
        }
    }

    /// Write bytes into the pipe buffer. Returns number of bytes successfully accepted.
    pub fn write(&mut self, data: &[u8]) -> usize {
        if self.writer_closed || self.reader_closed {
            return 0;
        }

        let mut written = 0;
        for &byte in data {
            if self.buffer.len() >= self.capacity {
                break;
            }
            self.buffer.push_back(byte);
            written += 1;
        }
        written
    }

    /// Write a string slice into the pipe buffer.
    pub fn write_str(&mut self, s: &str) -> usize {
        self.write(s.as_bytes())
    }

    /// Read available bytes from the pipe into `buf`. Returns number of bytes read.
    pub fn read(&mut self, buf: &mut [u8]) -> usize {
        if buf.is_empty() || self.reader_closed {
            return 0;
        }

        let mut read_bytes = 0;
        while read_bytes < buf.len() {
            if let Some(byte) = self.buffer.pop_front() {
                buf[read_bytes] = byte;
                read_bytes += 1;
            } else {
                break;
            }
        }
        read_bytes
    }

    /// Read all remaining buffered bytes into a vector until empty.
    pub fn read_all(&mut self) -> Vec<u8> {
        let mut out = Vec::with_capacity(self.buffer.len());
        while let Some(byte) = self.buffer.pop_front() {
            out.push(byte);
        }
        out
    }

    /// Signal End-Of-File (EOF) from the producing stage.
    pub fn close_write(&mut self) {
        self.writer_closed = true;
        self.eof = true;
    }

    /// Close the reading end of the pipe and discard unconsumed bytes.
    pub fn close_read(&mut self) {
        self.reader_closed = true;
        self.buffer.clear();
    }

    #[inline(always)]
    pub fn is_empty(&self) -> bool {
        self.buffer.is_empty()
    }

    #[inline(always)]
    pub fn is_eof(&self) -> bool {
        self.eof && self.buffer.is_empty()
    }

    #[inline(always)]
    pub fn available(&self) -> usize {
        self.buffer.len()
    }

    #[inline(always)]
    pub fn capacity(&self) -> usize {
        self.capacity
    }
}
