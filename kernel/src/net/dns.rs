//! Domain Name System (DNS, RFC 1035) Wire Protocol Parser & Serializer.
//!
//! Implements standard RFC 1035 wire framing:
//! - 12-byte DNS message header with bitfield flags
//! - Question section serialization with label length encoding
//! - Answer section parsing with uncompressed & pointer compression (`0xC000 | offset`)
//! - IPv4 address resolution (Type A, Class IN)

use alloc::string::String;
use alloc::vec::Vec;

pub const DNS_HEADER_LEN: usize = 12;

pub const DNS_TYPE_A: u16 = 1;
pub const DNS_CLASS_IN: u16 = 1;

pub const DNS_FLAG_QR: u16 = 0x8000; // 0 = Query, 1 = Response
pub const DNS_FLAG_AA: u16 = 0x0400; // Authoritative Answer
pub const DNS_FLAG_TC: u16 = 0x0200; // Truncation
pub const DNS_FLAG_RD: u16 = 0x0100; // Recursion Desired
pub const DNS_FLAG_RA: u16 = 0x0080; // Recursion Available
pub const DNS_RCODE_MASK: u16 = 0x000F;

pub const DNS_RCODE_NOERROR: u8 = 0;
pub const DNS_RCODE_FORMAT_ERROR: u8 = 1;
pub const DNS_RCODE_SERVER_FAILURE: u8 = 2;
pub const DNS_RCODE_NAME_ERROR: u8 = 3; // NXDomain

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DnsError {
    BufferTooShort,
    InvalidLabel,
    PointerLoop,
    InvalidPointerOffset,
    NameTooLong,
    NotAResponse,
    TransactionIdMismatch,
    NameError,
    ServerFailure,
    FormatError,
    NoAnswers,
}

/// 12-byte RFC 1035 DNS Packet Header.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DnsHeader {
    pub id: u16,
    pub flags: u16,
    pub qdcount: u16,
    pub ancount: u16,
    pub nscount: u16,
    pub arcount: u16,
}

impl DnsHeader {
    pub fn new_query(id: u16) -> Self {
        Self {
            id,
            flags: DNS_FLAG_RD, // Recursion Desired
            qdcount: 1,
            ancount: 0,
            nscount: 0,
            arcount: 0,
        }
    }

    pub fn parse(buf: &[u8]) -> Result<Self, DnsError> {
        if buf.len() < DNS_HEADER_LEN {
            return Err(DnsError::BufferTooShort);
        }

        let id = u16::from_be_bytes([buf[0], buf[1]]);
        let flags = u16::from_be_bytes([buf[2], buf[3]]);
        let qdcount = u16::from_be_bytes([buf[4], buf[5]]);
        let ancount = u16::from_be_bytes([buf[6], buf[7]]);
        let nscount = u16::from_be_bytes([buf[8], buf[9]]);
        let arcount = u16::from_be_bytes([buf[10], buf[11]]);

        Ok(Self {
            id,
            flags,
            qdcount,
            ancount,
            nscount,
            arcount,
        })
    }

    pub fn serialize(&self) -> [u8; DNS_HEADER_LEN] {
        let mut buf = [0u8; DNS_HEADER_LEN];
        buf[0..2].copy_from_slice(&self.id.to_be_bytes());
        buf[2..4].copy_from_slice(&self.flags.to_be_bytes());
        buf[4..6].copy_from_slice(&self.qdcount.to_be_bytes());
        buf[6..8].copy_from_slice(&self.ancount.to_be_bytes());
        buf[8..10].copy_from_slice(&self.nscount.to_be_bytes());
        buf[10..12].copy_from_slice(&self.arcount.to_be_bytes());
        buf
    }

    pub fn is_response(&self) -> bool {
        (self.flags & DNS_FLAG_QR) != 0
    }

    pub fn rcode(&self) -> u8 {
        (self.flags & DNS_RCODE_MASK) as u8
    }
}

/// DNS Question Entry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DnsQuestion {
    pub name: String,
    pub qtype: u16,
    pub qclass: u16,
}

/// DNS Resource Record (Answer).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DnsAnswer {
    pub name: String,
    pub rtype: u16,
    pub rclass: u16,
    pub ttl: u32,
    pub rdata: Vec<u8>,
    pub ip: Option<[u8; 4]>,
}

/// Encodes a dot-separated domain name into RFC 1035 length-prefixed labels.
/// Example: "knton.os" -> [0x05, b'k', b'n', b't', b'o', b'n', 0x02, b'o', b's', 0x00]
pub fn encode_domain_name(name: &str) -> Result<Vec<u8>, DnsError> {
    if name.is_empty() {
        return Ok(alloc::vec![0u8]);
    }

    let mut encoded = Vec::with_capacity(name.len() + 2);
    for part in name.split('.') {
        if part.is_empty() {
            continue;
        }
        if part.len() > 63 {
            return Err(DnsError::InvalidLabel);
        }
        encoded.push(part.len() as u8);
        encoded.extend_from_slice(part.as_bytes());
    }
    encoded.push(0x00); // Terminating null label
    Ok(encoded)
}

/// Reads a domain name from `buf` starting at `offset`, supporting pointer decompression.
/// Returns the parsed name and the number of bytes consumed from the initial stream position.
pub fn read_domain_name(buf: &[u8], mut offset: usize) -> Result<(String, usize), DnsError> {
    let mut name = String::new();
    let mut initial_bytes_consumed = 0usize;
    let mut jumped = false;
    let mut jumps = 0usize;
    let start_offset = offset;

    loop {
        if offset >= buf.len() {
            return Err(DnsError::BufferTooShort);
        }

        let len_byte = buf[offset];

        if len_byte == 0 {
            if !jumped {
                initial_bytes_consumed = offset + 1 - start_offset;
            }
            break;
        }

        // Check if top 2 bits are 11 (0xC0), indicating a compression pointer.
        if (len_byte & 0xC0) == 0xC0 {
            if offset + 1 >= buf.len() {
                return Err(DnsError::BufferTooShort);
            }
            let pointer_offset = (((len_byte & 0x3F) as usize) << 8) | (buf[offset + 1] as usize);
            if pointer_offset >= buf.len() {
                return Err(DnsError::InvalidPointerOffset);
            }

            if !jumped {
                initial_bytes_consumed = offset + 2 - start_offset;
                jumped = true;
            }

            jumps += 1;
            if jumps > 16 {
                return Err(DnsError::PointerLoop);
            }

            offset = pointer_offset;
            continue;
        }

        // Regular label
        let label_len = len_byte as usize;
        offset += 1;

        if offset + label_len > buf.len() {
            return Err(DnsError::BufferTooShort);
        }

        if !name.is_empty() {
            name.push('.');
        }

        for &b in &buf[offset..offset + label_len] {
            name.push(b as char);
        }

        offset += label_len;

        if !jumped {
            initial_bytes_consumed = offset - start_offset;
        }
    }

    Ok((name, initial_bytes_consumed))
}

/// Builds a complete RFC 1035 DNS A-record query packet for the given domain name.
pub fn build_dns_query(id: u16, domain: &str) -> Result<Vec<u8>, DnsError> {
    let header = DnsHeader::new_query(id);
    let encoded_name = encode_domain_name(domain)?;

    let mut packet = Vec::with_capacity(DNS_HEADER_LEN + encoded_name.len() + 4);
    packet.extend_from_slice(&header.serialize());
    packet.extend_from_slice(&encoded_name);
    packet.extend_from_slice(&DNS_TYPE_A.to_be_bytes());
    packet.extend_from_slice(&DNS_CLASS_IN.to_be_bytes());

    Ok(packet)
}

/// Parses an incoming DNS response wire packet and extracts resolved IPv4 addresses.
pub fn parse_dns_response(buf: &[u8], expected_id: Option<u16>) -> Result<Vec<[u8; 4]>, DnsError> {
    let header = DnsHeader::parse(buf)?;

    if !header.is_response() {
        return Err(DnsError::NotAResponse);
    }

    if let Some(exp_id) = expected_id {
        if header.id != exp_id {
            return Err(DnsError::TransactionIdMismatch);
        }
    }

    match header.rcode() {
        DNS_RCODE_NOERROR => {}
        DNS_RCODE_NAME_ERROR => return Err(DnsError::NameError),
        DNS_RCODE_SERVER_FAILURE => return Err(DnsError::ServerFailure),
        DNS_RCODE_FORMAT_ERROR => return Err(DnsError::FormatError),
        _ => return Err(DnsError::FormatError),
    }

    let mut offset = DNS_HEADER_LEN;

    // Skip question section
    for _ in 0..header.qdcount {
        let (_name, consumed) = read_domain_name(buf, offset)?;
        offset += consumed;
        if offset + 4 > buf.len() {
            return Err(DnsError::BufferTooShort);
        }
        // Skip QTYPE (2 bytes) and QCLASS (2 bytes)
        offset += 4;
    }

    let mut answers = Vec::new();

    // Parse answer section
    for _ in 0..header.ancount {
        let (name, consumed) = read_domain_name(buf, offset)?;
        offset += consumed;

        if offset + 10 > buf.len() {
            return Err(DnsError::BufferTooShort);
        }

        let rtype = u16::from_be_bytes([buf[offset], buf[offset + 1]]);
        let rclass = u16::from_be_bytes([buf[offset + 2], buf[offset + 3]]);
        let ttl = u32::from_be_bytes([
            buf[offset + 4],
            buf[offset + 5],
            buf[offset + 6],
            buf[offset + 7],
        ]);
        let rdlength = u16::from_be_bytes([buf[offset + 8], buf[offset + 9]]) as usize;
        offset += 10;

        if offset + rdlength > buf.len() {
            return Err(DnsError::BufferTooShort);
        }

        let rdata = buf[offset..offset + rdlength].to_vec();
        offset += rdlength;

        let mut ip = None;
        if rtype == DNS_TYPE_A && rclass == DNS_CLASS_IN && rdlength == 4 {
            let ipv4 = [rdata[0], rdata[1], rdata[2], rdata[3]];
            ip = Some(ipv4);
            answers.push(ipv4);
        }

        let _ = DnsAnswer {
            name,
            rtype,
            rclass,
            ttl,
            rdata,
            ip,
        };
    }

    if answers.is_empty() {
        return Err(DnsError::NoAnswers);
    }

    Ok(answers)
}

/// Self-test function verifying DNS encoding, query construction, and compressed response parsing.
pub fn verify_dns_wire_protocol() -> bool {
    let domain = "knton.os";
    let encoded = match encode_domain_name(domain) {
        Ok(e) => e,
        Err(_) => return false,
    };
    let expected_encoded: [u8; 10] = [
        0x05, b'k', b'n', b't', b'o', b'n', 0x02, b'o', b's', 0x00,
    ];
    if encoded != expected_encoded {
        return false;
    }

    let query_bytes = match build_dns_query(0x1234, domain) {
        Ok(q) => q,
        Err(_) => return false,
    };
    if query_bytes.len() != DNS_HEADER_LEN + 10 + 4 {
        return false;
    }

    // Synthesize mock response with compressed pointer to offset 12 (0xC00C)
    let mut resp = Vec::new();
    let resp_hdr = DnsHeader {
        id: 0x1234,
        flags: DNS_FLAG_QR | DNS_FLAG_RD | DNS_FLAG_RA,
        qdcount: 1,
        ancount: 1,
        nscount: 0,
        arcount: 0,
    };
    resp.extend_from_slice(&resp_hdr.serialize());
    // Echo question section
    resp.extend_from_slice(&expected_encoded);
    resp.extend_from_slice(&DNS_TYPE_A.to_be_bytes());
    resp.extend_from_slice(&DNS_CLASS_IN.to_be_bytes());

    // Answer section: Name compressed pointer (0xC00C)
    resp.extend_from_slice(&[0xC0, 0x0C]);
    resp.extend_from_slice(&DNS_TYPE_A.to_be_bytes());
    resp.extend_from_slice(&DNS_CLASS_IN.to_be_bytes());
    resp.extend_from_slice(&60u32.to_be_bytes()); // TTL = 60s
    resp.extend_from_slice(&4u16.to_be_bytes());  // RDLENGTH = 4
    resp.extend_from_slice(&[192, 168, 1, 200]); // IP

    let resolved = match parse_dns_response(&resp, Some(0x1234)) {
        Ok(ips) => ips,
        Err(_) => return false,
    };

    if resolved.len() != 1 || resolved[0] != [192, 168, 1, 200] {
        return false;
    }

    true
}

