//! Address Resolution Protocol (ARP, RFC 826) Framing and Cache Table.
//! Provides Ethernet-to-IPv4 address resolution, request/reply generation,

use alloc::vec::Vec;

pub const ARP_HTYPE_ETHERNET: u16 = 0x0001;
pub const ARP_PTYPE_IPV4: u16 = 0x0800;
pub const ARP_HLEN_ETHERNET: u8 = 6;
pub const ARP_PLEN_IPV4: u8 = 4;

pub const ARP_OP_REQUEST: u16 = 1;
pub const ARP_OP_REPLY: u16 = 2;

pub const ARP_PACKET_LEN: usize = 28;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ArpError {
    BufferTooShort,
    UnsupportedHardwareType,
    UnsupportedProtocolType,
    InvalidHardwareLength,
    InvalidProtocolLength,
    UnknownOperation,
}

/// Parsed RFC 826 ARP packet.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ArpPacket {
    pub htype: u16,
    pub ptype: u16,
    pub hlen: u8,
    pub plen: u8,
    pub oper: u16,
    pub sender_mac: [u8; 6],
    pub sender_ip: [u8; 4],
    pub target_mac: [u8; 6],
    pub target_ip: [u8; 4],
}

impl ArpPacket {
    /// Parses an ARP packet from a raw byte slice.
    pub fn parse(buf: &[u8]) -> Result<Self, ArpError> {
        if buf.len() < ARP_PACKET_LEN {
            return Err(ArpError::BufferTooShort);
        }

        let htype = u16::from_be_bytes([buf[0], buf[1]]);
        let ptype = u16::from_be_bytes([buf[2], buf[3]]);
        let hlen = buf[4];
        let plen = buf[5];
        let oper = u16::from_be_bytes([buf[6], buf[7]]);

        if htype != ARP_HTYPE_ETHERNET {
            return Err(ArpError::UnsupportedHardwareType);
        }
        if ptype != ARP_PTYPE_IPV4 {
            return Err(ArpError::UnsupportedProtocolType);
        }
        if hlen != ARP_HLEN_ETHERNET {
            return Err(ArpError::InvalidHardwareLength);
        }
        if plen != ARP_PLEN_IPV4 {
            return Err(ArpError::InvalidProtocolLength);
        }

        let mut sender_mac = [0u8; 6];
        sender_mac.copy_from_slice(&buf[8..14]);

        let mut sender_ip = [0u8; 4];
        sender_ip.copy_from_slice(&buf[14..18]);

        let mut target_mac = [0u8; 6];
        target_mac.copy_from_slice(&buf[18..24]);

        let mut target_ip = [0u8; 4];
        target_ip.copy_from_slice(&buf[24..28]);

        Ok(Self {
            htype,
            ptype,
            hlen,
            plen,
            oper,
            sender_mac,
            sender_ip,
            target_mac,
            target_ip,
        })
    }

    /// Serializes the ARP packet into a 28-byte raw buffer.
    pub fn serialize(&self) -> [u8; ARP_PACKET_LEN] {
        let mut buf = [0u8; ARP_PACKET_LEN];
        buf[0..2].copy_from_slice(&self.htype.to_be_bytes());
        buf[2..4].copy_from_slice(&self.ptype.to_be_bytes());
        buf[4] = self.hlen;
        buf[5] = self.plen;
        buf[6..8].copy_from_slice(&self.oper.to_be_bytes());
        buf[8..14].copy_from_slice(&self.sender_mac);
        buf[14..18].copy_from_slice(&self.sender_ip);
        buf[18..24].copy_from_slice(&self.target_mac);
        buf[24..28].copy_from_slice(&self.target_ip);
        buf
    }

    /// Builds an ARP Request ("Who has target_ip? Tell sender_ip").
    pub fn build_request(sender_mac: [u8; 6], sender_ip: [u8; 4], target_ip: [u8; 4]) -> Self {
        Self {
            htype: ARP_HTYPE_ETHERNET,
            ptype: ARP_PTYPE_IPV4,
            hlen: ARP_HLEN_ETHERNET,
            plen: ARP_PLEN_IPV4,
            oper: ARP_OP_REQUEST,
            sender_mac,
            sender_ip,
            target_mac: [0x00; 6],
            target_ip,
        }
    }

    /// Builds an ARP Reply ("target_ip is at sender_mac").
    pub fn build_reply(
        sender_mac: [u8; 6],
        sender_ip: [u8; 4],
        target_mac: [u8; 6],
        target_ip: [u8; 4],
    ) -> Self {
        Self {
            htype: ARP_HTYPE_ETHERNET,
            ptype: ARP_PTYPE_IPV4,
            hlen: ARP_HLEN_ETHERNET,
            plen: ARP_PLEN_IPV4,
            oper: ARP_OP_REPLY,
            sender_mac,
            sender_ip,
            target_mac,
            target_ip,
        }
    }
}

/// Dynamic ARP Cache Table mapping IPv4 addresses to MAC hardware addresses.
pub struct ArpCache {
    entries: Vec<([u8; 4], [u8; 6])>,
    max_entries: usize,
}

impl ArpCache {
    pub const fn new(max_entries: usize) -> Self {
        Self {
            entries: Vec::new(),
            max_entries,
        }
    }

    /// Inserts or updates an IP-to-MAC mapping in the ARP cache.
    pub fn insert(&mut self, ip: [u8; 4], mac: [u8; 6]) {
        for entry in self.entries.iter_mut() {
            if entry.0 == ip {
                entry.1 = mac;
                return;
            }
        }

        if self.entries.len() >= self.max_entries {
            self.entries.remove(0); // Evict oldest entry
        }
        self.entries.push((ip, mac));
    }

    /// Looks up a hardware MAC address for a given IPv4 destination.
    pub fn lookup(&self, ip: &[u8; 4]) -> Option<[u8; 6]> {
        for entry in &self.entries {
            if &entry.0 == ip {
                return Some(entry.1);
            }
        }
        None
    }

    /// Returns the number of active entries in the ARP cache.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}
