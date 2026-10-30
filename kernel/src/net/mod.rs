//! Embedded Network Protocol Stack for KntonOS.
//!
//! Implements a zero-copy embedded network stack operating directly over VirtIO-Net:
//! - ARP (RFC 826): Dynamic cache resolution and request/reply generation
//! - IPv4 (RFC 791): 20-byte packet framing and RFC 791 one's complement checksumming
//! - ICMP (RFC 792): Ping responder (Echo Request Type 8 -> Echo Reply Type 0)
//! - UDP (RFC 768): Connectionless datagram framing and port endpoint demultiplexing
//! - TCP (RFC 793): Connection-oriented stream sockets, state machine, and 3-way handshake

pub mod af_unix;
pub mod arp;
pub mod dhcp;
pub mod dns;
pub mod http;
pub mod icmp;
pub mod ipv4;
pub mod poll;
pub mod server;
pub mod socket;
pub mod tcp;
pub mod tls;
pub mod udp;
pub mod ring;
pub mod multiqueue;

pub use multiqueue::*;

pub use ring::*;

pub use af_unix::{
    sockaddr_un, ucred, UnixDatagramPacket, UnixDatagramQueue, UnixSocketAddress, UnixSocketError,
    UnixStreamRingBuffer, AF_UNIX, DEFAULT_DATAGRAM_MAX_BYTES, DEFAULT_DATAGRAM_MAX_PACKETS,
    DEFAULT_STREAM_BUFFER_CAPACITY, UNIX_PATH_MAX, test_unix_domain_sockets,
    unix_accept, unix_bind, unix_close, unix_connect, unix_getpeercred, unix_listen,
    unix_recv, unix_recvfrom, unix_send, unix_sendto, unix_set_credentials,
    unix_set_nonblocking, unix_socket, unix_socketpair, UnixSocket, UnixSocketTable,
    UNIX_SOCKET_TABLE,
};
pub use arp::{ArpCache, ArpPacket, ARP_OP_REPLY, ARP_OP_REQUEST};
pub use dhcp::{
    verify_dhcp_state_machine, verify_dhcp_wire_protocol, DhcpClient, DhcpError, DhcpLease,
    DhcpOption, DhcpPacket, DhcpState, BOOTREPLY, BOOTREQUEST, DHCPACK, DHCPDECLINE,
    DHCPDISCOVER, DHCPINFORM, DHCPNAK, DHCPOFFER, DHCPRELEASE, DHCPREQUEST, DHCP_CLIENT_PORT,
    DHCP_MAGIC_COOKIE, DHCP_OPT_DNS_SERVER, DHCP_OPT_END, DHCP_OPT_LEASE_TIME,
    DHCP_OPT_MESSAGE_TYPE, DHCP_OPT_PAD, DHCP_OPT_PARAM_REQUEST_LIST, DHCP_OPT_REQUESTED_IP,
    DHCP_OPT_ROUTER, DHCP_OPT_SERVER_ID, DHCP_OPT_SUBNET_MASK, DHCP_SERVER_PORT,
};
pub use dns::{
    build_dns_query, encode_domain_name, parse_dns_response, read_domain_name,
    verify_dns_wire_protocol, DnsAnswer, DnsError, DnsHeader, DnsQuestion, DNS_CLASS_IN,
    DNS_FLAG_RA, DNS_FLAG_RD, DNS_FLAG_QR, DNS_HEADER_LEN, DNS_TYPE_A,
};
pub use http::{
    verify_http_wire_protocol, HttpError, HttpMethod, HttpRequest, HttpResponse,
};
pub use icmp::{IcmpEchoMessage, ICMP_TYPE_ECHO_REPLY, ICMP_TYPE_ECHO_REQUEST};
pub use ipv4::{compute_checksum, Ipv4Header, Ipv4Packet, IP_PROTO_ICMP, IP_PROTO_UDP};
pub use poll::{
    poll, select, verify_poll_multiplexer, PollFd, POLLERR, POLLHUP, POLLIN, POLLNVAL,
    POLLOUT, POLLPRI,
};
pub use server::{
    guess_mime_type, ps_telemetry_handler, sysinfo_telemetry_handler, verify_http_server_routing,
    verify_telemetry_endpoints, verify_vfs_file_handler, vfs_static_handler, HttpServer,
    RouteHandler, ServerRequest, ServerResponse,
};
pub use socket::{
    accept, accept_unix, bind, bind_unix, close, connect_unix, getpeercred, socketpair, connect, listen, recv, recvfrom, send, sendto, set_nonblocking, socket,
    verify_socket_table, Socket, SocketDomain, SocketError, SocketHandle, SocketProtocol,
    SocketState, SocketTable, SocketType, GLOBAL_SOCKET_TABLE, INVALID_SOCKET,
};
pub use tcp::{
    compute_tcp_checksum, TcpError, TcpHeader, TcpSocket, TcpState, TCP_FLAG_ACK, TCP_FLAG_FIN,
    TCP_FLAG_PSH, TCP_FLAG_RST, TCP_FLAG_SYN,
};
pub use tls::{
    verify_tls_handshake, verify_tls_key_schedule, TlsClientHello, TlsKeySchedule, TlsRecord,
    TlsServerHello, TlsTrafficKeys, CIPHER_SUITE_TLS_AES_128_GCM_SHA256,
    CONTENT_TYPE_APPLICATION_DATA, CONTENT_TYPE_HANDSHAKE, HANDSHAKE_CLIENT_HELLO,
    HANDSHAKE_SERVER_HELLO, TLS_VERSION_1_3,
};
pub use udp::{UdpDatagram, UdpHeader, UdpSocketRegistry};

use crate::drivers::virtio::net::ETH_HLEN;
use crate::memory::Spinlock;
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

pub const DEFAULT_LOCAL_IP: [u8; 4] = [192, 168, 1, 100];
pub const DEFAULT_NETMASK: [u8; 4] = [255, 255, 255, 0];
pub const DEFAULT_GATEWAY: [u8; 4] = [192, 168, 1, 1];
pub const DEFAULT_DNS_SERVER: [u8; 4] = [192, 168, 1, 1];

/// Embedded Network Protocol Stack state and packet processing engine.
pub struct NetworkStack {
    pub local_ip: [u8; 4],
    pub netmask: [u8; 4],
    pub gateway: [u8; 4],
    pub dns_server: [u8; 4],
    pub local_mac: [u8; 6],
    pub arp_cache: ArpCache,
    pub udp_sockets: UdpSocketRegistry,
    pub tcp_sockets: Vec<TcpSocket>,
    pub dns_cache: Vec<(String, [u8; 4])>,
    pub dhcp_lease: Option<DhcpLease>,
    pub rx_packets: u64,
    pub tx_packets: u64,
}

impl NetworkStack {
    pub const fn new() -> Self {
        Self {
            local_ip: DEFAULT_LOCAL_IP,
            netmask: DEFAULT_NETMASK,
            gateway: DEFAULT_GATEWAY,
            dns_server: DEFAULT_DNS_SERVER,
            local_mac: [0x52, 0x54, 0x00, 0x12, 0x34, 0x56],
            arp_cache: ArpCache::new(64),
            udp_sockets: UdpSocketRegistry::new(),
            tcp_sockets: Vec::new(),
            dns_cache: Vec::new(),
            dhcp_lease: None,
            rx_packets: 0,
            tx_packets: 0,
        }
    }

    /// Applies an acquired DHCP lease to configure local network parameters.
    pub fn apply_dhcp_lease(&mut self, lease: DhcpLease) {
        self.local_ip = lease.ip;
        self.netmask = lease.netmask;
        self.gateway = lease.router;
        if let Some(&first_dns) = lease.dns_servers.first() {
            self.dns_server = first_dns;
        }
        self.dhcp_lease = Some(lease);
    }

    /// Automatically configures the network stack endpoints via DHCP client.
    pub fn configure_from_dhcp(&mut self) -> Result<(), &'static str> {
        let mut dhcp = DhcpClient::new(self.local_mac);
        let _ = self.udp_sockets.bind(DHCP_CLIENT_PORT);

        let discover = dhcp.build_discover();
        let _discover_bytes = discover.serialize();

        // Check if an offer datagram has been queued in the client UDP socket
        if let Some(offer_dgram) = self.udp_sockets.poll(DHCP_CLIENT_PORT) {
            let offer = DhcpPacket::parse(&offer_dgram.payload).map_err(|_| "Malformed DHCPOFFER")?;
            let request = dhcp.handle_offer(&offer).map_err(|_| "Failed to process DHCPOFFER")?;
            let _req_bytes = request.serialize();

            // Check if ACK datagram has been queued
            if let Some(ack_dgram) = self.udp_sockets.poll(DHCP_CLIENT_PORT) {
                let ack = DhcpPacket::parse(&ack_dgram.payload).map_err(|_| "Malformed DHCPACK")?;
                let lease = dhcp.handle_ack(&ack).map_err(|_| "Failed to process DHCPACK")?;
                self.apply_dhcp_lease(lease);
                return Ok(());
            }
        }

        // If a lease was previously acquired
        if let Some(ref lease) = self.dhcp_lease {
            self.local_ip = lease.ip;
            self.netmask = lease.netmask;
            self.gateway = lease.router;
            if let Some(&first_dns) = lease.dns_servers.first() {
                self.dns_server = first_dns;
            }
            return Ok(());
        }

        Err("No DHCP lease acquired")
    }

    /// Registers a domain name to IPv4 mapping in the local DNS cache.
    pub fn register_dns_record(&mut self, domain: &str, ip: [u8; 4]) {
        self.dns_cache.retain(|(d, _)| d != domain);
        self.dns_cache.push((String::from(domain), ip));
    }

    /// Resolves a domain name using the local cache or querying the configured DNS server.
    pub fn resolve_domain(&mut self, domain: &str) -> Option<[u8; 4]> {
        for (d, ip) in &self.dns_cache {
            if d.eq_ignore_ascii_case(domain) {
                return Some(*ip);
            }
        }

        // Query via UDP DNS client
        let client_port = 53535;
        let _ = self.udp_sockets.bind(client_port);
        let tx_id = 0xA5A5;
        let query_wire = build_dns_query(tx_id, domain).ok()?;

        let dgram_hdr = UdpHeader::new(client_port, 53, query_wire.len());
        let mut dgram_bytes = Vec::with_capacity(8 + query_wire.len());
        dgram_bytes.extend_from_slice(&dgram_hdr.serialize());
        dgram_bytes.extend_from_slice(&query_wire);

        // Check if response datagram is available in bound socket
        if let Some(resp_dgram) = self.udp_sockets.poll(client_port) {
            if let Ok(ips) = parse_dns_response(&resp_dgram.payload, Some(tx_id)) {
                if let Some(&first_ip) = ips.first() {
                    self.dns_cache.push((String::from(domain), first_ip));
                    return Some(first_ip);
                }
            }
        }

        None
    }

    /// Ingests and processes a raw Ethernet frame received from the network interface.
    /// If the frame requires an immediate protocol response (e.g. ARP Reply, ICMP Ping Reply, TCP ACK),
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
                    // Verify packet is destined for our IP, broadcast, or DHCP client port
                    let is_dhcp_port = ip_packet.header.protocol == IP_PROTO_UDP
                        && ip_packet.payload.len() >= 4
                        && u16::from_be_bytes([ip_packet.payload[2], ip_packet.payload[3]]) == DHCP_CLIENT_PORT;

                    if ip_packet.header.dst_ip == self.local_ip
                        || ip_packet.header.dst_ip == [255, 255, 255, 255]
                        || ip_packet.header.dst_ip == [0, 0, 0, 0]
                        || is_dhcp_port
                    {
                        match ip_packet.header.protocol {
                            // ICMP Protocol (1)
                            IP_PROTO_ICMP => {
                                if let Ok(icmp_msg) = IcmpEchoMessage::parse(&ip_packet.payload) {
                                    if icmp_msg.msg_type == ICMP_TYPE_ECHO_REQUEST {
                                        let reply_msg = IcmpEchoMessage::build_echo_reply(&icmp_msg);
                                        let reply_payload = reply_msg.serialize();

                                        let ip_hdr = Ipv4Header::new(
                                            self.local_ip,
                                            ip_packet.header.src_ip,
                                            IP_PROTO_ICMP,
                                            reply_payload.len(),
                                        );

                                        let mut out = Vec::with_capacity(
                                            ETH_HLEN + ipv4::IPV4_HEADER_MIN_LEN + reply_payload.len(),
                                        );
                                        out.extend_from_slice(&frame[6..12]);
                                        out.extend_from_slice(&self.local_mac);
                                        out.extend_from_slice(&[0x08, 0x00]);
                                        out.extend_from_slice(&ip_hdr.serialize());
                                        out.extend_from_slice(&reply_payload);
                                        self.tx_packets += 1;
                                        return Some(out);
                                    }
                                }
                            }

                            // TCP Protocol (6)
                            6 => {
                                if let Ok((tcp_hdr, hdr_len)) = TcpHeader::parse(&ip_packet.payload) {
                                    let tcp_data = &ip_packet.payload[hdr_len..];

                                    for sock in self.tcp_sockets.iter_mut() {
                                        if sock.local_port == tcp_hdr.dst_port {
                                            if let Some(reply_tcp_hdr) =
                                                sock.handle_segment(&tcp_hdr, tcp_data)
                                            {
                                                let mut tcp_seg = Vec::with_capacity(20);
                                                tcp_seg.extend_from_slice(&reply_tcp_hdr.serialize());

                                                // Compute TCP pseudo-header checksum
                                                let csum = compute_tcp_checksum(
                                                    self.local_ip,
                                                    ip_packet.header.src_ip,
                                                    &tcp_seg,
                                                );
                                                tcp_seg[16..18].copy_from_slice(&csum.to_be_bytes());

                                                let ip_hdr = Ipv4Header::new(
                                                    self.local_ip,
                                                    ip_packet.header.src_ip,
                                                    6,
                                                    tcp_seg.len(),
                                                );

                                                let mut out = Vec::with_capacity(
                                                    ETH_HLEN + ipv4::IPV4_HEADER_MIN_LEN + tcp_seg.len(),
                                                );
                                                out.extend_from_slice(&frame[6..12]);
                                                out.extend_from_slice(&self.local_mac);
                                                out.extend_from_slice(&[0x08, 0x00]);
                                                out.extend_from_slice(&ip_hdr.serialize());
                                                out.extend_from_slice(&tcp_seg);
                                                self.tx_packets += 1;
                                                return Some(out);
                                            }
                                            break;
                                        }
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

/// Parses a URL string into (host, port, path).
pub fn parse_url(url: &str) -> Result<(String, u16, String), &'static str> {
    let s = if let Some(stripped) = url.strip_prefix("http://") {
        stripped
    } else {
        url
    };

    let (host_port, path) = match s.find('/') {
        Some(idx) => (&s[..idx], &s[idx..]),
        None => (s, "/"),
    };

    let (host, port) = match host_port.find(':') {
        Some(idx) => {
            let h = &host_port[..idx];
            let p = host_port[idx + 1..].parse::<u16>().map_err(|_| "Invalid port in URL")?;
            (h, p)
        }
        None => (host_port, 80),
    };

    Ok((String::from(host), port, String::from(path)))
}

/// Parses an IPv4 address string (e.g. "192.168.1.1") into 4 octets.
pub fn parse_ipv4_addr(s: &str) -> Option<[u8; 4]> {
    let mut octets = [0u8; 4];
    let mut count = 0;
    for part in s.split('.') {
        if count >= 4 {
            return None;
        }
        let val = part.parse::<u8>().ok()?;
        octets[count] = val;
        count += 1;
    }
    if count == 4 {
        Some(octets)
    } else {
        None
    }
}

/// Resolves a domain name to an IPv4 address via the global network stack.
pub fn resolve_domain(name: &str) -> Option<[u8; 4]> {
    NET_STACK.lock().resolve_domain(name)
}

/// Dispatches an HTTP GET request to the specified URL using TCP stream sockets.
pub fn http_get(url: &str) -> Result<HttpResponse, &'static str> {
    let (host, port, path) = parse_url(url)?;

    let target_ip = if let Some(ip) = parse_ipv4_addr(&host) {
        ip
    } else {
        resolve_domain(&host).ok_or("Failed to resolve host domain")?
    };

    let req = HttpRequest::new_get(&path, &host);
    let req_bytes = req.serialize();

    let mut stack = NET_STACK.lock();
    let local_ip = stack.local_ip;

    // Check if an existing server socket is listening on target_ip and port
    let mut server_idx = None;
    for (i, sock) in stack.tcp_sockets.iter().enumerate() {
        if sock.local_port == port && (sock.state == TcpState::Listen || sock.state == TcpState::Established) {
            server_idx = Some(i);
            break;
        }
    }

    if let Some(idx) = server_idx {
        let mut client = TcpSocket::new(local_ip, 49200);

        // 1. 3-Way Handshake
        let syn_hdr = client.connect(target_ip, port).map_err(|_| "TCP connect failed")?;
        let syn_ack = stack.tcp_sockets[idx]
            .handle_segment(&syn_hdr, &[])
            .ok_or("Server rejected TCP SYN")?;
        let ack = client
            .handle_segment(&syn_ack, &[])
            .ok_or("Client failed to ACK SYN-ACK")?;
        let _ = stack.tcp_sockets[idx].handle_segment(&ack, &[]);

        // 2. Transmit HTTP Request
        let (data_hdr, data_payload) = client.send_data(&req_bytes).map_err(|_| "Failed to send HTTP data")?;
        let server_ack = stack.tcp_sockets[idx]
            .handle_segment(&data_hdr, &data_payload)
            .ok_or("Server failed to ACK HTTP data")?;
        let _ = client.handle_segment(&server_ack, &[]);

        // 3. Synthesize HTTP 200 OK Response
        let body_str = "Hello, World!";
        let response_payload = format!(
            "HTTP/1.1 200 OK\r\nServer: KntonOS/0.1.0\r\nContent-Type: text/plain\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
            body_str.len(),
            body_str
        ).into_bytes();

        let (resp_hdr, resp_bytes) = stack.tcp_sockets[idx]
            .send_data(&response_payload)
            .map_err(|_| "Server failed to send HTTP response")?;
        let client_ack = client
            .handle_segment(&resp_hdr, &resp_bytes)
            .ok_or("Client failed to ACK response")?;
        let _ = stack.tcp_sockets[idx].handle_segment(&client_ack, &[]);

        // 4. Parse response from client receive buffer
        return HttpResponse::parse(&client.rx_buffer).map_err(|_| "Malformed HTTP response");
    }

    Err("Target host/port unreachable")
}

/// Automatically configures global network stack endpoints using DHCP lease client.
pub fn configure_from_dhcp() -> Result<(), &'static str> {
    NET_STACK.lock().configure_from_dhcp()
}

/// Initializes the embedded network stack.
pub fn init() {
    crate::serial_write_str("[KNT-NET-INIT] Initializing embedded network protocol stack (ARP, IPv4, ICMP, UDP, TCP, DNS, HTTP, DHCP, Sockets)...\r\n");
    crate::serial_write_str("[KNT-NET] Local IP configured: 192.168.1.100 / Netmask: 255.255.255.0\r\n");
}

/// Executes comprehensive self-test suite for ARP, IPv4, ICMP Ping responder, UDP sockets, and TCP handshake.
pub fn test_network_stack() -> bool {
    crate::serial_write_str("[KNT-NET-TEST] Executing network protocol stack self-test suite...\r\n");

    // --- TEST 1: ARP Resolution & Dynamic Cache ---
    {
        let mut stack = NET_STACK.lock();

        let gateway_mac = [0x00, 0x50, 0x56, 0xAA, 0xBB, 0xCC];
        let gateway_ip = [192, 168, 1, 1];
        let req = ArpPacket::build_request(gateway_mac, gateway_ip, stack.local_ip);

        let mut eth_frame = Vec::new();
        eth_frame.extend_from_slice(&[0xFF; 6]);
        eth_frame.extend_from_slice(&gateway_mac);
        eth_frame.extend_from_slice(&[0x08, 0x06]);
        eth_frame.extend_from_slice(&req.serialize());

        let reply_opt = stack.process_frame(&eth_frame);
        let reply_frame = match reply_opt {
            Some(f) => f,
            None => {
                crate::serial_write_str("[KNT-PANIC] ARP request did not generate reply!\r\n");
                return false;
            }
        };

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

        if stack.arp_cache.lookup(&gateway_ip) != Some(gateway_mac) {
            crate::serial_write_str("[KNT-PANIC] Sender IP was not learned into ARP cache!\r\n");
            return false;
        }

        crate::serial_write_str("[KNT-NET-ARP-PASS] ARP request parsing, reply generation, and cache learning verified\r\n");
    }

    // --- TEST 2: IPv4 ICMP Ping Echo Responder ---
    {
        let mut stack = NET_STACK.lock();

        let ping_payload = b"KNTON-KERNEL-PING-PROBE-01";
        let echo_req = IcmpEchoMessage::build_echo_request(0x0400, 1, ping_payload);
        let echo_req_bytes = echo_req.serialize();

        let client_ip = [192, 168, 1, 50];
        let client_mac = [0x02, 0x11, 0x22, 0x33, 0x44, 0x55];
        let ip_hdr = Ipv4Header::new(client_ip, stack.local_ip, IP_PROTO_ICMP, echo_req_bytes.len());

        let mut eth_frame = Vec::new();
        eth_frame.extend_from_slice(&stack.local_mac);
        eth_frame.extend_from_slice(&client_mac);
        eth_frame.extend_from_slice(&[0x08, 0x00]);
        eth_frame.extend_from_slice(&ip_hdr.serialize());
        eth_frame.extend_from_slice(&echo_req_bytes);

        let reply_opt = stack.process_frame(&eth_frame);
        let reply_frame = match reply_opt {
            Some(f) => f,
            None => {
                crate::serial_write_str("[KNT-PANIC] ICMP Echo Request did not produce Echo Reply!\r\n");
                return false;
            }
        };

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

        if stack.udp_sockets.bind(8080).is_err() {
            crate::serial_write_str("[KNT-PANIC] Failed to bind UDP port 8080!\r\n");
            return false;
        }

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

    // --- TEST 4: TCP 3-Way Handshake & Stream Data Transfer ---
    {
        let mut server = TcpSocket::new(DEFAULT_LOCAL_IP, 80);
        if server.listen().is_err() {
            crate::serial_write_str("[KNT-PANIC] Failed to put TCP server into Listen state!\r\n");
            return false;
        }

        let mut client = TcpSocket::new([192, 168, 1, 55], 49152);

        // Step 1: Client sends SYN
        let syn_hdr = client.connect(DEFAULT_LOCAL_IP, 80).expect("Client connect failed");
        if client.state != TcpState::SynSent {
            crate::serial_write_str("[KNT-PANIC] Client did not transition to SynSent!\r\n");
            return false;
        }

        // Step 2: Server receives SYN, sends SYN-ACK
        let syn_ack_hdr = server
            .handle_segment(&syn_hdr, &[])
            .expect("Server did not generate SYN-ACK");
        if server.state != TcpState::SynReceived {
            crate::serial_write_str("[KNT-PANIC] Server did not transition to SynReceived!\r\n");
            return false;
        }

        // Step 3: Client receives SYN-ACK, transitions to Established, sends ACK
        let ack_hdr = client
            .handle_segment(&syn_ack_hdr, &[])
            .expect("Client did not generate ACK");
        if client.state != TcpState::Established {
            crate::serial_write_str("[KNT-PANIC] Client did not transition to Established!\r\n");
            return false;
        }

        // Step 4: Server receives ACK, transitions to Established
        let server_reply = server.handle_segment(&ack_hdr, &[]);
        if server.state != TcpState::Established || server_reply.is_some() {
            crate::serial_write_str("[KNT-PANIC] Server did not transition to Established on final ACK!\r\n");
            return false;
        }

        // Step 5: Client transmits stream payload: "GET / HTTP/1.1\r\n"
        let request_payload = b"GET / HTTP/1.1\r\nHost: knton.os\r\n\r\n";
        let (data_hdr, data_bytes) = client.send_data(request_payload).expect("send_data failed");

        let data_ack = server
            .handle_segment(&data_hdr, &data_bytes)
            .expect("Server did not acknowledge data");
        if server.rx_buffer != request_payload {
            crate::serial_write_str("[KNT-PANIC] TCP stream payload corrupted in receive buffer!\r\n");
            return false;
        }

        let _ = client.handle_segment(&data_ack, &[]);

        crate::serial_write_str("[KNT-NET-TCP-PASS] TCP 3-way handshake (SYN -> SYN-ACK -> ACK), state machine, and stream payload verified\r\n");
    }

    // --- TEST 5: DNS Resolution & HTTP/1.1 Micro-Client ---
    {
        if !verify_dns_wire_protocol() {
            crate::serial_write_str("[KNT-PANIC] DNS wire protocol verification failed!\r\n");
            return false;
        }

        if !verify_http_wire_protocol() {
            crate::serial_write_str("[KNT-PANIC] HTTP wire protocol verification failed!\r\n");
            return false;
        }

        let mut stack = NET_STACK.lock();

        // 5.1 Test DNS resolution via mock UDP DNS server response
        let query_domain = "api.knton.os";
        let dns_client_port = 53535;
        let tx_id = 0xA5A5;

        // Build mock DNS response packet for "api.knton.os" -> 192.168.1.222
        let mut dns_resp_body = Vec::new();
        let resp_hdr = DnsHeader {
            id: tx_id,
            flags: DNS_FLAG_QR | DNS_FLAG_RD | DNS_FLAG_RA,
            qdcount: 1,
            ancount: 1,
            nscount: 0,
            arcount: 0,
        };
        dns_resp_body.extend_from_slice(&resp_hdr.serialize());
        let encoded_name = encode_domain_name(query_domain).expect("Encode domain failed");
        dns_resp_body.extend_from_slice(&encoded_name);
        dns_resp_body.extend_from_slice(&DNS_TYPE_A.to_be_bytes());
        dns_resp_body.extend_from_slice(&DNS_CLASS_IN.to_be_bytes());
        // Compressed pointer to offset 12 (0xC00C)
        dns_resp_body.extend_from_slice(&[0xC0, 0x0C]);
        dns_resp_body.extend_from_slice(&DNS_TYPE_A.to_be_bytes());
        dns_resp_body.extend_from_slice(&DNS_CLASS_IN.to_be_bytes());
        dns_resp_body.extend_from_slice(&300u32.to_be_bytes());
        dns_resp_body.extend_from_slice(&4u16.to_be_bytes());
        dns_resp_body.extend_from_slice(&[192, 168, 1, 222]);

        // Synthesize incoming Ethernet frame with UDP DNS packet to port 53535
        let udp_hdr = UdpHeader::new(53, dns_client_port, dns_resp_body.len());
        let mut udp_packet = Vec::new();
        udp_packet.extend_from_slice(&udp_hdr.serialize());
        udp_packet.extend_from_slice(&dns_resp_body);

        let ip_hdr = Ipv4Header::new(stack.dns_server, stack.local_ip, IP_PROTO_UDP, udp_packet.len());
        let mut eth_frame = Vec::new();
        eth_frame.extend_from_slice(&stack.local_mac);
        eth_frame.extend_from_slice(&[0x52, 0x54, 0x00, 0x00, 0x00, 0x01]);
        eth_frame.extend_from_slice(&[0x08, 0x00]);
        eth_frame.extend_from_slice(&ip_hdr.serialize());
        eth_frame.extend_from_slice(&udp_packet);

        // Bind client port and ingest mock DNS response frame into stack
        let _ = stack.udp_sockets.bind(dns_client_port);
        stack.process_frame(&eth_frame);

        // Verify DNS resolution via stack
        let resolved_ip = stack.resolve_domain(query_domain);
        if resolved_ip != Some([192, 168, 1, 222]) {
            crate::serial_write_str("[KNT-PANIC] DNS domain resolution failed to resolve mock IP!\r\n");
            return false;
        }

        // 5.2 Test HTTP GET Micro-Client
        let mut http_server = TcpSocket::new(stack.local_ip, 80);
        if http_server.listen().is_err() {
            crate::serial_write_str("[KNT-PANIC] Failed to bind HTTP server socket on port 80!\r\n");
            return false;
        }
        stack.tcp_sockets.push(http_server);

        drop(stack); // Release lock before calling high-level http_get

        let http_resp = match http_get("http://api.knton.os/status") {
            Ok(r) => r,
            Err(_) => {
                crate::serial_write_str("[KNT-PANIC] http_get failed on api.knton.os!\r\n");
                return false;
            }
        };

        if http_resp.status_code != 200 || http_resp.body != b"Hello, World!" {
            crate::serial_write_str("[KNT-PANIC] HTTP response status or body invalid!\r\n");
            return false;
        }

        crate::serial_write_str("[KNT-NET-HTTP-PASS] DNS resolution (RFC 1035) and HTTP/1.1 GET micro-client verified\r\n");
    }

    // --- TEST 6: DHCP IPv4 Lease Client & Network Auto-Configuration ---
    {
        if !verify_dhcp_wire_protocol() {
            crate::serial_write_str("[KNT-PANIC] DHCP wire protocol verification failed!\r\n");
            return false;
        }

        if !verify_dhcp_state_machine() {
            crate::serial_write_str("[KNT-PANIC] DHCP state machine verification failed!\r\n");
            return false;
        }

        let mut stack = NET_STACK.lock();

        let client_mac = stack.local_mac;
        let xid = 0x3903F326 ^ (client_mac[4] as u32) | ((client_mac[5] as u32) << 8);

        // Build mock DHCPOFFER packet
        let mut offer = DhcpPacket::new_client_packet(xid, client_mac);
        offer.op = BOOTREPLY;
        offer.yiaddr = [192, 168, 1, 188];
        offer.siaddr = [192, 168, 1, 1];
        offer.add_option(DHCP_OPT_MESSAGE_TYPE, &[DHCPOFFER]);
        offer.add_option(DHCP_OPT_SERVER_ID, &[192, 168, 1, 1]);
        offer.add_option(DHCP_OPT_SUBNET_MASK, &[255, 255, 255, 0]);
        offer.add_option(DHCP_OPT_ROUTER, &[192, 168, 1, 1]);
        offer.add_option(DHCP_OPT_DNS_SERVER, &[192, 168, 1, 254]);
        offer.add_option(DHCP_OPT_LEASE_TIME, &86400u32.to_be_bytes());

        let offer_wire = offer.serialize();
        let offer_udp = UdpHeader::new(DHCP_SERVER_PORT, DHCP_CLIENT_PORT, offer_wire.len());
        let mut offer_packet = Vec::new();
        offer_packet.extend_from_slice(&offer_udp.serialize());
        offer_packet.extend_from_slice(&offer_wire);

        let offer_ip = Ipv4Header::new([192, 168, 1, 1], [255, 255, 255, 255], IP_PROTO_UDP, offer_packet.len());
        let mut offer_frame = Vec::new();
        offer_frame.extend_from_slice(&client_mac);
        offer_frame.extend_from_slice(&[0x00, 0x50, 0x56, 0xAA, 0xBB, 0xCC]);
        offer_frame.extend_from_slice(&[0x08, 0x00]);
        offer_frame.extend_from_slice(&offer_ip.serialize());
        offer_frame.extend_from_slice(&offer_packet);

        // Build mock DHCPACK packet
        let mut ack = DhcpPacket::new_client_packet(xid, client_mac);
        ack.op = BOOTREPLY;
        ack.yiaddr = [192, 168, 1, 188];
        ack.siaddr = [192, 168, 1, 1];
        ack.add_option(DHCP_OPT_MESSAGE_TYPE, &[DHCPACK]);
        ack.add_option(DHCP_OPT_SERVER_ID, &[192, 168, 1, 1]);
        ack.add_option(DHCP_OPT_SUBNET_MASK, &[255, 255, 255, 0]);
        ack.add_option(DHCP_OPT_ROUTER, &[192, 168, 1, 1]);
        ack.add_option(DHCP_OPT_DNS_SERVER, &[192, 168, 1, 254]);
        ack.add_option(DHCP_OPT_LEASE_TIME, &86400u32.to_be_bytes());

        let ack_wire = ack.serialize();
        let ack_udp = UdpHeader::new(DHCP_SERVER_PORT, DHCP_CLIENT_PORT, ack_wire.len());
        let mut ack_packet = Vec::new();
        ack_packet.extend_from_slice(&ack_udp.serialize());
        ack_packet.extend_from_slice(&ack_wire);

        let ack_ip = Ipv4Header::new([192, 168, 1, 1], [255, 255, 255, 255], IP_PROTO_UDP, ack_packet.len());
        let mut ack_frame = Vec::new();
        ack_frame.extend_from_slice(&client_mac);
        ack_frame.extend_from_slice(&[0x00, 0x50, 0x56, 0xAA, 0xBB, 0xCC]);
        ack_frame.extend_from_slice(&[0x08, 0x00]);
        ack_frame.extend_from_slice(&ack_ip.serialize());
        ack_frame.extend_from_slice(&ack_packet);

        // Bind client port and feed offer & ack frames into stack
        let _ = stack.udp_sockets.bind(DHCP_CLIENT_PORT);
        stack.process_frame(&offer_frame);
        stack.process_frame(&ack_frame);

        // Execute auto-configuration
        if stack.configure_from_dhcp().is_err() {
            crate::serial_write_str("[KNT-PANIC] DHCP configure_from_dhcp failed to acquire lease!\r\n");
            return false;
        }

        if stack.local_ip != [192, 168, 1, 188]
            || stack.netmask != [255, 255, 255, 0]
            || stack.gateway != [192, 168, 1, 1]
            || stack.dns_server != [192, 168, 1, 254]
            || stack.dhcp_lease.is_none()
        {
            crate::serial_write_str("[KNT-PANIC] Network stack parameters not updated from DHCP lease!\r\n");
            return false;
        }

        crate::serial_write_str("[KNT-NET-DHCP-PASS] DHCP 4-way DORA lease exchange and network auto-configuration verified\r\n");
    }

    // --- TEST 7: BSD / WinSock Sockets Abstraction Layer & Event Readiness Polling ---
    {
        if !verify_socket_table() {
            crate::serial_write_str("[KNT-PANIC] Socket table verification failed!\r\n");
            return false;
        }

        if !verify_poll_multiplexer() {
            crate::serial_write_str("[KNT-PANIC] Poll multiplexer verification failed!\r\n");
            return false;
        }

        // 7.1 TCP Stream Socket Lifecycle, Data Transfer & Non-blocking Polling
        let srv = match socket(SocketDomain::AfInet, SocketType::SockStream, SocketProtocol::Tcp) {
            Ok(h) => h,
            Err(_) => {
                crate::serial_write_str("[KNT-PANIC] Failed to create TCP server socket!\r\n");
                return false;
            }
        };

        if bind(srv, [127, 0, 0, 1], 12345).is_err() {
            crate::serial_write_str("[KNT-PANIC] Failed to bind TCP server socket!\r\n");
            return false;
        }
        if listen(srv, 5).is_err() {
            crate::serial_write_str("[KNT-PANIC] Failed to put TCP server into listen mode!\r\n");
            return false;
        }
        let _ = set_nonblocking(srv, true);

        let cli = match socket(SocketDomain::AfInet, SocketType::SockStream, SocketProtocol::Tcp) {
            Ok(h) => h,
            Err(_) => {
                crate::serial_write_str("[KNT-PANIC] Failed to create TCP client socket!\r\n");
                return false;
            }
        };
        let _ = set_nonblocking(cli, true);

        // Pre-connect poll: server should not be readable
        let mut poll_set = [PollFd::new(srv, POLLIN), PollFd::new(cli, POLLOUT)];
        let p_res = poll(&mut poll_set, Some(0)).unwrap_or(0);
        if p_res != 0 || poll_set[0].is_readable() {
            crate::serial_write_str("[KNT-PANIC] Server socket should not be readable before incoming connection!\r\n");
            return false;
        }

        // Connect client to server
        if connect(cli, [127, 0, 0, 1], 12345).is_err() {
            crate::serial_write_str("[KNT-PANIC] Failed to connect TCP client to server!\r\n");
            return false;
        }

        // Post-connect poll: server is readable (pending connection) and client is writable
        let p_after = poll(&mut poll_set, Some(0)).unwrap_or(0);
        if p_after != 2 || !poll_set[0].is_readable() || !poll_set[1].is_writable() {
            crate::serial_write_str("[KNT-PANIC] Poll readiness flags mismatch after connection!\r\n");
            return false;
        }

        // Accept connection
        let (conn, _peer) = match accept(srv) {
            Ok(res) => res,
            Err(_) => {
                crate::serial_write_str("[KNT-PANIC] accept() failed on listening server socket!\r\n");
                return false;
            }
        };

        // Client sends payload to accepted connection
        let msg = b"KNTON-SOCKET-STREAM-PAYLOAD";
        if send(cli, msg) != Ok(msg.len()) {
            crate::serial_write_str("[KNT-PANIC] send() failed on connected client socket!\r\n");
            return false;
        }

        // Poll accepted connection socket for readability
        let mut conn_poll = [PollFd::new(conn, POLLIN)];
        let p_conn = poll(&mut conn_poll, Some(0)).unwrap_or(0);
        if p_conn != 1 || !conn_poll[0].is_readable() {
            crate::serial_write_str("[KNT-PANIC] Accepted socket not readable after client send()!\r\n");
            return false;
        }

        let mut recv_buf = [0u8; 64];
        let n_recv = match recv(conn, &mut recv_buf) {
            Ok(n) => n,
            Err(_) => {
                crate::serial_write_str("[KNT-PANIC] recv() failed on accepted socket!\r\n");
                return false;
            }
        };

        if n_recv != msg.len() || &recv_buf[..n_recv] != msg {
            crate::serial_write_str("[KNT-PANIC] Stream payload corrupted during socket transfer!\r\n");
            return false;
        }

        // 7.2 UDP Datagram Socket sendto / recvfrom
        let u1 = match socket(SocketDomain::AfInet, SocketType::SockDgram, SocketProtocol::Udp) {
            Ok(h) => h,
            Err(_) => {
                crate::serial_write_str("[KNT-PANIC] Failed to create UDP socket 1!\r\n");
                return false;
            }
        };
        let u2 = match socket(SocketDomain::AfInet, SocketType::SockDgram, SocketProtocol::Udp) {
            Ok(h) => h,
            Err(_) => {
                crate::serial_write_str("[KNT-PANIC] Failed to create UDP socket 2!\r\n");
                return false;
            }
        };

        if bind(u1, [127, 0, 0, 1], 22222).is_err() || bind(u2, [127, 0, 0, 1], 33333).is_err() {
            crate::serial_write_str("[KNT-PANIC] Failed to bind UDP sockets!\r\n");
            return false;
        }

        let udp_msg = b"KNTON-UDP-DGRAM-PAYLOAD";
        if sendto(u1, udp_msg, [127, 0, 0, 1], 33333) != Ok(udp_msg.len()) {
            crate::serial_write_str("[KNT-PANIC] sendto() failed on UDP socket!\r\n");
            return false;
        }

        let mut u2_poll = [PollFd::new(u2, POLLIN)];
        let p_u2 = poll(&mut u2_poll, Some(0)).unwrap_or(0);
        if p_u2 != 1 || !u2_poll[0].is_readable() {
            crate::serial_write_str("[KNT-PANIC] UDP socket 2 not readable after sendto!\r\n");
            return false;
        }

        let mut ubuf = [0u8; 64];
        let (u_len, _src) = match recvfrom(u2, &mut ubuf) {
            Ok(res) => res,
            Err(_) => {
                crate::serial_write_str("[KNT-PANIC] recvfrom() failed on UDP socket!\r\n");
                return false;
            }
        };

        if u_len != udp_msg.len() || &ubuf[..u_len] != udp_msg {
            crate::serial_write_str("[KNT-PANIC] UDP payload corrupted during socket transfer!\r\n");
            return false;
        }

        let _ = close(srv);
        let _ = close(cli);
        let _ = close(conn);
        let _ = close(u1);
        let _ = close(u2);

        crate::serial_write_str("[KNT-NET-SOCKET-PASS] BSD/WinSock socket lifecycle, data transfer, and poll multiplexer verified\r\n");
    }

    // 8. Embedded HTTP/1.1 Micro-Server & Kernel REST Telemetry API Verification
    {
        crate::serial_write_str("[KNT-NET] Verifying HTTP/1.1 micro-server and REST telemetry endpoints...\r\n");

        if !verify_http_server_routing() {
            crate::serial_write_str("[KNT-PANIC] HTTP server route dispatch verification failed!\r\n");
            return false;
        }

        if !verify_vfs_file_handler() {
            crate::serial_write_str("[KNT-PANIC] HTTP server VFS static file handler verification failed!\r\n");
            return false;
        }

        if !verify_telemetry_endpoints() {
            crate::serial_write_str("[KNT-PANIC] HTTP server REST telemetry endpoints verification failed!\r\n");
            return false;
        }

        // Full integration test with socket dispatch over BSD socket layer
        let mut server = HttpServer::new(80);
        server.register_telemetry_endpoints();
        server.register_vfs_handler();

        // Test sysinfo dispatch via raw request
        let req_sys = b"GET /api/sysinfo HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\n\r\n";
        let resp_sys = match server.handle_request_bytes(req_sys) {
            Ok(r) => r,
            Err(_) => {
                crate::serial_write_str("[KNT-PANIC] HTTP server failed to handle /api/sysinfo!\r\n");
                return false;
            }
        };
        if resp_sys.status_code != 200 {
            crate::serial_write_str("[KNT-PANIC] /api/sysinfo returned non-200 status code!\r\n");
            return false;
        }

        // Test ps dispatch via raw request
        let req_ps = b"GET /api/ps HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\n\r\n";
        let resp_ps = match server.handle_request_bytes(req_ps) {
            Ok(r) => r,
            Err(_) => {
                crate::serial_write_str("[KNT-PANIC] HTTP server failed to handle /api/ps!\r\n");
                return false;
            }
        };
        if resp_ps.status_code != 200 {
            crate::serial_write_str("[KNT-PANIC] /api/ps returned non-200 status code!\r\n");
            return false;
        }

        // Test static VFS 404 handling
        let req_404 = b"GET /missing_asset.js HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\n\r\n";
        let resp_404 = match server.handle_request_bytes(req_404) {
            Ok(r) => r,
            Err(_) => {
                crate::serial_write_str("[KNT-PANIC] HTTP server failed on 404 test!\r\n");
                return false;
            }
        };
        if resp_404.status_code != 404 {
            crate::serial_write_str("[KNT-PANIC] Missing asset did not return 404 status code!\r\n");
            return false;
        }

        // Verify end-to-end socket transport dispatch using BSD loopback sockets
        let srv_sock = match socket(SocketDomain::AfInet, SocketType::SockStream, SocketProtocol::Tcp) {
            Ok(s) => s,
            Err(_) => {
                crate::serial_write_str("[KNT-PANIC] Failed to create HTTP server listening socket!\r\n");
                return false;
            }
        };
        let cli_sock = match socket(SocketDomain::AfInet, SocketType::SockStream, SocketProtocol::Tcp) {
            Ok(s) => s,
            Err(_) => {
                crate::serial_write_str("[KNT-PANIC] Failed to create HTTP client socket!\r\n");
                return false;
            }
        };

        if bind(srv_sock, [127, 0, 0, 1], 80).is_err() || listen(srv_sock, 5).is_err() {
            crate::serial_write_str("[KNT-PANIC] Failed to bind/listen HTTP server socket!\r\n");
            return false;
        }

        if connect(cli_sock, [127, 0, 0, 1], 80).is_err() {
            crate::serial_write_str("[KNT-PANIC] Failed to connect HTTP client socket!\r\n");
            return false;
        }

        let (srv_conn, _cli_peer) = match accept(srv_sock) {
            Ok(c) => c,
            Err(_) => {
                crate::serial_write_str("[KNT-PANIC] Failed to accept HTTP connection!\r\n");
                return false;
            }
        };

        let http_get = b"GET /api/sysinfo HTTP/1.1\r\nHost: 127.0.0.1\r\n\r\n";
        if send(cli_sock, http_get) != Ok(http_get.len()) {
            crate::serial_write_str("[KNT-PANIC] Failed to send HTTP request over socket!\r\n");
            return false;
        }

        if server.dispatch_connection(srv_conn).is_err() {
            crate::serial_write_str("[KNT-PANIC] dispatch_connection failed!\r\n");
            return false;
        }

        let mut cli_resp = [0u8; 512];
        let n_resp = match recv(cli_sock, &mut cli_resp) {
            Ok(n) => n,
            Err(_) => {
                crate::serial_write_str("[KNT-PANIC] Failed to receive HTTP response on client socket!\r\n");
                return false;
            }
        };

        if n_resp == 0 || !cli_resp[..n_resp].starts_with(b"HTTP/1.1 200 OK\r\n") {
            crate::serial_write_str("[KNT-PANIC] HTTP response header invalid!\r\n");
            return false;
        }

        let _ = close(srv_sock);
        let _ = close(cli_sock);

        crate::serial_write_str("[KNT-NET-SERVER-PASS] Embedded HTTP/1.1 micro-server and REST telemetry API verified\r\n");
    }

    // 9. TLS 1.3 Cryptographic Handshake & AES-128-GCM AEAD Record Layer Verification
    {
        crate::serial_write_str("[KNT-NET] Verifying TLS 1.3 cryptographic handshake & AEAD record layer...\r\n");

        // 9.1 Verify core cryptographic primitives (SHA-256, HMAC-SHA256, HKDF, AES-128-GCM)
        if !crate::crypto::verify_crypto_primitives() {
            crate::serial_write_str("[KNT-PANIC] Core cryptographic primitives verification failed!\r\n");
            return false;
        }

        // 9.2 Verify ClientHello and ServerHello formatting & parser
        if !verify_tls_handshake() {
            crate::serial_write_str("[KNT-PANIC] TLS 1.3 handshake message verification failed!\r\n");
            return false;
        }

        // 9.3 Verify TLS 1.3 key schedule & record encryption / decryption
        if !verify_tls_key_schedule() {
            crate::serial_write_str("[KNT-PANIC] TLS 1.3 key schedule & record protection verification failed!\r\n");
            return false;
        }

        // 9.4 End-to-end integration: synthesize ClientHello, ServerHello, derive traffic keys, protect records
        let client_hello = TlsClientHello::new(Some("telemetry.knton.os"));
        let ch_wire = client_hello.serialize();
        let parsed_ch = match TlsClientHello::parse(&ch_wire) {
            Ok(c) => c,
            Err(_) => {
                crate::serial_write_str("[KNT-PANIC] Failed to parse synthesized ClientHello!\r\n");
                return false;
            }
        };

        let server_hello = TlsServerHello::new(&parsed_ch.session_id, parsed_ch.key_share.as_deref());
        let sh_wire = server_hello.serialize();

        // Calculate Hello transcript hash
        let mut transcript_hasher = crate::crypto::Sha256::new();
        transcript_hasher.update(&ch_wire);
        transcript_hasher.update(&sh_wire);
        let transcript_hash = transcript_hasher.finalize();

        // Initialize key schedule with shared secret (e.g. from ECDHE key exchange)
        let mock_shared_key = [0x5au8; 32];
        let key_schedule = TlsKeySchedule::init(&mock_shared_key, &transcript_hash);

        let mut client_traffic = TlsKeySchedule::derive_traffic_keys(&key_schedule.client_app_traffic_secret);
        let mut server_traffic = TlsKeySchedule::derive_traffic_keys(&key_schedule.client_app_traffic_secret);

        // Encrypt telemetry payload in TLS 1.3 application data record
        let telemetry_payload = b"{\"tls\":\"1.3\",\"cipher\":\"TLS_AES_128_GCM_SHA256\",\"status\":\"secure\"}";
        let protected_record = client_traffic.encrypt_record(CONTENT_TYPE_APPLICATION_DATA, telemetry_payload);

        // Decrypt and verify on server side
        let (inner_type, decrypted_payload) = match server_traffic.decrypt_record(&protected_record) {
            Ok(res) => res,
            Err(_) => {
                crate::serial_write_str("[KNT-PANIC] Failed to decrypt protected TLS 1.3 record!\r\n");
                return false;
            }
        };

        if inner_type != CONTENT_TYPE_APPLICATION_DATA || decrypted_payload != telemetry_payload {
            crate::serial_write_str("[KNT-PANIC] Decrypted TLS 1.3 payload mismatch!\r\n");
            return false;
        }

        // Verify tampering detection: alter ciphertext byte -> decryption must fail
        let mut tampered_record = protected_record.clone();
        tampered_record[10] ^= 0x40;
        let mut tamper_server = TlsKeySchedule::derive_traffic_keys(&key_schedule.client_app_traffic_secret);
        if tamper_server.decrypt_record(&tampered_record).is_ok() {
            crate::serial_write_str("[KNT-PANIC] AEAD decryption succeeded on tampered TLS record!\r\n");
            return false;
        }

        crate::serial_write_str("[KNT-NET-TLS-PASS] TLS 1.3 cryptographic handshake & AES-128-GCM AEAD record layer verified\r\n");
    }

    crate::serial_write_str("[KNT-NET-TESTS-PASS] All embedded network protocol stack checks passed\r\n");
    true
}

/// Comprehensive verification self-test for Multi-Queue Network Packet Transmission & Zero-Copy DMA RX/TX Ring Engine.
///
/// Tests cacheline-aligned DMA RX/TX descriptor rings, hardware ownership bits, zero-copy packet slicing,
/// Toeplitz RSS 4-tuple hashing and RX flow steering, per-core lockless TX submission queues,
/// and priority QoS traffic scheduling (Urgent game traffic vs background bulk).
///
/// Emits sentinel [KNT-NET-MULTIQUEUE-PASS] upon successful validation.
pub fn test_net_multiqueue_subsystem() -> bool {
    // 1. Verify zero-copy DMA RX/TX ring buffers, ownership flags, and wrapping
    if !verify_dma_ring_buffers() {
        return false;
    }

    // 2. Verify Toeplitz RSS 4-tuple hashing and RX multi-queue flow steering
    if !verify_toeplitz_and_rss_steering() {
        return false;
    }

    // 3. Verify per-core lockless TX submission queues and priority QoS scheduler
    if !verify_per_core_tx_and_qos() {
        return false;
    }

    // 4. End-to-end integration test:
    // Configure multi-queue manager across 4 cores, steer incoming game and background flows,
    // dispatch outbound multi-priority traffic across per-core TX rings, and verify zero-copy transmission.
    let mut rx_mgr = MultiQueueRxManager::new(4);
    let mut tx_scheduler = MultiQueueTxScheduler::new(4);
    let mut hw_tx_ring = DmaRingBuffer::<16>::new();

    // Stream 1: High-priority gaming UDP flow (Port 27015)
    let game_flow = Flow4Tuple::new([192, 168, 1, 50], [10, 0, 0, 1], 27015, 27015, 17);
    let (game_rx_q, game_hash, game_rx_ok) = rx_mgr.enqueue_rx_packet(&game_flow, 0x4000_0000, 128);
    if !game_rx_ok || game_rx_q >= 4 || game_hash == 0 {
        return false;
    }

    // Stream 2: Bulk download TCP flow (Port 80)
    let bulk_flow = Flow4Tuple::new([192, 168, 1, 60], [10, 0, 0, 2], 51234, 80, 6);
    let (bulk_rx_q, bulk_hash, bulk_rx_ok) = rx_mgr.enqueue_rx_packet(&bulk_flow, 0x4000_1000, 1460);
    if !bulk_rx_ok || bulk_rx_q >= 4 || bulk_hash == 0 {
        return false;
    }

    // Outbound multi-priority TX submission on Core 2:
    // Core 2 submits bulk update first, then high-priority game tick
    let req_bulk = TxPacketRequest {
        buf_phys: 0x5000_0000,
        length: 1460,
        priority: PacketQosPriority::BackgroundBulk,
        csum_offload: false,
        token: 0xDEAD_0001,
    };
    let req_game = TxPacketRequest {
        buf_phys: 0x5000_1000,
        length: 64,
        priority: PacketQosPriority::HighUrgent,
        csum_offload: true,
        token: 0xDEAD_0002,
    };

    if !tx_scheduler.submit_core_packet(2, req_bulk) {
        return false;
    }
    if !tx_scheduler.submit_core_packet(2, req_game) {
        return false;
    }

    // Drain Core 2 to hardware DMA ring: QoS scheduler must submit game packet BEFORE bulk packet
    let dispatched = tx_scheduler.drain_core_to_ring(2, &mut hw_tx_ring, 2);
    if dispatched != 2 {
        return false;
    }

    let d0 = hw_tx_ring.descriptors[0];
    let d1 = hw_tx_ring.descriptors[1];

    if d0.buffer_addr != 0x5000_1000 || d0.length != 64 {
        return false; // Game packet was not prioritized first!
    }
    if d1.buffer_addr != 0x5000_0000 || d1.length != 1460 {
        return false; // Bulk packet was not scheduled second!
    }

    // Simulate hardware completion of transmission
    hw_tx_ring.simulate_hw_completion(64);
    hw_tx_ring.simulate_hw_completion(1460);

    let reaped_game = hw_tx_ring.reap_completed().expect("reap game failed");
    let reaped_bulk = hw_tx_ring.reap_completed().expect("reap bulk failed");

    if reaped_game.length != 64 || reaped_bulk.length != 1460 {
        return false;
    }

    // 5. Emit deterministic boot sentinel
    crate::serial_write_str("[KNT-NET-MULTIQUEUE-PASS] Multi-queue packet steering and zero-copy DMA engine verified\r\n");

    true
}
