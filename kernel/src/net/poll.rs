//! BSD / POSIX Event Readiness Polling & Multiplexing Engine.
//! Implements standard `poll` and `select` I/O readiness multiplexing over the socket table:

use crate::net::socket::{GLOBAL_SOCKET_TABLE, SocketState};

pub const POLLIN: u16 = 0x0001;   // Data is available to read
pub const POLLPRI: u16 = 0x0002;  // Urgent or out-of-band data is available
pub const POLLOUT: u16 = 0x0004;  // Socket is ready for writing without blocking
pub const POLLERR: u16 = 0x0008;  // Error condition occurred on socket
pub const POLLHUP: u16 = 0x0010;  // Remote peer hung up / connection closed
pub const POLLNVAL: u16 = 0x0020; // Invalid file descriptor (not open)

/// Socket polling request and result descriptor (POSIX struct pollfd).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PollFd {
    pub fd: usize,
    pub events: u16,
    pub revents: u16,
}

impl PollFd {
    /// Creates a new PollFd query for the given file descriptor and event mask.
    pub const fn new(fd: usize, events: u16) -> Self {
        Self {
            fd,
            events,
            revents: 0,
        }
    }

    /// Checks if the POLLIN flag was set in revents.
    pub fn is_readable(&self) -> bool {
        (self.revents & POLLIN) != 0
    }

    /// Checks if the POLLOUT flag was set in revents.
    pub fn is_writable(&self) -> bool {
        (self.revents & POLLOUT) != 0
    }

    /// Checks if an error condition was signaled.
    pub fn is_error(&self) -> bool {
        (self.revents & POLLERR) != 0
    }

    /// Checks if the connection hung up.
    pub fn is_hangup(&self) -> bool {
        (self.revents & POLLHUP) != 0
    }
}

/// Multiplexes event readiness across an array of PollFd descriptors.
/// Returns the number of descriptors with non-zero revents.
pub fn poll(fds: &mut [PollFd], _timeout_ticks: Option<u64>) -> Result<usize, &'static str> {
    let mut ready_count = 0;

    for pfd in fds.iter_mut() {
        pfd.revents = 0;

        if !GLOBAL_SOCKET_TABLE.exists(pfd.fd) {
            pfd.revents |= POLLNVAL;
            ready_count += 1;
            continue;
        }

        if let Ok(state) = GLOBAL_SOCKET_TABLE.get_state(pfd.fd) {
            if state == SocketState::Closed {
                pfd.revents |= POLLHUP;
                ready_count += 1;
                continue;
            }
        }

        let mut event_triggered = false;

        if (pfd.events & POLLIN) != 0 && GLOBAL_SOCKET_TABLE.is_readable(pfd.fd) {
            pfd.revents |= POLLIN;
            event_triggered = true;
        }

        if (pfd.events & POLLOUT) != 0 && GLOBAL_SOCKET_TABLE.is_writable(pfd.fd) {
            pfd.revents |= POLLOUT;
            event_triggered = true;
        }

        if event_triggered {
            ready_count += 1;
        }
    }

    Ok(ready_count)
}

/// Select-style multiplexer checking read and write descriptor sets.
pub fn select(
    read_fds: &mut [usize],
    write_fds: &mut [usize],
    _timeout_ticks: Option<u64>,
) -> Result<usize, &'static str> {
    let mut ready_count = 0;

    for fd in read_fds.iter_mut() {
        if GLOBAL_SOCKET_TABLE.exists(*fd) && GLOBAL_SOCKET_TABLE.is_readable(*fd) {
            ready_count += 1;
        } else {
            *fd = usize::MAX; // Mark unready
        }
    }

    for fd in write_fds.iter_mut() {
        if GLOBAL_SOCKET_TABLE.exists(*fd) && GLOBAL_SOCKET_TABLE.is_writable(*fd) {
            ready_count += 1;
        } else {
            *fd = usize::MAX; // Mark unready
        }
    }

    Ok(ready_count)
}

/// Self-test verifying PollFd initialization, bitmask checking, and poll readiness.
pub fn verify_poll_multiplexer() -> bool {
    use crate::net::socket::{bind, close, connect, listen, socket, SocketDomain, SocketProtocol, SocketType};

    let server_fd = match socket(SocketDomain::AfInet, SocketType::SockStream, SocketProtocol::Tcp) {
        Ok(h) => h,
        Err(_) => return false,
    };
    let client_fd = match socket(SocketDomain::AfInet, SocketType::SockStream, SocketProtocol::Tcp) {
        Ok(h) => h,
        Err(_) => return false,
    };

    if bind(server_fd, [127, 0, 0, 1], 9876).is_err() {
        return false;
    }
    if listen(server_fd, 5).is_err() {
        return false;
    }

    let mut poll_set = [
        PollFd::new(server_fd, POLLIN),
        PollFd::new(client_fd, POLLOUT | POLLIN),
    ];

    // Initial poll: server has no pending incoming connections
    let ready_initial = match poll(&mut poll_set, Some(0)) {
        Ok(r) => r,
        Err(_) => return false,
    };
    if ready_initial != 0 {
        return false;
    }

    // Connect client to server
    if connect(client_fd, [127, 0, 0, 1], 9876).is_err() {
        return false;
    }

    // Poll again: server has pending connection (POLLIN) and client is connected (POLLOUT)
    let ready_after_connect = match poll(&mut poll_set, Some(0)) {
        Ok(r) => r,
        Err(_) => return false,
    };
    if ready_after_connect != 2 {
        return false;
    }

    if !poll_set[0].is_readable() || !poll_set[1].is_writable() {
        return false;
    }

    let _ = close(server_fd);
    let _ = close(client_fd);

    true
}
