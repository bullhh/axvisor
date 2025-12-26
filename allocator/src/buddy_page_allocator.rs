//! Buddy page allocator implementation for Axvisor.
//! 
//! This module implements a complete buddy system for page-level allocation
//! with support for memory regions, statistics, and proper buddy merging.

use crate::{AllocError, AllocResult, BaseAllocator, PageAllocator};
use super::slab_byte_allocator::PageAllocatorForSlab;
use log::{info, warn};

#[cfg(test)]
use alloc::format;

/// Simple static linked list node
#[derive(Debug, Clone, Copy)]
struct ListNode<T> {
    data: T,
    next: Option<usize>,
}

/// Static linked list implementation that doesn't require dynamic allocation
#[derive(Debug)]
pub struct StaticLinkedList<T, const N: usize> {
    nodes: [Option<ListNode<T>>; N],
    head: Option<usize>,
    tail: Option<usize>,
    free_head: Option<usize>,
    len: usize,
}

impl<T, const N: usize> StaticLinkedList<T, N> {
    /// Create a new empty static linked list
    pub const fn new() -> Self {
        Self {
            nodes: [const { None }; N],
            head: None,
            tail: None,
            free_head: Some(0),
            len: 0,
        }
    }

    /// Initialize the free list
    pub fn init(&mut self) {
        // Initialize all nodes as free
        for i in 0..N {
            // Ensure i is within bounds (should always be true with 0..N)
            debug_assert!(i < N, "Index out of bounds in StaticLinkedList::init");
            
            self.nodes[i] = Some(ListNode {
                data: unsafe { core::mem::zeroed() },
                next: if i < N-1 { Some(i + 1) } else { None },
            });
        }
        self.free_head = Some(0);
        self.head = None;
        self.tail = None;
        self.len = 0;
    }

    /// Push an element to the back of the list
    pub fn push_back(&mut self, data: T) -> bool {
        if self.free_head.is_none() {
            return false; // No free nodes available
        }

        // Get a free node index and ensure it's within bounds
        let new_node_idx = self.free_head.unwrap();
        if new_node_idx >= N {
            return false; // Invalid free node index
        }
        
        // Get the next free node before modifying the current one
        let next_free = match &self.nodes[new_node_idx] {
            Some(node) => node.next,
            None => {
                // Node is invalid, try to initialize it
                self.nodes[new_node_idx] = Some(ListNode {
                    data: unsafe { core::mem::zeroed() }, 
                    next: if new_node_idx < N-1 { Some(new_node_idx + 1) } else { None }
                });
                self.nodes[new_node_idx].as_ref().unwrap().next
            }
        };
        
        // Update the node with new data
        self.nodes[new_node_idx] = Some(ListNode {
            data,
            next: None,
        });
        
        // Update free head
        self.free_head = next_free;
        
        // Add to list
        if self.tail.is_none() {
            // Empty list
            self.head = Some(new_node_idx);
            self.tail = Some(new_node_idx);
        } else {
            // Append to tail
            let tail_idx = self.tail.unwrap();
            if tail_idx >= N {
                return false; // Invalid tail index
            }
            
            // Ensure tail node exists
            if let Some(tail_node) = self.nodes[tail_idx].as_mut() {
                tail_node.next = Some(new_node_idx);
            } else {
                // Tail node is invalid, reinitialize it with dummy data
                self.nodes[tail_idx] = Some(ListNode { 
                    data: unsafe { core::mem::zeroed() }, 
                    next: Some(new_node_idx)
                });
            }
            self.tail = Some(new_node_idx);
        }
        
        self.len += 1;
        true
    }

    /// Pop an element from the front of the list
    pub fn pop_front(&mut self) -> Option<T> {
        if self.head.is_none() {
            return None;
        }

        // Get head node index and ensure it's within bounds
        let head_idx = self.head.unwrap();
        if head_idx >= N {
            return None; // Invalid head index
        }
        
        // Get head node
        let Some(head_node) = self.nodes[head_idx].take() else {
            return None; // Invalid head node
        };
        
        // Update head
        self.head = head_node.next;
        if self.head.is_none() {
            self.tail = None;
        }
        
        // Return node to free list
        // Create a new dummy node to return to free list
        let node = ListNode { data: unsafe { core::mem::zeroed() }, next: self.free_head };
        self.nodes[head_idx] = Some(node);
        self.free_head = Some(head_idx);
        
        self.len -= 1;
        Some(head_node.data)
    }

    /// Check if the list is empty
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// Get the length of the list
    pub fn len(&self) -> usize {
        self.len
    }
}

const PAGE_SIZE: usize = 0x1000;
pub const DEFAULT_MAX_ORDER: usize = 28; // Support up to 256GB allocations (2^28 * 4KB)

/// Buddy system statistics
#[derive(Debug, Clone, Copy)]
pub struct BuddyStats {
    pub total_pages: usize,
    pub free_pages: usize,
    pub used_pages: usize,
    pub free_pages_by_order: [usize; DEFAULT_MAX_ORDER + 1],
}

impl Default for BuddyStats {
    fn default() -> Self {
        Self {
            total_pages: 0,
            free_pages: 0,
            used_pages: 0,
            free_pages_by_order: [0; DEFAULT_MAX_ORDER + 1],
        }
    }
}

impl BuddyStats {
    pub const fn new() -> Self {
        Self {
            total_pages: 0,
            free_pages: 0,
            used_pages: 0,
            free_pages_by_order: [0; DEFAULT_MAX_ORDER + 1],
        }
    }
}

/// A memory region descriptor
#[derive(Debug, Clone)]
pub struct MemoryRegion {
    pub start_addr: usize,
    pub size: usize,
}

/// Buddy block metadata
#[derive(Debug, Clone, Copy)]
struct BuddyBlock {
    order: usize,
    addr: usize,
}

impl BuddyBlock {
    fn buddy_addr(&self, base_addr: usize) -> usize {
        self.addr ^ ((1 << self.order) * PAGE_SIZE)
    }

    fn is_first(&self, base_addr: usize) -> bool {
        ((self.addr - base_addr) / ((1 << self.order) * PAGE_SIZE)) % 2 == 0
    }
}

/// Maximum number of blocks in each free list
const MAX_BLOCKS_PER_LIST: usize = 64;

/// Buddy set implementation
#[derive(Debug)]
pub struct BuddySet {
    base_addr: usize,
    total_pages: usize,
    free_lists: [StaticLinkedList<BuddyBlock, MAX_BLOCKS_PER_LIST>; DEFAULT_MAX_ORDER + 1],
    is_initialized: bool,  // Track if already initialized
}

impl BuddySet {
    /// Create a new buddy set
    pub const fn new(base_addr: usize, total_pages: usize) -> Self {
        // Use const block for array initialization
        Self {
            base_addr,
            total_pages,
            free_lists: [const { StaticLinkedList::new() }; DEFAULT_MAX_ORDER + 1],
            is_initialized: false,
        }
    }

    /// Create an empty buddy set
    pub const fn empty() -> Self {
        Self::new(0, 0)
    }

    pub const fn max_order(&self) -> usize {
        DEFAULT_MAX_ORDER
    }

    pub fn init(&mut self, base_addr: usize, size: usize) {
        info!("buddy set: Initialize with region [{:#x}, {:#x})", base_addr, base_addr + size);

        self.base_addr = base_addr;
        self.total_pages = size / PAGE_SIZE;
        
        // Initialize all free lists
        for list in &mut self.free_lists {
            list.init();
        }
        
        // Proper buddy system initialization: create largest possible blocks
        let mut remaining_pages = self.total_pages;
        let mut current_addr = base_addr;
        
        // Start from the largest possible order and work our way down
        for order in (0..=DEFAULT_MAX_ORDER).rev() {
            let block_pages = 1 << order;
            
            // While we can fit a block of this order in the remaining memory
            while remaining_pages >= block_pages {
                // Push_back should not fail in initialization since we have sufficient capacity
                let success = self.free_lists[order].push_back(BuddyBlock { order, addr: current_addr });
                info!("buddy set: Added block of order {} at address {:#x}", order, current_addr);
                assert!(success, "Failed to push block to free list");
                remaining_pages -= block_pages;
                current_addr += block_pages * PAGE_SIZE;
            }
        }
        
        // In case there's any remaining memory (shouldn't happen if size is multiple of PAGE_SIZE)
        assert!(remaining_pages == 0, "Remaining pages after buddy init: {}", remaining_pages);
        
        info!("buddy set: {} pages initialized in buddy system", self.total_pages);
    }

    pub fn alloc_pages(&mut self, num_pages: usize, _align_pow2: usize) -> AllocResult<usize> {
        if num_pages == 0 {
            return Err(AllocError::InvalidParam);
        }
        
        // Find the required order
        let required_order = if num_pages.is_power_of_two() {
            num_pages.trailing_zeros() as usize
        } else {
            num_pages.next_power_of_two().trailing_zeros() as usize
        }.min(DEFAULT_MAX_ORDER);

        // Try to find a block of the required order or higher
        for order in required_order..=DEFAULT_MAX_ORDER {
            if !self.free_lists[order].is_empty() {
                let mut block = self.free_lists[order].pop_front().unwrap();
                
                // Split down to required order
                while block.order > required_order {
                    block.order -= 1;
                    let buddy_addr = block.addr + (1 << block.order) * PAGE_SIZE;
                    // Push_back should not fail in normal operation
                    let success = self.free_lists[block.order].push_back(BuddyBlock {
                        order: block.order,
                        addr: buddy_addr,
                    });
                    assert!(success, "Failed to push buddy block to free list");
                }
                
                return Ok(block.addr);
            }
        }
        info!("buddy set: Allocation of {} pages failed - no suitable block found", num_pages);
        Err(AllocError::NoMemory)
    }

    /// Find a block with the given address in the free list of the given order
    fn find_block_in_free_list(&self, order: usize, addr: usize) -> Option<usize> {
        let list = &self.free_lists[order];
        let mut current_idx = list.head;
        
        while let Some(idx) = current_idx {
            if let Some(node) = &list.nodes[idx] {
                if node.data.addr == addr {
                    return Some(idx);
                }
                current_idx = node.next;
            } else {
                break;
            }
        }
        
        None
    }
    
    /// Remove a block from the free list at the given position
    fn remove_block_from_free_list(&mut self, order: usize, node_idx: usize) {
        let list = &mut self.free_lists[order];
        
        // Find the previous node
        let mut prev_idx = None;
        let mut current_idx = list.head;
        
        while let Some(idx) = current_idx {
            if idx == node_idx {
                break;
            }
            prev_idx = current_idx;
            if let Some(node) = &list.nodes[idx] {
                current_idx = node.next;
            } else {
                break;
            }
        }
        
        // Remove the node
        if let Some(node) = list.nodes[node_idx].take() {
            // Update links
            if let Some(prev_idx) = prev_idx {
                // Node is in the middle or end
                if let Some(prev_node) = &mut list.nodes[prev_idx] {
                    prev_node.next = node.next;
                }
            } else {
                // Node is the head
                list.head = node.next;
            }
            
            // Update tail if needed
            if list.tail == Some(node_idx) {
                list.tail = prev_idx;
            }
            
            // Return node to free list
            let dummy_node = ListNode { 
                data: unsafe { core::mem::zeroed() }, 
                next: list.free_head 
            };
            list.nodes[node_idx] = Some(dummy_node);
            list.free_head = Some(node_idx);
            list.len -= 1;
        }
    }

    pub fn dealloc_pages(&mut self, addr: usize, num_pages: usize) {
        if num_pages == 0 {
            return;
        }
        
        let mut order = if num_pages.is_power_of_two() {
            num_pages.trailing_zeros() as usize
        } else {
            num_pages.next_power_of_two().trailing_zeros() as usize
        }.min(DEFAULT_MAX_ORDER);
        
        let mut block = BuddyBlock { order, addr };
        
        // Try to merge with buddy blocks
        while order < DEFAULT_MAX_ORDER {
            let buddy_addr = block.buddy_addr(self.base_addr);
            
            // Check if buddy is in the free list
            if let Some(buddy_pos) = self.find_block_in_free_list(order, buddy_addr) {
                // Remove buddy from free list
                self.remove_block_from_free_list(order, buddy_pos);
                
                // Merge into a larger block
                if block.is_first(self.base_addr) {
                    // Keep current address
                } else {
                    block.addr = buddy_addr; // Use buddy's address
                }
                block.order += 1;
                order += 1;
            } else {
                // Can't merge, add current block to free list
                break;
            }
        }
        
        // Add the final merged block to the appropriate free list
        let success = self.free_lists[block.order].push_back(block);
        assert!(success, "Failed to push block to free list during deallocation");
    }

    pub fn get_stats(&self) -> BuddyStats {
        let mut stats = BuddyStats::new();
        stats.total_pages = self.total_pages;
        
        for (order, list) in self.free_lists.iter().enumerate() {
            let pages_in_order = list.len() * (1 << order);
            stats.free_pages_by_order[order] = list.len();
            stats.free_pages += pages_in_order;
        }
        
        stats.used_pages = stats.total_pages.saturating_sub(stats.free_pages);
        stats
    }

    /// Get detailed free list information as a string
    pub fn get_free_lists_info(&self) -> alloc::string::String {
        let mut result = alloc::string::String::new();
        result.push_str("=== Buddy Free Lists Info ===\n");
        result.push_str(&alloc::format!("Base Address: {:#x}\n", self.base_addr));
        result.push_str(&alloc::format!("Total Pages: {}\n", self.total_pages));
        
        for (order, list) in self.free_lists.iter().enumerate() {
            if !list.is_empty() {
                let block_size = (1 << order) * PAGE_SIZE;
                let total_pages_in_order = list.len() * (1 << order);
                let total_bytes_in_order = total_pages_in_order * PAGE_SIZE;
                
                result.push_str(&alloc::format!(
                    "Order {}: {} blocks, {} bytes each, {} total pages ({} KB, {} MB)\n", 
                    order, 
                    list.len(), 
                    block_size,
                    total_pages_in_order,
                    total_bytes_in_order / 1024,
                    total_bytes_in_order / (1024 * 1024)
                ));
                
                // Print first few block addresses for debugging
                let mut current_idx = list.head;
                let mut count = 0;
                result.push_str("  Block addresses: ");
                while let Some(idx) = current_idx {
                    if let Some(node) = &list.nodes[idx] {
                        result.push_str(&alloc::format!("{:#x} ", node.data.addr));
                        current_idx = node.next;
                        count += 1;
                        if count >= 8 { // Show max 8 addresses to avoid spam
                            result.push_str("... ");
                            break;
                        }
                    } else {
                        break;
                    }
                }
                result.push_str("\n");
            }
        }
        
        // Print summary
        let stats = self.get_stats();
        result.push_str("\nSummary:\n");
        result.push_str(&alloc::format!("  Free pages: {} / {}\n", stats.free_pages, stats.total_pages));
        result.push_str(&alloc::format!("  Used pages: {}\n", stats.used_pages));
        result.push_str(&alloc::format!("  Free memory: {} KB / {} KB\n", 
            (stats.free_pages * PAGE_SIZE) / 1024,
            (stats.total_pages * PAGE_SIZE) / 1024
        ));
        result.push_str("================================\n");
        
        result
    }
}

impl Default for BuddySet {
    fn default() -> Self {
        Self::empty()
    }
}

/// Buddy page allocator
pub struct BuddyPageAllocator {
    global_pool: BuddySet,
    stats: BuddyStats,
}

impl BuddyPageAllocator {
    pub const fn new() -> Self {
        Self {
            global_pool: BuddySet::empty(),
            stats: BuddyStats::new(),
        }
    }

    pub fn bootstrap(&mut self, base_addr: usize, size: usize) {
        self.global_pool.init(base_addr, size);
        self.stats = self.global_pool.get_stats();
    }

    pub fn get_stats(&self) -> BuddyStats {
        self.global_pool.get_stats()
    }

    /// Get detailed free list information as a string
    pub fn get_free_lists_info(&self) -> alloc::string::String {
        self.global_pool.get_free_lists_info()
    }

    /// Add a new memory region to the buddy system
    fn add_memory_region(&mut self, start: usize, size: usize) -> AllocResult {
        info!("buddy allocator: Adding region [{:#x}, {:#x})", start, start + size);
        
        let new_pages = size / PAGE_SIZE;
        
        // Add new pages to total count
        self.global_pool.total_pages += new_pages;
        
        // Initialize this region as buddy blocks
        let mut remaining_pages = new_pages;
        let mut current_addr = start;
        
        // Start from the largest possible order and work our way down
        for order in (0..=DEFAULT_MAX_ORDER).rev() {
            let block_pages = 1 << order;
            
            // While we can fit a block of this order in the remaining memory
            while remaining_pages >= block_pages {
                let success = self.global_pool.free_lists[order].push_back(BuddyBlock { 
                    order, 
                    addr: current_addr 
                });
                if !success {
                    warn!("Failed to add block of order {} at {:#x} - free list full", order, current_addr);
                    break;
                }
                info!("buddy allocator: Added block of order {} at address {:#x}", order, current_addr);
                remaining_pages -= block_pages;
                current_addr += block_pages * PAGE_SIZE;
            }
            
            if remaining_pages == 0 {
                break;
            }
        }
        
        info!("buddy allocator: Added {} pages from new region, total pages now: {}", 
              new_pages, self.global_pool.total_pages);
        
        Ok(())
    }
}

impl Default for BuddyPageAllocator {
    fn default() -> Self {
        Self::new()
    }
}

impl BaseAllocator for BuddyPageAllocator {
    fn init(&mut self, start: usize, size: usize) {
        self.bootstrap(start, size);
    }

    fn add_memory(&mut self, start: usize, size: usize) -> AllocResult {
        info!("buddy allocator: Adding memory region [{:#x}, {:#x})", start, start + size);
        
        // Add the new memory region to the existing buddy system
        self.add_memory_region(start, size)?;
        self.stats = self.global_pool.get_stats();
        Ok(())
    }
}

impl PageAllocator for BuddyPageAllocator {
    const PAGE_SIZE: usize = PAGE_SIZE;

    fn alloc_pages(&mut self, num_pages: usize, align_pow2: usize) -> AllocResult<usize> {
        let addr = self.global_pool.alloc_pages(num_pages, align_pow2)?;
        self.stats = self.global_pool.get_stats();
        Ok(addr)
    }

    fn dealloc_pages(&mut self, pos: usize, num_pages: usize) {
        self.global_pool.dealloc_pages(pos, num_pages);
        self.stats = self.global_pool.get_stats();
    }

    fn alloc_pages_at(
        &mut self,
        base: usize,
        num_pages: usize,
        align_pow2: usize,
    ) -> AllocResult<usize> {
        // For simplicity, we'll try to allocate and check if it matches base
        let addr = self.global_pool.alloc_pages(num_pages, align_pow2)?;
        if addr != base {
            // If not the exact address, deallocate and return error
            self.global_pool.dealloc_pages(addr, num_pages);
            Err(AllocError::InvalidParam)
        } else {
            Ok(addr)
        }
    }

    fn total_pages(&self) -> usize {
        self.stats.total_pages
    }

    fn used_pages(&self) -> usize {
        self.stats.used_pages
    }

    fn available_pages(&self) -> usize {
        self.stats.free_pages
    }
}

impl PageAllocatorForSlab for BuddyPageAllocator {
    fn alloc_pages(&mut self, num_pages: usize, align_pow2: usize) -> AllocResult<usize> {
        self.global_pool.alloc_pages(num_pages, align_pow2)
    }

    fn dealloc_pages(&mut self, pos: usize, num_pages: usize) {
        self.global_pool.dealloc_pages(pos, num_pages)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_buddy_allocator_basic() {
        let mut allocator = BuddyPageAllocator::new();
        
        // Test initialization
        let base_addr = 0x80000000;
        let size = 0x100000; // 1MB
        allocator.init(base_addr, size);

        // Test allocation
        match PageAllocator::alloc_pages(&mut allocator, 1, PAGE_SIZE) {
            Ok(page_addr) => {
                PageAllocator::dealloc_pages(&mut allocator, page_addr, 1);
            }
            Err(_) => panic!("Page allocation failed"),
        }

        let stats = allocator.get_stats();
        assert!(stats.total_pages > 0);
    }

    #[test]
    fn test_buddy_merge() {
        let mut allocator = BuddyPageAllocator::new();
        
        let base_addr = 0x80000000;
        let size = 0x10000; // 64KB = 16 pages
        allocator.init(base_addr, size);

        // Allocate two adjacent pages
        let addr1 = PageAllocator::alloc_pages(&mut allocator, 1, PAGE_SIZE).unwrap();
        let addr2 = PageAllocator::alloc_pages(&mut allocator, 1, PAGE_SIZE).unwrap();
        
        // Deallocate both - they should merge back
        PageAllocator::dealloc_pages(&mut allocator, addr1, 1);
        PageAllocator::dealloc_pages(&mut allocator, addr2, 1);
        
        let stats = allocator.get_stats();
        assert_eq!(stats.free_pages, 16); // Should be back to all free
    }
}
