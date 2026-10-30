//! Internet Protocol Version 4 (IPv4, RFC 791) Header Parser, Checksum, and Routing.
//! Handles IPv4 packet framing, RFC 791 one's complement checksum calculation,

use alloc::vec::Vec;

pub const IP_PROTO_ICMP: u8 = 1;
pub const IP_PROTO_TCP: u8 = 6;
pub const IP_PROTO_UDP: u8 = 17;

pub const IPV4_HEADER_MIN_LEN: usize = 20;
pub const DEFAULT_TTL: u8 = 64;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Ipv4Error {
    BufferTooShort,
    InvalidVersion,
    InvalidHeaderLength,
    TotalLengthMismatch,
    InvalidChecksum,
}

/// Computes the standard RFC 791 16-bit one's complement Internet Checksum.
pub fn compute_checksum(data: &[u8]) -> u16 {
    let mut sum: u32 = 0;
    let mut i = 0;

    while i + 1 < data.len() {
        let word = u16::from_be_bytes([data[i], data[i + 1]]) as u32;
        sum = sum.wrapping_add(word);
        i += 2;
    }

    if i < data.len() {
        let word = ((data[i] as u16) << 8) as u32;
        sum = sum.wrapping_add(word);
    }

    while (sum >> 16) != 0 {
        sum = (sum & 0xFFFF) + (sum >> 16);
    }

    !(sum as u16)
}

/// IPv4 packet header representing standard 20-byte base fields.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Ipv4Header {
    pub version: u8,
    pub ihl: u8,
    pub dscp_ecn: u8,
    pub total_len: u16,
    pub identification: u16,
    pub flags_fragment: u16,
    pub ttl: u8,
    pub protocol: u8,
    pub checksum: u16,
    pub src_ip: [u8; 4],
    pub dst_ip: [u8; 4],
}

impl Ipv4Header {
    /// Creates a standard IPv4 header for an outgoing packet.
    pub fn new(src_ip: [u8; 4], dst_ip: [u8; 4], protocol: u8, payload_len: usize) -> Self {
        let total_len = (IPV4_HEADER_MIN_LEN + payload_len) as u16;
        let mut hdr = Self {
            version: 4,
            ihl: 5, // 5 * 4 = 20 bytes
            dscp_ecn: 0,
            total_len,
            identification: 0x1234,
            flags_fragment: 0x4000, // Don't Fragment flag
            ttl: DEFAULT_TTL,
            protocol,
            checksum: 0,
            src_ip,
            dst_ip,
        };

        // Compute checksum over 20-byte header with checksum field zeroed
        let bytes = hdr.serialize();
        hdr.checksum = compute_checksum(&bytes[0..IPV4_HEADER_MIN_LEN]);
        hdr
    }

    /// Serializes the IPv4 header into a 20-byte array.
    pub fn serialize(&self) -> [u8; IPV4_HEADER_MIN_LEN] {
        let mut buf = [0u8; IPV4_HEADER_MIN_LEN];
        buf[0] = (self.version << 4) | (self.ihl & 0x0F);
        buf[1] = self.dscp_ecn;
        buf[2..4].copy_from_slice(&self.total_len.to_be_bytes());
        buf[4..6].copy_from_slice(&self.identification.to_be_bytes());
        buf[6..8].copy_from_slice(&self.flags_fragment.to_be_bytes());
        buf[8] = self.ttl;
        buf[9] = self.protocol;
        buf[10..12].copy_from_slice(&self.checksum.to_be_bytes());
        buf[12..16].copy_from_slice(&self.src_ip);
        buf[16..20].copy_from_slice(&self.dst_ip);
        buf
    }

    /// Parses an IPv4 header from raw bytes and validates checksum.
    pub fn parse(buf: &[u8]) -> Result<(Self, usize), Ipv4Error> {
        if buf.len() < IPV4_HEADER_MIN_LEN {
            return Err(Ipv4Error::BufferTooShort);
        }

        let version = (buf[0] >> 4) & 0x0F;
        let ihl = buf[0] & 0x0F;
        if version != 4 {
            return Err(Ipv4Error::InvalidVersion);
        }
        if ihl < 5 {
            return Err(Ipv4Error::InvalidHeaderLength);
        }

        let header_len = (ihl as usize) * 4;
        if buf.len() < header_len {
            return Err(Ipv4Error::BufferTooShort);
        }

        let dscp_ecn = buf[1];
        let total_len = u16::from_be_bytes([buf[2], buf[3]]);
        let identification = u16::from_be_bytes([buf[4], buf[5]]);
        let flags_fragment = u16::from_be_bytes([buf[6], buf[7]]);
        let ttl = buf[8];
        let protocol = buf[9];
        let checksum = u16::from_be_bytes([buf[10], buf[11]]);

        let mut src_ip = [0u8; 4];
        src_ip.copy_from_slice(&buf[12..16]);

        let mut dst_ip = [0u8; 4];
        dst_ip.copy_from_slice(&buf[16..20]);

        // Verify checksum over header (result should be 0 when including valid checksum)
        if compute_checksum(&buf[0..header_len]) != 0 {
            return Err(Ipv4Error::InvalidChecksum);
        }

        Ok((
            Self {
                version,
                ihl,
                dscp_ecn,
                total_len,
                identification,
                flags_fragment,
                ttl,
                protocol,
                checksum,
                src_ip,
                dst_ip,
            },
            header_len,
        ))
    }
}

/// High-level parsed IPv4 packet containing header and extracted payload.
#[derive(Debug, Clone)]
pub struct Ipv4Packet {
    pub header: Ipv4Header,
    pub payload: Vec<u8>,
}

impl Ipv4Packet {
    pub fn parse(buf: &[u8]) -> Result<Self, Ipv4Error> {
        let (header, hdr_len) = Ipv4Header::parse(buf)?;
        let total_len = header.total_len as usize;

        if buf.len() < total_len || total_len < hdr_len {
            return Err(Ipv4Error::TotalLengthMismatch);
        }

        let mut payload = Vec::with_capacity(total_len - hdr_len);
        payload.extend_from_slice(&buf[hdr_len..total_len]);

        Ok(Self { header, payload })
    }

    pub fn serialize(&self) -> Vec<u8> {
        let mut packet = Vec::with_capacity(IPV4_HEADER_MIN_LEN + self.payload.len());
        packet.extend_from_slice(&self.header.serialize());
        packet.extend_from_slice(&self.payload);
        packet
    }
}

/// Tests whether an IP address belongs to the same local subnet.
pub fn is_same_subnet(ip: &[u8; 4], local_ip: &[u8; 4], netmask: &[u8; 4]) -> bool {
    for i in 0..4 {
        if (ip[i] & netmask[i]) != (local_ip[i] & netmask[i]) {
            return false;
        }
    }
    true
}
