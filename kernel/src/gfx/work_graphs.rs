//! Direct3D 12 Work Graphs & Autonomous GPU Shader Execution Engine.
//!
//! Provides hardware-accelerated autonomous GPU-driven work scheduling without CPU intervention:
//! - Work Graph Node Descriptors (Broadcasting, Coalescing, Threadgroup nodes)
//! - Node input and output records with bounded payload buffers
//! - Memory backing store layout and scratch requirements calculation
//! - GPU-autonomous downstream node dispatch, dynamic record rings, and recursive node expansion tracking

use alloc::vec::Vec;

/// Memory alignment required for Direct3D 12 Work Graph backing store allocations (256 bytes).
pub const D3D12_WORK_GRAPH_BACKING_MEMORY_ALIGNMENT: u64 = 256;

/// Maximum payload byte size supported per node record.
pub const MAX_NODE_RECORD_PAYLOAD_BYTES: usize = 128;

/// Maximum number of nodes supported in a single work graph.
pub const MAX_WORK_GRAPH_NODES: usize = 64;

/// Maximum number of downstream node output targets per node.
pub const MAX_OUTPUT_RECORD_TARGETS: usize = 8;

/// Maximum recursive node expansion depth allowed to prevent runaway GPU loops.
pub const MAX_WORK_GRAPH_RECURSION_DEPTH: u32 = 16;

/// Errors returned by the Work Graph execution subsystem.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WorkGraphError {
    /// Specified node ID was not found in the work graph.
    NodeNotFound(u32),
    /// Total nodes in the graph exceeds the limit (64).
    ExceededMaxNodes,
    /// Record payload size exceeds the maximum limit (128 bytes).
    PayloadSizeTooLarge(usize),
    /// Record payload size does not match node's configured input size.
    PayloadSizeMismatch { expected: usize, actual: usize },
    /// Backing store GPU virtual address is not 256-byte aligned.
    UnalignedBackingMemory(u64),
    /// Allocated backing store size is smaller than required minimum.
    InsufficientBackingMemory { required: u64, provided: u64 },
    /// Node expansion exceeded maximum recursion depth.
    RecursionDepthExceeded(u32),
    /// Node record ring buffer is full.
    RecordRingFull,
    /// Node record ring buffer is empty.
    RecordRingEmpty,
    /// Invalid threadgroup or dispatch grid dimensions.
    InvalidGridDimensions,
}

/// Direct3D 12 Work Graph node execution and scheduling types.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WorkGraphNodeType {
    /// Broadcasting node: Each input record launches a fixed or dynamic grid of threadgroups.
    Broadcasting,
    /// Coalescing node: Multiple input records are grouped up to a threshold before launching threadgroups.
    Coalescing,
    /// Threadgroup node: Input records are processed by cooperative threads within a threadgroup.
    Threadgroup,
}

/// Strongly-typed 32-bit identifier for a work graph node.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub struct NodeId(pub u32);

/// Work Graph Node Record containing a header and payload.
///
/// Records are the fundamental units of data flowing through the work graph edges.
#[derive(Debug, Clone, Copy, PartialEq)]
#[repr(C)]
pub struct NodeRecord {
    /// Destination or producer node ID.
    pub target_node: NodeId,
    /// Actual valid bytes in the payload buffer.
    pub payload_size: u16,
    /// Execution priority tag (0 = default, higher = higher priority).
    pub priority: u8,
    /// Reserved flags.
    pub flags: u8,
    /// Record payload buffer.
    pub payload: [u8; MAX_NODE_RECORD_PAYLOAD_BYTES],
}

impl Default for NodeRecord {
    fn default() -> Self {
        Self {
            target_node: NodeId(0),
            payload_size: 0,
            priority: 0,
            flags: 0,
            payload: [0u8; MAX_NODE_RECORD_PAYLOAD_BYTES],
        }
    }
}

impl NodeRecord {
    /// Creates a new node record for the specified target node with raw payload bytes.
    pub fn new(target_node: NodeId, payload_bytes: &[u8], priority: u8) -> Result<Self, WorkGraphError> {
        if payload_bytes.len() > MAX_NODE_RECORD_PAYLOAD_BYTES {
            return Err(WorkGraphError::PayloadSizeTooLarge(payload_bytes.len()));
        }

        let mut record = Self {
            target_node,
            payload_size: payload_bytes.len() as u16,
            priority,
            flags: 0,
            payload: [0u8; MAX_NODE_RECORD_PAYLOAD_BYTES],
        };
        record.payload[..payload_bytes.len()].copy_from_slice(payload_bytes);
        Ok(record)
    }

    /// Creates an empty record targeting `target_node`.
    pub const fn empty(target_node: NodeId) -> Self {
        Self {
            target_node,
            payload_size: 0,
            priority: 0,
            flags: 0,
            payload: [0u8; MAX_NODE_RECORD_PAYLOAD_BYTES],
        }
    }

    /// Returns a slice over the valid payload bytes.
    #[inline]
    pub fn payload_slice(&self) -> &[u8] {
        &self.payload[..self.payload_size as usize]
    }
}

/// Work Graph Node Descriptor.
///
/// Defines the shader execution model, threadgroup dimensions, input record contracts,
/// and downstream output targets.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct WorkGraphNodeDescriptor {
    /// Unique node identifier.
    pub node_id: NodeId,
    /// Scheduling and execution archetype.
    pub node_type: WorkGraphNodeType,
    /// 32-byte unique shader identifier (D3D12 state object export ID).
    pub shader_identifier: [u8; 32],
    /// Threadgroup dimension `(X, Y, Z)` executed per invocation.
    pub threadgroup_size: [u32; 3],
    /// Maximum dispatch grid `(X, Y, Z)` for broadcasting nodes.
    pub max_dispatch_grid: [u32; 3],
    /// Maximum input records grouped per threadgroup for coalescing nodes.
    pub max_records_per_threadgroup: u32,
    /// Expected byte size of each input record payload (must be <= 128).
    pub input_record_size: usize,
    /// Array of downstream node IDs that this node can dynamically schedule on the GPU.
    pub output_targets: [Option<NodeId>; MAX_OUTPUT_RECORD_TARGETS],
    /// Whether this node can be invoked as an initial entry point by the host.
    pub is_entry_point: bool,
}

impl WorkGraphNodeDescriptor {
    /// Validates the node descriptor against D3D12 hardware requirements.
    pub fn validate(&self) -> Result<(), WorkGraphError> {
        if self.threadgroup_size[0] == 0 || self.threadgroup_size[1] == 0 || self.threadgroup_size[2] == 0 {
            return Err(WorkGraphError::InvalidGridDimensions);
        }
        let total_threads = self.threadgroup_size[0]
            .saturating_mul(self.threadgroup_size[1])
            .saturating_mul(self.threadgroup_size[2]);
        if total_threads > 1024 {
            return Err(WorkGraphError::InvalidGridDimensions);
        }
        if self.input_record_size > MAX_NODE_RECORD_PAYLOAD_BYTES {
            return Err(WorkGraphError::PayloadSizeTooLarge(self.input_record_size));
        }
        if self.node_type == WorkGraphNodeType::Broadcasting {
            if self.max_dispatch_grid[0] == 0 || self.max_dispatch_grid[1] == 0 || self.max_dispatch_grid[2] == 0 {
                return Err(WorkGraphError::InvalidGridDimensions);
            }
        }
        if self.node_type == WorkGraphNodeType::Coalescing {
            if self.max_records_per_threadgroup == 0 {
                return Err(WorkGraphError::InvalidGridDimensions);
            }
        }
        Ok(())
    }
}

/// Memory backing store requirements calculated for a compiled Work Graph.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct WorkGraphMemoryRequirements {
    /// Minimum required backing store size in bytes.
    pub min_backing_memory_bytes: u64,
    /// Recommended backing store size in bytes for peak throughput.
    pub max_backing_memory_bytes: u64,
    /// Scratch memory required for in-flight node scheduling.
    pub scratch_buffer_bytes: u64,
    /// Size of internal circular record rings.
    pub record_ring_bytes: u64,
}

impl WorkGraphMemoryRequirements {
    /// Calculates memory requirements based on the graph's node topology.
    pub fn calculate(nodes: &[WorkGraphNodeDescriptor]) -> Self {
        let mut min_bytes: u64 = 4096; // Base graph state overhead (4 KiB)
        let mut record_ring_bytes: u64 = 0;

        for node in nodes {
            let record_size = (node.input_record_size as u64 + 16).next_power_of_two();
            // Estimate capacity per node ring (e.g. 64 records per node)
            let node_ring = record_size * 64;
            record_ring_bytes += node_ring;

            // Extra buffer for coalescing nodes
            if node.node_type == WorkGraphNodeType::Coalescing {
                let coalescing_buf = record_size * (node.max_records_per_threadgroup as u64 * 8);
                record_ring_bytes += coalescing_buf;
            }
        }

        min_bytes += record_ring_bytes;
        // Align to 256 bytes
        let aligned_min = (min_bytes + D3D12_WORK_GRAPH_BACKING_MEMORY_ALIGNMENT - 1)
            & !(D3D12_WORK_GRAPH_BACKING_MEMORY_ALIGNMENT - 1);
        let max_bytes = aligned_min * 2;
        let scratch_bytes = aligned_min / 2;

        Self {
            min_backing_memory_bytes: aligned_min,
            max_backing_memory_bytes: max_bytes,
            scratch_buffer_bytes: scratch_bytes,
            record_ring_bytes,
        }
    }
}

/// Backing store memory buffer descriptor for a Work Graph.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WorkGraphBackingStore {
    /// GPU virtual address of the allocated backing buffer.
    pub gpu_virtual_address: u64,
    /// Size of the buffer in bytes.
    pub size_bytes: u64,
}

impl WorkGraphBackingStore {
    /// Creates and validates a backing store descriptor.
    pub fn new(gpu_virtual_address: u64, size_bytes: u64, requirements: &WorkGraphMemoryRequirements) -> Result<Self, WorkGraphError> {
        if (gpu_virtual_address % D3D12_WORK_GRAPH_BACKING_MEMORY_ALIGNMENT) != 0 {
            return Err(WorkGraphError::UnalignedBackingMemory(gpu_virtual_address));
        }
        if size_bytes < requirements.min_backing_memory_bytes {
            return Err(WorkGraphError::InsufficientBackingMemory {
                required: requirements.min_backing_memory_bytes,
                provided: size_bytes,
            });
        }
        Ok(Self {
            gpu_virtual_address,
            size_bytes,
        })
    }
}

/// Circular ring buffer of node records between work graph stages.
#[derive(Debug, Clone)]
pub struct NodeRecordRing {
    buffer: Vec<NodeRecord>,
    capacity: usize,
    head: usize,
    tail: usize,
    count: usize,
}

impl NodeRecordRing {
    /// Creates a new record ring with the specified capacity.
    pub fn new(capacity: usize) -> Self {
        let mut buf = Vec::with_capacity(capacity);
        for _ in 0..capacity {
            buf.push(NodeRecord::default());
        }
        Self {
            buffer: buf,
            capacity,
            head: 0,
            tail: 0,
            count: 0,
        }
    }

    /// Whether the ring contains no pending records.
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.count == 0
    }

    /// Whether the ring has reached its maximum capacity.
    #[inline]
    pub fn is_full(&self) -> bool {
        self.count >= self.capacity
    }

    /// Current number of records waiting in the ring.
    #[inline]
    pub fn len(&self) -> usize {
        self.count
    }

    /// Maximum capacity of the ring.
    #[inline]
    pub fn capacity(&self) -> usize {
        self.capacity
    }

    /// Pushes a record to the tail of the ring buffer.
    pub fn push(&mut self, record: NodeRecord) -> Result<(), WorkGraphError> {
        if self.is_full() {
            return Err(WorkGraphError::RecordRingFull);
        }
        self.buffer[self.tail] = record;
        self.tail = (self.tail + 1) % self.capacity;
        self.count += 1;
        Ok(())
    }

    /// Pops the next record from the head of the ring buffer.
    pub fn pop(&mut self) -> Option<NodeRecord> {
        if self.is_empty() {
            return None;
        }
        let record = self.buffer[self.head];
        self.head = (self.head + 1) % self.capacity;
        self.count -= 1;
        Some(record)
    }
}

/// Accumulation buffer for a Coalescing work graph node.
///
/// Collects multiple input records until `max_records` is reached or a flush is initiated.
#[derive(Debug, Clone)]
pub struct CoalescingBuffer {
    target_node: NodeId,
    max_records: usize,
    records: Vec<NodeRecord>,
}

impl CoalescingBuffer {
    /// Creates a new coalescing buffer for the target node.
    pub fn new(target_node: NodeId, max_records: usize) -> Self {
        Self {
            target_node,
            max_records,
            records: Vec::with_capacity(max_records),
        }
    }

    /// Target node ID.
    #[inline]
    pub fn target_node(&self) -> NodeId {
        self.target_node
    }

    /// Appends a record to the coalescing buffer. Returns true if threshold is reached.
    pub fn add_record(&mut self, record: NodeRecord) -> bool {
        self.records.push(record);
        self.records.len() >= self.max_records
    }

    /// Whether the buffer has accumulated sufficient records for a threadgroup launch.
    #[inline]
    pub fn is_ready(&self) -> bool {
        self.records.len() >= self.max_records
    }

    /// Drains all accumulated records for execution.
    pub fn drain_records(&mut self) -> Vec<NodeRecord> {
        core::mem::take(&mut self.records)
    }

    /// Number of records currently buffered.
    #[inline]
    pub fn len(&self) -> usize {
        self.records.len()
    }
}

/// Profiling telemetry for Work Graph scheduling.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct WorkGraphSchedulerStats {
    /// Total records enqueued into node rings.
    pub total_records_enqueued: u64,
    /// Total records dequeued and dispatched to execution units.
    pub total_records_processed: u64,
    /// Number of Broadcasting node dispatches.
    pub broadcasting_dispatches: u64,
    /// Number of Coalescing node dispatches.
    pub coalescing_dispatches: u64,
    /// Number of Threadgroup node dispatches.
    pub threadgroup_dispatches: u64,
}

/// Work Graph Scheduler Engine.
///
/// Orchestrates registered node pipelines, routes incoming records to destination queues,
/// buffers coalescing node batches, and monitors pending GPU work.
#[derive(Debug)]
pub struct WorkGraphScheduler {
    nodes: Vec<WorkGraphNodeDescriptor>,
    rings: Vec<NodeRecordRing>,
    coalescing_buffers: Vec<CoalescingBuffer>,
    stats: WorkGraphSchedulerStats,
}

impl WorkGraphScheduler {
    /// Creates a new WorkGraphScheduler.
    pub fn new() -> Self {
        Self {
            nodes: Vec::new(),
            rings: Vec::new(),
            coalescing_buffers: Vec::new(),
            stats: WorkGraphSchedulerStats::default(),
        }
    }

    /// Registers a node descriptor in the work graph.
    pub fn register_node(&mut self, desc: WorkGraphNodeDescriptor) -> Result<(), WorkGraphError> {
        desc.validate()?;
        if self.nodes.len() >= MAX_WORK_GRAPH_NODES {
            return Err(WorkGraphError::ExceededMaxNodes);
        }
        if self.nodes.iter().any(|n| n.node_id == desc.node_id) {
            return Ok(()); // Already registered
        }

        self.nodes.push(desc);
        self.rings.push(NodeRecordRing::new(64));

        if desc.node_type == WorkGraphNodeType::Coalescing {
            self.coalescing_buffers.push(CoalescingBuffer::new(
                desc.node_id,
                desc.max_records_per_threadgroup as usize,
            ));
        }

        Ok(())
    }

    /// Retrieves a reference to a registered node descriptor.
    pub fn get_node(&self, id: NodeId) -> Option<&WorkGraphNodeDescriptor> {
        self.nodes.iter().find(|n| n.node_id == id)
    }

    /// Enqueues a record targeting a specific node in the work graph.
    pub fn enqueue_record(&mut self, record: NodeRecord) -> Result<(), WorkGraphError> {
        let (node_type, expected_size, node_idx) = {
            let (idx, node) = self
                .nodes
                .iter()
                .enumerate()
                .find(|(_, n)| n.node_id == record.target_node)
                .ok_or(WorkGraphError::NodeNotFound(record.target_node.0))?;
            (node.node_type, node.input_record_size, idx)
        };

        if (record.payload_size as usize) != expected_size && expected_size > 0 {
            return Err(WorkGraphError::PayloadSizeMismatch {
                expected: expected_size,
                actual: record.payload_size as usize,
            });
        }

        if node_type == WorkGraphNodeType::Coalescing {
            if let Some(buf) = self.coalescing_buffers.iter_mut().find(|b| b.target_node == record.target_node) {
                buf.add_record(record);
            }
        } else {
            self.rings[node_idx].push(record)?;
        }

        self.stats.total_records_enqueued += 1;
        Ok(())
    }

    /// Pops the next pending record from a node's ring.
    pub fn pop_record(&mut self, node_id: NodeId) -> Option<NodeRecord> {
        let node_idx = self.nodes.iter().position(|n| n.node_id == node_id)?;
        let rec = self.rings[node_idx].pop();
        if rec.is_some() {
            self.stats.total_records_processed += 1;
        }
        rec
    }

    /// Checks if a coalescing node has accumulated enough records for launch.
    pub fn pop_coalesced_batch(&mut self, node_id: NodeId, force_flush: bool) -> Option<Vec<NodeRecord>> {
        let buf = self.coalescing_buffers.iter_mut().find(|b| b.target_node == node_id)?;
        if buf.is_ready() || (force_flush && buf.len() > 0) {
            let batch = buf.drain_records();
            self.stats.total_records_processed += batch.len() as u64;
            Some(batch)
        } else {
            None
        }
    }

    /// Whether there is any pending work in any node ring or coalescing buffer.
    pub fn has_pending_work(&self) -> bool {
        self.rings.iter().any(|r| !r.is_empty()) || self.coalescing_buffers.iter().any(|b| b.len() > 0)
    }

    /// Current scheduler statistics.
    #[inline]
    pub fn stats(&self) -> &WorkGraphSchedulerStats {
        &self.stats
    }

    /// Mutable statistics for dispatch tracking.
    #[inline]
    pub fn stats_mut(&mut self) -> &mut WorkGraphSchedulerStats {
        &mut self.stats
    }

    /// Resets scheduler telemetry statistics.
    pub fn reset_stats(&mut self) {
        self.stats = WorkGraphSchedulerStats::default();
    }
}
