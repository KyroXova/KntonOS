//! Memory Management Architecture (MM) - Transparent Huge Pages & NUMA Subsystem.
//!
//! Subsystem providing 2MB Transparent Huge Pages (THP), large page collapse/split,
//! zero-fill optimization, AMD 3D V-Cache topology detection, and NUMA core affinity.

pub mod thp;
pub mod numa;

pub use numa::{
    CoreAffinityHint, CpuVendor, MemoryAffinityPolicy, NumaCore, NumaDistanceMatrix, NumaNode,
    NumaNodeType, NumaTopologyManager, WorkloadProfile, MAX_NUMA_CORES, MAX_NUMA_NODES,
};
pub use thp::{
    HugePageAllocator, HugePageEntry, HugePageFlags, HugePhysFrame, HugeZeroPool, PageDirectoryTable,
    PageTable4KB, PageTableEntry4KB, ThpMode, ThpStats, TransparentHugePageDaemon,
    VirtualAddressComponents, OFFSET_MASK_2MB, PAGES_PER_2MB, PAGE_SIZE_2MB, PAGE_SIZE_4KB,
};

/// Unit test verifying 2MB physical allocator and PML4/PDPT/PD translation structures.
pub fn verify_thp_allocator_and_structures() -> bool {
    let mut allocator = HugePageAllocator::new();
    let base_phys = 0x0200_0000; // 32 MiB physical (2MB aligned)

    if !allocator.init(base_phys, 16) {
        return false;
    }

    if allocator.total_pages() != 16 || allocator.free_pages() != 16 {
        return false;
    }

    // Allocate 1st huge page
    let frame1 = match allocator.allocate_2mb_frame() {
        Some(f) => f,
        None => return false,
    };

    if frame1.base_address() != base_phys {
        return false;
    }

    // Allocate 2nd huge page
    let frame2 = match allocator.allocate_2mb_frame() {
        Some(f) => f,
        None => return false,
    };

    if frame2.base_address() != base_phys + PAGE_SIZE_2MB {
        return false;
    }

    if allocator.allocated_pages() != 2 || allocator.free_pages() != 14 {
        return false;
    }

    // Verify Page Directory 2MB mapping and translation
    let mut pd = PageDirectoryTable::new();
    let test_vaddr = 0x0000_7FFF_8020_4000; // Virtual address with offset 0x4000
    let flags = HugePageFlags::PRESENT | HugePageFlags::WRITABLE | HugePageFlags::USER_ACCESSIBLE;

    if pd.map_2mb(test_vaddr, frame1.base_address(), flags).is_err() {
        return false;
    }

    let translated = match pd.translate_2mb(test_vaddr) {
        Some(p) => p,
        None => return false,
    };

    if translated != frame1.base_address() + 0x4000 {
        return false;
    }

    // Unmap and verify
    let unmapped = match pd.unmap_2mb(test_vaddr) {
        Some(f) => f,
        None => return false,
    };

    if unmapped != frame1 {
        return false;
    }

    if pd.translate_2mb(test_vaddr).is_some() {
        return false;
    }

    // Deallocate and check pool counters
    if !allocator.deallocate_2mb_frame(frame1) || !allocator.deallocate_2mb_frame(frame2) {
        return false;
    }

    if allocator.free_pages() != 16 || allocator.allocated_pages() != 0 {
        return false;
    }

    true
}

/// Unit test verifying THP collapse daemon, split engine, and pre-zeroed pool optimization.
pub fn verify_thp_collapse_split_and_zero_pool() -> bool {
    let mut allocator = HugePageAllocator::new();
    let base_phys = 0x0400_0000; // 64 MiB physical (2MB aligned)

    if !allocator.init(base_phys, 16) {
        return false;
    }

    let mut daemon = TransparentHugePageDaemon::new(ThpMode::Always);

    // 1. Replenish pre-zeroed pool with 8 frames
    let replenished = daemon.zero_pool.replenish(&mut allocator);
    if replenished != 8 || daemon.zero_pool.available() != 8 {
        return false;
    }

    // 2. Build a simulated 512-entry 4KB Page Table
    let mut pt = PageTable4KB::new();
    let pt_flags = HugePageFlags::PRESENT | HugePageFlags::WRITABLE | HugePageFlags::USER_ACCESSIBLE;
    let mock_base = 0x0100_0000;
    for i in 0..PAGES_PER_2MB {
        pt.entries[i] = PageTableEntry4KB::new(mock_base + i * PAGE_SIZE_4KB, pt_flags);
    }

    if pt.present_count() != 512 || !pt.has_uniform_permissions() {
        return false;
    }

    // 3. Scan and collapse into a 2MB huge page
    let mut pd = PageDirectoryTable::new();
    let vaddr_collapse = 0x0000_7FFF_A000_0000;

    let huge_frame = match daemon.scan_and_collapse(&mut pd, &pt, vaddr_collapse, &mut allocator) {
        Ok(f) => f,
        Err(_) => return false,
    };

    if daemon.stats.pages_collapsed != 1 || daemon.stats.zero_pool_hits != 1 {
        return false;
    }

    // Validate that PD maps this address as 2MB huge page
    let trans_mid = match pd.translate_2mb(vaddr_collapse + 0x1000) {
        Some(addr) => addr,
        None => return false,
    };
    if trans_mid != huge_frame.base_address() + 0x1000 {
        return false;
    }

    // 4. Split the 2MB huge page back into 512 4KB entries
    let split_pt = match daemon.split_huge_page(&mut pd, vaddr_collapse, &mut allocator) {
        Ok(t) => t,
        Err(_) => return false,
    };

    if daemon.stats.pages_split != 1 {
        return false;
    }

    // PD entry should now be unmapped
    if pd.translate_2mb(vaddr_collapse).is_some() {
        return false;
    }

    // Verify reconstructed 4KB PTEs
    if split_pt.present_count() != 512 {
        return false;
    }
    if split_pt.entries[0].physical_address() != huge_frame.base_address() {
        return false;
    }
    if split_pt.entries[511].physical_address() != huge_frame.base_address() + 511 * PAGE_SIZE_4KB {
        return false;
    }

    true
}

/// Unit test verifying AMD Zen 3D V-Cache and Intel Hybrid topology discovery & affinity routing.
pub fn verify_numa_vcache_and_hybrid_topology() -> bool {
    // 1. AMD Zen 3D V-Cache Topology Verification
    let mut amd_topo = NumaTopologyManager::create_simulated_amd_x3d_topology();
    if amd_topo.vendor != CpuVendor::Amd || amd_topo.node_count != 2 || amd_topo.core_count != 16 {
        return false;
    }

    // Node 0 should be 3D V-Cache with 96 MiB L3
    let node0 = match amd_topo.nodes[0] {
        Some(n) => n,
        None => return false,
    };
    if node0.node_type != NumaNodeType::VCache3D || node0.l3_cache_kib != 98_304 {
        return false;
    }

    // Node 1 should be Frequency Boosted with 32 MiB L3
    let node1 = match amd_topo.nodes[1] {
        Some(n) => n,
        None => return false,
    };
    if node1.node_type != NumaNodeType::FrequencyBoosted || node1.l3_cache_kib != 32_768 {
        return false;
    }

    // Cross-CCD distance check (local = 10, cross = 22)
    if amd_topo.distances.distance(0, 0) != 10 || amd_topo.distances.distance(0, 1) != 22 {
        return false;
    }

    // Workload routing: GamingCacheBound -> Cores 0-7 (CCD0 V-Cache)
    let gaming_core = match amd_topo.find_best_core_for_workload(WorkloadProfile::GamingCacheBound) {
        Some(c) => c,
        None => return false,
    };
    if gaming_core >= 8 {
        return false;
    }

    // Workload routing: ThroughputCompute -> Cores 8-15 (CCD1 Frequency Boost)
    let compute_core = match amd_topo.find_best_core_for_workload(WorkloadProfile::ThroughputCompute) {
        Some(c) => c,
        None => return false,
    };
    if compute_core < 8 || compute_core >= 16 {
        return false;
    }

    // Memory routing: PreferVCache -> Node 0
    let vcache_node = amd_topo.select_allocation_node(10, MemoryAffinityPolicy::PreferVCache);
    if vcache_node != 0 {
        return false;
    }

    // 2. Intel Hybrid Topology Verification
    let intel_topo = NumaTopologyManager::create_simulated_intel_hybrid_topology();
    if intel_topo.vendor != CpuVendor::Intel || intel_topo.node_count != 2 || intel_topo.core_count != 16 {
        return false;
    }

    // Background I/O workload -> Intel Efficient E-core (Cores 8-15)
    let ecore = match intel_topo.find_best_core_for_workload(WorkloadProfile::BackgroundIo) {
        Some(c) => c,
        None => return false,
    };
    if ecore < 8 || ecore >= 16 {
        return false;
    }

    true
}

/// Comprehensive verification self-test suite for Transparent Huge Pages (THP 2MB) & AMD 3D V-Cache NUMA Topology Awareness.
pub fn test_thp_and_numa_subsystem() -> bool {
    // 1. Verify 2MB large page physical allocator and PML4/PDPT/PD page table translation structures
    if !verify_thp_allocator_and_structures() {
        return false;
    }

    // 2. Verify Transparent Huge Page collapse/split daemon simulation and zero-fill optimization
    if !verify_thp_collapse_split_and_zero_pool() {
        return false;
    }

    // 3. Verify AMD Zen 3D V-Cache and Intel hybrid CPUID cache topology discovery and NUMA core affinity
    if !verify_numa_vcache_and_hybrid_topology() {
        return false;
    }

    // 4. End-to-end integration: allocate a 2MB huge page with V-Cache NUMA affinity for cache-bound gaming workload
    let mut amd_topo = NumaTopologyManager::create_simulated_amd_x3d_topology();
    let best_core = match amd_topo.find_best_core_for_workload(WorkloadProfile::GamingCacheBound) {
        Some(c) => c,
        None => return false,
    };
    if best_core >= 8 {
        return false;
    }

    let target_node = amd_topo.select_allocation_node(best_core, MemoryAffinityPolicy::PreferVCache);
    if target_node != 0 {
        return false;
    }

    let mut allocator = HugePageAllocator::new();
    let base_phys = 0x0600_0000; // 96 MiB physical (2MB aligned)
    if !allocator.init(base_phys, 16) {
        return false;
    }

    let frame = match allocator.allocate_2mb_frame() {
        Some(f) => f,
        None => return false,
    };

    let mut pd = PageDirectoryTable::new();
    let game_buffer_vaddr = 0x0000_7FFF_B000_0000;
    let flags = HugePageFlags::PRESENT | HugePageFlags::WRITABLE | HugePageFlags::USER_ACCESSIBLE;
    if pd.map_2mb(game_buffer_vaddr, frame.base_address(), flags).is_err() {
        return false;
    }

    let translated = match pd.translate_2mb(game_buffer_vaddr + 0x8000) {
        Some(p) => p,
        None => return false,
    };
    if translated != frame.base_address() + 0x8000 {
        return false;
    }

    if pd.unmap_2mb(game_buffer_vaddr).is_none() {
        return false;
    }
    if !allocator.deallocate_2mb_frame(frame) {
        return false;
    }

    // 5. Emit boot sentinel
    crate::serial_write_str("[KNT-MEM-THP-PASS] Transparent Huge Pages (2MB) and NUMA V-Cache affinity operational\r\n");

    true
}
