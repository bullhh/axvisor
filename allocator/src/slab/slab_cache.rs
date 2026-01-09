//! Slab cache implementation for a single size class.
//!
//! This module implements SlabCache which manages three lists (empty, partial, full)
//! of slab nodes for a specific size class.

use super::slab_byte_allocator::{PageAllocatorForSlab, SizeClass};
use super::slab_node::SlabNode;
use super::slab_node_pool::GlobalSlabNodePool;
use super::slab_pooled_list::SlabPooledLinkedList;
use crate::{AllocError, AllocResult};

/// Slab cache for a specific size class
pub struct SlabCache {
    size_class: SizeClass,
    pub(crate) empty: SlabPooledLinkedList,
    partial: SlabPooledLinkedList,
    full: SlabPooledLinkedList,
}

impl SlabCache {
    pub const fn new(size_class: SizeClass) -> Self {
        Self {
            size_class,
            empty: SlabPooledLinkedList::new(),
            partial: SlabPooledLinkedList::new(),
            full: SlabPooledLinkedList::new(),
        }
    }

    /// Allocate an object from this cache
    /// Returns (object_addr, bytes_allocated_from_page_allocator)
    pub fn alloc_object(
        &mut self,
        pool: &mut GlobalSlabNodePool,
        page_allocator: &mut dyn PageAllocatorForSlab,
        page_size: usize,
    ) -> AllocResult<(usize, usize)> {
        // 1. Try to allocate from partial list
        if let Some(node_idx) = self.partial.back() {
            let idx_copy = node_idx;
            if let Some(node) = pool.get_mut(idx_copy) {
                if let Some(obj_idx) = node.alloc_object() {
                    let obj_addr = node.object_addr(obj_idx);

                    if node.is_full() {
                        // Move from partial to full
                        self.partial.remove(pool, idx_copy);
                        self.full.push_back(pool, idx_copy);
                    }

                    return Ok((obj_addr, 0));
                }
            }
        }

        // 2. Try to allocate from empty list
        if let Some(node_idx) = self.empty.pop_back(pool) {
            let idx_copy = node_idx;
            if let Some(node) = pool.get_mut(idx_copy) {
                if let Some(obj_idx) = node.alloc_object() {
                    let obj_addr = node.object_addr(obj_idx);
                    self.partial.push_back(pool, idx_copy);

                    // Pre-allocate one empty node for future use
                    let prealloc_bytes =
                        self.preallocate_empty_node(pool, page_allocator, page_size);

                    return Ok((obj_addr, prealloc_bytes));
                }
            }
        }

        // 3. Allocate a new node from page allocator
        let (obj_addr, bytes) = self.allocate_new_node(pool, page_allocator, page_size)?;
        Ok((obj_addr, bytes))
    }

    /// Allocate a new slab node from page allocator
    /// Returns (object_addr, bytes_allocated_from_page_allocator)
    fn allocate_new_node(
        &mut self,
        pool: &mut GlobalSlabNodePool,
        page_allocator: &mut dyn PageAllocatorForSlab,
        page_size: usize,
    ) -> AllocResult<(usize, usize)> {
        let object_size = self.size_class.size();
        let bytes_needed = 512 * object_size;
        let page_count = (bytes_needed + page_size - 1) / page_size;

        let start_addr = page_allocator.alloc_pages(page_count, page_size)?;

        let new_node = SlabNode::new(start_addr, self.size_class);

        if let Some(node_idx) = pool.alloc_node(new_node) {
            if let Some(node) = pool.get_mut(node_idx) {
                if let Some(obj_idx) = node.alloc_object() {
                    let obj_addr = node.object_addr(obj_idx);
                    self.partial.push_back(pool, node_idx);

                    // Pre-allocate one empty node for future use
                    let prealloc_bytes =
                        self.preallocate_empty_node(pool, page_allocator, page_size);

                    return Ok((obj_addr, page_count * page_size + prealloc_bytes));
                }
            }
        }

        // Failed, deallocate pages
        page_allocator.dealloc_pages(start_addr, page_count);
        Err(AllocError::NoMemory)
    }

    /// Pre-allocate an empty node for future allocations
    /// Returns bytes allocated from page allocator (0 if already has empty nodes)
    fn preallocate_empty_node(
        &mut self,
        pool: &mut GlobalSlabNodePool,
        page_allocator: &mut dyn PageAllocatorForSlab,
        page_size: usize,
    ) -> usize {
        if self.empty.len() > 0 {
            return 0; // Already have empty nodes
        }

        let object_size = self.size_class.size();
        let bytes_needed = 512 * object_size;
        let page_count = (bytes_needed + page_size - 1) / page_size;

        if let Ok(start_addr) = page_allocator.alloc_pages(page_count, page_size) {
            let new_node = SlabNode::new(start_addr, self.size_class);
            if let Some(node_idx) = pool.alloc_node(new_node) {
                self.empty.push_back(pool, node_idx);
                return page_count * page_size;
            } else {
                page_allocator.dealloc_pages(start_addr, page_count);
            }
        }

        0
    }

    /// Deallocate an object
    /// Returns bytes freed from page allocator (if node was deallocated)
    pub fn dealloc_object(
        &mut self,
        pool: &mut GlobalSlabNodePool,
        obj_addr: usize,
        page_allocator: &mut dyn PageAllocatorForSlab,
        page_size: usize,
    ) -> usize {
        // First pass: search in partial list
        let mut found_idx = None;
        let mut is_in_partial = false;

        let mut current_partial = self.partial.front();
        while let Some(idx) = current_partial {
            if let Some(node) = pool.get(idx) {
                if node.object_index_from_addr(obj_addr).is_some() {
                    found_idx = Some(idx);
                    is_in_partial = true;
                    break;
                }
            }
            current_partial = unsafe { pool.get_next(idx) };
        }

        // Second pass: search in full list
        if found_idx.is_none() {
            let mut current_full = self.full.front();
            while let Some(idx) = current_full {
                if let Some(node) = pool.get(idx) {
                    if node.object_index_from_addr(obj_addr).is_some() {
                        found_idx = Some(idx);
                        break;
                    }
                }
                current_full = unsafe { pool.get_next(idx) };
            }
        }

        // Now deallocate with mutable access
        if let Some(idx) = found_idx {
            // First borrow: get node info and deallocate
            let (node_addr, page_count, should_dealloc_node) = if let Some(node) = pool.get_mut(idx)
            {
                if let Some(obj_idx) = node.object_index_from_addr(obj_addr) {
                    node.dealloc_object(obj_idx);
                    let is_empty = node.is_empty();
                    let addr = node.addr;
                    let pages = node.page_count(page_size);

                    (addr, pages, is_empty)
                } else {
                    panic!("Address mismatch during deallocation");
                }
            } else {
                panic!("Node not found");
            };

            // Second phase: manage lists based on state
            if should_dealloc_node {
                // Remove from current list
                if is_in_partial {
                    self.partial.remove(pool, idx);
                } else {
                    self.full.remove(pool, idx);
                }

                // Check if too many empty nodes (keep at most 2)
                if self.empty.len() >= 2 {
                    page_allocator.dealloc_pages(node_addr, page_count);
                    pool.free_node(idx);
                    return page_count * page_size;
                } else {
                    self.empty.push_back(pool, idx);
                    return 0;
                }
            } else if !is_in_partial {
                // Move from full to partial
                self.full.remove(pool, idx);
                self.partial.push_back(pool, idx);
            }

            return 0;
        }

        panic!(
            "Object address {:#x} not found in slab cache for size class {:?}",
            obj_addr, self.size_class
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use core::sync::atomic::{AtomicUsize, Ordering};

    // Re-import SizeClass for tests
    use super::super::slab_byte_allocator::SizeClass;

    struct MockPageAllocator {
        next_addr: AtomicUsize,
        allocated: alloc::vec::Vec<(usize, usize)>,
    }

    impl MockPageAllocator {
        fn new() -> Self {
            Self {
                next_addr: AtomicUsize::new(0x100000),
                allocated: alloc::vec::Vec::new(),
            }
        }
    }

    impl PageAllocatorForSlab for MockPageAllocator {
        fn alloc_pages(&mut self, num_pages: usize, _alignment: usize) -> AllocResult<usize> {
            let addr = self.next_addr.fetch_add(num_pages * 4096, Ordering::SeqCst);
            self.allocated.push((addr, num_pages));
            Ok(addr)
        }

        fn dealloc_pages(&mut self, pos: usize, num_pages: usize) {
            self.allocated
                .retain(|&(addr, count)| !(addr == pos && count == num_pages));
        }
    }

    #[test]
    fn test_alloc_dealloc() {
        let mut pool = GlobalSlabNodePool::new();
        pool.init_free_list();

        let mut cache = SlabCache::new(SizeClass::Bytes64);
        let mut page_allocator = MockPageAllocator::new();

        // Allocate an object
        let (obj_addr, _) = cache
            .alloc_object(&mut pool, &mut page_allocator, 4096)
            .unwrap();

        assert_ne!(obj_addr, 0);

        // Deallocate it
        cache.dealloc_object(&mut pool, obj_addr, &mut page_allocator, 4096);
    }

    #[test]
    fn test_multiple_allocs() {
        let mut pool = GlobalSlabNodePool::new();
        pool.init_free_list();

        let mut cache = SlabCache::new(SizeClass::Bytes64);
        let mut page_allocator = MockPageAllocator::new();

        // Allocate multiple objects
        let mut addrs = alloc::vec::Vec::new();
        for _ in 0..10 {
            let (addr, _) = cache
                .alloc_object(&mut pool, &mut page_allocator, 4096)
                .unwrap();
            addrs.push(addr);
        }

        // All allocations should succeed
        assert_eq!(addrs.len(), 10);

        // Deallocate all
        for addr in addrs {
            cache.dealloc_object(&mut pool, addr, &mut page_allocator, 4096);
        }
    }

    #[test]
    fn test_empty_node_management() {
        let mut pool = GlobalSlabNodePool::new();
        pool.init_free_list();

        let mut cache = SlabCache::new(SizeClass::Bytes64);
        let mut page_allocator = MockPageAllocator::new();

        // Allocate and deallocate to create empty nodes
        let (addr1, _) = cache
            .alloc_object(&mut pool, &mut page_allocator, 4096)
            .unwrap();
        cache.dealloc_object(&mut pool, addr1, &mut page_allocator, 4096);

        let (addr2, _) = cache
            .alloc_object(&mut pool, &mut page_allocator, 4096)
            .unwrap();
        cache.dealloc_object(&mut pool, addr2, &mut page_allocator, 4096);

        let (addr3, _) = cache
            .alloc_object(&mut pool, &mut page_allocator, 4096)
            .unwrap();
        cache.dealloc_object(&mut pool, addr3, &mut page_allocator, 4096);

        // Should have at most 2 empty nodes
        assert!(cache.empty.len() <= 2);
    }
}
