//! Unix Domain Sockets (AF_UNIX / AF_LOCAL) & Local IPC Engine.
//!
//! Implements POSIX.1-2008 / Linux compatible Unix domain socket abstractions:
//! - POSIX `sockaddr_un` structure (sun_family = AF_UNIX / 1, sun_path with null-termination)
//! - Filesystem pathname addressing, abstract socket namespace (\0name), and unnamed sockets
//! - Circular FIFO stream buffer (`UnixStreamRingBuffer`) for zero-loss bidirectional streams
//! - Packetized message-boundary preserving datagram queue (`UnixDatagramQueue`)
//! - Peer credentials (`ucred` struct: pid, uid, gid) for SO_PEERCRED IPC authentication

use alloc::collections::BTreeMap;
use crate::fs::{FilePerms, FsError, VFS};
use crate::memory::Spinlock;
use crate::net::socket::{
    SocketDomain, SocketHandle, SocketProtocol, SocketState, SocketType, GLOBAL_SOCKET_TABLE,
};
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
    AlreadyBound,
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


/// Individual Unix domain socket state descriptor.
#[derive(Debug)]
pub struct UnixSocket {
    pub handle: SocketHandle,
    pub sock_type: SocketType,
    pub state: SocketState,
    pub address: Option<UnixSocketAddress>,
    pub cred: ucred,
    pub peer_cred: Option<ucred>,
    pub peer_handle: Option<SocketHandle>,
    pub peer_address: Option<UnixSocketAddress>,
    pub nonblocking: bool,
    pub backlog: usize,
    pub pending_connections: VecDeque<SocketHandle>,
    pub stream_rx: UnixStreamRingBuffer,
    pub dgram_rx: UnixDatagramQueue,
    pub bound_path: Option<String>,
    pub peer_closed: bool,
}

impl UnixSocket {
    pub fn new(handle: SocketHandle, sock_type: SocketType) -> Self {
        Self {
            handle,
            sock_type,
            state: SocketState::Unbound,
            address: None,
            cred: ucred::kernel_root(),
            peer_cred: None,
            peer_handle: None,
            peer_address: None,
            nonblocking: false,
            backlog: 128,
            pending_connections: VecDeque::new(),
            stream_rx: UnixStreamRingBuffer::with_default_capacity(),
            dgram_rx: UnixDatagramQueue::with_default_limits(),
            bound_path: None,
            peer_closed: false,
        }
    }
}

/// Global registry and state machine manager for Unix Domain Sockets.
pub struct UnixSocketTable {
    sockets: BTreeMap<SocketHandle, UnixSocket>,
    pathname_bindings: BTreeMap<String, SocketHandle>,
    abstract_bindings: BTreeMap<Vec<u8>, SocketHandle>,
}

impl UnixSocketTable {
    pub const fn new() -> Self {
        Self {
            sockets: BTreeMap::new(),
            pathname_bindings: BTreeMap::new(),
            abstract_bindings: BTreeMap::new(),
        }
    }

    pub fn create_socket_with_handle(&mut self, handle: SocketHandle, sock_type: SocketType) -> Result<(), UnixSocketError> {
        let sock = UnixSocket::new(handle, sock_type);
        self.sockets.insert(handle, sock);
        Ok(())
    }

    pub fn bind(&mut self, handle: SocketHandle, sun: &sockaddr_un) -> Result<(), UnixSocketError> {
        let addr = sun.to_address();
        if addr.is_unnamed() {
            return Err(UnixSocketError::InvalidArgument);
        }

        let sock = self.sockets.get_mut(&handle).ok_or(UnixSocketError::NotFound)?;
        if sock.state != SocketState::Unbound {
            return Err(UnixSocketError::AlreadyBound);
        }

        match addr {
            UnixSocketAddress::Pathname(ref path) => {
                if self.pathname_bindings.contains_key(path) {
                    return Err(UnixSocketError::AddressInUse);
                }
                match VFS.lock().bind_socket(path, handle, FilePerms::rwx()) {
                    Ok(_) => {}
                    Err(FsError::AlreadyExists) => return Err(UnixSocketError::AddressInUse),
                    Err(_) => return Err(UnixSocketError::InvalidArgument),
                }
                self.pathname_bindings.insert(path.clone(), handle);
                sock.bound_path = Some(path.clone());
            }
            UnixSocketAddress::Abstract(ref name) => {
                if self.abstract_bindings.contains_key(name) {
                    return Err(UnixSocketError::AddressInUse);
                }
                self.abstract_bindings.insert(name.clone(), handle);
            }
            UnixSocketAddress::Unnamed => return Err(UnixSocketError::InvalidArgument),
        }

        sock.address = Some(addr);
        sock.state = SocketState::Bound;
        Ok(())
    }

    pub fn listen(&mut self, handle: SocketHandle, backlog: usize) -> Result<(), UnixSocketError> {
        let sock = self.sockets.get_mut(&handle).ok_or(UnixSocketError::NotFound)?;
        if sock.sock_type != SocketType::SockStream && sock.sock_type != SocketType::SockSeqPacket {
            return Err(UnixSocketError::InvalidArgument);
        }
        if sock.state != SocketState::Bound && sock.state != SocketState::Unbound {
            return Err(UnixSocketError::AlreadyConnected);
        }

        sock.state = SocketState::Listening;
        sock.backlog = if backlog == 0 { 128 } else { backlog.min(128) };
        Ok(())
    }

    pub fn connect(&mut self, handle: SocketHandle, sun: &sockaddr_un) -> Result<(), UnixSocketError> {
        let target_addr = sun.to_address();
        if target_addr.is_unnamed() {
            return Err(UnixSocketError::InvalidArgument);
        }

        let (target_handle, is_dgram) = {
            let client = self.sockets.get(&handle).ok_or(UnixSocketError::NotFound)?;
            if client.state == SocketState::Connected {
                return Err(UnixSocketError::AlreadyConnected);
            }
            if client.state == SocketState::Listening {
                return Err(UnixSocketError::InvalidArgument);
            }

            let th = match target_addr {
                UnixSocketAddress::Pathname(ref path) => {
                    if let Some(&h) = self.pathname_bindings.get(path) {
                        h
                    } else if let Some(h) = VFS.lock().get_socket_node(path) {
                        h
                    } else {
                        return Err(UnixSocketError::NotFound);
                    }
                }
                UnixSocketAddress::Abstract(ref name) => {
                    *self.abstract_bindings.get(name).ok_or(UnixSocketError::ConnectionRefused)?
                }
                UnixSocketAddress::Unnamed => return Err(UnixSocketError::InvalidArgument),
            };

            (th, client.sock_type == SocketType::SockDgram)
        };

        if is_dgram {
            let client = self.sockets.get_mut(&handle).ok_or(UnixSocketError::NotFound)?;
            client.peer_address = Some(target_addr);
            client.peer_handle = Some(target_handle);
            client.state = SocketState::Connected;
            return Ok(());
        }

        let (server_backlog, pending_count) = {
            let server = self.sockets.get(&target_handle).ok_or(UnixSocketError::ConnectionRefused)?;
            if server.state != SocketState::Listening {
                return Err(UnixSocketError::ConnectionRefused);
            }
            (server.backlog, server.pending_connections.len())
        };

        if pending_count >= server_backlog {
            return Err(UnixSocketError::ConnectionRefused);
        }

        let server = self.sockets.get_mut(&target_handle).ok_or(UnixSocketError::ConnectionRefused)?;
        server.pending_connections.push_back(handle);

        let client = self.sockets.get_mut(&handle).ok_or(UnixSocketError::NotFound)?;
        client.state = SocketState::Connecting;
        client.peer_address = Some(target_addr);

        Ok(())
    }

    pub fn accept(&mut self, handle: SocketHandle, new_handle: SocketHandle) -> Result<(SocketHandle, Option<ucred>), UnixSocketError> {
        let (server_type, server_cred, client_handle) = {
            let server = self.sockets.get_mut(&handle).ok_or(UnixSocketError::NotFound)?;
            if server.state != SocketState::Listening {
                return Err(UnixSocketError::InvalidArgument);
            }

            let client_h = server.pending_connections.pop_front().ok_or(UnixSocketError::WouldBlock)?;
            (server.sock_type, server.cred, client_h)
        };

        let client_cred = {
            let client = self.sockets.get_mut(&client_handle).ok_or(UnixSocketError::ConnectionReset)?;
            client.state = SocketState::Connected;
            client.peer_handle = Some(new_handle);
            client.peer_cred = Some(server_cred);
            client.cred
        };

        let mut child = UnixSocket::new(new_handle, server_type);
        child.state = SocketState::Connected;
        child.peer_handle = Some(client_handle);
        child.peer_cred = Some(client_cred);

        self.sockets.insert(new_handle, child);

        Ok((new_handle, Some(client_cred)))
    }

    pub fn close(&mut self, handle: SocketHandle) -> Result<(), UnixSocketError> {
        if let Some(sock) = self.sockets.remove(&handle) {
            if let Some(path) = &sock.bound_path {
                self.pathname_bindings.remove(path);
            }
            if let Some(UnixSocketAddress::Abstract(name)) = &sock.address {
                self.abstract_bindings.remove(name);
            }
            if let Some(peer_h) = sock.peer_handle {
                if let Some(peer) = self.sockets.get_mut(&peer_h) {
                    peer.peer_closed = true;
                }
            }
            Ok(())
        } else {
            Err(UnixSocketError::NotFound)
        }
    }

    pub fn getpeercred(&self, handle: SocketHandle) -> Result<ucred, UnixSocketError> {
        let sock = self.sockets.get(&handle).ok_or(UnixSocketError::NotFound)?;
        sock.peer_cred.ok_or(UnixSocketError::NotConnected)
    }

    pub fn set_credentials(&mut self, handle: SocketHandle, cred: ucred) -> Result<(), UnixSocketError> {
        let sock = self.sockets.get_mut(&handle).ok_or(UnixSocketError::NotFound)?;
        sock.cred = cred;
        Ok(())
    }

    pub fn set_nonblocking(&mut self, handle: SocketHandle, nonblocking: bool) -> Result<(), UnixSocketError> {
        let sock = self.sockets.get_mut(&handle).ok_or(UnixSocketError::NotFound)?;
        sock.nonblocking = nonblocking;
        Ok(())
    }

    pub fn get_state(&self, handle: SocketHandle) -> Result<SocketState, UnixSocketError> {
        let sock = self.sockets.get(&handle).ok_or(UnixSocketError::NotFound)?;
        Ok(sock.state)
    }

    pub fn exists(&self, handle: SocketHandle) -> bool {
        self.sockets.contains_key(&handle)
    }

    pub fn socketpair(
        &mut self,
        sock_type: SocketType,
        h1: SocketHandle,
        h2: SocketHandle,
    ) -> Result<(), UnixSocketError> {
        let mut s1 = UnixSocket::new(h1, sock_type);
        let mut s2 = UnixSocket::new(h2, sock_type);

        s1.state = SocketState::Connected;
        s1.address = Some(UnixSocketAddress::Unnamed);
        s1.peer_handle = Some(h2);
        s1.peer_cred = Some(s2.cred);

        s2.state = SocketState::Connected;
        s2.address = Some(UnixSocketAddress::Unnamed);
        s2.peer_handle = Some(h1);
        s2.peer_cred = Some(s1.cred);

        self.sockets.insert(h1, s1);
        self.sockets.insert(h2, s2);
        Ok(())
    }

    pub fn send(&mut self, handle: SocketHandle, data: &[u8]) -> Result<usize, UnixSocketError> {
        let (peer_handle, sock_type, nonblocking) = {
            let sock = self.sockets.get(&handle).ok_or(UnixSocketError::NotFound)?;
            if sock.state != SocketState::Connected {
                return Err(UnixSocketError::NotConnected);
            }
            if sock.peer_closed {
                return Err(UnixSocketError::BrokenPipe);
            }
            let ph = sock.peer_handle.ok_or(UnixSocketError::NotConnected)?;
            (ph, sock.sock_type, sock.nonblocking)
        };

        let sender_meta = {
            let sock = self.sockets.get(&handle).ok_or(UnixSocketError::NotFound)?;
            (sock.address.clone(), sock.cred)
        };

        let peer = self.sockets.get_mut(&peer_handle).ok_or(UnixSocketError::BrokenPipe)?;
        if peer.peer_closed {
            return Err(UnixSocketError::BrokenPipe);
        }

        match sock_type {
            SocketType::SockStream => {
                let written = peer.stream_rx.write(data);
                if written == 0 && !data.is_empty() {
                    if nonblocking {
                        return Err(UnixSocketError::WouldBlock);
                    }
                    return Err(UnixSocketError::WouldBlock);
                }
                Ok(written)
            }
            SocketType::SockSeqPacket => {
                let pkt = UnixDatagramPacket::new(sender_meta.0, Some(sender_meta.1), data.to_vec());
                peer.dgram_rx.push(pkt)?;
                Ok(data.len())
            }
            SocketType::SockDgram => {
                let pkt = UnixDatagramPacket::new(sender_meta.0, Some(sender_meta.1), data.to_vec());
                peer.dgram_rx.push(pkt)?;
                Ok(data.len())
            }
            _ => Err(UnixSocketError::InvalidArgument),
        }
    }

    pub fn recv(&mut self, handle: SocketHandle, buf: &mut [u8]) -> Result<usize, UnixSocketError> {
        let sock = self.sockets.get_mut(&handle).ok_or(UnixSocketError::NotFound)?;
        if sock.state != SocketState::Connected && sock.state != SocketState::Bound {
            return Err(UnixSocketError::NotConnected);
        }

        match sock.sock_type {
            SocketType::SockStream => {
                let read_bytes = sock.stream_rx.read(buf);
                if read_bytes > 0 {
                    Ok(read_bytes)
                } else if sock.peer_closed {
                    Ok(0)
                } else if sock.nonblocking {
                    Err(UnixSocketError::WouldBlock)
                } else {
                    Err(UnixSocketError::WouldBlock)
                }
            }
            SocketType::SockSeqPacket => {
                if let Some(pkt) = sock.dgram_rx.pop() {
                    let to_read = core::cmp::min(buf.len(), pkt.data.len());
                    buf[..to_read].copy_from_slice(&pkt.data[..to_read]);
                    Ok(to_read)
                } else if sock.peer_closed {
                    Ok(0)
                } else if sock.nonblocking {
                    Err(UnixSocketError::WouldBlock)
                } else {
                    Err(UnixSocketError::WouldBlock)
                }
            }
            SocketType::SockDgram => {
                if let Some(pkt) = sock.dgram_rx.pop() {
                    let to_read = core::cmp::min(buf.len(), pkt.data.len());
                    buf[..to_read].copy_from_slice(&pkt.data[..to_read]);
                    Ok(to_read)
                } else if sock.nonblocking {
                    Err(UnixSocketError::WouldBlock)
                } else {
                    Err(UnixSocketError::WouldBlock)
                }
            }
            _ => Err(UnixSocketError::InvalidArgument),
        }
    }

    pub fn sendto(
        &mut self,
        handle: SocketHandle,
        data: &[u8],
        target_sun: Option<&sockaddr_un>,
    ) -> Result<usize, UnixSocketError> {
        if data.len() > DEFAULT_DATAGRAM_MAX_BYTES {
            return Err(UnixSocketError::MessageTooLong);
        }

        let target_handle = if let Some(sun) = target_sun {
            let addr = sun.to_address();
            match addr {
                UnixSocketAddress::Pathname(ref path) => {
                    if let Some(&h) = self.pathname_bindings.get(path) {
                        h
                    } else if let Some(h) = VFS.lock().get_socket_node(path) {
                        h
                    } else {
                        return Err(UnixSocketError::NotFound);
                    }
                }
                UnixSocketAddress::Abstract(ref name) => {
                    *self.abstract_bindings.get(name).ok_or(UnixSocketError::NotFound)?
                }
                UnixSocketAddress::Unnamed => return Err(UnixSocketError::InvalidArgument),
            }
        } else {
            let sock = self.sockets.get(&handle).ok_or(UnixSocketError::NotFound)?;
            sock.peer_handle.ok_or(UnixSocketError::NotConnected)?
        };

        let sender_meta = {
            let sock = self.sockets.get(&handle).ok_or(UnixSocketError::NotFound)?;
            (sock.address.clone(), sock.cred)
        };

        let target_sock = self.sockets.get_mut(&target_handle).ok_or(UnixSocketError::NotFound)?;
        let pkt = UnixDatagramPacket::new(sender_meta.0, Some(sender_meta.1), data.to_vec());
        target_sock.dgram_rx.push(pkt)?;
        Ok(data.len())
    }

    pub fn recvfrom(
        &mut self,
        handle: SocketHandle,
        buf: &mut [u8],
    ) -> Result<(usize, Option<UnixSocketAddress>, Option<ucred>), UnixSocketError> {
        let sock = self.sockets.get_mut(&handle).ok_or(UnixSocketError::NotFound)?;
        if let Some(pkt) = sock.dgram_rx.pop() {
            let to_read = core::cmp::min(buf.len(), pkt.data.len());
            buf[..to_read].copy_from_slice(&pkt.data[..to_read]);
            Ok((to_read, pkt.sender_addr, pkt.sender_cred))
        } else if sock.peer_closed {
            Ok((0, None, None))
        } else if sock.nonblocking {
            Err(UnixSocketError::WouldBlock)
        } else {
            Err(UnixSocketError::WouldBlock)
        }
    }

    pub fn is_readable(&self, handle: SocketHandle) -> bool {
        if let Some(sock) = self.sockets.get(&handle) {
            match sock.state {
                SocketState::Listening => !sock.pending_connections.is_empty(),
                SocketState::Connected | SocketState::Bound => {
                    !sock.stream_rx.is_empty() || !sock.dgram_rx.is_empty() || sock.peer_closed
                }
                _ => false,
            }
        } else {
            false
        }
    }

    pub fn is_writable(&self, handle: SocketHandle) -> bool {
        if let Some(sock) = self.sockets.get(&handle) {
            match sock.state {
                SocketState::Connected => {
                    if sock.peer_closed {
                        false
                    } else if let Some(peer_h) = sock.peer_handle {
                        if let Some(peer) = self.sockets.get(&peer_h) {
                            !peer.peer_closed && peer.stream_rx.available_write() > 0
                        } else {
                            false
                        }
                    } else {
                        false
                    }
                }
                SocketState::Bound => true,
                _ => false,
            }
        } else {
            false
        }
    }
}

pub static UNIX_SOCKET_TABLE: Spinlock<UnixSocketTable> = Spinlock::new(UnixSocketTable::new());

pub fn unix_socket(sock_type: SocketType) -> Result<SocketHandle, UnixSocketError> {
    GLOBAL_SOCKET_TABLE
        .create_socket(SocketDomain::AfUnix, sock_type, SocketProtocol::Default)
        .map_err(|_| UnixSocketError::BufferFull)
}

pub fn unix_bind(handle: SocketHandle, sun: &sockaddr_un) -> Result<(), UnixSocketError> {
    UNIX_SOCKET_TABLE.lock().bind(handle, sun)
}

pub fn unix_listen(handle: SocketHandle, backlog: usize) -> Result<(), UnixSocketError> {
    UNIX_SOCKET_TABLE.lock().listen(handle, backlog)
}

pub fn unix_connect(handle: SocketHandle, sun: &sockaddr_un) -> Result<(), UnixSocketError> {
    UNIX_SOCKET_TABLE.lock().connect(handle, sun)
}

pub fn unix_accept(handle: SocketHandle) -> Result<(SocketHandle, Option<ucred>), UnixSocketError> {
    let new_handle = GLOBAL_SOCKET_TABLE
        .create_socket(SocketDomain::AfUnix, SocketType::SockStream, SocketProtocol::Default)
        .map_err(|_| UnixSocketError::BufferFull)?;
    UNIX_SOCKET_TABLE.lock().accept(handle, new_handle)
}

pub fn unix_getpeercred(handle: SocketHandle) -> Result<ucred, UnixSocketError> {
    UNIX_SOCKET_TABLE.lock().getpeercred(handle)
}

pub fn unix_set_credentials(handle: SocketHandle, cred: ucred) -> Result<(), UnixSocketError> {
    UNIX_SOCKET_TABLE.lock().set_credentials(handle, cred)
}

pub fn unix_close(handle: SocketHandle) -> Result<(), UnixSocketError> {
    let _ = GLOBAL_SOCKET_TABLE.close(handle);
    UNIX_SOCKET_TABLE.lock().close(handle)
}

pub fn unix_set_nonblocking(handle: SocketHandle, nonblocking: bool) -> Result<(), UnixSocketError> {
    let _ = GLOBAL_SOCKET_TABLE.set_nonblocking(handle, nonblocking);
    UNIX_SOCKET_TABLE.lock().set_nonblocking(handle, nonblocking)
}

pub fn unix_socketpair(sock_type: SocketType) -> Result<(SocketHandle, SocketHandle), UnixSocketError> {
    let h1 = GLOBAL_SOCKET_TABLE
        .create_socket(SocketDomain::AfUnix, sock_type, SocketProtocol::Default)
        .map_err(|_| UnixSocketError::BufferFull)?;
    let h2 = GLOBAL_SOCKET_TABLE
        .create_socket(SocketDomain::AfUnix, sock_type, SocketProtocol::Default)
        .map_err(|_| UnixSocketError::BufferFull)?;

    UNIX_SOCKET_TABLE.lock().socketpair(sock_type, h1, h2)?;
    Ok((h1, h2))
}

pub fn unix_send(handle: SocketHandle, data: &[u8]) -> Result<usize, UnixSocketError> {
    UNIX_SOCKET_TABLE.lock().send(handle, data)
}

pub fn unix_recv(handle: SocketHandle, buf: &mut [u8]) -> Result<usize, UnixSocketError> {
    UNIX_SOCKET_TABLE.lock().recv(handle, buf)
}

pub fn unix_sendto(handle: SocketHandle, data: &[u8], target: Option<&sockaddr_un>) -> Result<usize, UnixSocketError> {
    UNIX_SOCKET_TABLE.lock().sendto(handle, data, target)
}

pub fn unix_recvfrom(
    handle: SocketHandle,
    buf: &mut [u8],
) -> Result<(usize, Option<UnixSocketAddress>, Option<ucred>), UnixSocketError> {
    UNIX_SOCKET_TABLE.lock().recvfrom(handle, buf)
}