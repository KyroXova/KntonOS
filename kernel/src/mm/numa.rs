//! NUMA Topology & Cache Affinity Subsystem.
//! Provides AMD Zen CCD / 3D V-Cache and Intel Hybrid P/E-core CPUID topology discovery,

/// Identifies the microarchitectural CPU vendor.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CpuVendor {
    Amd,
    Intel,
    Unknown,
}

/// Core affinity specialization hint based on cache hierarchy and core type.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CoreAffinityHint {
    /// AMD 3D V-Cache core (e.g. CCD0 with 96MB stacked L3 cache, optimal for gaming & cache-bound tasks).
    VCacheOptimized,
    /// AMD frequency-boosted core (e.g. CCD1 with higher clock ceiling, optimal for compute/rendering).
    FrequencyOptimized,
    /// Intel Performance-core (P-Core, Golden Cove / Raptor Cove).
    IntelPerformance,
    /// Intel Efficient-core (E-Core, Gracemont / Crestmont).
    IntelEfficiency,
    /// Standard uniform symmetric core.
    Standard,
}

/// Profile of a workload requesting CPU core affinity or memory placement.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WorkloadProfile {
    /// Cache-sensitive low-latency workload (e.g. 3D game engine, physics, frametime critical).
    GamingCacheBound,
    /// Throughput-bound compute workload (e.g. shader compilation, offline audio render).
    ThroughputCompute,
    /// Background system worker or I/O daemon.
    BackgroundIo,
    /// Standard interactive thread.
    General,
}

/// Classification of a NUMA node in modern asymmetric multi-die systems.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NumaNodeType {
    /// Node equipped with high-density stacked 3D V-Cache.
    VCache3D,
    /// Node equipped with high-clock frequency-boosted cores.
    FrequencyBoosted,
    /// Intel Performance hybrid domain.
    IntelPerformance,
    /// Intel Efficient hybrid domain.
    IntelEfficient,
    /// Standard uniform symmetric node.
    Standard,
}

/// Represents an individual logical CPU core and its cache/NUMA associations.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NumaCore {
    pub logical_id: usize,
    pub apic_id: u32,
    pub node_id: usize,
    pub ccd_id: u8,
    pub ccx_id: u8,
    pub affinity_hint: CoreAffinityHint,
    pub l3_cache_kib: u32,
}

impl NumaCore {
    pub const fn new(
        logical_id: usize,
        apic_id: u32,
        node_id: usize,
        ccd_id: u8,
        ccx_id: u8,
        affinity_hint: CoreAffinityHint,
        l3_cache_kib: u32,
    ) -> Self {
        Self {
            logical_id,
            apic_id,
            node_id,
            ccd_id,
            ccx_id,
            affinity_hint,
            l3_cache_kib,
        }
    }
}

/// Represents a NUMA node with associated memory and CPU core bitmask.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NumaNode {
    pub id: usize,
    pub node_type: NumaNodeType,
    pub core_mask: u64,
    pub memory_base: usize,
    pub memory_size: usize,
    pub l3_cache_kib: u32,
}

impl NumaNode {
    pub const fn new(
        id: usize,
        node_type: NumaNodeType,
        core_mask: u64,
        memory_base: usize,
        memory_size: usize,
        l3_cache_kib: u32,
    ) -> Self {
        Self {
            id,
            node_type,
            core_mask,
            memory_base,
            memory_size,
            l3_cache_kib,
        }
    }

    #[inline(always)]
    pub fn contains_core(&self, core_id: usize) -> bool {
        if core_id < 64 {
            (self.core_mask & (1u64 << core_id)) != 0
        } else {
            false
        }
    }
}

/// Memory allocation policy under asymmetric NUMA / V-Cache topology.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MemoryAffinityPolicy {
    /// Allocate from the calling core's local NUMA / V-Cache node; fall back to remote if depleted.
    LocalFirst,
    /// Allocate strictly from the local node; fail if depleted.
    StrictLocal,
    /// Prioritize allocating memory backed by the 3D V-Cache node regardless of calling core.
    PreferVCache,
    /// Round-robin interleave across all registered nodes.
    Interleave,
}

pub const MAX_NUMA_NODES: usize = 4;
pub const MAX_NUMA_CORES: usize = 32;

/// Standard NUMA inter-node distance matrix.
#[derive(Debug, Clone, Copy)]
pub struct NumaDistanceMatrix {
    /// Distances between node pairs. Default: local = 10, cross-CCD = 20, remote socket = 32.
    distances: [[u8; MAX_NUMA_NODES]; MAX_NUMA_NODES],
}

impl NumaDistanceMatrix {
    pub const fn new() -> Self {
        let mut mat = [[20u8; MAX_NUMA_NODES]; MAX_NUMA_NODES];
        let mut i = 0;
        while i < MAX_NUMA_NODES {
            mat[i][i] = 10;
            i += 1;
        }
        Self { distances: mat }
    }

    pub fn set_distance(&mut self, from: usize, to: usize, dist: u8) {
        if from < MAX_NUMA_NODES && to < MAX_NUMA_NODES {
            self.distances[from][to] = dist;
            self.distances[to][from] = dist;
        }
    }

    #[inline(always)]
    pub fn distance(&self, from: usize, to: usize) -> u8 {
        if from < MAX_NUMA_NODES && to < MAX_NUMA_NODES {
            self.distances[from][to]
        } else {
            255
        }
    }
}

/// Subsystem manager for NUMA node routing, CPU topology, and cache-aware core scheduling.
pub struct NumaTopologyManager {
    pub vendor: CpuVendor,
    pub nodes: [Option<NumaNode>; MAX_NUMA_NODES],
    pub node_count: usize,
    pub cores: [Option<NumaCore>; MAX_NUMA_CORES],
    pub core_count: usize,
    pub distances: NumaDistanceMatrix,
    interleave_cursor: usize,
}

impl NumaTopologyManager {
    pub const fn new(vendor: CpuVendor) -> Self {
        Self {
            vendor,
            nodes: [None; MAX_NUMA_NODES],
            node_count: 0,
            cores: [None; MAX_NUMA_CORES],
            core_count: 0,
            distances: NumaDistanceMatrix::new(),
            interleave_cursor: 0,
        }
    }

    /// Registers a NUMA node into the topology table.
    pub fn register_node(&mut self, node: NumaNode) -> bool {
        if node.id < MAX_NUMA_NODES && self.nodes[node.id].is_none() {
            self.nodes[node.id] = Some(node);
            self.node_count += 1;
            true
        } else {
            false
        }
    }

    /// Registers a logical CPU core into the topology table.
    pub fn register_core(&mut self, core: NumaCore) -> bool {
        if core.logical_id < MAX_NUMA_CORES && self.cores[core.logical_id].is_none() {
            self.cores[core.logical_id] = Some(core);
            self.core_count += 1;
            true
        } else {
            false
        }
    }

    /// Finds the optimal CPU core for a given workload profile based on cache & frequency characteristics.
    pub fn find_best_core_for_workload(&self, profile: WorkloadProfile) -> Option<usize> {
        if self.core_count == 0 {
            return None;
        }

        match profile {
            WorkloadProfile::GamingCacheBound => {
                // First preference: AMD 3D V-Cache or Intel P-Core
                for i in 0..MAX_NUMA_CORES {
                    if let Some(core) = self.cores[i] {
                        if core.affinity_hint == CoreAffinityHint::VCacheOptimized {
                            return Some(core.logical_id);
                        }
                    }
                }
                for i in 0..MAX_NUMA_CORES {
                    if let Some(core) = self.cores[i] {
                        if core.affinity_hint == CoreAffinityHint::IntelPerformance {
                            return Some(core.logical_id);
                        }
                    }
                }
            }
            WorkloadProfile::ThroughputCompute => {
                // First preference: Frequency optimized boost core or Intel P-Core
                for i in 0..MAX_NUMA_CORES {
                    if let Some(core) = self.cores[i] {
                        if core.affinity_hint == CoreAffinityHint::FrequencyOptimized {
                            return Some(core.logical_id);
                        }
                    }
                }
                for i in 0..MAX_NUMA_CORES {
                    if let Some(core) = self.cores[i] {
                        if core.affinity_hint == CoreAffinityHint::IntelPerformance {
                            return Some(core.logical_id);
                        }
                    }
                }
            }
            WorkloadProfile::BackgroundIo => {
                // First preference: Intel E-Core, or frequency core
                for i in 0..MAX_NUMA_CORES {
                    if let Some(core) = self.cores[i] {
                        if core.affinity_hint == CoreAffinityHint::IntelEfficiency {
                            return Some(core.logical_id);
                        }
                    }
                }
            }
            WorkloadProfile::General => {}
        }

        // Fallback: return first available core
        for i in 0..MAX_NUMA_CORES {
            if let Some(core) = self.cores[i] {
                return Some(core.logical_id);
            }
        }
        None
    }

    /// Selects the target NUMA node ID for memory allocation according to policy.
    pub fn select_allocation_node(&mut self, calling_core: usize, policy: MemoryAffinityPolicy) -> usize {
        let calling_node = self.cores[calling_core.min(MAX_NUMA_CORES - 1)]
            .map(|c| c.node_id)
            .unwrap_or(0);

        match policy {
            MemoryAffinityPolicy::LocalFirst | MemoryAffinityPolicy::StrictLocal => calling_node,
            MemoryAffinityPolicy::PreferVCache => {
                // Locate node with 3D V-Cache
                for i in 0..MAX_NUMA_NODES {
                    if let Some(node) = self.nodes[i] {
                        if node.node_type == NumaNodeType::VCache3D {
                            return node.id;
                        }
                    }
                }
                calling_node
            }
            MemoryAffinityPolicy::Interleave => {
                if self.node_count == 0 {
                    return 0;
                }
                let mut attempts = 0;
                while attempts < MAX_NUMA_NODES {
                    let node_idx = self.interleave_cursor;
                    self.interleave_cursor = (self.interleave_cursor + 1) % MAX_NUMA_NODES;
                    if self.nodes[node_idx].is_some() {
                        return node_idx;
                    }
                    attempts += 1;
                }
                calling_node
            }
        }
    }

    /// Helper creating simulated AMD Ryzen 9 7950X3D dual-CCD topology:
    /// CCD0: Node 0 (8 Cores, 16 Threads), 96MB L3 (3D V-Cache), 16 GiB Memory
    pub fn create_simulated_amd_x3d_topology() -> Self {
        let mut mgr = Self::new(CpuVendor::Amd);

        // Node 0 (CCD0 with 3D V-Cache)
        mgr.register_node(NumaNode::new(
            0,
            NumaNodeType::VCache3D,
            0x0000_00FF,          // Cores 0-7
            0x0000_0000_1000_0000, // 256MB Base
            0x0000_0000_2000_0000, // 512MB Pool
            98_304,                // 96 MiB L3
        ));

        // Node 1 (CCD1 standard frequency boosted)
        mgr.register_node(NumaNode::new(
            1,
            NumaNodeType::FrequencyBoosted,
            0x0000_FF00,          // Cores 8-15
            0x0000_0000_3000_0000, // Base
            0x0000_0000_2000_0000, // Pool
            32_768,                // 32 MiB L3
        ));

        // Register cores 0-7 on CCD0 (V-Cache)
        for i in 0..8 {
            mgr.register_core(NumaCore::new(
                i,
                i as u32,
                0,
                0, // CCD0
                0, // CCX0
                CoreAffinityHint::VCacheOptimized,
                98_304,
            ));
        }

        // Register cores 8-15 on CCD1 (Frequency)
        for i in 8..16 {
            mgr.register_core(NumaCore::new(
                i,
                i as u32,
                1,
                1, // CCD1
                0, // CCX0
                CoreAffinityHint::FrequencyOptimized,
                32_768,
            ));
        }

        mgr.distances.set_distance(0, 1, 22); // Infinity Fabric cross-CCD penalty
        mgr
    }

    /// Helper creating simulated Intel Hybrid (12th-14th Gen) topology:
    /// P-Cores: Node 0, Golden Cove P-Cores (8 cores)
    pub fn create_simulated_intel_hybrid_topology() -> Self {
        let mut mgr = Self::new(CpuVendor::Intel);

        mgr.register_node(NumaNode::new(
            0,
            NumaNodeType::IntelPerformance,
            0x0000_00FF,
            0x0000_0000_1000_0000,
            0x0000_0000_2000_0000,
            36_864, // 36 MiB L3
        ));

        mgr.register_node(NumaNode::new(
            1,
            NumaNodeType::IntelEfficient,
            0x0000_FF00,
            0x0000_0000_3000_0000,
            0x0000_0000_2000_0000,
            36_864,
        ));

        for i in 0..8 {
            mgr.register_core(NumaCore::new(
                i,
                i as u32,
                0,
                0,
                0,
                CoreAffinityHint::IntelPerformance,
                36_864,
            ));
        }

        for i in 8..16 {
            mgr.register_core(NumaCore::new(
                i,
                i as u32,
                1,
                1,
                0,
                CoreAffinityHint::IntelEfficiency,
                36_864,
            ));
        }

        mgr
    }
}
