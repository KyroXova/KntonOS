//! Internet Control Message Protocol (ICMP, RFC 792) Ping Responder.
//!
//! Implements ICMP Echo Request (Type 8) reception and automated Echo Reply (Type 0)
//! generation for bare-metal kernel network diagnostics and ICMP ping validation.

use super::ipv4::compute_checksum;
use alloc::vec::Vec;

pub const ICMP_TYPE_ECHO_REPLY: u8 = 0;
pub const ICMP_TYPE_DEST_UNREACHABLE: u8 = 3;
pub const ICMP_TYPE_ECHO_REQUEST: u8 = 8;
pub const ICMP_TYPE_TIME_EXCEEDED: u8 = 11;

pub const ICMP_HEADER_LEN: usize = 8;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IcmpError {
    BufferTooShort,
    InvalidChecksum,
    UnsupportedType(u8),
}

/// Parsed ICMP Echo message (Echo Request or Echo Reply).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IcmpEchoMessage {
    pub msg_type: u8,
    pub code: u8,
    pub checksum: u16,
    pub identifier: u16,
    pub sequence_number: u16,
    pub payload: Vec<u8>,
}

impl IcmpEchoMessage {
    /// Parses an ICMP message from an IPv4 packet payload and validates checksum.
    pub fn parse(buf: &[u8]) -> Result<Self, IcmpError> {
        if buf.len() < ICMP_HEADER_LEN {
            return Err(IcmpError::BufferTooShort);
        }

        // Verify ICMP checksum over the entire message
        if compute_checksum(buf) != 0 {
            return Err(IcmpError::InvalidChecksum);
        }

        let msg_type = buf[0];
        let code = buf[1];
        let checksum = u16::from_be_bytes([buf[2], buf[3]]);
        let identifier = u16::from_be_bytes([buf[4], buf[5]]);
        let sequence_number = u16::from_be_bytes([buf[6], buf[7]]);

        let mut payload = Vec::with_capacity(buf.len() - ICMP_HEADER_LEN);
        payload.extend_from_slice(&buf[ICMP_HEADER_LEN..]);

        Ok(Self {
            msg_type,
            code,
            checksum,
            identifier,
            sequence_number,
            payload,
        })
    }

    /// Serializes the ICMP message and calculates the correct RFC 792 checksum.
    pub fn serialize(&self) -> Vec<u8> {
        let mut buf = Vec::with_capacity(ICMP_HEADER_LEN + self.payload.len());
        buf.push(self.msg_type);
        buf.push(self.code);
        buf.extend_from_slice(&[0, 0]); // Zero checksum for calculation
        buf.extend_from_slice(&self.identifier.to_be_bytes());
        buf.extend_from_slice(&self.sequence_number.to_be_bytes());
        buf.extend_from_slice(&self.payload);

        let csum = compute_checksum(&buf);
        buf[2..4].copy_from_slice(&csum.to_be_bytes());
        buf
    }

    /// Creates an ICMP Echo Request packet (Type 8, Code 0).
    pub fn build_echo_request(id: u16, seq: u16, payload: &[u8]) -> Self {
        let mut msg = Self {
            msg_type: ICMP_TYPE_ECHO_REQUEST,
            code: 0,
            checksum: 0,
            identifier: id,
            sequence_number: seq,
            payload: payload.to_vec(),
        };
        // Compute checksum
        let serialized = msg.serialize();
        msg.checksum = u16::from_be_bytes([serialized[2], serialized[3]]);
        msg
    }

    /// Generates an automated ICMP Echo Reply (Type 0, Code 0) responding to an Echo Request.
    pub fn build_echo_reply(request: &Self) -> Self {
        let mut reply = Self {
            msg_type: ICMP_TYPE_ECHO_REPLY,
            code: 0,
            checksum: 0,
            identifier: request.identifier,
            sequence_number: request.sequence_number,
            payload: request.payload.clone(),
        };
        let serialized = reply.serialize();
        reply.checksum = u16::from_be_bytes([serialized[2], serialized[3]]);
        reply
    }
}
