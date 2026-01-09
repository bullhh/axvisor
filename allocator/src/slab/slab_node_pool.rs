//! Global slab node pool implementation.
//!
//! This module implements a global pool of slab nodes using a single linked list
//! for managing free nodes, avoiding heap allocation.

use super::slab_node::SlabNode;

/// Linked list node wrapper
#[derive(Debug, Clone, Copy)]
pub struct ListNode<T> {
    pub data: Option<T>,
    pub prev: Option<usize>,
    pub next: Option<usize>,
}

impl<T> ListNode<T> {
    pub const fn empty() -> Self {
        Self {
            data: None,
            prev: None,
            next: None,
        }
    }
}

/// Global node pool with single linked list for free nodes
pub struct GlobalSlabNodePool {
    pub(crate) nodes: [ListNode<SlabNode>; GLOBAL_SLAB_NODES],
    free_head: Option<usize>,
}

const GLOBAL_SLAB_NODES: usize = 1024;

impl GlobalSlabNodePool {
    pub const fn new() -> Self {
        Self {
            nodes: [ListNode::empty(); GLOBAL_SLAB_NODES],
            free_head: None,
        }
    }

    /// Initialize free linked list
    pub fn init_free_list(&mut self) {
        for i in 0..GLOBAL_SLAB_NODES - 1 {
            self.nodes[i].data = None;
            self.nodes[i].prev = None;
            self.nodes[i].next = Some(i + 1);
        }
        self.nodes[GLOBAL_SLAB_NODES - 1].data = None;
        self.nodes[GLOBAL_SLAB_NODES - 1].prev = None;
        self.nodes[GLOBAL_SLAB_NODES - 1].next = None;

        self.free_head = Some(0);
    }

    /// Allocate a node from the free list
    pub fn alloc_node(&mut self, data: SlabNode) -> Option<usize> {
        let idx = self.free_head?;
        self.free_head = self.nodes[idx].next;

        self.nodes[idx].data = Some(data);
        self.nodes[idx].prev = None;
        self.nodes[idx].next = None;

        Some(idx)
    }

    /// Free a node back to the pool
    pub fn free_node(&mut self, idx: usize) {
        if idx < GLOBAL_SLAB_NODES && self.nodes[idx].data.is_some() {
            self.nodes[idx] = ListNode::empty();
            self.nodes[idx].next = self.free_head;
            self.free_head = Some(idx);
        }
    }

    /// Get mutable reference to node data
    pub fn get_mut(&mut self, idx: usize) -> Option<&mut SlabNode> {
        if idx < GLOBAL_SLAB_NODES {
            self.nodes[idx].data.as_mut()
        } else {
            None
        }
    }

    /// Get reference to node data
    pub fn get(&self, idx: usize) -> Option<&SlabNode> {
        if idx < GLOBAL_SLAB_NODES {
            self.nodes[idx].data.as_ref()
        } else {
            None
        }
    }

    /// Check if there are available nodes
    pub fn has_available(&self) -> bool {
        self.free_head.is_some()
    }

    /// Get count of available nodes
    pub fn available_count(&self) -> usize {
        let mut count = 0;
        let mut current = self.free_head;
        while let Some(idx) = current {
            count += 1;
            current = self.nodes[idx].next;
        }
        count
    }

    /// Get count of used nodes
    pub fn used_count(&self) -> usize {
        GLOBAL_SLAB_NODES - self.available_count()
    }

    /// Get node's prev pointer
    pub unsafe fn get_prev(&self, idx: usize) -> Option<usize> {
        if idx < GLOBAL_SLAB_NODES {
            self.nodes[idx].prev
        } else {
            None
        }
    }

    /// Get node's next pointer
    pub unsafe fn get_next(&self, idx: usize) -> Option<usize> {
        if idx < GLOBAL_SLAB_NODES {
            self.nodes[idx].next
        } else {
            None
        }
    }

    /// Set node's prev pointer
    pub unsafe fn set_prev(&mut self, idx: usize, prev: Option<usize>) {
        if idx < GLOBAL_SLAB_NODES {
            self.nodes[idx].prev = prev;
        }
    }

    /// Set node's next pointer
    pub unsafe fn set_next(&mut self, idx: usize, next: Option<usize>) {
        if idx < GLOBAL_SLAB_NODES {
            self.nodes[idx].next = next;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Re-import for tests
    use super::super::slab_node::SlabNode;
    use super::super::slab_byte_allocator::SizeClass;

    #[test]
    fn test_free_list_initialization() {
        let mut pool = GlobalSlabNodePool::new();
        pool.init_free_list();

        assert!(pool.has_available());
        assert_eq!(pool.available_count(), 1024);
        assert_eq!(pool.used_count(), 0);
    }

    #[test]
    fn test_alloc_free_cycle() {
        let mut pool = GlobalSlabNodePool::new();
        pool.init_free_list();

        let node = SlabNode::new(0x1000, SizeClass::Bytes64);

        // Allocate
        let idx = pool.alloc_node(node).unwrap();
        assert_eq!(pool.available_count(), 1023);
        assert_eq!(pool.used_count(), 1);

        // Check data
        let data = pool.get(idx).unwrap();
        assert_eq!(data.addr, 0x1000);

        // Free
        pool.free_node(idx);
        assert_eq!(pool.available_count(), 1024);
        assert_eq!(pool.used_count(), 0);
    }

    #[test]
    fn test_multiple_allocs() {
        let mut pool = GlobalSlabNodePool::new();
        pool.init_free_list();

        let mut indices = alloc::vec::Vec::new();

        // Allocate 10 nodes
        for i in 0..10 {
            let node = SlabNode::new(0x1000 + i * 0x1000, SizeClass::Bytes64);
            let idx = pool.alloc_node(node).unwrap();
            indices.push(idx);
        }

        assert_eq!(pool.available_count(), 1024 - 10);
        assert_eq!(pool.used_count(), 10);

        // Free all
        for idx in indices {
            pool.free_node(idx);
        }

        assert_eq!(pool.available_count(), 1024);
        assert_eq!(pool.used_count(), 0);
    }

    #[test]
    fn test_exhaustion() {
        let mut pool = GlobalSlabNodePool::new();
        pool.init_free_list();

        // Allocate all nodes
        let mut indices = alloc::vec::Vec::new();
        for i in 0..1024 {
            let node = SlabNode::new(0x1000 + i * 0x1000, SizeClass::Bytes64);
            let idx = pool.alloc_node(node).unwrap();
            indices.push(idx);
        }

        assert_eq!(pool.available_count(), 0);
        assert!(!pool.has_available());

        // Try to allocate one more
        let extra_node = SlabNode::new(0x200000, SizeClass::Bytes64);
        assert!(pool.alloc_node(extra_node).is_none());
    }
}
