//! Single-zone buddy allocator implementation
//!
//! Implements the core buddy system for a single memory zone with
//! sorted free lists for efficient contiguity checking.

use crate::{AllocError, AllocResult};
use log::{debug, error, info, trace, warn};

use super::{
    buddy_block::{BuddyBlock, ZoneInfo, DEFAULT_MAX_ORDER},
    list_pool::StaticSharedPool,
    linked_list::ListNode,
    PAGE_SIZE,
};

/// Pool configuration
pub const POOL_TOTAL_LISTS: usize = 128;
pub const POOL_LIST_CAPACITY: usize = 32;
const POOL_MAX_ORDERS: usize = DEFAULT_MAX_ORDER + 1;

/// A buddy set implementation - represents a single zone
///
/// Uses a shared list pool instead of fixed per-order lists to avoid
/// memory leaks when many blocks cannot merge.
pub struct BuddySet {
    pub(crate) base_addr: usize,
    pub(crate) end_addr: usize,
    total_pages: usize,
    zone_id: usize,
    /// Shared pool of lists for all orders
    pub(crate) list_pool:
        StaticSharedPool<POOL_TOTAL_LISTS, POOL_LIST_CAPACITY, POOL_MAX_ORDERS>,
    /// First list index for each order (may be None if no lists allocated)
    pub(crate) first_list_by_order: [Option<usize>; DEFAULT_MAX_ORDER + 1],
}

impl BuddySet {
    /// Create a new buddy set for a zone (uninitialized, must call init())
    pub const fn new(base_addr: usize, size: usize, zone_id: usize) -> Self {
        Self {
            base_addr,
            end_addr: base_addr + size,
            total_pages: size / PAGE_SIZE,
            zone_id,
            list_pool: StaticSharedPool::new(),
            first_list_by_order: [const { None }; DEFAULT_MAX_ORDER + 1],
        }
    }

    /// Create an empty buddy set
    pub const fn empty() -> Self {
        Self::new(0, 0, 0)
    }

    pub const fn max_order(&self) -> usize {
        DEFAULT_MAX_ORDER
    }

    /// Add a block to the appropriate list for its order
    /// Allocates a new list if needed
    fn add_block_to_order(&mut self, order: usize, block: BuddyBlock) -> bool {
        // First, try to find an existing list for this order with available space
        if let Some(list_idx) = self.list_pool.find_available_list_for_order(order) {
            let list = self.list_pool.get_list_mut(list_idx);
            return list.insert_sorted(block);
        }

        // All existing lists are full (or no lists exist), need to allocate a new list
        if let Some(new_list_idx) = self.list_pool.alloc_list(order) {
            let list = self.list_pool.get_list_mut(new_list_idx);
            let success = list.insert_sorted(block);
            if success {
                // Update first list pointer if this is the first list for this order
                self.first_list_by_order[order] = Some(new_list_idx);
            } else {
                // Rollback allocation
                self.list_pool.free_list(new_list_idx, order);
            }
            success
        } else {
            error!(
                "zone {}: No free lists available for order {}",
                self.zone_id, order
            );
            false
        }
    }

    /// Find a block with the given address in all lists of the given order
    fn find_block_in_order(&self, order: usize, addr: usize) -> Option<(usize, usize)> {
        for list_idx in self.list_pool.get_order_lists(order) {
            let list = self.list_pool.get_list(list_idx);
            let mut current_idx = list.head;
            let mut visited = 0;

            while let Some(node_idx) = current_idx {
                if visited > list.len() {
                    warn!("Potential cycle detected in free list during search");
                    return None;
                }

                if let Some(node) = &list.nodes[node_idx] {
                    // Early termination: list is sorted by address
                    if node.data.addr > addr {
                        break;
                    }
                    if node.data.addr == addr {
                        return Some((list_idx, node_idx));
                    }
                    current_idx = node.next;
                } else {
                    break;
                }
                visited += 1;
            }
        }
        None
    }

    /// Remove a block from its list
    fn remove_block_from_order(&mut self, list_idx: usize, node_idx: usize, order: usize) {
        let list = self.list_pool.get_list_mut(list_idx);

        if node_idx >= POOL_LIST_CAPACITY || list.nodes[node_idx].is_none() {
            return;
        }

        // Find the node to remove
        let mut prev_idx = None;
        let mut current_idx = list.head;
        let mut visited = 0;

        while let Some(idx) = current_idx {
            if visited > list.len {
                warn!("Potential cycle detected in free list");
                return;
            }

            if idx == node_idx {
                break;
            }
            prev_idx = current_idx;
            if let Some(node) = &list.nodes[idx] {
                current_idx = node.next;
            } else {
                break;
            }
            visited += 1;
        }

        if current_idx != Some(node_idx) {
            return;
        }

        // Remove the node
        if let Some(node) = list.nodes[node_idx].take() {
            // Update links
            if let Some(prev_idx) = prev_idx {
                if let Some(prev_node) = &mut list.nodes[prev_idx] {
                    prev_node.next = node.next;
                }
            } else {
                list.head = node.next;
            }

            // Update tail if needed
            if list.tail == Some(node_idx) {
                if node.next.is_none() {
                    list.tail = prev_idx;
                }
            } else if list.head.is_none() {
                list.tail = None;
            }

            // Return node to free list
            let dummy_node = ListNode {
                data: unsafe { core::mem::zeroed() },
                next: list.free_head,
            };
            list.nodes[node_idx] = Some(dummy_node);
            list.free_head = Some(node_idx);
            list.len -= 1;

            // If the list becomes empty, free it back to the pool
            if list.is_empty() {
                self.list_pool.free_list(list_idx, order);
                // Update first list pointer if needed
                if self.first_list_by_order[order] == Some(list_idx) {
                    // Find the next list for this order, if any
                    if let Some(next_list) = self.list_pool.get_order_lists(order).next() {
                        self.first_list_by_order[order] = Some(next_list);
                    } else {
                        self.first_list_by_order[order] = None;
                    }
                }
            }
        }
    }

    /// Check if an address belongs to this zone
    pub fn addr_in_zone(&self, addr: usize) -> bool {
        addr >= self.base_addr && addr < self.end_addr
    }

    /// Get zone information
    pub fn zone_info(&self) -> ZoneInfo {
        ZoneInfo {
            start_addr: self.base_addr,
            end_addr: self.end_addr,
            total_pages: self.total_pages,
            zone_id: self.zone_id,
        }
    }

    /// Initialize the buddy set with a memory region
    pub fn init(&mut self, base_addr: usize, size: usize) {
        info!(
            "zone {}: Initialize with region [{:#x}, {:#x})",
            self.zone_id,
            base_addr,
            base_addr + size
        );

        // Align to page boundaries
        let aligned_base = base_addr & !(PAGE_SIZE - 1);
        let end = base_addr + size;
        let aligned_end = (end + PAGE_SIZE - 1) & !(PAGE_SIZE - 1);
        let aligned_size = aligned_end - aligned_base;

        if aligned_size == 0 || aligned_size < PAGE_SIZE {
            panic!("Aligned size is too small: {:#x}", aligned_size);
        }

        debug!(
            "zone {}: Adjusted region [{:#x}, {:#x}) (original [{:#x}, {:#x}))",
            self.zone_id, aligned_base, aligned_end, base_addr, end
        );

        self.base_addr = aligned_base;
        self.end_addr = aligned_end;
        self.total_pages = aligned_size / PAGE_SIZE;

        // Initialize the shared list pool
        self.list_pool.init();

        info!("zone {}: Initialized with {} pages", self.zone_id, self.total_pages);
        // Reset first list indices
        for i in 0..=DEFAULT_MAX_ORDER {
            self.first_list_by_order[i] = None;
        }

        // Linux-style initialization: release pages one by one
        // This naturally handles memory regions of any size
        for pfn in 0..self.total_pages {
            let page_addr = self.base_addr + pfn * PAGE_SIZE;
            self.dealloc_pages(page_addr, 1);
        }
    }

    /// Allocate pages using buddy system
    pub fn alloc_pages(&mut self, num_pages: usize, alignment: usize) -> AllocResult<usize> {
        if num_pages == 0 {
            return Err(AllocError::InvalidParam);
        }

        // Find the required order (round up to next power of 2)
        let required_order = if num_pages.is_power_of_two() {
            num_pages.trailing_zeros() as usize
        } else {
            num_pages.next_power_of_two().trailing_zeros() as usize
        };

        if required_order > self.max_order() {
            return Err(AllocError::NoMemory);
        }

        // Convert byte alignment to page alignment order
        // Ensure at least page-level alignment
        let align_pages = (alignment + PAGE_SIZE - 1) / PAGE_SIZE;
        let align_order = align_pages.trailing_zeros() as usize;

        let order_needed = required_order.max(align_order);
        debug!(
            "zone {}: Allocating {} pages, required order: {}, alignment: {}, order_needed: {}",
            self.zone_id, num_pages, required_order, alignment, order_needed
        );

        // Try to find a block of the required order or higher
        for order in order_needed..=self.max_order() {
            // Check all lists for this order, not just the first one
            // Collect indices first to avoid borrow issues
            let mut list_indices = [None; POOL_TOTAL_LISTS];
            let mut list_count = 0;
            for list_idx in self.list_pool.get_order_lists(order) {
                if list_count < POOL_TOTAL_LISTS {
                    list_indices[list_count] = Some(list_idx);
                    list_count += 1;
                }
            }

            for i in 0..list_count {
                if let Some(list_idx) = list_indices[i] {
                    if !self.list_pool.get_list(list_idx).is_empty() {
                        let mut block = self.list_pool.get_list_mut(list_idx).pop_front().unwrap();

                        // Split down to required order
                        while block.order > order_needed {
                            block.order -= 1;
                            let split_size = (1 << block.order) * PAGE_SIZE;
                            let buddy_addr = block.addr + split_size;

                            // Push the second half back to free list (sorted!)
                            let success = self.add_block_to_order(
                                block.order,
                                BuddyBlock {
                                    order: block.order,
                                    addr: buddy_addr,
                                },
                            );
                            if !success {
                                warn!(
                                    "Failed to push buddy block to free list during split at order {}",
                                    block.order
                                );
                                // Put the original block back
                                self.add_block_to_order(block.order + 1, block);
                                return Err(AllocError::NoMemory);
                            }
                        }

                        // Verify alignment requirement
                        assert!(
                            block.addr % alignment == 0,
                            "Allocated address {:#x} is not aligned to {:#x} bytes ",
                            block.addr,
                            alignment
                        );

                        return Ok(block.addr);
                    }
                }
            }
        }

        Err(AllocError::NoMemory)
    }



    /// Deallocate pages back to buddy system with automatic merging
    pub fn dealloc_pages(&mut self, addr: usize, num_pages: usize) {
        if num_pages == 0 {
            warn!("zone {}: Trying to deallocate 0 pages", self.zone_id);
            return;
        }

        // Validate address belongs to this zone
        if !self.addr_in_zone(addr) {
            error!(
                "zone {}: Address {:#x} not in zone [{:#x}, {:#x})",
                self.zone_id, addr, self.base_addr, self.end_addr
            );
            return;
        }

        // Buddy system can only handle power-of-2 allocations
        if !num_pages.is_power_of_two() {
            error!(
                "zone {}: Cannot free {} pages: must be power of 2",
                self.zone_id, num_pages
            );
            return;
        }

        // Calculate order for this deallocation
        let mut order = num_pages.trailing_zeros() as usize;
        if order > DEFAULT_MAX_ORDER {
            error!(
                "zone {}: Order {} exceeds maximum supported order {}",
                self.zone_id, order, DEFAULT_MAX_ORDER
            );
            return;
        }

        // Convert address and pages to PFN (Page Frame Number)
        let pfn = addr / PAGE_SIZE;

        // Check alignment using PFN
        if pfn & ((1 << order) - 1) != 0 {
            error!(
                "zone {}: Page PFN {} is not properly aligned for order {} (needs alignment to {} pages)",
                self.zone_id, pfn, order, 1 << order
            );
            return;
        }

        // Check page alignment
        if addr & (PAGE_SIZE - 1) != 0 {
            error!(
                "zone {}: Attempt to free page at non-page-aligned address {:#x}",
                self.zone_id, addr
            );
            return;
        }

        // Initialize block for merging
        let mut current_pfn = pfn;

        // Try to merge with buddy blocks (Linux-style)
        while order < self.max_order() {
            // Calculate buddy PFN using XOR operation (same as Linux kernel)
            let buddy_pfn = current_pfn ^ (1 << order);

            // Verify buddy is within the zone
            let buddy_addr = buddy_pfn * PAGE_SIZE;

            if !self.addr_in_zone(buddy_addr) {
                break;
            }

            // Try to find buddy in free lists
            if let Some((list_idx, node_idx)) = self.find_block_in_order(order, buddy_addr) {
                // Verify buddy has correct order and address
                let list = self.list_pool.get_list(list_idx);
                if let Some(buddy_node) = &list.nodes[node_idx] {
                    if buddy_node.data.order != order || buddy_node.data.addr != buddy_addr {
                        warn!(
                            "zone {}: Inconsistent buddy block found at PFN {}",
                            self.zone_id, buddy_pfn
                        );
                        break;
                    }
                }

                // Remove buddy from free list
                self.remove_block_from_order(list_idx, node_idx, order);

                // Merge: use the aligned address (lower address)
                current_pfn = current_pfn & buddy_pfn;

                // Move to next order
                order += 1;

                trace!(
                    "zone {}: Merged blocks at PFN {} and {} to order {}",
                    self.zone_id,
                    current_pfn,
                    buddy_pfn,
                    order
                );
            } else {
                // No buddy found, cannot merge further
                break;
            }
        }

        // Add the final merged block to the appropriate free list (sorted!)
        let final_addr = current_pfn * PAGE_SIZE;
        let block = BuddyBlock {
            order,
            addr: final_addr,
        };

        let success = self.add_block_to_order(order, block);
        if !success {
            error!(
                "zone {}: Failed to push block to free list: addr={:#x}, order={}, PFN={}",
                self.zone_id, final_addr, order, current_pfn
            );
        }
    }

    /// Get statistics for this zone
    pub fn get_stats(&self) -> super::stats::BuddyStats {
        let mut stats = super::stats::BuddyStats::new();
        stats.total_pages = self.total_pages;

        for order in 0..=DEFAULT_MAX_ORDER {
            let list_count = self.list_pool.order_list_count(order);
            let block_count = self.list_pool.order_total_blocks(order);
            stats.free_pages_by_order[order] = block_count;
            stats.free_pages += block_count * (1 << order);
        }

        stats.used_pages = stats.total_pages.saturating_sub(stats.free_pages);
        stats
    }

    /// Get detailed free list information as a string
    pub fn get_free_lists_info(&self) -> alloc::string::String {
        let mut result = alloc::string::String::new();

        for order in 0..=DEFAULT_MAX_ORDER {
            let count = self.list_pool.order_total_blocks(order);
            if count > 0 {
                let block_size = 1usize << order;
                let size_mb = (block_size * PAGE_SIZE) / (1024 * 1024);
                let list_count = self.list_pool.order_list_count(order);
                result.push_str(&alloc::format!(
                    "  Order {} ({} MB per block): {} free blocks in {} lists\n",
                    order,
                    size_mb,
                    count,
                    list_count
                ));
            }
        }

        result
    }

    /// Get free blocks of a specific order as an iterator
    pub fn get_free_blocks_by_order(&self, order: u32) -> impl Iterator<Item = &BuddyBlock> {
        self.list_pool.get_order_lists(order as usize).flat_map(|list_idx| {
            self.list_pool.get_list(list_idx).iter()
        })
    }

    /// Get the number of lists allocated to a specific order
    pub fn get_order_list_count(&self, order: usize) -> usize {
        self.list_pool.order_list_count(order)
    }

    /// Get the total number of blocks in all lists of a specific order
    pub fn get_order_total_blocks(&self, order: usize) -> usize {
        self.list_pool.order_total_blocks(order)
    }

    /// Get the pool statistics
    pub fn get_pool_stats(&self) -> super::list_pool::PoolStats {
        self.list_pool.get_stats()
    }
}

impl Default for BuddySet {
    fn default() -> Self {
        Self::empty()
    }
}
