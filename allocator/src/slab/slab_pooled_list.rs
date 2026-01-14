//! Pooled linked list implementation.
//!
//! This module implements a linked list that uses nodes from GlobalSlabNodePool,
//! providing O(1) operations without heap allocation.

use super::slab_node_pool::GlobalSlabNodePool;
use super::slab_node::SlabNode;

/// Pooled linked list using GlobalSlabNodePool
pub struct SlabPooledLinkedList {
    head: Option<usize>,
    tail: Option<usize>,
    len: usize,
}

impl SlabPooledLinkedList {
    pub const fn new() -> Self {
        Self {
            head: None,
            tail: None,
            len: 0,
        }
    }

    pub fn is_empty(&self) -> bool {
        self.head.is_none()
    }

    pub fn len(&self) -> usize {
        self.len
    }

    /// Push to back of list
    pub fn push_back(&mut self, pool: &mut GlobalSlabNodePool, idx: usize) {
        if let Some(node_addr) = pool.get_node_addr(idx) {
            unsafe {
                let node = &mut *(node_addr as *mut super::slab_node_pool::ListNode<SlabNode>);
                node.next = None;

                if let Some(tail_idx) = self.tail {
                    if let Some(tail_addr) = pool.get_node_addr(tail_idx) {
                        let tail = &mut *(tail_addr as *mut super::slab_node_pool::ListNode<SlabNode>);
                        tail.next = Some(idx);
                        node.prev = Some(tail_idx);
                    }
                } else {
                    node.prev = None;
                    self.head = Some(idx);
                }

                self.tail = Some(idx);
            }
        }
        self.len += 1;
    }

    /// Pop from back of list
    pub fn pop_back(&mut self, pool: &mut GlobalSlabNodePool) -> Option<usize> {
        let idx = self.tail?;

        if let Some(node_addr) = pool.get_node_addr(idx) {
            unsafe {
                let node = &*(node_addr as *const super::slab_node_pool::ListNode<SlabNode>);
                self.tail = node.prev;

                if let Some(new_tail) = self.tail {
                    if let Some(tail_addr) = pool.get_node_addr(new_tail) {
                        let tail = &mut *(tail_addr as *mut super::slab_node_pool::ListNode<SlabNode>);
                        tail.next = None;
                    }
                } else {
                    self.head = None;
                }
            }
        }

        self.len -= 1;
        Some(idx)
    }

    /// Pop from front of list
    pub fn pop_front(&mut self, pool: &mut GlobalSlabNodePool) -> Option<usize> {
        let idx = self.head?;

        if let Some(node_addr) = pool.get_node_addr(idx) {
            unsafe {
                let node = &*(node_addr as *const super::slab_node_pool::ListNode<SlabNode>);
                self.head = node.next;

                if let Some(new_head) = self.head {
                    if let Some(head_addr) = pool.get_node_addr(new_head) {
                        let head = &mut *(head_addr as *mut super::slab_node_pool::ListNode<SlabNode>);
                        head.prev = None;
                    }
                } else {
                    self.tail = None;
                }
            }
        }

        self.len -= 1;
        Some(idx)
    }

    /// Remove node by index
    pub fn remove(&mut self, pool: &mut GlobalSlabNodePool, idx: usize) {
        if let Some(node_addr) = pool.get_node_addr(idx) {
            unsafe {
                let node = &*(node_addr as *const super::slab_node_pool::ListNode<SlabNode>);
                let prev = node.prev;
                let next = node.next;

                match prev {
                    Some(prev_idx) => {
                        if let Some(prev_addr) = pool.get_node_addr(prev_idx) {
                            let prev_node = &mut *(prev_addr as *mut super::slab_node_pool::ListNode<SlabNode>);
                            prev_node.next = next;
                        }
                    }
                    None => {
                        self.head = next;
                    }
                }

                match next {
                    Some(next_idx) => {
                        if let Some(next_addr) = pool.get_node_addr(next_idx) {
                            let next_node = &mut *(next_addr as *mut super::slab_node_pool::ListNode<SlabNode>);
                            next_node.prev = prev;
                        }
                    }
                    None => {
                        self.tail = prev;
                    }
                }
            }
        }

        self.len -= 1;
    }

    /// Get back node index
    pub fn back(&self) -> Option<usize> {
        self.tail
    }

    /// Get front node index
    pub fn front(&self) -> Option<usize> {
        self.head
    }

    /// Iterate over all indices and apply a callback function
    pub fn for_each_index<F>(&self, pool: &GlobalSlabNodePool, mut f: F)
    where
        F: FnMut(usize),
    {
        let mut current = self.head;
        while let Some(idx) = current {
            f(idx);
            if let Some(node_addr) = pool.get_node_addr(idx) {
                unsafe {
                    let node = &*(node_addr as *const super::slab_node_pool::ListNode<SlabNode>);
                    current = node.next;
                }
            } else {
                break;
            }
        }
    }

    /// Check if list contains a specific index
    pub fn contains(&self, pool: &GlobalSlabNodePool, target: usize) -> bool {
        let mut current = self.head;
        while let Some(idx) = current {
            if idx == target {
                return true;
            }
            if let Some(node_addr) = pool.get_node_addr(idx) {
                unsafe {
                    let node = &*(node_addr as *const super::slab_node_pool::ListNode<SlabNode>);
                    current = node.next;
                }
            } else {
                break;
            }
        }
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Re-import for tests
    use super::super::slab_node::{SizeClass, SlabNode};

    #[test]
    fn test_empty_list() {
        let list = SlabPooledLinkedList::new();
        assert!(list.is_empty());
        assert_eq!(list.len(), 0);
        assert!(list.front().is_none());
        assert!(list.back().is_none());
    }

    #[test]
    fn test_push_back() {
        let mut pool = GlobalSlabNodePool::new();
        pool.init();

        let mut list = SlabPooledLinkedList::new();
        let mut page_allocator = MockPageAllocator::new();

        let node1 = SlabNode::new(0x1000, SizeClass::Bytes64);
        let idx1 = pool.alloc_node(node1, &mut page_allocator, 4096).unwrap();
        list.push_back(&mut pool, idx1);

        assert!(!list.is_empty());
        assert_eq!(list.len(), 1);
        assert_eq!(list.front(), Some(idx1));
        assert_eq!(list.back(), Some(idx1));
    }

    #[test]
    fn test_push_multiple() {
        let mut pool = GlobalSlabNodePool::new();
        pool.init();

        let mut list = SlabPooledLinkedList::new();
        let mut page_allocator = MockPageAllocator::new();

        let mut indices = alloc::vec::Vec::new();
        for i in 0..5 {
            let node = SlabNode::new(0x1000 + i * 0x1000, SizeClass::Bytes64);
            let idx = pool.alloc_node(node, &mut page_allocator, 4096).unwrap();
            indices.push(idx);
            list.push_back(&mut pool, idx);
        }

        assert_eq!(list.len(), 5);
        assert_eq!(list.front(), Some(indices[0]));
        assert_eq!(list.back(), Some(indices[4]));

        let mut collected = alloc::vec::Vec::new();
        list.for_each_index(&pool, |idx| collected.push(idx));
        assert_eq!(collected, indices);
    }

    #[test]
    fn test_pop_back() {
        let mut pool = GlobalSlabNodePool::new();
        pool.init();

        let mut list = SlabPooledLinkedList::new();
        let mut page_allocator = MockPageAllocator::new();

        let node1 = SlabNode::new(0x1000, SizeClass::Bytes64);
        let node2 = SlabNode::new(0x2000, SizeClass::Bytes64);

        let idx1 = pool.alloc_node(node1, &mut page_allocator, 4096).unwrap();
        let idx2 = pool.alloc_node(node2, &mut page_allocator, 4096).unwrap();

        list.push_back(&mut pool, idx1);
        list.push_back(&mut pool, idx2);

        let popped = list.pop_back(&mut pool).unwrap();
        assert_eq!(popped, idx2);
        assert_eq!(list.len(), 1);
        assert_eq!(list.back(), Some(idx1));

        let popped = list.pop_back(&mut pool).unwrap();
        assert_eq!(popped, idx1);
        assert!(list.is_empty());
    }

    #[test]
    fn test_remove_middle() {
        let mut pool = GlobalSlabNodePool::new();
        pool.init();

        let mut list = SlabPooledLinkedList::new();
        let mut page_allocator = MockPageAllocator::new();

        let mut indices = alloc::vec::Vec::new();
        for i in 0..5 {
            let node = SlabNode::new(0x1000 + i * 0x1000, SizeClass::Bytes64);
            let idx = pool.alloc_node(node, &mut page_allocator, 4096).unwrap();
            indices.push(idx);
            list.push_back(&mut pool, idx);
        }

        list.remove(&mut pool, indices[2]);

        assert_eq!(list.len(), 4);

        let mut collected = alloc::vec::Vec::new();
        list.for_each_index(&pool, |idx| collected.push(idx));
        assert_eq!(
            collected,
            alloc::vec![indices[0], indices[1], indices[3], indices[4]]
        );
    }

    #[test]
    fn test_remove_front() {
        let mut pool = GlobalSlabNodePool::new();
        pool.init();

        let mut list = SlabPooledLinkedList::new();
        let mut page_allocator = MockPageAllocator::new();

        let mut indices = alloc::vec::Vec::new();
        for i in 0..3 {
            let node = SlabNode::new(0x1000 + i * 0x1000, SizeClass::Bytes64);
            let idx = pool.alloc_node(node, &mut page_allocator, 4096).unwrap();
            indices.push(idx);
            list.push_back(&mut pool, idx);
        }

        list.remove(&mut pool, indices[0]);

        assert_eq!(list.front(), Some(indices[1]));
        assert_eq!(list.len(), 2);
    }

    #[test]
    fn test_remove_back() {
        let mut pool = GlobalSlabNodePool::new();
        pool.init();

        let mut list = SlabPooledLinkedList::new();
        let mut page_allocator = MockPageAllocator::new();

        let mut indices = alloc::vec::Vec::new();
        for i in 0..3 {
            let node = SlabNode::new(0x1000 + i * 0x1000, SizeClass::Bytes64);
            let idx = pool.alloc_node(node, &mut page_allocator, 4096).unwrap();
            indices.push(idx);
            list.push_back(&mut pool, idx);
        }

        list.remove(&mut pool, indices[2]);

        assert_eq!(list.back(), Some(indices[1]));
        assert_eq!(list.len(), 2);
    }

    struct MockPageAllocator {
        next_addr: core::sync::atomic::AtomicUsize,
    }

    impl MockPageAllocator {
        fn new() -> Self {
            Self {
                next_addr: core::sync::atomic::AtomicUsize::new(0x2000000),
            }
        }
    }

    impl super::super::slab_node_pool::PageAllocatorForSlab for MockPageAllocator {
        fn alloc_pages(&mut self, count: usize, _page_size: usize) -> crate::AllocResult<usize> {
            Ok(self.next_addr.fetch_add(count * 4096, core::sync::atomic::Ordering::SeqCst))
        }

        fn dealloc_pages(&mut self, _addr: usize, _count: usize) {}
    }
}
