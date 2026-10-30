//! User Datagram Protocol (UDP, RFC 768) Framing and Socket Endpoint Registry.
//! Provides connectionless UDP datagram transmission, header parsing, port demultiplexing,

use alloc::vec::Vec;

pub const UDP_HEADER_LEN: usize = 8;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UdpError {
    BufferTooShort,
    LengthMismatch,
    PortAlreadyBound,
    PortNotBound,
}

/// Standard 8-byte RFC 768 UDP Header.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UdpHeader {
    pub src_port: u16,
    pub dst_port: u16,
    pub length: u16,
    pub checksum: u16,
}

impl UdpHeader {
    /// Creates a new UDP header with the specified source/destination ports and payload length.
    pub fn new(src_port: u16, dst_port: u16, payload_len: usize) -> Self {
        Self {
            src_port,
            dst_port,
            length: (UDP_HEADER_LEN + payload_len) as u16,
            checksum: 0, // 0 = checksum disabled in IPv4
        }
    }

    /// Serializes the UDP header into an 8-byte array.
    pub fn serialize(&self) -> [u8; UDP_HEADER_LEN] {
        let mut buf = [0u8; UDP_HEADER_LEN];
        buf[0..2].copy_from_slice(&self.src_port.to_be_bytes());
        buf[2..4].copy_from_slice(&self.dst_port.to_be_bytes());
        buf[4..6].copy_from_slice(&self.length.to_be_bytes());
        buf[6..8].copy_from_slice(&self.checksum.to_be_bytes());
        buf
    }

    /// Parses a UDP header from raw bytes.
    pub fn parse(buf: &[u8]) -> Result<Self, UdpError> {
        if buf.len() < UDP_HEADER_LEN {
            return Err(UdpError::BufferTooShort);
        }

        let src_port = u16::from_be_bytes([buf[0], buf[1]]);
        let dst_port = u16::from_be_bytes([buf[2], buf[3]]);
        let length = u16::from_be_bytes([buf[4], buf[5]]);
        let checksum = u16::from_be_bytes([buf[6], buf[7]]);

        if (length as usize) < UDP_HEADER_LEN || buf.len() < (length as usize) {
            return Err(UdpError::LengthMismatch);
        }

        Ok(Self {
            src_port,
            dst_port,
            length,
            checksum,
        })
    }
}

/// High-level parsed UDP datagram including endpoints and payload.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UdpDatagram {
    pub src_ip: [u8; 4],
    pub dst_ip: [u8; 4],
    pub src_port: u16,
    pub dst_port: u16,
    pub payload: Vec<u8>,
}

impl UdpDatagram {
    /// Parses a complete UDP datagram from an IPv4 payload slice.
    pub fn parse(src_ip: [u8; 4], dst_ip: [u8; 4], buf: &[u8]) -> Result<Self, UdpError> {
        let header = UdpHeader::parse(buf)?;
        let data_len = (header.length as usize) - UDP_HEADER_LEN;

        let mut payload = Vec::with_capacity(data_len);
        payload.extend_from_slice(&buf[UDP_HEADER_LEN..UDP_HEADER_LEN + data_len]);

        Ok(Self {
            src_ip,
            dst_ip,
            src_port: header.src_port,
            dst_port: header.dst_port,
            payload,
        })
    }

    /// Serializes the UDP datagram into bytes (header + payload).
    pub fn serialize(&self) -> Vec<u8> {
        let header = UdpHeader::new(self.src_port, self.dst_port, self.payload.len());
        let mut bytes = Vec::with_capacity(UDP_HEADER_LEN + self.payload.len());
        bytes.extend_from_slice(&header.serialize());
        bytes.extend_from_slice(&self.payload);
        bytes
    }
}

/// Dynamic socket endpoint registry demultiplexing UDP packets by destination port.
pub struct UdpSocketRegistry {
    bound_ports: Vec<(u16, Vec<UdpDatagram>)>,
}

impl UdpSocketRegistry {
    pub const fn new() -> Self {
        Self {
            bound_ports: Vec::new(),
        }
    }

    /// Binds a UDP port to receive incoming datagrams.
    pub fn bind(&mut self, port: u16) -> Result<(), UdpError> {
        for (p, _) in &self.bound_ports {
            if *p == port {
                return Err(UdpError::PortAlreadyBound);
            }
        }
        self.bound_ports.push((port, Vec::new()));
        Ok(())
    }

    /// Unbinds a previously bound UDP port.
    pub fn unbind(&mut self, port: u16) {
        self.bound_ports.retain(|(p, _)| *p != port);
    }

    /// Checks if a port is actively bound.
    pub fn is_bound(&self, port: u16) -> bool {
        self.bound_ports.iter().any(|(p, _)| *p == port)
    }

    /// Dispatches an incoming datagram to the matching listening socket queue.
    pub fn dispatch(&mut self, dgram: UdpDatagram) -> bool {
        for (port, queue) in self.bound_ports.iter_mut() {
            if *port == dgram.dst_port {
                queue.push(dgram);
                return true;
            }
        }
        false
    }

    /// Polls and retrieves the next received datagram for a bound port.
    pub fn poll(&mut self, port: u16) -> Option<UdpDatagram> {
        for (p, queue) in self.bound_ports.iter_mut() {
            if *p == port && !queue.is_empty() {
                return Some(queue.remove(0));
            }
        }
        None
    }
}
