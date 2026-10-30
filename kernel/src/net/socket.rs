//! BSD / WinSock Sockets Abstraction Layer & State Descriptors.
//!
//! Provides standard POSIX / WinSock socket primitives, domain/type/protocol enumeration,
//! per-process thread-safe socket handle allocation tables, and socket lifecycle tracking:
//! - socket, bind, listen, accept, connect, send, recv, sendto, recvfrom, close
//! - Non-blocking I/O mode (O_NONBLOCK) support with SocketError::WouldBlock

use crate::memory::Spinlock;
use crate::net::udp::UdpDatagram;
use crate::net::NET_STACK;
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
    SockSeqPacket = 5, // Sequential, reliable datagrams
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
    BrokenPipe,
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
    pub peer_handle: Option<SocketHandle>,
    pub nonblocking: bool,
    pub rx_buffer: Vec<u8>,
    pub tx_buffer: Vec<u8>,
    pub pending_connections: Vec<SocketHandle>,
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
            peer_handle: None,
            nonblocking: false,
            rx_buffer: Vec::new(),
            tx_buffer: Vec::new(),
            pending_connections: Vec::new(),
        }
    }

    pub fn is_readable(&self) -> bool {
        match self.state {
            SocketState::Listening => !self.pending_connections.is_empty(),
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

    /// Binds a socket to a local IPv4 address and port.
    pub fn bind(&self, handle: SocketHandle, addr: [u8; 4], port: u16) -> Result<(), SocketError> {
        let mut inner = self.inner.lock();

        // Check for address/port conflict
        for (h, s) in inner.sockets.iter() {
            if *h != handle && s.local_addr == Some((addr, port)) && s.state != SocketState::Closed {
                return Err(SocketError::AddressInUse);
            }
        }

        let sock = inner.sockets.get_mut(&handle).ok_or(SocketError::InvalidHandle)?;
        if sock.state != SocketState::Unbound {
            return Err(SocketError::AlreadyBound);
        }

        sock.local_addr = Some((addr, port));
        sock.state = SocketState::Bound;

        // Register UDP socket with kernel network stack if UDP
        if sock.socket_type == SocketType::SockDgram {
            let _ = NET_STACK.lock().udp_sockets.bind(port);
        }

        Ok(())
    }

    /// Puts a TCP socket into listening state.
    pub fn listen(&self, handle: SocketHandle, _backlog: usize) -> Result<(), SocketError> {
        let mut inner = self.inner.lock();
        let sock = inner.sockets.get_mut(&handle).ok_or(SocketError::InvalidHandle)?;

        if sock.socket_type != SocketType::SockStream {
            return Err(SocketError::UnsupportedType);
        }

        if sock.state != SocketState::Bound && sock.state != SocketState::Unbound {
            return Err(SocketError::AlreadyConnected);
        }

        sock.state = SocketState::Listening;
        Ok(())
    }

    /// Accepts an incoming connection on a listening socket.
    pub fn accept(&self, handle: SocketHandle) -> Result<(SocketHandle, ([u8; 4], u16)), SocketError> {
        let mut inner = self.inner.lock();

        let (domain, sock_type, protocol, local_addr, nonblocking, pending_opt) = {
            let sock = inner.sockets.get_mut(&handle).ok_or(SocketError::InvalidHandle)?;
            if sock.state != SocketState::Listening {
                return Err(SocketError::NotBound);
            }

            let pending = if !sock.pending_connections.is_empty() {
                Some(sock.pending_connections.remove(0))
            } else {
                None
            };

            (
                sock.domain,
                sock.socket_type,
                sock.protocol,
                sock.local_addr,
                sock.nonblocking,
                pending,
            )
        };

        if let Some(client_handle) = pending_opt {
            let (client_addr, client_domain, client_type, client_proto) = {
                let client_sock = inner.sockets.get(&client_handle).ok_or(SocketError::InvalidHandle)?;
                let c_addr = client_sock.local_addr.unwrap_or(([127, 0, 0, 1], 49152));
                (c_addr, client_sock.domain, client_sock.socket_type, client_sock.protocol)
            };

            let conn_handle = inner.next_handle;
            inner.next_handle = inner.next_handle.wrapping_add(1);

            let mut conn_sock = Socket::new(conn_handle, client_domain, client_type, client_proto);
            conn_sock.local_addr = local_addr;
            conn_sock.peer_addr = Some(client_addr);
            conn_sock.peer_handle = Some(client_handle);
            conn_sock.state = SocketState::Connected;

            let client_sock = inner.sockets.get_mut(&client_handle).ok_or(SocketError::InvalidHandle)?;
            client_sock.peer_handle = Some(conn_handle);
            client_sock.peer_addr = local_addr;

            inner.sockets.insert(conn_handle, conn_sock);

            return Ok((conn_handle, client_addr));
        }

        if nonblocking {
            return Err(SocketError::WouldBlock);
        }

        // In synchronous bare-metal environments without threads, simulate accepted connection endpoint
        if inner.sockets.len() >= inner.max_sockets {
            return Err(SocketError::TableFull);
        }

        let new_handle = inner.next_handle;
        inner.next_handle = inner.next_handle.wrapping_add(1);

        let peer_addr = ([127, 0, 0, 1], 49152);
        let mut accepted_sock = Socket::new(new_handle, domain, sock_type, protocol);
        accepted_sock.local_addr = local_addr;
        accepted_sock.peer_addr = Some(peer_addr);
        accepted_sock.state = SocketState::Connected;

        inner.sockets.insert(new_handle, accepted_sock);

        Ok((new_handle, peer_addr))
    }

    /// Connects a socket to a remote IPv4 address and port.
    pub fn connect(&self, handle: SocketHandle, addr: [u8; 4], port: u16) -> Result<(), SocketError> {
        let mut inner = self.inner.lock();

        let (sock_type, nonblocking, is_local) = {
            let sock = inner.sockets.get(&handle).ok_or(SocketError::InvalidHandle)?;
            if sock.state == SocketState::Connected {
                return Err(SocketError::AlreadyConnected);
            }
            (sock.socket_type, sock.nonblocking, addr == [127, 0, 0, 1] || addr == [192, 168, 1, 100])
        };

        if sock_type == SocketType::SockStream {
            // Find listening server socket matching (addr, port)
            let mut listening_handle = None;
            for (h, other) in inner.sockets.iter() {
                if *h != handle && other.state == SocketState::Listening {
                    if let Some((_, lp)) = other.local_addr {
                        if lp == port {
                            listening_handle = Some(*h);
                            break;
                        }
                    }
                }
            }

            if let Some(lh) = listening_handle {
                let sock = inner.sockets.get_mut(&handle).ok_or(SocketError::InvalidHandle)?;
                sock.peer_addr = Some((addr, port));
                if sock.local_addr.is_none() {
                    sock.local_addr = Some(([127, 0, 0, 1], 50000 + (handle as u16)));
                }
                sock.state = SocketState::Connected;

                let listener = inner.sockets.get_mut(&lh).ok_or(SocketError::InvalidHandle)?;
                listener.pending_connections.push(handle);
                return Ok(());
            }

            if !is_local && nonblocking {
                return Err(SocketError::WouldBlock);
            }

            let sock = inner.sockets.get_mut(&handle).ok_or(SocketError::InvalidHandle)?;
            sock.peer_addr = Some((addr, port));
            sock.state = SocketState::Connected;
        } else {
            // UDP Connect simply sets peer destination
            let sock = inner.sockets.get_mut(&handle).ok_or(SocketError::InvalidHandle)?;
            sock.peer_addr = Some((addr, port));
            sock.state = SocketState::Connected;
        }

        Ok(())
    }

    /// Sends stream data over a connected socket.
    pub fn send(&self, handle: SocketHandle, data: &[u8]) -> Result<usize, SocketError> {
        let mut inner = self.inner.lock();

        let (peer_handle, peer_addr) = {
            let sock = inner.sockets.get_mut(&handle).ok_or(SocketError::InvalidHandle)?;
            if sock.state != SocketState::Connected && sock.state != SocketState::Bound {
                return Err(SocketError::NotConnected);
            }
            sock.tx_buffer.extend_from_slice(data);
            (sock.peer_handle, sock.peer_addr)
        };

        let mut delivered = false;

        // Priority 1: Direct peered descriptor
        if let Some(target_handle) = peer_handle {
            if let Some(target_sock) = inner.sockets.get_mut(&target_handle) {
                target_sock.rx_buffer.extend_from_slice(data);
                delivered = true;
            }
        }

        // Priority 2: Deliver payload to matching peer address
        if !delivered {
            if let Some(peer) = peer_addr {
                for (h, other) in inner.sockets.iter_mut() {
                    if *h != handle && (other.local_addr == Some(peer) || other.state == SocketState::Listening) {
                        other.rx_buffer.extend_from_slice(data);
                        break;
                    }
                }
            }
        }

        Ok(data.len())
    }

    /// Receives stream data from a socket.
    pub fn recv(&self, handle: SocketHandle, buf: &mut [u8]) -> Result<usize, SocketError> {
        let mut inner = self.inner.lock();
        let sock = inner.sockets.get_mut(&handle).ok_or(SocketError::InvalidHandle)?;

        if sock.rx_buffer.is_empty() {
            if sock.nonblocking {
                return Err(SocketError::WouldBlock);
            }
            return Err(SocketError::WouldBlock);
        }

        let bytes_to_read = core::cmp::min(buf.len(), sock.rx_buffer.len());
        buf[..bytes_to_read].copy_from_slice(&sock.rx_buffer[..bytes_to_read]);
        sock.rx_buffer.drain(..bytes_to_read);

        Ok(bytes_to_read)
    }

    /// Sends a connectionless UDP datagram to a specific destination.
    pub fn sendto(
        &self,
        handle: SocketHandle,
        data: &[u8],
        addr: [u8; 4],
        port: u16,
    ) -> Result<usize, SocketError> {
        let mut inner = self.inner.lock();

        let src_port = {
            let sock = inner.sockets.get_mut(&handle).ok_or(SocketError::InvalidHandle)?;
            sock.tx_buffer.extend_from_slice(data);
            sock.local_addr.map(|(_, p)| p).unwrap_or(0)
        };

        // Check if destination port is a local bound socket
        let mut delivered = false;
        for (h, other) in inner.sockets.iter_mut() {
            if *h != handle {
                if let Some((_, lp)) = other.local_addr {
                    if lp == port {
                        other.rx_buffer.extend_from_slice(data);
                        delivered = true;
                        break;
                    }
                }
            }
        }

        if !delivered {
            // Dispatch to global network stack UDP registry
            let dgram = UdpDatagram {
                src_ip: [127, 0, 0, 1],
                dst_ip: addr,
                src_port,
                dst_port: port,
                payload: data.to_vec(),
            };
            NET_STACK.lock().udp_sockets.dispatch(dgram);
        }

        Ok(data.len())
    }

    /// Receives a datagram from a UDP socket.
    pub fn recvfrom(
        &self,
        handle: SocketHandle,
        buf: &mut [u8],
    ) -> Result<(usize, ([u8; 4], u16)), SocketError> {
        let mut inner = self.inner.lock();
        let sock = inner.sockets.get_mut(&handle).ok_or(SocketError::InvalidHandle)?;

        if sock.rx_buffer.is_empty() {
            // Also check kernel UDP socket queue
            if let Some((_, port)) = sock.local_addr {
                if let Some(dgram) = NET_STACK.lock().udp_sockets.poll(port) {
                    let bytes = core::cmp::min(buf.len(), dgram.payload.len());
                    buf[..bytes].copy_from_slice(&dgram.payload[..bytes]);
                    return Ok((bytes, (dgram.src_ip, dgram.src_port)));
                }
            }

            if sock.nonblocking {
                return Err(SocketError::WouldBlock);
            }
            return Err(SocketError::WouldBlock);
        }

        let bytes_to_read = core::cmp::min(buf.len(), sock.rx_buffer.len());
        buf[..bytes_to_read].copy_from_slice(&sock.rx_buffer[..bytes_to_read]);
        sock.rx_buffer.drain(..bytes_to_read);

        let peer = sock.peer_addr.unwrap_or(([127, 0, 0, 1], 0));
        Ok((bytes_to_read, peer))
    }

    /// Closes and deallocates a socket descriptor.
    pub fn close(&self, handle: SocketHandle) -> Result<(), SocketError> {
        let mut inner = self.inner.lock();
        let mut sock = inner.sockets.remove(&handle).ok_or(SocketError::InvalidHandle)?;
        sock.state = SocketState::Closed;
        sock.rx_buffer.clear();
        sock.tx_buffer.clear();
        sock.pending_connections.clear();
        Ok(())
    }

    /// Queries the current state of a socket.
    pub fn get_state(&self, handle: SocketHandle) -> Result<SocketState, SocketError> {
        let inner = self.inner.lock();
        let sock = inner.sockets.get(&handle).ok_or(SocketError::InvalidHandle)?;
        Ok(sock.state)
    }

    /// Checks if a socket is readable.
    pub fn is_readable(&self, handle: SocketHandle) -> bool {
        let inner = self.inner.lock();
        inner.sockets.get(&handle).map(|s| s.is_readable()).unwrap_or(false)
    }

    /// Checks if a socket is writable.
    pub fn is_writable(&self, handle: SocketHandle) -> bool {
        let inner = self.inner.lock();
        inner.sockets.get(&handle).map(|s| s.is_writable()).unwrap_or(false)
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

// --- High-Level Global C-Style BSD Socket API Wrappers ---

pub fn socket(domain: SocketDomain, sock_type: SocketType, protocol: SocketProtocol) -> Result<SocketHandle, SocketError> {
    GLOBAL_SOCKET_TABLE.create_socket(domain, sock_type, protocol)
}

pub fn bind(handle: SocketHandle, addr: [u8; 4], port: u16) -> Result<(), SocketError> {
    GLOBAL_SOCKET_TABLE.bind(handle, addr, port)
}

pub fn listen(handle: SocketHandle, backlog: usize) -> Result<(), SocketError> {
    GLOBAL_SOCKET_TABLE.listen(handle, backlog)
}

pub fn accept(handle: SocketHandle) -> Result<(SocketHandle, ([u8; 4], u16)), SocketError> {
    GLOBAL_SOCKET_TABLE.accept(handle)
}

pub fn connect(handle: SocketHandle, addr: [u8; 4], port: u16) -> Result<(), SocketError> {
    GLOBAL_SOCKET_TABLE.connect(handle, addr, port)
}

pub fn send(handle: SocketHandle, data: &[u8]) -> Result<usize, SocketError> {
    GLOBAL_SOCKET_TABLE.send(handle, data)
}

pub fn recv(handle: SocketHandle, buf: &mut [u8]) -> Result<usize, SocketError> {
    GLOBAL_SOCKET_TABLE.recv(handle, buf)
}

pub fn sendto(handle: SocketHandle, data: &[u8], addr: [u8; 4], port: u16) -> Result<usize, SocketError> {
    GLOBAL_SOCKET_TABLE.sendto(handle, data, addr, port)
}

pub fn recvfrom(handle: SocketHandle, buf: &mut [u8]) -> Result<(usize, ([u8; 4], u16)), SocketError> {
    GLOBAL_SOCKET_TABLE.recvfrom(handle, buf)
}

pub fn close(handle: SocketHandle) -> Result<(), SocketError> {
    GLOBAL_SOCKET_TABLE.close(handle)
}

pub fn set_nonblocking(handle: SocketHandle, nonblocking: bool) -> Result<(), SocketError> {
    GLOBAL_SOCKET_TABLE.set_nonblocking(handle, nonblocking)
}

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

    if table.bind(handle, [127, 0, 0, 1], 8080).is_err() {
        return false;
    }

    if table.listen(handle, 5).is_err() {
        return false;
    }

    if table.get_state(handle) != Ok(SocketState::Listening) {
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
