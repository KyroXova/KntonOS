//! Transmission Control Protocol (TCP, RFC 793) Header Parser and Builder.
//!
//! Implements RFC 793 20-byte standard TCP header layout, control flags (SYN, ACK, FIN, RST, PSH),
//! data offset parsing, window size, and pseudo-header checksum calculation.

use super::ipv4::compute_checksum;
use alloc::vec::Vec;

pub const TCP_FLAG_FIN: u16 = 1 << 0;
pub const TCP_FLAG_SYN: u16 = 1 << 1;
pub const TCP_FLAG_RST: u16 = 1 << 2;
pub const TCP_FLAG_PSH: u16 = 1 << 3;
pub const TCP_FLAG_ACK: u16 = 1 << 4;
pub const TCP_FLAG_URG: u16 = 1 << 5;
pub const TCP_FLAG_ECE: u16 = 1 << 6;
pub const TCP_FLAG_CWR: u16 = 1 << 7;

pub const TCP_HEADER_MIN_LEN: usize = 20;
pub const DEFAULT_TCP_WINDOW: u16 = 65535;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TcpError {
    BufferTooShort,
    InvalidDataOffset,
    InvalidChecksum,
    UnexpectedState,
    ConnectionReset,
}

/// Standard 20-byte RFC 793 TCP Header.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TcpHeader {
    pub src_port: u16,
    pub dst_port: u16,
    pub seq_num: u32,
    pub ack_num: u32,
    pub data_offset: u8, // In 32-bit words (5 = 20 bytes)
    pub flags: u16,
    pub window_size: u16,
    pub checksum: u16,
    pub urgent_ptr: u16,
}

impl TcpHeader {
    /// Creates a standard TCP header with the specified sequence/acknowledgment numbers and flags.
    pub fn new(
        src_port: u16,
        dst_port: u16,
        seq_num: u32,
        ack_num: u32,
        flags: u16,
        window_size: u16,
    ) -> Self {
        Self {
            src_port,
            dst_port,
            seq_num,
            ack_num,
            data_offset: 5, // 5 * 4 = 20 bytes
            flags,
            window_size,
            checksum: 0,
            urgent_ptr: 0,
        }
    }

    /// Serializes the TCP header into a 20-byte array.
    pub fn serialize(&self) -> [u8; TCP_HEADER_MIN_LEN] {
        let mut buf = [0u8; TCP_HEADER_MIN_LEN];
        buf[0..2].copy_from_slice(&self.src_port.to_be_bytes());
        buf[2..4].copy_from_slice(&self.dst_port.to_be_bytes());
        buf[4..8].copy_from_slice(&self.seq_num.to_be_bytes());
        buf[8..12].copy_from_slice(&self.ack_num.to_be_bytes());

        let offset_and_flags = ((self.data_offset as u16) << 12) | (self.flags & 0x0FFF);
        buf[12..14].copy_from_slice(&offset_and_flags.to_be_bytes());
        buf[14..16].copy_from_slice(&self.window_size.to_be_bytes());
        buf[16..18].copy_from_slice(&self.checksum.to_be_bytes());
        buf[18..20].copy_from_slice(&self.urgent_ptr.to_be_bytes());
        buf
    }

    /// Parses a TCP header from raw bytes.
    pub fn parse(buf: &[u8]) -> Result<(Self, usize), TcpError> {
        if buf.len() < TCP_HEADER_MIN_LEN {
            return Err(TcpError::BufferTooShort);
        }

        let src_port = u16::from_be_bytes([buf[0], buf[1]]);
        let dst_port = u16::from_be_bytes([buf[2], buf[3]]);
        let seq_num = u32::from_be_bytes([buf[4], buf[5], buf[6], buf[7]]);
        let ack_num = u32::from_be_bytes([buf[8], buf[9], buf[10], buf[11]]);

        let offset_and_flags = u16::from_be_bytes([buf[12], buf[13]]);
        let data_offset = ((offset_and_flags >> 12) & 0x0F) as u8;
        let flags = offset_and_flags & 0x0FFF;

        if data_offset < 5 {
            return Err(TcpError::InvalidDataOffset);
        }

        let header_len = (data_offset as usize) * 4;
        if buf.len() < header_len {
            return Err(TcpError::BufferTooShort);
        }

        let window_size = u16::from_be_bytes([buf[14], buf[15]]);
        let checksum = u16::from_be_bytes([buf[16], buf[17]]);
        let urgent_ptr = u16::from_be_bytes([buf[18], buf[19]]);

        Ok((
            Self {
                src_port,
                dst_port,
                seq_num,
                ack_num,
                data_offset,
                flags,
                window_size,
                checksum,
                urgent_ptr,
            },
            header_len,
        ))
    }
}

/// Computes the RFC 793 TCP checksum using the IPv4 pseudo-header.
pub fn compute_tcp_checksum(
    src_ip: [u8; 4],
    dst_ip: [u8; 4],
    tcp_segment: &[u8],
) -> u16 {
    let mut pseudo = Vec::with_capacity(12 + tcp_segment.len());
    // 1. Source IP (4)
    pseudo.extend_from_slice(&src_ip);
    // 2. Dest IP (4)
    pseudo.extend_from_slice(&dst_ip);
    // 3. Zero byte (1)
    pseudo.push(0);
    // 4. Protocol (1) -> 6 for TCP
    pseudo.push(6);
    // 5. TCP segment length (2)
    pseudo.extend_from_slice(&(tcp_segment.len() as u16).to_be_bytes());
    // 6. TCP header + payload
    pseudo.extend_from_slice(tcp_segment);

    compute_checksum(&pseudo)
}
