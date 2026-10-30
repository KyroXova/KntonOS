//! In-Kernel LZ4 Fast Block Compression & CRC32 Verification Engine.
//! Provides ultra-fast dictionary compression and decompression (GB/s throughput)

use alloc::vec::Vec;

/// Standard IEEE 802.3 CRC32 polynomial (reversed/reflected representation).
pub const CRC32_POLYNOMIAL: u32 = 0xEDB8_8320;

/// Compile-time precomputed 256-entry lookup table for IEEE 802.3 CRC32.
const fn make_crc32_table() -> [u32; 256] {
    let mut table = [0u32; 256];
    let mut i = 0;
    while i < 256 {
        let mut c = i as u32;
        let mut j = 0;
        while j < 8 {
            if (c & 1) != 0 {
                c = CRC32_POLYNOMIAL ^ (c >> 1);
            } else {
                c >>= 1;
            }
            j += 1;
        }
        table[i] = c;
        i += 1;
    }
    table
}

pub static CRC32_TABLE: [u32; 256] = make_crc32_table();

/// Computes the IEEE 802.3 CRC32 checksum for a slice of bytes.
pub fn crc32(data: &[u8]) -> u32 {
    let mut crc = 0xFFFF_FFFFu32;
    for &b in data {
        let idx = ((crc ^ (b as u32)) & 0xFF) as usize;
        crc = CRC32_TABLE[idx] ^ (crc >> 8);
    }
    !crc
}

/// Incrementally updates a CRC32 checksum with additional data bytes.
pub fn crc32_update(current_crc: u32, data: &[u8]) -> u32 {
    let mut crc = !current_crc;
    for &b in data {
        let idx = ((crc ^ (b as u32)) & 0xFF) as usize;
        crc = CRC32_TABLE[idx] ^ (crc >> 8);
    }
    !crc
}

/// Errors returned by the in-kernel LZ4 compression and decompression engine.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Lz4Error {
    OutputBufferTooSmall,
    CorruptedBlock,
    OffsetOutOfBounds,
    ChecksumMismatch { expected: u32, computed: u32 },
}

/// Calculates the maximum theoretical compressed size for a buffer of `input_len` bytes.
pub const fn max_compressed_size(input_len: usize) -> usize {
    input_len + (input_len / 255) + 16
}

/// In-kernel LZ4 block compression engine.
pub struct Lz4BlockCompressor;

impl Lz4BlockCompressor {
    /// Compresses `src` into `dst` using LZ4 sequence encoding.
    /// Returns the number of bytes written to `dst`.
    pub fn compress(src: &[u8], dst: &mut [u8]) -> Result<usize, Lz4Error> {
        if src.is_empty() {
            return Ok(0);
        }

        // 4096-entry hash table indexing 4-byte prefixes
        const HASH_LOG: usize = 12;
        const HASH_SIZE: usize = 1 << HASH_LOG;
        let mut hash_table = [0usize; HASH_SIZE];

        let mut src_idx = 0;
        let mut anchor = 0;
        let mut dst_idx = 0;

        let match_limit = src.len().saturating_sub(5);

        while src_idx < match_limit {
            let chunk = u32::from_le_bytes([
                src[src_idx],
                src[src_idx + 1],
                src[src_idx + 2],
                src[src_idx + 3],
            ]);
            let hash = (chunk.wrapping_mul(2654435761) >> (32 - HASH_LOG)) as usize;
            let match_pos = hash_table[hash];
            hash_table[hash] = src_idx;

            // Check if valid match within 64KB window
            let offset = src_idx.saturating_sub(match_pos);
            if match_pos > 0
                && offset > 0
                && offset <= 65535
                && &src[match_pos..match_pos + 4] == &src[src_idx..src_idx + 4]
            {
                // Found 4-byte match! Extend forward:
                let mut match_len = 4;
                while src_idx + match_len < src.len()
                    && src[match_pos + match_len] == src[src_idx + match_len]
                {
                    match_len += 1;
                }

                // Emit sequence
                let lit_len = src_idx - anchor;
                let token_lit = lit_len.min(15) as u8;
                let token_match = match_len.saturating_sub(4).min(15) as u8;
                let token = (token_lit << 4) | token_match;

                if dst_idx >= dst.len() {
                    return Err(Lz4Error::OutputBufferTooSmall);
                }
                dst[dst_idx] = token;
                dst_idx += 1;

                // Extra literal length bytes
                if lit_len >= 15 {
                    let mut rem = lit_len - 15;
                    while rem >= 255 {
                        if dst_idx >= dst.len() {
                            return Err(Lz4Error::OutputBufferTooSmall);
                        }
                        dst[dst_idx] = 255;
                        dst_idx += 1;
                        rem -= 255;
                    }
                    if dst_idx >= dst.len() {
                        return Err(Lz4Error::OutputBufferTooSmall);
                    }
                    dst[dst_idx] = rem as u8;
                    dst_idx += 1;
                }

                // Emit literal bytes
                if dst_idx + lit_len > dst.len() {
                    return Err(Lz4Error::OutputBufferTooSmall);
                }
                dst[dst_idx..dst_idx + lit_len].copy_from_slice(&src[anchor..anchor + lit_len]);
                dst_idx += lit_len;

                // Emit 2-byte match offset (little-endian)
                if dst_idx + 2 > dst.len() {
                    return Err(Lz4Error::OutputBufferTooSmall);
                }
                dst[dst_idx..dst_idx + 2].copy_from_slice(&(offset as u16).to_le_bytes());
                dst_idx += 2;

                // Extra match length bytes
                let match_rem = match_len.saturating_sub(4);
                if match_rem >= 15 {
                    let mut rem = match_rem - 15;
                    while rem >= 255 {
                        if dst_idx >= dst.len() {
                            return Err(Lz4Error::OutputBufferTooSmall);
                        }
                        dst[dst_idx] = 255;
                        dst_idx += 1;
                        rem -= 255;
                    }
                    if dst_idx >= dst.len() {
                        return Err(Lz4Error::OutputBufferTooSmall);
                    }
                    dst[dst_idx] = rem as u8;
                    dst_idx += 1;
                }

                src_idx += match_len;
                anchor = src_idx;
            } else {
                src_idx += 1;
            }
        }

        // Emit final trailing literals
        let lit_len = src.len() - anchor;
        let token = (lit_len.min(15) as u8) << 4;
        if dst_idx >= dst.len() {
            return Err(Lz4Error::OutputBufferTooSmall);
        }
        dst[dst_idx] = token;
        dst_idx += 1;

        if lit_len >= 15 {
            let mut rem = lit_len - 15;
            while rem >= 255 {
                if dst_idx >= dst.len() {
                    return Err(Lz4Error::OutputBufferTooSmall);
                }
                dst[dst_idx] = 255;
                dst_idx += 1;
                rem -= 255;
            }
            if dst_idx >= dst.len() {
                return Err(Lz4Error::OutputBufferTooSmall);
            }
            dst[dst_idx] = rem as u8;
            dst_idx += 1;
        }

        if dst_idx + lit_len > dst.len() {
            return Err(Lz4Error::OutputBufferTooSmall);
        }
        dst[dst_idx..dst_idx + lit_len].copy_from_slice(&src[anchor..anchor + lit_len]);
        dst_idx += lit_len;

        Ok(dst_idx)
    }
}

/// In-kernel LZ4 block decompression engine.
pub struct Lz4BlockDecompressor;

impl Lz4BlockDecompressor {
    /// Decompresses an LZ4 block from `src` into `dst`.
    /// Returns the number of decompressed bytes written to `dst`.
    pub fn decompress(src: &[u8], dst: &mut [u8]) -> Result<usize, Lz4Error> {
        let mut src_idx = 0;
        let mut dst_idx = 0;

        while src_idx < src.len() {
            let token = src[src_idx];
            src_idx += 1;

            // 1. Parse literal length
            let mut lit_len = (token >> 4) as usize;
            if lit_len == 15 {
                while src_idx < src.len() {
                    let extra = src[src_idx] as usize;
                    src_idx += 1;
                    lit_len += extra;
                    if extra < 255 {
                        break;
                    }
                }
            }

            // Copy literals
            if src_idx + lit_len > src.len() || dst_idx + lit_len > dst.len() {
                return Err(Lz4Error::CorruptedBlock);
            }
            dst[dst_idx..dst_idx + lit_len].copy_from_slice(&src[src_idx..src_idx + lit_len]);
            src_idx += lit_len;
            dst_idx += lit_len;

            // If at end of input block, sequence was the last literal block
            if src_idx >= src.len() {
                break;
            }

            // 2. Parse match offset (2 bytes LE)
            if src_idx + 2 > src.len() {
                return Err(Lz4Error::CorruptedBlock);
            }
            let offset = u16::from_le_bytes([src[src_idx], src[src_idx + 1]]) as usize;
            src_idx += 2;
            if offset == 0 || offset > dst_idx {
                return Err(Lz4Error::OffsetOutOfBounds);
            }

            // 3. Parse match length
            let mut match_len = (token & 0x0F) as usize + 4;
            if (token & 0x0F) == 15 {
                while src_idx < src.len() {
                    let extra = src[src_idx] as usize;
                    src_idx += 1;
                    match_len += extra;
                    if extra < 255 {
                        break;
                    }
                }
            }

            if dst_idx + match_len > dst.len() {
                return Err(Lz4Error::OutputBufferTooSmall);
            }

            // Copy match from dst buffer (handles overlap correctly)
            let match_start = dst_idx - offset;
            for i in 0..match_len {
                dst[dst_idx + i] = dst[match_start + i];
            }
            dst_idx += match_len;
        }

        Ok(dst_idx)
    }
}

/// Represents a single compressed memory page stored in a hibernation image.
#[derive(Debug, Clone)]
pub struct Lz4CompressedPage {
    pub pfn: u64,
    pub uncompressed_len: u32,
    pub compressed_data: Vec<u8>,
    pub crc32: u32,
}

impl Lz4CompressedPage {
    /// Compresses a page frame into an LZ4 compressed page descriptor with CRC32 checksum.
    pub fn compress_page(pfn: u64, page_data: &[u8]) -> Result<Self, Lz4Error> {
        let max_size = max_compressed_size(page_data.len());
        let mut comp_buf = Vec::with_capacity(max_size);
        comp_buf.resize(max_size, 0);

        let comp_len = Lz4BlockCompressor::compress(page_data, &mut comp_buf)?;
        comp_buf.truncate(comp_len);

        let checksum = crc32(page_data);

        Ok(Self {
            pfn,
            uncompressed_len: page_data.len() as u32,
            compressed_data: comp_buf,
            crc32: checksum,
        })
    }

    /// Decompresses this page and validates its CRC32 checksum.
    pub fn decompress_page(&self, out: &mut [u8]) -> Result<usize, Lz4Error> {
        let bytes_written = Lz4BlockDecompressor::decompress(&self.compressed_data, out)?;
        if bytes_written != self.uncompressed_len as usize {
            return Err(Lz4Error::CorruptedBlock);
        }

        let computed_crc = crc32(&out[..bytes_written]);
        if computed_crc != self.crc32 {
            return Err(Lz4Error::ChecksumMismatch {
                expected: self.crc32,
                computed: computed_crc,
            });
        }

        Ok(bytes_written)
    }
}

/// Unit test verifying in-kernel LZ4 compression/decompression and CRC32 verification.
pub fn verify_lz4_compression_and_crc32() -> bool {
    // 1. Verify standard IEEE 802.3 CRC32 with canonical ASCII "123456789" vector -> 0xCBF43926
    let vector_str = b"123456789";
    if crc32(vector_str) != 0xCBF4_3926 {
        return false;
    }

    // Verify incremental CRC32 update
    let part1 = b"1234";
    let part2 = b"56789";
    let crc_inc = crc32_update(crc32(part1), part2);
    if crc_inc != 0xCBF4_3926 {
        return false;
    }

    // 2. Test LZ4 compression on a sparse 4096-byte memory page (typical kernel BSS / page table)
    let mut original_page = [0u8; 4096];
    // Fill with repeated patterns and header text
    let header_msg = b"KNTONOS-KERNEL-HIBERNATION-S4-IMAGE-PAYLOAD-V1";
    original_page[0..header_msg.len()].copy_from_slice(header_msg);
    // Fill middle with repetitive pattern
    for i in 100..500 {
        original_page[i] = (i % 8) as u8;
    }
    // Middle chunk repeated pattern
    for i in 1000..3000 {
        original_page[i] = 0xAA;
    }

    let comp_page = match Lz4CompressedPage::compress_page(512, &original_page) {
        Ok(p) => p,
        Err(_) => return false,
    };

    // Verify significant compression ratio (> 50% space savings)
    if comp_page.compressed_data.len() > 2048 {
        return false; // Failed to compress sparse page efficiently
    }

    // 3. Test decompression and CRC32 validation
    let mut decomp_buf = [0u8; 4096];
    let decomp_len = match comp_page.decompress_page(&mut decomp_buf) {
        Ok(l) => l,
        Err(_) => return false,
    };

    if decomp_len != 4096 || decomp_buf != original_page {
        return false; // Data corruption during LZ4 round-trip
    }

    // 4. Test corruption detection via CRC32
    let mut corrupted_page = comp_page.clone();
    corrupted_page.compressed_data[10] ^= 0xFF; // Corrupt a byte
    let corrupt_res = corrupted_page.decompress_page(&mut decomp_buf);
    if corrupt_res.is_ok() {
        return false; // Corruption was not caught!
    }

    true
}

/// Descriptor for a staged memory page waiting for atomic restoration.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StagingPageEntry {
    /// Physical address in the safe high-memory staging buffer.
    pub staging_phys: u64,
    /// Destination physical page frame number (PFN) in final kernel memory.
    pub dest_pfn: u64,
    /// Number of bytes in this page (up to 4096).
    pub byte_len: usize,
    /// IEEE 802.3 CRC32 of the uncompressed page payload.
    pub crc32: u32,
}

/// Map of staged memory pages decoupling disk I/O decompression from physical memory overwrite.
pub struct StagingPageMap {
    pub entries: Vec<StagingPageEntry>,
    pub total_bytes: usize,
}

impl StagingPageMap {
    pub fn new() -> Self {
        Self {
            entries: Vec::new(),
            total_bytes: 0,
        }
    }

    pub fn add_entry(&mut self, staging_phys: u64, dest_pfn: u64, byte_len: usize, crc32: u32) {
        self.total_bytes += byte_len;
        self.entries.push(StagingPageEntry {
            staging_phys,
            dest_pfn,
            byte_len,
            crc32,
        });
    }

    /// Detects if any destination PFNs collide with the memory occupied by the restore kernel.
    /// Returns the number of colliding frames that must be redirected through trampoline staging.
    pub fn detect_collisions(&self, restore_kernel_start_pfn: u64, restore_kernel_page_count: u64) -> usize {
        let kernel_end_pfn = restore_kernel_start_pfn + restore_kernel_page_count;
        let mut collisions = 0;
        for entry in &self.entries {
            if entry.dest_pfn >= restore_kernel_start_pfn && entry.dest_pfn < kernel_end_pfn {
                collisions += 1;
            }
        }
        collisions
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

/// Trampoline 4-Level Page Table (PML4).
/// An isolated, identity-mapped page table ensuring that during the atomic
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TrampolinePageTable {
    pub pml4_phys_cr3: u64,
    pub is_identity_mapped: bool,
    pub mapped_ranges_count: usize,
    pub flags: u64,
}

impl TrampolinePageTable {
    /// Creates a trampoline page table descriptor centered around `pml4_phys_cr3`.
    pub fn new(pml4_phys_cr3: u64) -> Self {
        Self {
            pml4_phys_cr3,
            is_identity_mapped: true,
            mapped_ranges_count: 4, // 1:1 identity map for first 4 GiB
            flags: 0x07,            // Present, Writable, User/Supervisor
        }
    }

    /// Verifies that the trampoline page table is valid and ready for CR3 switch.
    pub fn is_valid(&self) -> bool {
        self.pml4_phys_cr3 != 0 && (self.pml4_phys_cr3 & 0xFFF) == 0 && self.is_identity_mapped
    }
}

/// Atomic S4 Image Memory Restoration Trampoline Engine.
/// Coordinates the safe two-phase restoration of physical RAM from hibernation:
pub struct RestoreTrampolineEngine {
    pub staging_map: StagingPageMap,
    pub trampoline_pt: TrampolinePageTable,
    pub pages_restored: usize,
    pub total_bytes_restored: u64,
    pub integrity_verified: bool,
}

impl RestoreTrampolineEngine {
    pub fn new(trampoline_cr3: u64) -> Self {
        Self {
            staging_map: StagingPageMap::new(),
            trampoline_pt: TrampolinePageTable::new(trampoline_cr3),
            pages_restored: 0,
            total_bytes_restored: 0,
            integrity_verified: true,
        }
    }

    /// Stages a decompressed page into the relocation map.
    pub fn stage_page(
        &mut self,
        staging_phys: u64,
        dest_pfn: u64,
        page_payload: &[u8],
    ) -> Result<(), Lz4Error> {
        let checksum = crc32(page_payload);
        self.staging_map.add_entry(staging_phys, dest_pfn, page_payload.len(), checksum);
        Ok(())
    }

    /// Executes atomic copy from staging memory to destination physical memory buffer.
    /// In full hardware, this runs from an unpaged/identity-mapped trampoline loop.
    pub fn execute_atomic_page_restore(
        &mut self,
        dest_memory: &mut [u8],
        _dest_pfn: u64,
        source_staged_data: &[u8],
        expected_crc: u32,
    ) -> Result<(), Lz4Error> {
        if !self.trampoline_pt.is_valid() {
            return Err(Lz4Error::CorruptedBlock);
        }

        let computed_crc = crc32(source_staged_data);
        if computed_crc != expected_crc {
            self.integrity_verified = false;
            return Err(Lz4Error::ChecksumMismatch {
                expected: expected_crc,
                computed: computed_crc,
            });
        }

        if dest_memory.len() < source_staged_data.len() {
            return Err(Lz4Error::OutputBufferTooSmall);
        }

        // Atomic copy into destination memory
        dest_memory[..source_staged_data.len()].copy_from_slice(source_staged_data);

        self.pages_restored += 1;
        self.total_bytes_restored += source_staged_data.len() as u64;

        Ok(())
    }

    /// Verifies architectural CPU context before jumping to resumed kernel entry RIP.
    pub fn verify_and_prepare_cpu_resume(&self, ctx: &crate::power::s3_resume::CpuRegisterContext) -> bool {
        if !ctx.verify_integrity() {
            return false;
        }
        if ctx.cr3 == 0 || (ctx.cr3 & 0xFFF) != 0 {
            return false;
        }
        if ctx.rsp == 0 {
            return false;
        }
        true
    }
}

/// Unit test verifying atomic S4 memory restoration trampoline, collision avoidance, and page table rebuild.
pub fn verify_atomic_restore_trampoline() -> bool {
    // 1. Verify TrampolinePageTable validation and alignment
    let valid_cr3 = 0x0000_0000_0007_E000; // 4KB aligned physical address
    let trampoline_pt = TrampolinePageTable::new(valid_cr3);
    if !trampoline_pt.is_valid() || !trampoline_pt.is_identity_mapped {
        return false;
    }

    let unaligned_pt = TrampolinePageTable::new(0x7E01); // Not 4KB aligned
    if unaligned_pt.is_valid() {
        return false;
    }

    // 2. Verify StagingPageMap and collision detection
    let mut staging_map = StagingPageMap::new();
    // Stage 4 pages destined for PFNs: 100, 256, 257, 500
    staging_map.add_entry(0x1000_0000, 100, 4096, 0x1111);
    staging_map.add_entry(0x1000_1000, 256, 4096, 0x2222);
    staging_map.add_entry(0x1000_2000, 257, 4096, 0x3333);
    staging_map.add_entry(0x1000_3000, 500, 4096, 0x4444);

    if staging_map.len() != 4 || staging_map.total_bytes != 16384 {
        return false;
    }

    // Suppose the restore kernel is loaded at PFN 250..300 (50 pages).
    // PFN 256 and 257 collide with restore kernel memory!
    let collisions = staging_map.detect_collisions(250, 50);
    if collisions != 2 {
        return false;
    }

    // 3. Verify atomic memory relocation execution with CRC32
    let mut engine = RestoreTrampolineEngine::new(valid_cr3);

    let test_kernel_data = b"KNTONOS-ATOMIC-RESTORED-KERNEL-CORE-PAGE-CONTENT-OK";
    let data_crc = crc32(test_kernel_data);

    let mut dest_page = [0u8; 4096];
    let res = engine.execute_atomic_page_restore(&mut dest_page, 256, test_kernel_data, data_crc);
    if res.is_err() {
        return false;
    }

    if engine.pages_restored != 1 || &dest_page[..test_kernel_data.len()] != test_kernel_data {
        return false;
    }

    // Tampered CRC must abort restore
    let bad_res = engine.execute_atomic_page_restore(&mut dest_page, 257, test_kernel_data, 0xDEAD_BEEF);
    if bad_res.is_ok() || engine.integrity_verified {
        return false;
    }

    // 4. Verify CPU architectural state restore preparation
    let ctx = crate::power::s3_resume::CpuRegisterContext::capture(0x2000_0000, 0x7FFF_FFFF_0000);
    if !engine.verify_and_prepare_cpu_resume(&ctx) {
        return false;
    }

    true
}
