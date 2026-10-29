//! Transmission Control Protocol (TCP, RFC 793) Header Parser and Builder.
//!
//! Implements RFC 793 20-byte standard TCP header layout, control flags (SYN, ACK, FIN, RST, PSH),
//! data offset parsing, window size, pseudo-header checksum calculation, connection state machine,
//! and stream socket 3-way handshake generation.

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

/// RFC 793 TCP Connection States.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TcpState {
    Closed,
    Listen,
    SynSent,
    SynReceived,
    Established,
    FinWait1,
    FinWait2,
    CloseWait,
    Closing,
    LastAck,
    TimeWait,
}

/// Events driving TCP state machine transitions.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TcpEvent {
    PassiveOpen,
    ActiveOpen,
    SendSyn,
    RecvSyn,
    SendSynAck,
    RecvSynAck,
    RecvAck,
    SendFin,
    RecvFin,
    Close,
    Timeout,
}

impl TcpState {
    /// Returns true if the TCP connection is fully established and capable of bidirectional data transfer.
    pub fn is_established(&self) -> bool {
        *self == TcpState::Established
    }

    /// Evaluates RFC 793 state transition table based on an incoming event.
    pub fn transition(&mut self, event: TcpEvent) -> Result<(), TcpError> {
        let next_state = match (*self, event) {
            (TcpState::Closed, TcpEvent::PassiveOpen) => TcpState::Listen,
            (TcpState::Closed, TcpEvent::ActiveOpen) => TcpState::SynSent,
            (TcpState::Listen, TcpEvent::RecvSyn) => TcpState::SynReceived,
            (TcpState::SynSent, TcpEvent::RecvSynAck) => TcpState::Established,
            (TcpState::SynSent, TcpEvent::RecvSyn) => TcpState::SynReceived,
            (TcpState::SynReceived, TcpEvent::RecvAck) => TcpState::Established,
            (TcpState::Established, TcpEvent::Close) => TcpState::FinWait1,
            (TcpState::Established, TcpEvent::RecvFin) => TcpState::CloseWait,
            (TcpState::FinWait1, TcpEvent::RecvAck) => TcpState::FinWait2,
            (TcpState::FinWait1, TcpEvent::RecvFin) => TcpState::Closing,
            (TcpState::FinWait2, TcpEvent::RecvFin) => TcpState::TimeWait,
            (TcpState::CloseWait, TcpEvent::Close) => TcpState::LastAck,
            (TcpState::Closing, TcpEvent::RecvAck) => TcpState::TimeWait,
            (TcpState::LastAck, TcpEvent::RecvAck) => TcpState::Closed,
            (TcpState::TimeWait, TcpEvent::Timeout) => TcpState::Closed,
            _ => return Err(TcpError::UnexpectedState),
        };

        *self = next_state;
        Ok(())
    }
}

/// TCP Stream Socket tracking sequence numbers, sliding windows, and 3-way handshake.
#[derive(Debug, Clone)]
pub struct TcpSocket {
    pub local_ip: [u8; 4],
    pub local_port: u16,
    pub remote_ip: [u8; 4],
    pub remote_port: u16,
    pub state: TcpState,
    pub local_seq: u32,
    pub remote_seq: u32,
    pub window_size: u16,
    pub rx_buffer: Vec<u8>,
}

impl TcpSocket {
    /// Creates a new TCP socket bound to a local endpoint in the Closed state.
    pub fn new(local_ip: [u8; 4], local_port: u16) -> Self {
        Self {
            local_ip,
            local_port,
            remote_ip: [0; 4],
            remote_port: 0,
            state: TcpState::Closed,
            local_seq: 1000, // Initial Sequence Number (ISN)
            remote_seq: 0,
            window_size: DEFAULT_TCP_WINDOW,
            rx_buffer: Vec::new(),
        }
    }

    /// Puts the socket into the Listen state to await incoming SYN connections.
    pub fn listen(&mut self) -> Result<(), TcpError> {
        self.state.transition(TcpEvent::PassiveOpen)
    }

    /// Initiates a 3-way handshake (Active Open) by transitioning to SynSent and building a SYN segment.
    pub fn connect(&mut self, remote_ip: [u8; 4], remote_port: u16) -> Result<TcpHeader, TcpError> {
        self.remote_ip = remote_ip;
        self.remote_port = remote_port;
        self.state.transition(TcpEvent::ActiveOpen)?;

        let syn_header = TcpHeader::new(
            self.local_port,
            self.remote_port,
            self.local_seq,
            0,
            TCP_FLAG_SYN,
            self.window_size,
        );
        self.local_seq = self.local_seq.wrapping_add(1);
        Ok(syn_header)
    }

    /// Ingests an incoming TCP segment, updates connection state, and produces an immediate response segment if required.
    pub fn handle_segment(&mut self, header: &TcpHeader, payload: &[u8]) -> Option<TcpHeader> {
        match self.state {
            TcpState::Listen => {
                if (header.flags & TCP_FLAG_SYN) != 0 {
                    self.remote_port = header.src_port;
                    self.remote_seq = header.seq_num.wrapping_add(1);
                    let _ = self.state.transition(TcpEvent::RecvSyn);

                    let syn_ack = TcpHeader::new(
                        self.local_port,
                        self.remote_port,
                        self.local_seq,
                        self.remote_seq,
                        TCP_FLAG_SYN | TCP_FLAG_ACK,
                        self.window_size,
                    );
                    self.local_seq = self.local_seq.wrapping_add(1);
                    return Some(syn_ack);
                }
            }
            TcpState::SynSent => {
                if (header.flags & (TCP_FLAG_SYN | TCP_FLAG_ACK)) == (TCP_FLAG_SYN | TCP_FLAG_ACK) {
                    if header.ack_num == self.local_seq {
                        self.remote_seq = header.seq_num.wrapping_add(1);
                        let _ = self.state.transition(TcpEvent::RecvSynAck);

                        let ack = TcpHeader::new(
                            self.local_port,
                            self.remote_port,
                            self.local_seq,
                            self.remote_seq,
                            TCP_FLAG_ACK,
                            self.window_size,
                        );
                        return Some(ack);
                    }
                }
            }
            TcpState::SynReceived => {
                if (header.flags & TCP_FLAG_ACK) != 0 {
                    if header.ack_num == self.local_seq {
                        let _ = self.state.transition(TcpEvent::RecvAck);
                    }
                }
            }
            TcpState::Established => {
                if !payload.is_empty() {
                    self.rx_buffer.extend_from_slice(payload);
                    self.remote_seq = self.remote_seq.wrapping_add(payload.len() as u32);

                    let ack = TcpHeader::new(
                        self.local_port,
                        self.remote_port,
                        self.local_seq,
                        self.remote_seq,
                        TCP_FLAG_ACK,
                        self.window_size,
                    );
                    return Some(ack);
                }

                if (header.flags & TCP_FLAG_FIN) != 0 {
                    self.remote_seq = self.remote_seq.wrapping_add(1);
                    let _ = self.state.transition(TcpEvent::RecvFin);

                    let ack = TcpHeader::new(
                        self.local_port,
                        self.remote_port,
                        self.local_seq,
                        self.remote_seq,
                        TCP_FLAG_ACK,
                        self.window_size,
                    );
                    return Some(ack);
                }
            }
            _ => {}
        }

        None
    }

    /// Formats an outgoing data segment with ACK and PSH flags over an established connection.
    pub fn send_data(&mut self, data: &[u8]) -> Result<(TcpHeader, Vec<u8>), TcpError> {
        if !self.state.is_established() {
            return Err(TcpError::UnexpectedState);
        }

        let header = TcpHeader::new(
            self.local_port,
            self.remote_port,
            self.local_seq,
            self.remote_seq,
            TCP_FLAG_ACK | TCP_FLAG_PSH,
            self.window_size,
        );
        self.local_seq = self.local_seq.wrapping_add(data.len() as u32);
        Ok((header, data.to_vec()))
    }
}
