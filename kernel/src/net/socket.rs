//! BSD / WinSock Sockets Abstraction Layer & State Descriptors.
//!
//! Provides standard POSIX / WinSock socket primitives, domain/type/protocol enumeration,
//! per-process thread-safe socket handle allocation tables, and socket lifecycle tracking.

use crate::memory::Spinlock;
use alloc::collections::BTreeMap;
use alloc::vec::Vec;

pub type SocketHandle = usize;

pub const INVALID_SOCKET: SocketHandle = usize::MAX;

/// Socket Address Families (Domains).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SocketDomain {
    AfUnspec = 0,
    AfUnix = 1,
    AfInet = 2,
    AfInet6 = 10,
}

/// Socket Types.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SocketType {
    SockStream = 1, // Sequential, reliable, two-way connection (TCP)
    SockDgram = 2,  // Connectionless, unreliable datagrams (UDP)
    SockRaw = 3,    // Raw network protocol access
}

/// Socket Protocols.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SocketProtocol {
    Default = 0,
    Tcp = 6,
    Udp = 17,
}

/// Socket Operational States.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SocketState {
    Unbound,
    Bound,
    Listening,
    Connecting,
    Connected,
    Closed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SocketError {
    InvalidHandle,
    TableFull,
    AlreadyBound,
    NotBound,
    AlreadyConnected,
    NotConnected,
    ConnectionRefused,
    WouldBlock,
    UnsupportedDomain,
    UnsupportedType,
    AddressInUse,
    InvalidAddress,
}

/// Socket descriptor holding state, addresses, and transmission buffers.
#[derive(Debug, Clone)]
pub struct Socket {
    pub handle: SocketHandle,
    pub domain: SocketDomain,
    pub socket_type: SocketType,
    pub protocol: SocketProtocol,
    pub state: SocketState,
    pub local_addr: Option<([u8; 4], u16)>,
    pub peer_addr: Option<([u8; 4], u16)>,
    pub nonblocking: bool,
    pub rx_buffer: Vec<u8>,
    pub tx_buffer: Vec<u8>,
}

impl Socket {
    pub fn new(
        handle: SocketHandle,
        domain: SocketDomain,
        socket_type: SocketType,
        protocol: SocketProtocol,
    ) -> Self {
        Self {
            handle,
            domain,
            socket_type,
            protocol,
            state: SocketState::Unbound,
            local_addr: None,
            peer_addr: None,
            nonblocking: false,
            rx_buffer: Vec::new(),
            tx_buffer: Vec::new(),
        }
    }

    pub fn is_readable(&self) -> bool {
        match self.state {
            SocketState::Listening => true, // Pending connections ready to accept
            SocketState::Connected | SocketState::Bound => !self.rx_buffer.is_empty(),
            _ => false,
        }
    }

    pub fn is_writable(&self) -> bool {
        match self.state {
            SocketState::Connected | SocketState::Bound => true,
            _ => false,
        }
    }
}

/// Thread-safe process socket table managing descriptor allocation and lifecycle.
pub struct SocketTable {
    inner: Spinlock<SocketTableInner>,
}

struct SocketTableInner {
    next_handle: SocketHandle,
    max_sockets: usize,
    sockets: BTreeMap<SocketHandle, Socket>,
}

impl SocketTable {
    pub const fn new(max_sockets: usize) -> Self {
        Self {
            inner: Spinlock::new(SocketTableInner {
                next_handle: 3, // 0=stdin, 1=stdout, 2=stderr
                max_sockets,
                sockets: BTreeMap::new(),
            }),
        }
    }

    /// Allocates a new socket handle in the table.
    pub fn create_socket(
        &self,
        domain: SocketDomain,
        sock_type: SocketType,
        protocol: SocketProtocol,
    ) -> Result<SocketHandle, SocketError> {
        let mut inner = self.inner.lock();

        if inner.sockets.len() >= inner.max_sockets {
            return Err(SocketError::TableFull);
        }

        let handle = inner.next_handle;
        inner.next_handle = inner.next_handle.wrapping_add(1);

        let socket = Socket::new(handle, domain, sock_type, protocol);
        inner.sockets.insert(handle, socket);

        Ok(handle)
    }

    /// Sets non-blocking I/O mode on the socket descriptor.
    pub fn set_nonblocking(&self, handle: SocketHandle, nonblocking: bool) -> Result<(), SocketError> {
        let mut inner = self.inner.lock();
        let sock = inner.sockets.get_mut(&handle).ok_or(SocketError::InvalidHandle)?;
        sock.nonblocking = nonblocking;
        Ok(())
    }

    /// Closes and deallocates a socket descriptor.
    pub fn close(&self, handle: SocketHandle) -> Result<(), SocketError> {
        let mut inner = self.inner.lock();
        let mut sock = inner.sockets.remove(&handle).ok_or(SocketError::InvalidHandle)?;
        sock.state = SocketState::Closed;
        sock.rx_buffer.clear();
        sock.tx_buffer.clear();
        Ok(())
    }

    /// Queries the current state of a socket.
    pub fn get_state(&self, handle: SocketHandle) -> Result<SocketState, SocketError> {
        let inner = self.inner.lock();
        let sock = inner.sockets.get(&handle).ok_or(SocketError::InvalidHandle)?;
        Ok(sock.state)
    }

    /// Checks if a socket exists in the table.
    pub fn exists(&self, handle: SocketHandle) -> bool {
        let inner = self.inner.lock();
        inner.sockets.contains_key(&handle)
    }

    /// Returns the number of active sockets in the table.
    pub fn count(&self) -> usize {
        let inner = self.inner.lock();
        inner.sockets.len()
    }
}

/// Global system socket table singleton.
pub static GLOBAL_SOCKET_TABLE: SocketTable = SocketTable::new(256);

/// Self-test verifying socket descriptor allocation, nonblocking flag, and lifecycle.
pub fn verify_socket_table() -> bool {
    let table = SocketTable::new(16);

    let handle = match table.create_socket(SocketDomain::AfInet, SocketType::SockStream, SocketProtocol::Tcp) {
        Ok(h) => h,
        Err(_) => return false,
    };

    if !table.exists(handle) || table.count() != 1 {
        return false;
    }

    if table.set_nonblocking(handle, true).is_err() {
        return false;
    }

    if table.get_state(handle) != Ok(SocketState::Unbound) {
        return false;
    }

    if table.close(handle).is_err() {
        return false;
    }

    if table.exists(handle) || table.count() != 0 {
        return false;
    }

    true
}
