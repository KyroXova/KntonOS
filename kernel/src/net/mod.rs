//! Embedded Network Protocol Stack for KntonOS.
//!
//! Implements a zero-copy embedded network stack operating directly over VirtIO-Net:
//! - ARP (RFC 826): Dynamic cache resolution and request/reply generation
//! - IPv4 (RFC 791): 20-byte packet framing and RFC 791 one's complement checksumming
//! - ICMP (RFC 792): Ping responder (Echo Request Type 8 -> Echo Reply Type 0)
//! - UDP (RFC 768): Connectionless datagram framing and port endpoint demultiplexing

pub mod arp;
pub mod icmp;
pub mod ipv4;
pub mod udp;

pub use arp::{ArpCache, ArpPacket, ARP_OP_REPLY, ARP_OP_REQUEST};
pub use icmp::{IcmpEchoMessage, ICMP_TYPE_ECHO_REPLY, ICMP_TYPE_ECHO_REQUEST};
pub use ipv4::{compute_checksum, Ipv4Header, Ipv4Packet, IP_PROTO_ICMP, IP_PROTO_UDP};
pub use udp::{UdpDatagram, UdpHeader, UdpSocketRegistry};

use crate::drivers::virtio::net::ETH_HLEN;
use crate::memory::Spinlock;
use alloc::vec::Vec;

pub const DEFAULT_LOCAL_IP: [u8; 4] = [192, 168, 1, 100];
pub const DEFAULT_NETMASK: [u8; 4] = [255, 255, 255, 0];
pub const DEFAULT_GATEWAY: [u8; 4] = [192, 168, 1, 1];

/// Embedded Network Protocol Stack state and packet processing engine.
pub struct NetworkStack {
    pub local_ip: [u8; 4],
    pub netmask: [u8; 4],
    pub gateway: [u8; 4],
    pub local_mac: [u8; 6],
    pub arp_cache: ArpCache,
    pub udp_sockets: UdpSocketRegistry,
    pub rx_packets: u64,
    pub tx_packets: u64,
}

impl NetworkStack {
    pub const fn new() -> Self {
        Self {
            local_ip: DEFAULT_LOCAL_IP,
            netmask: DEFAULT_NETMASK,
            gateway: DEFAULT_GATEWAY,
            local_mac: [0x52, 0x54, 0x00, 0x12, 0x34, 0x56],
            arp_cache: ArpCache::new(64),
            udp_sockets: UdpSocketRegistry::new(),
            rx_packets: 0,
            tx_packets: 0,
        }
    }

    /// Ingests and processes a raw Ethernet frame received from the network interface.
    /// If the frame requires an immediate protocol response (e.g. ARP Reply, ICMP Ping Reply),
    /// the serialized outgoing Ethernet frame is returned.
    pub fn process_frame(&mut self, frame: &[u8]) -> Option<Vec<u8>> {
        if frame.len() < ETH_HLEN {
            return None;
        }

        self.rx_packets += 1;

        let ethertype = u16::from_be_bytes([frame[12], frame[13]]);
        let payload = &frame[ETH_HLEN..];

        match ethertype {
            // --- ARP (0x0806) ---
            0x0806 => {
                if let Ok(arp) = ArpPacket::parse(payload) {
                    // Learn sender mapping into dynamic ARP cache
                    self.arp_cache.insert(arp.sender_ip, arp.sender_mac);

                    // If request is for our local IP, construct ARP reply
                    if arp.oper == ARP_OP_REQUEST && arp.target_ip == self.local_ip {
                        let reply = ArpPacket::build_reply(
                            self.local_mac,
                            self.local_ip,
                            arp.sender_mac,
                            arp.sender_ip,
                        );

                        // Build Ethernet frame
                        let mut out = Vec::with_capacity(ETH_HLEN + arp::ARP_PACKET_LEN);
                        out.extend_from_slice(&arp.sender_mac); // Destination
                        out.extend_from_slice(&self.local_mac); // Source
                        out.extend_from_slice(&[0x08, 0x06]);   // EtherType ARP
                        out.extend_from_slice(&reply.serialize());
                        self.tx_packets += 1;
                        return Some(out);
                    }
                }
            }

            // --- IPv4 (0x0800) ---
            0x0800 => {
                if let Ok(ip_packet) = Ipv4Packet::parse(payload) {
                    // Verify packet is destined for our IP or broadcast
                    if ip_packet.header.dst_ip == self.local_ip
                        || ip_packet.header.dst_ip == [255, 255, 255, 255]
                    {
                        match ip_packet.header.protocol {
                            // ICMP Protocol (1)
                            IP_PROTO_ICMP => {
                                if let Ok(icmp_msg) = IcmpEchoMessage::parse(&ip_packet.payload) {
                                    if icmp_msg.msg_type == ICMP_TYPE_ECHO_REQUEST {
                                        // Build Echo Reply
                                        let reply_msg = IcmpEchoMessage::build_echo_reply(&icmp_msg);
                                        let reply_payload = reply_msg.serialize();

                                        // Wrap in IPv4 packet
                                        let ip_hdr = Ipv4Header::new(
                                            self.local_ip,
                                            ip_packet.header.src_ip,
                                            IP_PROTO_ICMP,
                                            reply_payload.len(),
                                        );

                                        let mut out = Vec::with_capacity(
                                            ETH_HLEN + ipv4::IPV4_HEADER_MIN_LEN + reply_payload.len(),
                                        );
                                        // Destination MAC (from incoming frame source)
                                        out.extend_from_slice(&frame[6..12]);
                                        out.extend_from_slice(&self.local_mac);
                                        out.extend_from_slice(&[0x08, 0x00]); // EtherType IPv4
                                        out.extend_from_slice(&ip_hdr.serialize());
                                        out.extend_from_slice(&reply_payload);
                                        self.tx_packets += 1;
                                        return Some(out);
                                    }
                                }
                            }

                            // UDP Protocol (17)
                            IP_PROTO_UDP => {
                                if let Ok(dgram) = UdpDatagram::parse(
                                    ip_packet.header.src_ip,
                                    ip_packet.header.dst_ip,
                                    &ip_packet.payload,
                                ) {
                                    self.udp_sockets.dispatch(dgram);
                                }
                            }

                            _ => {}
                        }
                    }
                }
            }

            _ => {}
        }

        None
    }
}

/// Global Network Protocol Stack singleton.
pub static NET_STACK: Spinlock<NetworkStack> = Spinlock::new(NetworkStack::new());

/// Initializes the embedded network stack.
pub fn init() {
    crate::serial_write_str("[KNT-NET-INIT] Initializing embedded network protocol stack (ARP, IPv4, ICMP, UDP)...\r\n");
    crate::serial_write_str("[KNT-NET] Local IP configured: 192.168.1.100 / Netmask: 255.255.255.0\r\n");
}

/// Executes comprehensive self-test suite for ARP, IPv4, ICMP Ping responder, and UDP sockets.
pub fn test_network_stack() -> bool {
    crate::serial_write_str("[KNT-NET-TEST] Executing network protocol stack self-test suite...\r\n");

    // --- TEST 1: ARP Resolution & Dynamic Cache ---
    {
        let mut stack = NET_STACK.lock();

        // 1.1 Synthesize incoming ARP Request: "Who has 192.168.1.100? Tell 192.168.1.1"
        let gateway_mac = [0x00, 0x50, 0x56, 0xAA, 0xBB, 0xCC];
        let gateway_ip = [192, 168, 1, 1];
        let req = ArpPacket::build_request(gateway_mac, gateway_ip, stack.local_ip);

        let mut eth_frame = Vec::new();
        eth_frame.extend_from_slice(&[0xFF; 6]); // Broadcast destination
        eth_frame.extend_from_slice(&gateway_mac);
        eth_frame.extend_from_slice(&[0x08, 0x06]); // EtherType ARP
        eth_frame.extend_from_slice(&req.serialize());

        let reply_opt = stack.process_frame(&eth_frame);
        let reply_frame = match reply_opt {
            Some(f) => f,
            None => {
                crate::serial_write_str("[KNT-PANIC] ARP request did not generate reply!\r\n");
                return false;
            }
        };

        // Verify ARP Reply parameters
        let reply_arp = ArpPacket::parse(&reply_frame[ETH_HLEN..]).expect("Parse ARP reply failed");
        if reply_arp.oper != ARP_OP_REPLY
            || reply_arp.sender_mac != stack.local_mac
            || reply_arp.sender_ip != stack.local_ip
            || reply_arp.target_mac != gateway_mac
            || reply_arp.target_ip != gateway_ip
        {
            crate::serial_write_str("[KNT-PANIC] ARP reply fields mismatch!\r\n");
            return false;
        }

        // Verify ARP cache learning
        if stack.arp_cache.lookup(&gateway_ip) != Some(gateway_mac) {
            crate::serial_write_str("[KNT-PANIC] Sender IP was not learned into ARP cache!\r\n");
            return false;
        }

        crate::serial_write_str("[KNT-NET-ARP-PASS] ARP request parsing, reply generation, and cache learning verified\r\n");
    }

    // --- TEST 2: IPv4 ICMP Ping Echo Responder ---
    {
        let mut stack = NET_STACK.lock();

        // 2.1 Build ICMP Echo Request
        let ping_payload = b"KNTON-KERNEL-PING-PROBE-01";
        let echo_req = IcmpEchoMessage::build_echo_request(0x0400, 1, ping_payload);
        let echo_req_bytes = echo_req.serialize();

        // Wrap in IPv4 packet
        let client_ip = [192, 168, 1, 50];
        let client_mac = [0x02, 0x11, 0x22, 0x33, 0x44, 0x55];
        let ip_hdr = Ipv4Header::new(client_ip, stack.local_ip, IP_PROTO_ICMP, echo_req_bytes.len());

        let mut eth_frame = Vec::new();
        eth_frame.extend_from_slice(&stack.local_mac);
        eth_frame.extend_from_slice(&client_mac);
        eth_frame.extend_from_slice(&[0x08, 0x00]); // EtherType IPv4
        eth_frame.extend_from_slice(&ip_hdr.serialize());
        eth_frame.extend_from_slice(&echo_req_bytes);

        // Process incoming ping
        let reply_opt = stack.process_frame(&eth_frame);
        let reply_frame = match reply_opt {
            Some(f) => f,
            None => {
                crate::serial_write_str("[KNT-PANIC] ICMP Echo Request did not produce Echo Reply!\r\n");
                return false;
            }
        };

        // Parse returned IPv4 packet and ICMP payload
        let reply_ip = Ipv4Packet::parse(&reply_frame[ETH_HLEN..]).expect("Parse reply IP failed");
        if reply_ip.header.src_ip != stack.local_ip || reply_ip.header.dst_ip != client_ip {
            crate::serial_write_str("[KNT-PANIC] Ping reply IP endpoints incorrect!\r\n");
            return false;
        }

        let reply_icmp = IcmpEchoMessage::parse(&reply_ip.payload).expect("Parse reply ICMP failed");
        if reply_icmp.msg_type != ICMP_TYPE_ECHO_REPLY
            || reply_icmp.identifier != 0x0400
            || reply_icmp.sequence_number != 1
            || reply_icmp.payload != ping_payload
        {
            crate::serial_write_str("[KNT-PANIC] ICMP Echo Reply payload or metadata corrupted!\r\n");
            return false;
        }

        crate::serial_write_str("[KNT-NET-ICMP-PASS] IPv4 ICMP Echo Request ping response and checksum verified\r\n");
    }

    // --- TEST 3: UDP Socket Binding & Port Demultiplexing ---
    {
        let mut stack = NET_STACK.lock();

        // 3.1 Bind port 8080
        if stack.udp_sockets.bind(8080).is_err() {
            crate::serial_write_str("[KNT-PANIC] Failed to bind UDP port 8080!\r\n");
            return false;
        }

        // 3.2 Synthesize incoming UDP Datagram for port 8080
        let udp_data = b"KNTON-UDP-RPC-TEST-DATA";
        let udp_hdr = UdpHeader::new(5000, 8080, udp_data.len());
        let mut udp_packet = Vec::new();
        udp_packet.extend_from_slice(&udp_hdr.serialize());
        udp_packet.extend_from_slice(udp_data);

        let client_ip = [192, 168, 1, 60];
        let ip_hdr = Ipv4Header::new(client_ip, stack.local_ip, IP_PROTO_UDP, udp_packet.len());

        let mut eth_frame = Vec::new();
        eth_frame.extend_from_slice(&stack.local_mac);
        eth_frame.extend_from_slice(&[0x02, 0x00, 0x00, 0x00, 0x00, 0x01]);
        eth_frame.extend_from_slice(&[0x08, 0x00]);
        eth_frame.extend_from_slice(&ip_hdr.serialize());
        eth_frame.extend_from_slice(&udp_packet);

        stack.process_frame(&eth_frame);

        // 3.3 Poll bound socket queue
        let received_opt = stack.udp_sockets.poll(8080);
        let received_dgram = match received_opt {
            Some(d) => d,
            None => {
                crate::serial_write_str("[KNT-PANIC] Bound UDP socket did not receive datagram!\r\n");
                return false;
            }
        };

        if received_dgram.src_port != 5000
            || received_dgram.dst_port != 8080
            || received_dgram.payload != udp_data
        {
            crate::serial_write_str("[KNT-PANIC] UDP datagram corrupted upon socket reception!\r\n");
            return false;
        }

        crate::serial_write_str("[KNT-NET-UDP-PASS] UDP port binding, datagram demultiplexing, and socket delivery verified\r\n");
    }

    crate::serial_write_str("[KNT-NET-TESTS-PASS] All embedded network protocol stack checks passed\r\n");
    true
}
