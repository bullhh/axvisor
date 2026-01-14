//! Global slab node pool implementation.
//!
//! This module implements a global pool of slab nodes using:
//! - Static initial nodes (27 nodes for fast startup)
//! - Dynamic expansion region (27 nodes per batch)
//! - Single linked list for managing free nodes

use super::slab_node::SlabNode;
use crate::AllocResult;

/// Page allocator trait for dynamic expansion
pub trait PageAllocatorForSlab {
    fn alloc_pages(&mut self, count: usize, page_size: usize) -> AllocResult<usize>;
    fn dealloc_pages(&mut self, addr: usize, count: usize);
}

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

/// Dynamic region for expanded nodes
pub struct DynamicRegion {
    regions: [DynamicNodeRegion; MAX_REGIONS],
    region_count: usize,
    total_allocated: usize,
}

const MAX_REGIONS: usize = 256;

#[derive(Clone, Copy)]
struct DynamicNodeRegion {
    base_addr: usize,
    node_count: usize,
}

impl DynamicRegion {
    pub const fn new() -> Self {
        Self {
            regions: [DynamicNodeRegion {
                base_addr: 0,
                node_count: 0,
            }; MAX_REGIONS],
            region_count: 0,
            total_allocated: 0,
        }
    }

    pub fn init(&mut self) {}

    /// Get node address from global index
    pub fn get_node_addr(&self, global_idx: usize) -> Option<usize> {
        let offset = global_idx - INITIAL_NODES;
        let region_idx = offset / EXPAND_SIZE;
        let node_in_region = offset % EXPAND_SIZE;

        if region_idx >= self.region_count {
            return None;
        }

        let region = self.regions[region_idx];
        if node_in_region >= region.node_count {
            return None;
        }

        let node_addr = region.base_addr + node_in_region * NODE_SIZE;
        Some(node_addr)
    }

    /// Add new region
    pub fn add_region(&mut self, base_addr: usize, node_count: usize) {
        if self.region_count < MAX_REGIONS {
            self.regions[self.region_count] = DynamicNodeRegion {
                base_addr,
                node_count,
            };
            self.region_count += 1;
            self.total_allocated += node_count;
        }
    }

    pub fn total_allocated(&self) -> usize {
        self.total_allocated
    }
}

/// Global node pool with static and dynamic nodes
pub struct GlobalSlabNodePool {
    /// Static initial nodes (27 nodes)
    initial_nodes: [ListNode<SlabNode>; INITIAL_NODES],

    /// Dynamic region for expanded nodes
    dynamic_region: DynamicRegion,

    /// Free list head (only for dynamic nodes)
    dynamic_free_head: Option<usize>,

    /// Static nodes free list
    static_free_head: Option<usize>,
}

const INITIAL_NODES: usize = 27;
const EXPAND_SIZE: usize = 27;
const NODE_SIZE: usize = core::mem::size_of::<ListNode<SlabNode>>();

impl GlobalSlabNodePool {
    pub const fn new() -> Self {
        Self {
            initial_nodes: [ListNode::empty(); INITIAL_NODES],
            dynamic_region: DynamicRegion::new(),
            dynamic_free_head: None,
            static_free_head: None,
        }
    }

    /// Initialize static free list
    pub fn init(&mut self) {
        self.dynamic_region.init();

        for i in 0..INITIAL_NODES - 1 {
            self.initial_nodes[i].data = None;
            self.initial_nodes[i].prev = None;
            self.initial_nodes[i].next = Some(i + 1);
        }
        self.initial_nodes[INITIAL_NODES - 1].data = None;
        self.initial_nodes[INITIAL_NODES - 1].prev = None;
        self.initial_nodes[INITIAL_NODES - 1].next = None;

        self.static_free_head = Some(0);
    }

    /// Check if index is static node
    #[inline(always)]
    fn is_static(&self, idx: usize) -> bool {
        idx < INITIAL_NODES
    }

    /// Check if index is dynamic node
    #[inline(always)]
    #[allow(dead_code)]
    fn is_dynamic(&self, idx: usize) -> bool {
        idx >= INITIAL_NODES
    }

    /// Get node address from global index
    pub fn get_node_addr(&self, idx: usize) -> Option<usize> {
        if self.is_static(idx) {
            Some(&self.initial_nodes[idx] as *const _ as usize)
        } else {
            self.dynamic_region.get_node_addr(idx)
        }
    }

    /// Allocate a node
    pub fn alloc_node(
        &mut self,
        data: SlabNode,
        page_allocator: &mut dyn PageAllocatorForSlab,
        page_size: usize,
    ) -> AllocResult<usize> {
        // 1. Try static free node first
        if let Some(idx) = self.pop_from_static_free() {
            self.initial_nodes[idx].data = Some(data);
            return Ok(idx);
        }

        // 2. Try dynamic free node
        if let Some(idx) = self.pop_from_dynamic_free() {
            self.set_node_data(idx, data)?;
            return Ok(idx);
        }

        // 3. Expand dynamic region
        self.expand_and_alloc(data, page_allocator, page_size)
    }

    /// Pop from static free list
    fn pop_from_static_free(&mut self) -> Option<usize> {
        let idx = self.static_free_head?;
        self.static_free_head = self.initial_nodes[idx].next;
        Some(idx)
    }

    /// Pop from dynamic free list
    fn pop_from_dynamic_free(&mut self) -> Option<usize> {
        let idx = self.dynamic_free_head?;
        let node_addr = self.get_node_addr(idx)?;

        unsafe {
            let node = &mut *(node_addr as *mut ListNode<SlabNode>);
            self.dynamic_free_head = node.next;
            node.prev = None;
            node.next = None;
        }

        Some(idx)
    }

    /// Set node data
    fn set_node_data(&mut self, idx: usize, data: SlabNode) -> AllocResult<()> {
        let node_addr = self.get_node_addr(idx).ok_or_else(|| {
            crate::AllocError::NoMemory
        })?;

        unsafe {
            let node = &mut *(node_addr as *mut ListNode<SlabNode>);
            node.data = Some(data);
        }

        Ok(())
    }

    /// Expand dynamic region and allocate
    fn expand_and_alloc(
        &mut self,
        data: SlabNode,
        page_allocator: &mut dyn PageAllocatorForSlab,
        page_size: usize,
    ) -> AllocResult<usize> {
        let bytes_needed = EXPAND_SIZE * NODE_SIZE;
        let pages_needed = (bytes_needed + page_size - 1) / page_size;

        let base_addr = page_allocator.alloc_pages(pages_needed, page_size)?;

        let start_idx = INITIAL_NODES + self.dynamic_region.total_allocated();

        self.dynamic_region.add_region(base_addr, EXPAND_SIZE);

        self.batch_init_nodes(base_addr, start_idx);

        let idx = self.pop_from_dynamic_free().unwrap();
        self.set_node_data(idx, data)?;

        Ok(idx)
    }

    /// Batch initialize nodes into dynamic free list
    fn batch_init_nodes(&mut self, base: usize, start_global_idx: usize) {
        for i in (0..EXPAND_SIZE).rev() {
            let node_addr = base + i * NODE_SIZE;
            let global_idx = start_global_idx + i;

            unsafe {
                let node = &mut *(node_addr as *mut ListNode<SlabNode>);
                node.data = None;
                node.prev = None;
                node.next = self.dynamic_free_head;
            }

            self.dynamic_free_head = Some(global_idx);
        }
    }

    /// Free a node back to the pool
    pub fn free_node(&mut self, idx: usize, page_allocator: &mut dyn PageAllocatorForSlab, page_size: usize) {
        if self.is_static(idx) {
            self.initial_nodes[idx] = ListNode::empty();
            self.initial_nodes[idx].next = self.static_free_head;
            self.static_free_head = Some(idx);
        } else if let Some(node_addr) = self.get_node_addr(idx) {
            let offset = idx - INITIAL_NODES;
            let region_idx = offset / EXPAND_SIZE;

            if region_idx < self.dynamic_region.region_count {
                unsafe {
                    let node = &mut *(node_addr as *mut ListNode<SlabNode>);
                    node.data = None;
                    node.prev = None;
                    node.next = self.dynamic_free_head;
                }
                self.dynamic_free_head = Some(idx);

            } else {
                panic!("Invalid node index: {}", idx);
            }
        }
    }

    /// Get mutable reference to node data
    pub fn get_mut(&mut self, idx: usize) -> Option<&mut SlabNode> {
        let node_addr = self.get_node_addr(idx)?;
        unsafe { (&mut *(node_addr as *mut ListNode<SlabNode>)).data.as_mut() }
    }

    /// Get reference to node data
    pub fn get(&self, idx: usize) -> Option<&SlabNode> {
        let node_addr = self.get_node_addr(idx)?;
        unsafe { (&*(node_addr as *const ListNode<SlabNode>)).data.as_ref() }
    }

    /// Check if there are available nodes
    pub fn has_available(&self) -> bool {
        self.static_free_head.is_some() || self.dynamic_free_head.is_some()
    }

    /// Get count of available nodes
    pub fn available_count(&self) -> usize {
        let static_count = self.count_static_free();
        let dynamic_count = self.count_dynamic_free();
        static_count + dynamic_count
    }

    /// Get count of used nodes
    pub fn used_count(&self) -> usize {
        let static_used = INITIAL_NODES - self.count_static_free();
        let dynamic_used = self.dynamic_region.total_allocated() - self.count_dynamic_free();
        static_used + dynamic_used
    }

    /// Count static free nodes
    fn count_static_free(&self) -> usize {
        let mut count = 0;
        let mut current = self.static_free_head;
        while let Some(idx) = current {
            count += 1;
            current = self.initial_nodes[idx].next;
        }
        count
    }

    /// Count dynamic free nodes
    fn count_dynamic_free(&self) -> usize {
        let mut count = 0;
        let mut current = self.dynamic_free_head;
        while let Some(idx) = current {
            count += 1;
            if let Some(node_addr) = self.get_node_addr(idx) {
                unsafe {
                    let node = &*(node_addr as *const ListNode<SlabNode>);
                    current = node.next;
                }
            } else {
                break;
            }
        }
        count
    }

    /// Get node's prev pointer
    pub unsafe fn get_prev(&self, idx: usize) -> Option<usize> {
        let node_addr = self.get_node_addr(idx)?;
        let node = &*(node_addr as *const ListNode<SlabNode>);
        node.prev
    }

    /// Get node's next pointer
    pub unsafe fn get_next(&self, idx: usize) -> Option<usize> {
        let node_addr = self.get_node_addr(idx)?;
        let node = &*(node_addr as *const ListNode<SlabNode>);
        node.next
    }

    /// Set node's prev pointer
    pub unsafe fn set_prev(&mut self, idx: usize, prev: Option<usize>) {
        if let Some(node_addr) = self.get_node_addr(idx) {
            let node = &mut *(node_addr as *mut ListNode<SlabNode>);
            node.prev = prev;
        }
    }

    /// Set node's next pointer
    pub unsafe fn set_next(&mut self, idx: usize, next: Option<usize>) {
        if let Some(node_addr) = self.get_node_addr(idx) {
            let node = &mut *(node_addr as *mut ListNode<SlabNode>);
            node.next = next;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use super::super::slab_byte_allocator::SizeClass;
    use super::super::slab_node::SlabNode;
    use core::sync::atomic::{AtomicUsize, Ordering};

    struct MockPageAllocator {
        next_addr: AtomicUsize,
    }

    impl MockPageAllocator {
        fn new() -> Self {
            Self {
                next_addr: AtomicUsize::new(0x2000000),
            }
        }
    }

        impl PageAllocatorForSlab for MockPageAllocator {
        fn alloc_pages(&mut self, count: usize, _page_size: usize) -> AllocResult<usize> {
            Ok(self.next_addr.fetch_add(count * 4096, Ordering::SeqCst))
        }

        fn dealloc_pages(&mut self, _addr: usize, _count: usize) {}
    }

    #[test]
    fn test_static_pool_initialization() {
        let mut pool = GlobalSlabNodePool::new();
        pool.init();

        assert!(pool.has_available());
        assert_eq!(pool.available_count(), 27);
        assert_eq!(pool.used_count(), 0);
    }

    #[test]
    fn test_alloc_free_cycle() {
        let mut pool = GlobalSlabNodePool::new();
        pool.init();
        let mut page_allocator = MockPageAllocator::new();

        let node = SlabNode::new(0x1000, SizeClass::Bytes64);

        let idx = pool.alloc_node(node, &mut page_allocator, 4096).unwrap();
        assert_eq!(pool.available_count(), 26);
        assert_eq!(pool.used_count(), 1);

        let data = pool.get(idx).unwrap();
        assert_eq!(data.addr, 0x1000);

        pool.free_node(idx, &mut page_allocator, 4096);
        assert_eq!(pool.available_count(), 27);
        assert_eq!(pool.used_count(), 0);
    }

    #[test]
    fn test_multiple_allocs() {
        let mut pool = GlobalSlabNodePool::new();
        pool.init();
        let mut page_allocator = MockPageAllocator::new();

        let mut indices = alloc::vec::Vec::new();

        for i in 0..10 {
            let node = SlabNode::new(0x1000 + i * 0x1000, SizeClass::Bytes64);
            let idx = pool.alloc_node(node, &mut page_allocator, 4096).unwrap();
            indices.push(idx);
        }

        assert_eq!(pool.available_count(), 17);
        assert_eq!(pool.used_count(), 10);

        for idx in indices {
            pool.free_node(idx, &mut page_allocator, 4096);
        }

        assert_eq!(pool.available_count(), 27);
        assert_eq!(pool.used_count(), 0);
    }

    #[test]
    fn test_dynamic_expansion() {
        let mut pool = GlobalSlabNodePool::new();
        pool.init();
        let mut page_allocator = MockPageAllocator::new();

        let mut indices = alloc::vec::Vec::new();

        for i in 0..30 {
            let node = SlabNode::new(0x1000 + i * 0x1000, SizeClass::Bytes64);
            let idx = pool.alloc_node(node, &mut page_allocator, 4096).unwrap();
            indices.push(idx);
        }

        assert_eq!(pool.used_count(), 30);

        for idx in indices {
            pool.free_node(idx, &mut page_allocator, 4096);
        }

        assert_eq!(pool.available_count(), 54);
        assert_eq!(pool.used_count(), 0);
    }
}
