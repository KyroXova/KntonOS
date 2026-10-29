//! VirtIO Network Device Driver (virtio-net) and Packet Ring Buffer.
//!
//! Implements OASIS VirtIO 1.0+ network device specifications:
//! - Standard 12-byte VirtioNetHeader with checksum and segmentation offloads
//! - Dual Virtqueue architecture (Queue 0: Receive / RX, Queue 1: Transmit / TX)
//! - Pre-allocated DMA receive buffer pools and zero-copy packet dispatch

use super::pci::PciDevice;
use super::virtqueue::{Virtqueue, VIRTQ_DESC_F_WRITE};
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

/// VirtIO Network Device Instance managing RX and TX virtqueues.
pub struct VirtioNetDevice {
    pub pci: Option<PciDevice>,
    pub mac_addr: [u8; ETH_ALEN],
    pub rx_queue: Virtqueue,
    pub tx_queue: Virtqueue,
    pub rx_buffers: Box<[Box<[u8; NET_BUFFER_SIZE]>]>,
    // In-memory loopback queue for simulated bare-metal and headless testing
    pub mock_loopback_queue: Vec<Vec<u8>>,
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
}
