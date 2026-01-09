//! Slab node implementation.
//!
//! This module defines the SlabNode structure which manages exactly 512 objects
//! using a fixed bitmap.

use log::{error, warn};

pub use super::slab_byte_allocator::SizeClass;

/// Slab node managing exactly 512 objects
#[derive(Debug, Clone, Copy)]
pub struct SlabNode {
    pub addr: usize,           // Starting physical address
    pub size_class: SizeClass, // Size class
    pub free_bitmap: [u64; 8], // Bitmap (512 bits)
}

impl SlabNode {
    pub const MAX_OBJECTS: usize = 512;

    /// Create a new slab node with all objects free
    pub const fn new(addr: usize, size_class: SizeClass) -> Self {
        Self {
            addr,
            size_class,
            free_bitmap: [u64::MAX; 8], // All free
        }
    }

    /// Calculate used objects from bitmap
    pub fn in_use(&self) -> u32 {
        self.free_bitmap.iter().map(|w| w.count_zeros()).sum()
    }

    /// Calculate free objects from bitmap
    pub fn free_count(&self) -> u32 {
        self.free_bitmap.iter().map(|w| w.count_ones()).sum()
    }

    /// Check if node is full
    pub fn is_full(&self) -> bool {
        self.free_bitmap.iter().all(|&w| w == 0)
    }

    /// Check if node is empty
    pub fn is_empty(&self) -> bool {
        self.free_bitmap.iter().all(|&w| w == u64::MAX)
    }

    /// Allocate one object, return object index
    pub fn alloc_object(&mut self) -> Option<usize> {
        for (word_idx, &word) in self.free_bitmap.iter().enumerate() {
            if word != 0 {
                let bit_pos = word.trailing_zeros() as usize;
                let object_index = word_idx * 64 + bit_pos;

                if object_index >= Self::MAX_OBJECTS {
                    continue;
                }

                self.free_bitmap[word_idx] &= !(1u64 << bit_pos);
                return Some(object_index);
            }
        }
        None
    }

    /// Deallocate one object by index
    pub fn dealloc_object(&mut self, object_index: usize) {
        if object_index < Self::MAX_OBJECTS {
            let word_idx = object_index / 64;
            let bit_idx = object_index % 64;
            self.free_bitmap[word_idx] |= 1u64 << bit_idx;
        }
    }

    /// Get object physical address
    pub fn object_addr(&self, object_index: usize) -> usize {
        self.addr + object_index * self.size_class.size()
    }

    /// Get object index from physical address
    pub fn object_index_from_addr(&self, obj_addr: usize) -> Option<usize> {
        if obj_addr < self.addr {
            return None;
        }

        let offset = obj_addr - self.addr;
        if offset % self.size_class.size() != 0 {
            error!("Invalid object address: {:x}", obj_addr);
            return None;
        }

        let object_index = offset / self.size_class.size();

        if object_index < Self::MAX_OBJECTS {
            Some(object_index)
        } else {
            None
        }
    }

    /// Calculate required page count
    pub fn page_count(&self, page_size: usize) -> usize {
        let object_size = self.size_class.size();
        let bytes_needed = Self::MAX_OBJECTS * object_size;
        (bytes_needed + page_size - 1) / page_size
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_slab_node() {
        let mut node = SlabNode::new(0x1000, SizeClass::Bytes64);

        assert!(node.is_empty());
        assert!(!node.is_full());
        assert_eq!(node.free_count(), 512);
        assert_eq!(node.in_use(), 0);

        // Test allocation
        let obj_idx = node.alloc_object().unwrap();
        assert_eq!(obj_idx, 0);
        assert_eq!(node.object_addr(obj_idx), 0x1000);
        assert_eq!(node.in_use(), 1);
        assert_eq!(node.free_count(), 511);

        // Test deallocation
        node.dealloc_object(obj_idx);
        assert!(node.is_empty());
        assert_eq!(node.in_use(), 0);
        assert_eq!(node.free_count(), 512);
    }

    #[test]
    fn test_object_index_from_addr() {
        let node = SlabNode::new(0x1000, SizeClass::Bytes64);

        assert_eq!(node.object_index_from_addr(0x1000), Some(0));
        assert_eq!(node.object_index_from_addr(0x1000 + 64), Some(1));
        assert_eq!(
            node.object_index_from_addr(0x1000 + 63),
            None // Not aligned
        );
        assert_eq!(
            node.object_index_from_addr(0x1000 + 512 * 64),
            None // Out of range
        );
    }

    #[test]
    fn test_page_count() {
        let node8 = SlabNode::new(0, SizeClass::Bytes8);
        assert_eq!(node8.page_count(4096), 1); // 512 * 8 = 4096

        let node64 = SlabNode::new(0, SizeClass::Bytes64);
        assert_eq!(node64.page_count(4096), 8); // 512 * 64 = 32768

        let node2048 = SlabNode::new(0, SizeClass::Bytes2048);
        assert_eq!(node2048.page_count(4096), 256); // 512 * 2048 = 1,048,576
    }
}
