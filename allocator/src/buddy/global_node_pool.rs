//! Global node pool for buddy allocator
//!
//! Provides a single pool of list nodes shared across all zones and orders.
//! This eliminates the need for per-zone list pools and improves memory efficiency.

#[cfg(feature = "log")]
use log::error;

use super::buddy_block::BuddyBlock;

/// Simple linked list node used by the global node pool
#[derive(Debug, Clone, Copy)]
pub struct ListNode<T> {
    pub data: T,
    pub next: Option<usize>,
}

/// Total number of nodes in the global pool
/// Each node can hold one BuddyBlock
/// This should be large enough to handle fragmentation scenarios
/// Reduced for testing to avoid stack overflow
#[cfg(test)]
pub const GLOBAL_TOTAL_NODES: usize = 512;

#[cfg(not(test))]
pub const GLOBAL_TOTAL_NODES: usize = 819200;

/// Global node pool - all zones and orders share nodes from this pool
pub struct GlobalNodePool {
    /// Array of all nodes in the pool
    nodes: [Option<ListNode<BuddyBlock>>; GLOBAL_TOTAL_NODES],
    /// Free list head - points to first available node
    free_head: Option<usize>,
    /// Allocation statistics
    total_allocations: usize,
    total_deallocations: usize,
}

impl GlobalNodePool {
    /// Create a new global node pool (uninitialized, must call init())
    pub const fn new() -> Self {
        Self {
            nodes: [const { None }; GLOBAL_TOTAL_NODES],
            free_head: None,
            total_allocations: 0,
            total_deallocations: 0,
        }
    }

    /// Initialize the global node pool
    pub fn init(&mut self) {
        // Initialize all nodes as free
        for i in 0..GLOBAL_TOTAL_NODES {
            self.nodes[i] = Some(ListNode {
                data: unsafe { core::mem::zeroed() },
                next: if i < GLOBAL_TOTAL_NODES - 1 {
                    Some(i + 1)
                } else {
                    None
                },
            });
        }
        self.free_head = Some(0);
        self.total_allocations = 0;
        self.total_deallocations = 0;
    }

    /// Allocate a node from the pool
    ///
    /// Returns the index of the allocated node, or None if pool is exhausted
    pub fn alloc_node(&mut self) -> Option<usize> {
        if self.free_head.is_none() {
            return None;
        }

        let node_idx = self.free_head?;
        if node_idx >= GLOBAL_TOTAL_NODES {
            return None;
        }

        // Get next free node
        let next_free = self.nodes[node_idx].as_ref().and_then(|n| n.next);

        // Clear the node's next pointer
        if let Some(node) = self.nodes[node_idx].as_mut() {
            node.next = None;
        }

        self.free_head = next_free;
        self.total_allocations += 1;

        Some(node_idx)
    }

    /// Deallocate a node back to the pool
    ///
    /// The node should not be part of any active list when freed
    pub fn dealloc_node(&mut self, node_idx: usize) {
        if node_idx >= GLOBAL_TOTAL_NODES {
            panic!("Invalid node index: {}", node_idx);
        }

        if self.nodes[node_idx].is_none() {
            panic!("Node {} already deallocated", node_idx);
        }

        // Clear the node data
        self.nodes[node_idx] = Some(ListNode {
            data: unsafe { core::mem::zeroed() },
            next: self.free_head,
        });

        self.free_head = Some(node_idx);
        self.total_deallocations += 1;
    }

    /// Get a reference to a node by index
    pub fn get_node(&self, node_idx: usize) -> Option<&ListNode<BuddyBlock>> {
        if node_idx >= GLOBAL_TOTAL_NODES {
            error!("Invalid node index: {}", node_idx);
            return None;
        }
        self.nodes[node_idx].as_ref()
    }

    /// Get a mutable reference to a node by index
    pub fn get_node_mut(&mut self, node_idx: usize) -> Option<&mut ListNode<BuddyBlock>> {
        if node_idx >= GLOBAL_TOTAL_NODES {
            return None;
        }
        self.nodes[node_idx].as_mut()
    }

    /// Get the number of free nodes in the pool
    pub fn free_node_count(&self) -> usize {
        let mut count = 0;
        let mut current = self.free_head;
        while let Some(idx) = current {
            if idx >= GLOBAL_TOTAL_NODES {
                break;
            }
            count += 1;
            current = self.nodes[idx].as_ref().and_then(|n| n.next);
        }
        count
    }

    /// Get the number of allocated nodes
    pub fn allocated_node_count(&self) -> usize {
        GLOBAL_TOTAL_NODES - self.free_node_count()
    }

    /// Get pool statistics
    pub fn get_stats(&self) -> GlobalPoolStats {
        GlobalPoolStats {
            total_nodes: GLOBAL_TOTAL_NODES,
            free_nodes: self.free_node_count(),
            allocated_nodes: self.allocated_node_count(),
            total_allocations: self.total_allocations,
            total_deallocations: self.total_deallocations,
        }
    }
}

impl Default for GlobalNodePool {
    fn default() -> Self {
        Self::new()
    }
}

/// Global pool statistics
#[derive(Debug, Default, Clone)]
pub struct GlobalPoolStats {
    pub total_nodes: usize,
    pub free_nodes: usize,
    pub allocated_nodes: usize,
    pub total_allocations: usize,
    pub total_deallocations: usize,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_pool_init() {
        let mut pool: GlobalNodePool = GlobalNodePool::new();
        pool.init();

        assert_eq!(pool.free_node_count(), GLOBAL_TOTAL_NODES);
        assert_eq!(pool.allocated_node_count(), 0);
    }

    #[test]
    fn test_alloc_dealloc() {
        let mut pool: GlobalNodePool = GlobalNodePool::new();
        pool.init();

        let idx1 = pool.alloc_node().unwrap();
        assert_eq!(pool.free_node_count(), GLOBAL_TOTAL_NODES - 1);
        assert_eq!(pool.allocated_node_count(), 1);

        let idx2 = pool.alloc_node().unwrap();
        assert_eq!(pool.free_node_count(), GLOBAL_TOTAL_NODES - 2);

        pool.dealloc_node(idx1);
        assert_eq!(pool.free_node_count(), GLOBAL_TOTAL_NODES - 1);

        pool.dealloc_node(idx2);
        assert_eq!(pool.free_node_count(), GLOBAL_TOTAL_NODES);
    }

    #[test]
    fn test_pool_exhaustion() {
        let mut pool: GlobalNodePool = GlobalNodePool::new();
        pool.init();

        // Allocate all nodes
        let mut indices = alloc::vec::Vec::new();
        for _ in 0..GLOBAL_TOTAL_NODES {
            indices.push(pool.alloc_node().unwrap());
        }

        assert_eq!(pool.free_node_count(), 0);
        assert!(pool.alloc_node().is_none());

        // Free one and allocate again
        pool.dealloc_node(indices[0]);
        assert_eq!(pool.free_node_count(), 1);
        assert!(pool.alloc_node().is_some());
    }

    #[test]
    fn test_node_access() {
        let mut pool: GlobalNodePool = GlobalNodePool::new();
        pool.init();

        let idx = pool.alloc_node().unwrap();

        // Get mutable reference and set data
        if let Some(node) = pool.get_node_mut(idx) {
            node.data = BuddyBlock {
                order: 0,
                addr: 0x1000,
            };
        }

        // Get reference and read data
        if let Some(node) = pool.get_node(idx) {
            assert_eq!(node.data.order, 0);
            assert_eq!(node.data.addr, 0x1000);
        }
    }

    #[test]
    fn test_stats() {
        let mut pool: GlobalNodePool = GlobalNodePool::new();
        pool.init();

        let idx1 = pool.alloc_node().unwrap();
        let idx2 = pool.alloc_node().unwrap();

        let stats = pool.get_stats();
        assert_eq!(stats.total_nodes, GLOBAL_TOTAL_NODES);
        assert_eq!(stats.free_nodes, GLOBAL_TOTAL_NODES - 2);
        assert_eq!(stats.allocated_nodes, 2);
        assert_eq!(stats.total_allocations, 2);
        assert_eq!(stats.total_deallocations, 0);

        pool.dealloc_node(idx1);
        let stats2 = pool.get_stats();
        assert_eq!(stats2.free_nodes, GLOBAL_TOTAL_NODES - 1);
        assert_eq!(stats2.total_deallocations, 1);
    }
}
