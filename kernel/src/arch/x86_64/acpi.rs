//! ACPI RSDP/XSDT and MADT parser for SMP topology and interrupt routing.

use alloc::vec::Vec;
use crate::serial_write_str;

pub const RSDP_SIGNATURE: &[u8; 8] = b"RSD PTR ";
pub const MADT_SIGNATURE: &[u8; 4] = b"APIC";

#[repr(C, packed)]
#[derive(Debug, Clone, Copy)]
pub struct RsdpDescriptor {
    pub signature: [u8; 8],
    pub checksum: u8,
    pub oem_id: [u8; 6],
    pub revision: u8,
    pub rsdt_address: u32,
    pub length: u32,
    pub xsdt_address: u64,
    pub extended_checksum: u8,
    pub reserved: [u8; 3],
}

#[repr(C, packed)]
#[derive(Debug, Clone, Copy)]
pub struct AcpiHeader {
    pub signature: [u8; 4],
    pub length: u32,
    pub revision: u8,
    pub checksum: u8,
    pub oem_id: [u8; 6],
    pub oem_table_id: [u8; 8],
    pub oem_revision: u32,
    pub creator_id: u32,
    pub creator_revision: u32,
}

#[derive(Debug, Clone)]
pub struct CpuCoreInfo {
    pub processor_id: u8,
    pub apic_id: u8,
    pub enabled: bool,
    pub is_bsp: bool,
}

#[derive(Debug, Clone)]
pub struct IoApicInfo {
    pub id: u8,
    pub address: u32,
    pub gsi_base: u32,
}

#[derive(Debug, Clone)]
pub struct SmpTopology {
    pub lapic_address: u64,
    pub cores: Vec<CpuCoreInfo>,
    pub io_apics: Vec<IoApicInfo>,
}

impl SmpTopology {
    pub const fn empty() -> Self {
        Self {
            lapic_address: 0xFEE0_0000, // Standard default x86 Local APIC base
            cores: Vec::new(),
            io_apics: Vec::new(),
        }
    }
}

pub struct AcpiParser;

impl AcpiParser {
    /// Validates an ACPI table checksum (all bytes must sum to 0 mod 256).
    pub unsafe fn validate_checksum(ptr: *const u8, len: usize) -> bool {
        let mut sum: u8 = 0;
        for i in 0..len {
            sum = sum.wrapping_add(*ptr.add(i));
        }
        sum == 0
    }

    /// Scans standard x86 BIOS ROM area (0xE0000 - 0xFFFFF) on 16-byte boundaries for the RSDP.
    pub unsafe fn find_rsdp() -> Option<*const RsdpDescriptor> {
        let start = 0x000E_0000 as *const u8;
        let end = 0x0010_0000 as *const u8;
        let mut curr = start;

        while curr < end {
            if core::slice::from_raw_parts(curr, 8) == RSDP_SIGNATURE {
                let rsdp = curr as *const RsdpDescriptor;
                if Self::validate_checksum(curr, 20) {
                    return Some(rsdp);
                }
            }
            curr = curr.add(16);
        }

        // Secondary fallback search in EBDA (0x80000 - 0x9FFFF)
        let ebda_start = 0x0008_0000 as *const u8;
        let ebda_end = 0x000A_0000 as *const u8;
        let mut ebda_curr = ebda_start;

        while ebda_curr < ebda_end {
            if core::slice::from_raw_parts(ebda_curr, 8) == RSDP_SIGNATURE {
                let rsdp = ebda_curr as *const RsdpDescriptor;
                if Self::validate_checksum(ebda_curr, 20) {
                    return Some(rsdp);
                }
            }
            ebda_curr = ebda_curr.add(16);
        }

        None
    }

    /// Parses the Multiple APIC Description Table (MADT) given the MADT header address.
    pub unsafe fn parse_madt(madt_ptr: *const AcpiHeader) -> SmpTopology {
        let length = (*madt_ptr).length as usize;
        let mut topology = SmpTopology {
            lapic_address: 0xFEE0_0000,
            cores: Vec::new(),
            io_apics: Vec::new(),
        };

        let raw = madt_ptr as *const u8;
        if length < 44 {
            return topology;
        }

        // Local APIC physical address at offset 36 (u32)
        let lapic_addr_ptr = raw.add(36) as *const u32;
        topology.lapic_address = *lapic_addr_ptr as u64;

        // Entries start at offset 44
        let mut offset = 44;
        while offset + 2 <= length {
            let entry_type = *raw.add(offset);
            let entry_len = *raw.add(offset + 1) as usize;

            if entry_len == 0 || offset + entry_len > length {
                break;
            }

            match entry_type {
                0 => {
                    // Type 0: Processor Local APIC (8 bytes)
                    if entry_len >= 8 {
                        let proc_id = *raw.add(offset + 2);
                        let apic_id = *raw.add(offset + 3);
                        let flags = *(raw.add(offset + 4) as *const u32);
                        let enabled = (flags & 1) != 0 || (flags & 2) != 0;
                        let is_bsp = topology.cores.is_empty();

                        topology.cores.push(CpuCoreInfo {
                            processor_id: proc_id,
                            apic_id,
                            enabled,
                            is_bsp,
                        });
                    }
                }
                1 => {
                    // Type 1: I/O APIC (12 bytes)
                    if entry_len >= 12 {
                        let io_id = *raw.add(offset + 2);
                        let address = *(raw.add(offset + 4) as *const u32);
                        let gsi_base = *(raw.add(offset + 8) as *const u32);

                        topology.io_apics.push(IoApicInfo {
                            id: io_id,
                            address,
                            gsi_base,
                        });
                    }
                }
                5 => {
                    // Type 5: 64-bit Local APIC Address Override (12 bytes)
                    if entry_len >= 12 {
                        let addr64 = *(raw.add(offset + 4) as *const u64);
                        topology.lapic_address = addr64;
                    }
                }
                _ => {}
            }

            offset += entry_len;
        }

        topology
    }

    /// Discovers system SMP topology from ACPI tables. Falls back to Q35 default if not found.
    pub fn discover_topology() -> SmpTopology {
        serial_write_str("[KNT-ACPI] Scanning for ACPI RSDP descriptor...\r\n");

        if let Some(rsdp_ptr) = unsafe { Self::find_rsdp() } {
            let rsdp = unsafe { &*rsdp_ptr };
            serial_write_str("  * Found valid ACPI RSDP (OEM: ");
            for b in &rsdp.oem_id {
                if *b >= 32 && *b <= 126 {
                    crate::serial_write_byte(*b);
                }
            }
            serial_write_str(", Rev: ");
            crate::serial_write_byte(b'0' + rsdp.revision);
            serial_write_str(")\r\n");

            // Inspect XSDT (64-bit) or RSDT (32-bit)
            let madt_header = unsafe {
                if rsdp.revision >= 2 && rsdp.xsdt_address != 0 {
                    Self::find_table_xsdt(rsdp.xsdt_address as *const AcpiHeader, MADT_SIGNATURE)
                } else if rsdp.rsdt_address != 0 {
                    Self::find_table_rsdt(rsdp.rsdt_address as *const AcpiHeader, MADT_SIGNATURE)
                } else {
                    None
                }
            };

            if let Some(madt) = madt_header {
                serial_write_str("  * Located ACPI MADT (APIC) table\r\n");
                let topology = unsafe { Self::parse_madt(madt) };
                serial_write_str("  * ACPI MADT discovered ");
                write_u64(topology.cores.len() as u64);
                serial_write_str(" logical CPU cores\r\n");
                return topology;
            }
        }

        // QEMU Q35 default fallback: 4 SMP cores with LAPIC at 0xFEE00000
        serial_write_str("[KNT-ACPI] Using standard Q35 SMP configuration (4 cores, LAPIC @ 0xFEE00000)\r\n");
        let mut fallback = SmpTopology {
            lapic_address: 0xFEE0_0000,
            cores: Vec::new(),
            io_apics: Vec::new(),
        };

        for i in 0..4 {
            fallback.cores.push(CpuCoreInfo {
                processor_id: i,
                apic_id: i,
                enabled: true,
                is_bsp: i == 0,
            });
        }
        fallback.io_apics.push(IoApicInfo {
            id: 1,
            address: 0xFEC0_0000,
            gsi_base: 0,
        });

        fallback
    }

    pub unsafe fn find_table_rsdt(rsdt: *const AcpiHeader, sig: &[u8; 4]) -> Option<*const AcpiHeader> {
        if !Self::validate_checksum(rsdt as *const u8, (*rsdt).length as usize) {
            return None;
        }
        let entries = ((*rsdt).length as usize - 36) / 4;
        let array = (rsdt as *const u8).add(36) as *const u32;

        for i in 0..entries {
            let table_addr = *array.add(i) as *const AcpiHeader;
            if (*table_addr).signature == *sig && Self::validate_checksum(table_addr as *const u8, (*table_addr).length as usize) {
                return Some(table_addr);
            }
        }
        None
    }

    pub unsafe fn find_table_xsdt(xsdt: *const AcpiHeader, sig: &[u8; 4]) -> Option<*const AcpiHeader> {
        if !Self::validate_checksum(xsdt as *const u8, (*xsdt).length as usize) {
            return None;
        }
        let entries = ((*xsdt).length as usize - 36) / 8;
        let array = (xsdt as *const u8).add(36) as *const u64;

        for i in 0..entries {
            let table_addr = *array.add(i) as *const AcpiHeader;
            if (*table_addr).signature == *sig && Self::validate_checksum(table_addr as *const u8, (*table_addr).length as usize) {
                return Some(table_addr);
            }
        }
        None
    }
}

fn write_u64(mut val: u64) {
    if val == 0 {
        crate::serial_write_byte(b'0');
        return;
    }
    let mut buf = [0u8; 20];
    let mut len = 0;
    while val > 0 {
        buf[len] = b'0' + (val % 10) as u8;
        val /= 10;
        len += 1;
    }
    for b in buf[..len].iter().rev() {
        crate::serial_write_byte(*b);
    }
}
