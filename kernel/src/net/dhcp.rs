//! Dynamic Host Configuration Protocol (DHCPv4, RFC 2131) Packet Format & Option TLVs.
//!
//! Implements standard RFC 2131 wire framing:
//! - 240-byte base BOOTP/DHCP message layout with magic cookie (`0x63825363`)
//! - Option TLV encoding and decoding (Option 53 Message Type, Option 1 Subnet Mask,
//!   Option 3 Router, Option 6 DNS Servers, Option 51 Lease Time, Option 54 Server Identifier)
//! - Client hardware address (chaddr) MAC formatting and broadcast flag handling

use alloc::vec::Vec;

pub const DHCP_MIN_PACKET_LEN: usize = 240;
pub const DHCP_MAGIC_COOKIE: [u8; 4] = [0x63, 0x82, 0x53, 0x63];

pub const BOOTREQUEST: u8 = 1;
pub const BOOTREPLY: u8 = 2;

pub const HTYPE_ETHERNET: u8 = 1;
pub const HLEN_ETHERNET: u8 = 6;
pub const DHCP_FLAG_BROADCAST: u16 = 0x8000;

// Standard DHCP Option Tags (RFC 2132)
pub const DHCP_OPT_PAD: u8 = 0;
pub const DHCP_OPT_SUBNET_MASK: u8 = 1;
pub const DHCP_OPT_ROUTER: u8 = 3;
pub const DHCP_OPT_DNS_SERVER: u8 = 6;
pub const DHCP_OPT_REQUESTED_IP: u8 = 50;
pub const DHCP_OPT_LEASE_TIME: u8 = 51;
pub const DHCP_OPT_MESSAGE_TYPE: u8 = 53;
pub const DHCP_OPT_SERVER_ID: u8 = 54;
pub const DHCP_OPT_PARAM_REQUEST_LIST: u8 = 55;
pub const DHCP_OPT_END: u8 = 255;

// DHCP Message Types (Option 53)
pub const DHCPDISCOVER: u8 = 1;
pub const DHCPOFFER: u8 = 2;
pub const DHCPREQUEST: u8 = 3;
pub const DHCPDECLINE: u8 = 4;
pub const DHCPACK: u8 = 5;
pub const DHCPNAK: u8 = 6;
pub const DHCPRELEASE: u8 = 7;
pub const DHCPINFORM: u8 = 8;

// Standard UDP Ports
pub const DHCP_SERVER_PORT: u16 = 67;
pub const DHCP_CLIENT_PORT: u16 = 68;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DhcpError {
    BufferTooShort,
    InvalidMagicCookie,
    MalformedOption,
    UnexpectedMessageType,
    LeaseExpired,
}

/// DHCP Option TLV (Tag, Length, Value).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DhcpOption {
    pub tag: u8,
    pub data: Vec<u8>,
}

impl DhcpOption {
    pub fn new(tag: u8, data: &[u8]) -> Self {
        Self {
            tag,
            data: data.to_vec(),
        }
    }
}

/// Standard 240-byte RFC 2131 BOOTP/DHCP message with options.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DhcpPacket {
    pub op: u8,
    pub htype: u8,
    pub hlen: u8,
    pub hops: u8,
    pub xid: u32,
    pub secs: u16,
    pub flags: u16,
    pub ciaddr: [u8; 4],
    pub yiaddr: [u8; 4],
    pub siaddr: [u8; 4],
    pub giaddr: [u8; 4],
    pub chaddr: [u8; 16],
    pub sname: [u8; 64],
    pub file: [u8; 128],
    pub options: Vec<DhcpOption>,
}

impl DhcpPacket {
    /// Creates a base client request packet pre-filled with client MAC and broadcast flag.
    pub fn new_client_packet(xid: u32, client_mac: [u8; 6]) -> Self {
        let mut chaddr = [0u8; 16];
        chaddr[..6].copy_from_slice(&client_mac);

        Self {
            op: BOOTREQUEST,
            htype: HTYPE_ETHERNET,
            hlen: HLEN_ETHERNET,
            hops: 0,
            xid,
            secs: 0,
            flags: DHCP_FLAG_BROADCAST,
            ciaddr: [0; 4],
            yiaddr: [0; 4],
            siaddr: [0; 4],
            giaddr: [0; 4],
            chaddr,
            sname: [0; 64],
            file: [0; 128],
            options: Vec::new(),
        }
    }

    /// Adds a DHCP option TLV.
    pub fn add_option(&mut self, tag: u8, data: &[u8]) {
        self.options.push(DhcpOption::new(tag, data));
    }

    /// Serializes the DHCP packet to wire format bytes.
    pub fn serialize(&self) -> Vec<u8> {
        let mut buf = Vec::with_capacity(300);

        buf.push(self.op);
        buf.push(self.htype);
        buf.push(self.hlen);
        buf.push(self.hops);
        buf.extend_from_slice(&self.xid.to_be_bytes());
        buf.extend_from_slice(&self.secs.to_be_bytes());
        buf.extend_from_slice(&self.flags.to_be_bytes());
        buf.extend_from_slice(&self.ciaddr);
        buf.extend_from_slice(&self.yiaddr);
        buf.extend_from_slice(&self.siaddr);
        buf.extend_from_slice(&self.giaddr);
        buf.extend_from_slice(&self.chaddr);
        buf.extend_from_slice(&self.sname);
        buf.extend_from_slice(&self.file);
        buf.extend_from_slice(&DHCP_MAGIC_COOKIE);

        for opt in &self.options {
            buf.push(opt.tag);
            buf.push(opt.data.len() as u8);
            buf.extend_from_slice(&opt.data);
        }

        buf.push(DHCP_OPT_END);

        // Pad to minimum 300 bytes if needed for legacy BOOTP alignment
        while buf.len() < 300 {
            buf.push(DHCP_OPT_PAD);
        }

        buf
    }

    /// Parses an incoming DHCP packet from raw bytes.
    pub fn parse(buf: &[u8]) -> Result<Self, DhcpError> {
        if buf.len() < DHCP_MIN_PACKET_LEN {
            return Err(DhcpError::BufferTooShort);
        }

        if buf[236..240] != DHCP_MAGIC_COOKIE {
            return Err(DhcpError::InvalidMagicCookie);
        }

        let op = buf[0];
        let htype = buf[1];
        let hlen = buf[2];
        let hops = buf[3];
        let xid = u32::from_be_bytes([buf[4], buf[5], buf[6], buf[7]]);
        let secs = u16::from_be_bytes([buf[8], buf[9]]);
        let flags = u16::from_be_bytes([buf[10], buf[11]]);

        let mut ciaddr = [0u8; 4];
        ciaddr.copy_from_slice(&buf[12..16]);

        let mut yiaddr = [0u8; 4];
        yiaddr.copy_from_slice(&buf[16..20]);

        let mut siaddr = [0u8; 4];
        siaddr.copy_from_slice(&buf[20..24]);

        let mut giaddr = [0u8; 4];
        giaddr.copy_from_slice(&buf[24..28]);

        let mut chaddr = [0u8; 16];
        chaddr.copy_from_slice(&buf[28..44]);

        let mut sname = [0u8; 64];
        sname.copy_from_slice(&buf[44..108]);

        let mut file = [0u8; 128];
        file.copy_from_slice(&buf[108..236]);

        let mut options = Vec::new();
        let mut offset = 240;

        while offset < buf.len() {
            let tag = buf[offset];
            offset += 1;

            if tag == DHCP_OPT_END {
                break;
            }
            if tag == DHCP_OPT_PAD {
                continue;
            }

            if offset >= buf.len() {
                return Err(DhcpError::MalformedOption);
            }

            let len = buf[offset] as usize;
            offset += 1;

            if offset + len > buf.len() {
                return Err(DhcpError::BufferTooShort);
            }

            let data = buf[offset..offset + len].to_vec();
            offset += len;

            options.push(DhcpOption { tag, data });
        }

        Ok(Self {
            op,
            htype,
            hlen,
            hops,
            xid,
            secs,
            flags,
            ciaddr,
            yiaddr,
            siaddr,
            giaddr,
            chaddr,
            sname,
            file,
            options,
        })
    }

    /// Finds a specific DHCP option by tag.
    pub fn get_option(&self, tag: u8) -> Option<&[u8]> {
        for opt in &self.options {
            if opt.tag == tag {
                return Some(&opt.data);
            }
        }
        None
    }

    /// Retrieves the DHCP message type (Option 53).
    pub fn message_type(&self) -> Option<u8> {
        self.get_option(DHCP_OPT_MESSAGE_TYPE).and_then(|d| d.first().copied())
    }

    /// Retrieves the Subnet Mask (Option 1).
    pub fn subnet_mask(&self) -> Option<[u8; 4]> {
        self.get_option(DHCP_OPT_SUBNET_MASK).and_then(|d| {
            if d.len() == 4 {
                Some([d[0], d[1], d[2], d[3]])
            } else {
                None
            }
        })
    }

    /// Retrieves the Default Router/Gateway (Option 3).
    pub fn router(&self) -> Option<[u8; 4]> {
        self.get_option(DHCP_OPT_ROUTER).and_then(|d| {
            if d.len() >= 4 {
                Some([d[0], d[1], d[2], d[3]])
            } else {
                None
            }
        })
    }

    /// Retrieves DNS Server addresses (Option 6).
    pub fn dns_servers(&self) -> Vec<[u8; 4]> {
        let mut servers = Vec::new();
        if let Some(d) = self.get_option(DHCP_OPT_DNS_SERVER) {
            for chunk in d.chunks_exact(4) {
                servers.push([chunk[0], chunk[1], chunk[2], chunk[3]]);
            }
        }
        servers
    }

    /// Retrieves IP Address Lease Time in seconds (Option 51).
    pub fn lease_time(&self) -> Option<u32> {
        self.get_option(DHCP_OPT_LEASE_TIME).and_then(|d| {
            if d.len() == 4 {
                Some(u32::from_be_bytes([d[0], d[1], d[2], d[3]]))
            } else {
                None
            }
        })
    }

    /// Retrieves Server Identifier (Option 54).
    pub fn server_id(&self) -> Option<[u8; 4]> {
        self.get_option(DHCP_OPT_SERVER_ID).and_then(|d| {
            if d.len() == 4 {
                Some([d[0], d[1], d[2], d[3]])
            } else {
                None
            }
        })
    }
}

/// DHCP Client State Machine (RFC 2131 Section 4.4).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DhcpState {
    Init,
    Selecting,
    Requesting,
    Bound,
}

/// Acquired DHCP Network Lease Parameters.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DhcpLease {
    pub ip: [u8; 4],
    pub netmask: [u8; 4],
    pub router: [u8; 4],
    pub dns_servers: Vec<[u8; 4]>,
    pub server_id: [u8; 4],
    pub lease_time: u32,
}

/// DHCP IPv4 Client State Machine & Dispatch Engine.
#[derive(Debug, Clone)]
pub struct DhcpClient {
    pub state: DhcpState,
    pub client_mac: [u8; 6],
    pub xid: u32,
    pub offered_ip: Option<[u8; 4]>,
    pub server_id: Option<[u8; 4]>,
    pub lease: Option<DhcpLease>,
}

impl DhcpClient {
    /// Creates a new DHCP client bound to the specified Ethernet MAC address in Init state.
    pub fn new(client_mac: [u8; 6]) -> Self {
        // Deterministic transaction ID derived from MAC
        let xid = 0x3903F326 ^ (client_mac[4] as u32) | ((client_mac[5] as u32) << 8);
        Self {
            state: DhcpState::Init,
            client_mac,
            xid,
            offered_ip: None,
            server_id: None,
            lease: None,
        }
    }

    /// Transitions to Selecting and builds an outgoing broadcast DHCPDISCOVER packet.
    pub fn build_discover(&mut self) -> DhcpPacket {
        self.state = DhcpState::Selecting;

        let mut pkt = DhcpPacket::new_client_packet(self.xid, self.client_mac);
        pkt.add_option(DHCP_OPT_MESSAGE_TYPE, &[DHCPDISCOVER]);
        pkt.add_option(DHCP_OPT_PARAM_REQUEST_LIST, &[
            DHCP_OPT_SUBNET_MASK,
            DHCP_OPT_ROUTER,
            DHCP_OPT_DNS_SERVER,
            DHCP_OPT_LEASE_TIME,
        ]);
        pkt
    }

    /// Handles an incoming DHCPOFFER packet and produces a DHCPREQUEST packet.
    pub fn handle_offer(&mut self, offer: &DhcpPacket) -> Result<DhcpPacket, DhcpError> {
        if self.state != DhcpState::Selecting {
            return Err(DhcpError::UnexpectedMessageType);
        }
        if offer.xid != self.xid || offer.op != BOOTREPLY {
            return Err(DhcpError::UnexpectedMessageType);
        }
        if offer.message_type() != Some(DHCPOFFER) {
            return Err(DhcpError::UnexpectedMessageType);
        }

        let offered_ip = offer.yiaddr;
        let server_id = offer.server_id().unwrap_or(offer.siaddr);

        self.offered_ip = Some(offered_ip);
        self.server_id = Some(server_id);
        self.state = DhcpState::Requesting;

        let mut req = DhcpPacket::new_client_packet(self.xid, self.client_mac);
        req.add_option(DHCP_OPT_MESSAGE_TYPE, &[DHCPREQUEST]);
        req.add_option(DHCP_OPT_SERVER_ID, &server_id);
        req.add_option(DHCP_OPT_REQUESTED_IP, &offered_ip);
        req.add_option(DHCP_OPT_PARAM_REQUEST_LIST, &[
            DHCP_OPT_SUBNET_MASK,
            DHCP_OPT_ROUTER,
            DHCP_OPT_DNS_SERVER,
            DHCP_OPT_LEASE_TIME,
        ]);

        Ok(req)
    }

    /// Handles an incoming DHCPACK packet, transitions to Bound, and stores the lease parameters.
    pub fn handle_ack(&mut self, ack: &DhcpPacket) -> Result<DhcpLease, DhcpError> {
        if self.state != DhcpState::Requesting {
            return Err(DhcpError::UnexpectedMessageType);
        }
        if ack.xid != self.xid || ack.op != BOOTREPLY {
            return Err(DhcpError::UnexpectedMessageType);
        }
        if ack.message_type() != Some(DHCPACK) {
            return Err(DhcpError::UnexpectedMessageType);
        }

        let ip = if ack.yiaddr != [0, 0, 0, 0] {
            ack.yiaddr
        } else {
            self.offered_ip.ok_or(DhcpError::UnexpectedMessageType)?
        };

        let netmask = ack.subnet_mask().unwrap_or([255, 255, 255, 0]);
        let router = ack.router().unwrap_or([ip[0], ip[1], ip[2], 1]);
        let mut dns_servers = ack.dns_servers();
        if dns_servers.is_empty() {
            dns_servers.push(router);
        }

        let server_id = ack.server_id().unwrap_or(ack.siaddr);
        let lease_time = ack.lease_time().unwrap_or(86400);

        let lease = DhcpLease {
            ip,
            netmask,
            router,
            dns_servers,
            server_id,
            lease_time,
        };

        self.lease = Some(lease.clone());
        self.state = DhcpState::Bound;

        Ok(lease)
    }
}

/// Self-test function verifying DHCP state transitions and lease acquisition.
pub fn verify_dhcp_state_machine() -> bool {
    let mac = [0x52, 0x54, 0x00, 0x12, 0x34, 0x56];
    let mut client = DhcpClient::new(mac);

    if client.state != DhcpState::Init {
        return false;
    }

    let discover = client.build_discover();
    if client.state != DhcpState::Selecting || discover.message_type() != Some(DHCPDISCOVER) {
        return false;
    }

    // Synthesize mock DHCPOFFER
    let mut offer = DhcpPacket::new_client_packet(client.xid, mac);
    offer.op = BOOTREPLY;
    offer.yiaddr = [192, 168, 1, 150];
    offer.siaddr = [192, 168, 1, 1];
    offer.add_option(DHCP_OPT_MESSAGE_TYPE, &[DHCPOFFER]);
    offer.add_option(DHCP_OPT_SERVER_ID, &[192, 168, 1, 1]);
    offer.add_option(DHCP_OPT_SUBNET_MASK, &[255, 255, 255, 0]);
    offer.add_option(DHCP_OPT_ROUTER, &[192, 168, 1, 1]);
    offer.add_option(DHCP_OPT_DNS_SERVER, &[192, 168, 1, 1]);
    offer.add_option(DHCP_OPT_LEASE_TIME, &3600u32.to_be_bytes());

    let req = match client.handle_offer(&offer) {
        Ok(r) => r,
        Err(_) => return false,
    };

    if client.state != DhcpState::Requesting || req.message_type() != Some(DHCPREQUEST) {
        return false;
    }

    // Synthesize mock DHCPACK
    let mut ack = DhcpPacket::new_client_packet(client.xid, mac);
    ack.op = BOOTREPLY;
    ack.yiaddr = [192, 168, 1, 150];
    ack.siaddr = [192, 168, 1, 1];
    ack.add_option(DHCP_OPT_MESSAGE_TYPE, &[DHCPACK]);
    ack.add_option(DHCP_OPT_SERVER_ID, &[192, 168, 1, 1]);
    ack.add_option(DHCP_OPT_SUBNET_MASK, &[255, 255, 255, 0]);
    ack.add_option(DHCP_OPT_ROUTER, &[192, 168, 1, 1]);
    ack.add_option(DHCP_OPT_DNS_SERVER, &[192, 168, 1, 1]);
    ack.add_option(DHCP_OPT_LEASE_TIME, &3600u32.to_be_bytes());

    let lease = match client.handle_ack(&ack) {
        Ok(l) => l,
        Err(_) => return false,
    };

    if client.state != DhcpState::Bound
        || lease.ip != [192, 168, 1, 150]
        || lease.netmask != [255, 255, 255, 0]
        || lease.router != [192, 168, 1, 1]
    {
        return false;
    }

    true
}

/// Self-test function verifying DHCP packet serialization, option TLVs, and parsing.
pub fn verify_dhcp_wire_protocol() -> bool {
    let mac = [0x52, 0x54, 0x00, 0x12, 0x34, 0x56];
    let xid = 0x3903F326;

    let mut packet = DhcpPacket::new_client_packet(xid, mac);
    packet.add_option(DHCP_OPT_MESSAGE_TYPE, &[DHCPDISCOVER]);
    packet.add_option(DHCP_OPT_PARAM_REQUEST_LIST, &[
        DHCP_OPT_SUBNET_MASK,
        DHCP_OPT_ROUTER,
        DHCP_OPT_DNS_SERVER,
        DHCP_OPT_LEASE_TIME,
    ]);

    let serialized = packet.serialize();
    if serialized.len() < 300 {
        return false;
    }

    let parsed = match DhcpPacket::parse(&serialized) {
        Ok(p) => p,
        Err(_) => return false,
    };

    if parsed.op != BOOTREQUEST
        || parsed.xid != xid
        || parsed.chaddr[..6] != mac
        || parsed.message_type() != Some(DHCPDISCOVER)
    {
        return false;
    }

    true
}
