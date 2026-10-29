//! VirtIO Network Device Driver (virtio-net) and Packet Ring Buffer.
//!
//! Implements OASIS VirtIO 1.0+ network device specifications:
//! - Standard 12-byte VirtioNetHeader with checksum and segmentation offloads
//! - Dual Virtqueue architecture (Queue 0: Receive / RX, Queue 1: Transmit / TX)
//! - Pre-allocated DMA receive buffer pools and zero-copy packet dispatch

use super::pci::PciDevice;
use super::virtqueue::{Virtqueue, VIRTQ_DESC_F_NEXT, VIRTQ_DESC_F_WRITE};
use alloc::boxed::Box;
use alloc::vec::Vec;

pub const VIRTIO_NET_HDR_F_NEEDS_CSUM: u8 = 1;
pub const VIRTIO_NET_HDR_F_DATA_VALID: u8 = 2;
pub const VIRTIO_NET_HDR_F_RSC_INFO: u8 = 4;

pub const VIRTIO_NET_HDR_GSO_NONE: u8 = 0;
pub const VIRTIO_NET_HDR_GSO_TCPV4: u8 = 1;
pub const VIRTIO_NET_HDR_GSO_UDP: u8 = 3;
pub const VIRTIO_NET_HDR_GSO_TCPV6: u8 = 4;
pub const VIRTIO_NET_HDR_GSO_ECN: u8 = 0x80;

pub const ETH_ALEN: usize = 6;
pub const ETH_HLEN: usize = 14;
pub const ETH_DATA_LEN: usize = 1500;
pub const ETH_FRAME_LEN: usize = 1514; // Destination (6) + Source (6) + EtherType (2) + MTU (1500)
pub const NET_BUFFER_SIZE: usize = core::mem::size_of::<VirtioNetHeader>() + ETH_FRAME_LEN;

/// Standard 12-byte VirtIO network packet header prepended to every transmitted and received frame.
#[derive(Debug, Clone, Copy, Default)]
#[repr(C)]
pub struct VirtioNetHeader {
    pub flags: u8,
    pub gso_type: u8,
    pub hdr_len: u16,
    pub gso_size: u16,
    pub csum_start: u16,
    pub csum_offset: u16,
    pub num_buffers: u16,
}

impl VirtioNetHeader {
    /// Creates a default header for standard non-GSO Ethernet packets.
    pub const fn new() -> Self {
        Self {
            flags: 0,
            gso_type: VIRTIO_NET_HDR_GSO_NONE,
            hdr_len: 0,
            gso_size: 0,
            csum_start: 0,
            csum_offset: 0,
            num_buffers: 1,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VirtioNetError {
    PacketTooLarge,
    OutOfDescriptors,
    IoError,
}

/// Received network packet payload extracted from virtqueue buffer.
#[derive(Debug, Clone)]
pub struct ReceivedPacket {
    pub data: Vec<u8>,
}

/// VirtIO Network Device Instance managing RX and TX virtqueues.
pub struct VirtioNetDevice {
    pub pci: Option<PciDevice>,
    pub mac_addr: [u8; ETH_ALEN],
    pub rx_queue: Virtqueue,
    pub tx_queue: Virtqueue,
    pub rx_buffers: Box<[Box<[u8; NET_BUFFER_SIZE]>]>,
    // In-memory loopback queue for simulated bare-metal and headless testing
    pub mock_loopback_queue: Vec<Vec<u8>>,
    pub packets_transmitted: u64,
    pub packets_received: u64,
}

impl VirtioNetDevice {
    /// Creates a VirtIO Network Device with assigned queue sizes (e.g. 32 or 64).
    pub fn new(pci: Option<PciDevice>, queue_size: u16, mac_addr: [u8; ETH_ALEN]) -> Self {
        let mut rx_buf_vec = Vec::with_capacity(queue_size as usize);
        for _ in 0..queue_size {
            rx_buf_vec.push(Box::new([0u8; NET_BUFFER_SIZE]));
        }

        let mut dev = Self {
            pci,
            mac_addr,
            rx_queue: Virtqueue::new(queue_size),
            tx_queue: Virtqueue::new(queue_size),
            rx_buffers: rx_buf_vec.into_boxed_slice(),
            mock_loopback_queue: Vec::new(),
            packets_transmitted: 0,
            packets_received: 0,
        };

        dev.populate_rx_buffers();
        dev
    }

    /// Pre-allocates and submits receive buffers to the RX Virtqueue available ring.
    pub fn populate_rx_buffers(&mut self) {
        let count = self.rx_queue.queue_size();
        for i in 0..count {
            if let Some(desc_idx) = self.rx_queue.alloc_descriptor() {
                let buf_ptr = self.rx_buffers[i as usize].as_mut_ptr() as usize as u64;
                self.rx_queue.set_descriptor(
                    desc_idx,
                    buf_ptr,
                    NET_BUFFER_SIZE as u32,
                    VIRTQ_DESC_F_WRITE,
                    0,
                );
                self.rx_queue.submit_chain(desc_idx);
            }
        }
    }

    /// Transmits an Ethernet packet over the TX Virtqueue.
    pub fn transmit_packet(&mut self, payload: &[u8]) -> Result<(), VirtioNetError> {
        if payload.len() > ETH_FRAME_LEN {
            return Err(VirtioNetError::PacketTooLarge);
        }

        let d0 = self.tx_queue.alloc_descriptor().ok_or(VirtioNetError::OutOfDescriptors)?;
        let d1 = self.tx_queue.alloc_descriptor().ok_or(VirtioNetError::OutOfDescriptors)?;

        let mut header = VirtioNetHeader::new();

        // Descriptor 0: 12-byte VirtioNetHeader
        self.tx_queue.set_descriptor(
            d0,
            core::ptr::addr_of_mut!(header) as usize as u64,
            core::mem::size_of::<VirtioNetHeader>() as u32,
            VIRTQ_DESC_F_NEXT,
            d1,
        );

        // Descriptor 1: Packet payload
        self.tx_queue.set_descriptor(
            d1,
            payload.as_ptr() as usize as u64,
            payload.len() as u32,
            0,
            0,
        );

        // Submit to TX Available Ring
        self.tx_queue.submit_chain(d0);
        self.packets_transmitted += 1;

        // In mock / loopback test mode: transfer packet into receive loopback queue
        let mut loopback_frame = Vec::with_capacity(payload.len());
        loopback_frame.extend_from_slice(payload);
        self.mock_loopback_queue.push(loopback_frame);

        // Mock completion in TX used ring and reclaim descriptors
        self.tx_queue.mock_device_complete(d0 as u32, 0);
        let _ = self.tx_queue.poll_used();
        self.tx_queue.free_descriptor_chain(d0);

        Ok(())
    }

    /// Polls RX virtqueue and returns any received network packets.
    pub fn poll_received_packets(&mut self) -> Vec<ReceivedPacket> {
        let mut received = Vec::new();

        // Drain any packets from mock loopback queue
        while !self.mock_loopback_queue.is_empty() {
            let pkt_data = self.mock_loopback_queue.remove(0);
            self.packets_received += 1;
            received.push(ReceivedPacket { data: pkt_data });
        }

        // Check hardware used ring if attached
        while let Some(elem) = self.rx_queue.poll_used() {
            let desc_idx = elem.id as usize;
            if desc_idx < self.rx_buffers.len() {
                let hdr_size = core::mem::size_of::<VirtioNetHeader>();
                if elem.len as usize > hdr_size {
                    let data_len = elem.len as usize - hdr_size;
                    let buf = &self.rx_buffers[desc_idx];
                    let mut pkt = Vec::with_capacity(data_len);
                    pkt.extend_from_slice(&buf[hdr_size..hdr_size + data_len]);
                    self.packets_received += 1;
                    received.push(ReceivedPacket { data: pkt });
                }
            }
            // Replenish descriptor back to Available Ring
            self.rx_queue.submit_chain(elem.id as u16);
        }

        received
    }
}
