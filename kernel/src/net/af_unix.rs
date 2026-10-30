//! Unix Domain Sockets (AF_UNIX / AF_LOCAL) & Local IPC Engine.
//!
//! Implements POSIX.1-2008 / Linux compatible Unix domain socket abstractions:
//! - POSIX `sockaddr_un` structure (sun_family = AF_UNIX / 1, sun_path with null-termination)
//! - Filesystem pathname addressing, abstract socket namespace (\0name), and unnamed sockets
//! - Circular FIFO stream buffer (`UnixStreamRingBuffer`) for zero-loss bidirectional streams
//! - Packetized message-boundary preserving datagram queue (`UnixDatagramQueue`)
//! - Peer credentials (`ucred` struct: pid, uid, gid) for SO_PEERCRED IPC authentication

use alloc::collections::VecDeque;
use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;
use core::fmt;

/// Address family for Unix domain sockets (AF_UNIX / AF_LOCAL = 1).
pub const AF_UNIX: u16 = 1;

/// Maximum size of `sun_path` in standard POSIX `sockaddr_un`.
pub const UNIX_PATH_MAX: usize = 108;

/// Default stream ring buffer capacity in bytes (8 KiB).
pub const DEFAULT_STREAM_BUFFER_CAPACITY: usize = 8192;

/// Default datagram packet queue limit.
pub const DEFAULT_DATAGRAM_MAX_PACKETS: usize = 64;

/// Default datagram total payload byte limit (64 KiB).
pub const DEFAULT_DATAGRAM_MAX_BYTES: usize = 65536;

/// Error codes returned by Unix domain socket operations.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnixSocketError {
    AddressInUse,
    AddressNotAvailable,
    ConnectionRefused,
    ConnectionReset,
    NotConnected,
    AlreadyConnected,
    BrokenPipe,
    WouldBlock,
    InvalidArgument,
    BufferFull,
    MessageTooLong,
    NotFound,
    NotASocket,
    PermissionDenied,
}

/// Linux/POSIX peer credentials structure (SO_PEERCRED).
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ucred {
    pub pid: u32,
    pub uid: u32,
    pub gid: u32,
}

impl ucred {
    pub const fn new(pid: u32, uid: u32, gid: u32) -> Self {
        Self { pid, uid, gid }
    }

    pub const fn kernel_root() -> Self {
        Self {
            pid: 0,
            uid: 0,
            gid: 0,
        }
    }
}

/// Unix domain socket address abstraction.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub enum UnixSocketAddress {
    /// Filesystem pathname socket (e.g. `/tmp/knton.sock`).
    Pathname(String),
    /// Abstract namespace socket starting with a null byte (e.g. `\0knton_abstract`).
    Abstract(Vec<u8>),
    /// Unnamed socket (e.g. created by `socketpair()`).
    Unnamed,
}

impl UnixSocketAddress {
    pub fn is_abstract(&self) -> bool {
        matches!(self, Self::Abstract(_))
    }

    pub fn is_pathname(&self) -> bool {
        matches!(self, Self::Pathname(_))
    }

    pub fn is_unnamed(&self) -> bool {
        matches!(self, Self::Unnamed)
    }

    pub fn as_pathname(&self) -> Option<&str> {
        match self {
            Self::Pathname(ref p) => Some(p.as_str()),
            _ => None,
        }
    }
}

/// Standard POSIX sockaddr_un memory structure.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct sockaddr_un {
    pub sun_family: u16,
    pub sun_path: [u8; UNIX_PATH_MAX],
}

impl Default for sockaddr_un {
    fn default() -> Self {
        Self {
            sun_family: AF_UNIX,
            sun_path: [0u8; UNIX_PATH_MAX],
        }
    }
}

impl fmt::Debug for sockaddr_un {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("sockaddr_un")
            .field("sun_family", &self.sun_family)
            .field("address", &self.to_address())
            .finish()
    }
}

impl PartialEq for sockaddr_un {
    fn eq(&self, other: &Self) -> bool {
        self.sun_family == other.sun_family && self.sun_path[..] == other.sun_path[..]
    }
}

impl Eq for sockaddr_un {}

impl sockaddr_un {
    /// Creates an empty/unnamed sockaddr_un.
    pub const fn new() -> Self {
        Self {
            sun_family: AF_UNIX,
            sun_path: [0u8; UNIX_PATH_MAX],
        }
    }

    /// Creates a sockaddr_un from a filesystem path.
    pub fn from_path(path: &str) -> Result<Self, UnixSocketError> {
        let bytes = path.as_bytes();
        if bytes.is_empty() || bytes.len() >= UNIX_PATH_MAX {
            return Err(UnixSocketError::InvalidArgument);
        }

        let mut sun = Self::new();
        sun.sun_path[..bytes.len()].copy_from_slice(bytes);
        sun.sun_path[bytes.len()] = 0; // Null-terminate
        Ok(sun)
    }

    /// Creates an abstract namespace sockaddr_un with leading null byte.
    pub fn from_abstract(name: &[u8]) -> Result<Self, UnixSocketError> {
        if name.is_empty() || name.len() >= UNIX_PATH_MAX {
            return Err(UnixSocketError::InvalidArgument);
        }

        let mut sun = Self::new();
        sun.sun_path[0] = 0; // Abstract namespace marker
        sun.sun_path[1..1 + name.len()].copy_from_slice(name);
        Ok(sun)
    }

    /// Parses the sockaddr_un into a `UnixSocketAddress`.
    pub fn to_address(&self) -> UnixSocketAddress {
        if self.sun_path[0] == 0 {
            // Check if there is data after the leading null byte (abstract socket)
            let mut end = 1;
            while end < UNIX_PATH_MAX && self.sun_path[end] != 0 {
                end += 1;
            }
            if end > 1 {
                UnixSocketAddress::Abstract(self.sun_path[1..end].to_vec())
            } else {
                UnixSocketAddress::Unnamed
            }
        } else {
            // Null-terminated pathname
            let mut end = 0;
            while end < UNIX_PATH_MAX && self.sun_path[end] != 0 {
                end += 1;
            }
            if let Ok(s) = core::str::from_utf8(&self.sun_path[..end]) {
                UnixSocketAddress::Pathname(String::from(s))
            } else {
                UnixSocketAddress::Unnamed
            }
        }
    }
}

/// In-memory circular FIFO stream buffer for byte-oriented IPC streaming.
#[derive(Debug, Clone)]
pub struct UnixStreamRingBuffer {
    buffer: Vec<u8>,
    head: usize,
    tail: usize,
    count: usize,
    capacity: usize,
}

impl UnixStreamRingBuffer {
    /// Creates a ring buffer with the specified capacity.
    pub fn new(capacity: usize) -> Self {
        let cap = capacity.max(16);
        Self {
            buffer: vec![0u8; cap],
            head: 0,
            tail: 0,
            count: 0,
            capacity: cap,
        }
    }

    /// Creates a ring buffer with default capacity (8 KiB).
    pub fn with_default_capacity() -> Self {
        Self::new(DEFAULT_STREAM_BUFFER_CAPACITY)
    }

    /// Total capacity in bytes.
    pub const fn capacity(&self) -> usize {
        self.capacity
    }

    /// Currently buffered byte count.
    pub const fn len(&self) -> usize {
        self.count
    }

    /// Checks if the buffer contains no bytes.
    pub const fn is_empty(&self) -> bool {
        self.count == 0
    }

    /// Checks if the buffer is completely full.
    pub const fn is_full(&self) -> bool {
        self.count == self.capacity
    }

    /// Returns the number of readable bytes available.
    pub const fn available_read(&self) -> usize {
        self.count
    }

    /// Returns the remaining write capacity in bytes.
    pub const fn available_write(&self) -> usize {
        self.capacity - self.count
    }

    /// Writes data into the ring buffer, returning the number of bytes written.
    pub fn write(&mut self, data: &[u8]) -> usize {
        let to_write = core::cmp::min(data.len(), self.available_write());
        if to_write == 0 {
            return 0;
        }

        let first_chunk = core::cmp::min(to_write, self.capacity - self.tail);
        self.buffer[self.tail..self.tail + first_chunk].copy_from_slice(&data[..first_chunk]);

        let second_chunk = to_write - first_chunk;
        if second_chunk > 0 {
            self.buffer[..second_chunk].copy_from_slice(&data[first_chunk..to_write]);
        }

        self.tail = (self.tail + to_write) % self.capacity;
        self.count += to_write;
        to_write
    }

    /// Reads data from the ring buffer, returning the number of bytes read.
    pub fn read(&mut self, buf: &mut [u8]) -> usize {
        let to_read = core::cmp::min(buf.len(), self.available_read());
        if to_read == 0 {
            return 0;
        }

        let first_chunk = core::cmp::min(to_read, self.capacity - self.head);
        buf[..first_chunk].copy_from_slice(&self.buffer[self.head..self.head + first_chunk]);

        let second_chunk = to_read - first_chunk;
        if second_chunk > 0 {
            buf[first_chunk..to_read].copy_from_slice(&self.buffer[..second_chunk]);
        }

        self.head = (self.head + to_read) % self.capacity;
        self.count -= to_read;
        to_read
    }

    /// Peeks data from the ring buffer without advancing the read pointer.
    pub fn peek(&self, buf: &mut [u8]) -> usize {
        let to_read = core::cmp::min(buf.len(), self.available_read());
        if to_read == 0 {
            return 0;
        }

        let first_chunk = core::cmp::min(to_read, self.capacity - self.head);
        buf[..first_chunk].copy_from_slice(&self.buffer[self.head..self.head + first_chunk]);

        let second_chunk = to_read - first_chunk;
        if second_chunk > 0 {
            buf[first_chunk..to_read].copy_from_slice(&self.buffer[..second_chunk]);
        }

        to_read
    }

    /// Clears the ring buffer.
    pub fn clear(&mut self) {
        self.head = 0;
        self.tail = 0;
        self.count = 0;
    }
}

/// Discrete packet holding datagram payload and sender metadata.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnixDatagramPacket {
    pub sender_addr: Option<UnixSocketAddress>,
    pub sender_cred: Option<ucred>,
    pub data: Vec<u8>,
}

impl UnixDatagramPacket {
    pub fn new(sender_addr: Option<UnixSocketAddress>, sender_cred: Option<ucred>, data: Vec<u8>) -> Self {
        Self {
            sender_addr,
            sender_cred,
            data,
        }
    }
}

/// Message-boundary preserving queue for AF_UNIX datagrams.
#[derive(Debug, Clone)]
pub struct UnixDatagramQueue {
    packets: VecDeque<UnixDatagramPacket>,
    max_packets: usize,
    max_bytes: usize,
    current_bytes: usize,
}

impl UnixDatagramQueue {
    /// Creates a datagram queue with limits on packet count and total payload bytes.
    pub fn new(max_packets: usize, max_bytes: usize) -> Self {
        Self {
            packets: VecDeque::new(),
            max_packets: max_packets.max(1),
            max_bytes: max_bytes.max(512),
            current_bytes: 0,
        }
    }

    /// Creates a datagram queue with default limits.
    pub fn with_default_limits() -> Self {
        Self::new(DEFAULT_DATAGRAM_MAX_PACKETS, DEFAULT_DATAGRAM_MAX_BYTES)
    }

    /// Pushes a discrete datagram packet into the queue.
    pub fn push(&mut self, packet: UnixDatagramPacket) -> Result<(), UnixSocketError> {
        if self.packets.len() >= self.max_packets {
            return Err(UnixSocketError::BufferFull);
        }
        if self.current_bytes + packet.data.len() > self.max_bytes {
            return Err(UnixSocketError::BufferFull);
        }

        self.current_bytes += packet.data.len();
        self.packets.push_back(packet);
        Ok(())
    }

    /// Pops the next discrete datagram packet from the queue.
    pub fn pop(&mut self) -> Option<UnixDatagramPacket> {
        if let Some(packet) = self.packets.pop_front() {
            self.current_bytes = self.current_bytes.saturating_sub(packet.data.len());
            Some(packet)
        } else {
            None
        }
    }

    /// Peeks at the next discrete datagram packet without removing it.
    pub fn peek(&self) -> Option<&UnixDatagramPacket> {
        self.packets.front()
    }

    /// Number of packets currently queued.
    pub fn len(&self) -> usize {
        self.packets.len()
    }

    /// Total bytes of payload across all queued packets.
    pub const fn byte_len(&self) -> usize {
        self.current_bytes
    }

    /// Returns true if the queue has no packets.
    pub fn is_empty(&self) -> bool {
        self.packets.is_empty()
    }

    /// Clears all packets from the queue.
    pub fn clear(&mut self) {
        self.packets.clear();
        self.current_bytes = 0;
    }
}
