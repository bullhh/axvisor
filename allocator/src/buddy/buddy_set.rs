//! Single-zone buddy allocator implementation
//!
//! Implements the core buddy system for a single memory zone with
//! sorted free lists for efficient contiguity checking.

use crate::{AllocError, AllocResult};
use log::{info, trace, warn};

use super::{
    buddy_block::{BuddyBlock, ZoneInfo, MAX_BLOCKS_PER_LIST},
    linked_list::{StaticLinkedList, ListNode},
    DEFAULT_MAX_ORDER,
    PAGE_SIZE,
};

/// A buddy set implementation - represents a single zone
pub struct BuddySet {
    pub(crate) base_addr: usize,
    pub(crate) end_addr: usize,
    total_pages: usize,
    zone_id: usize,
    pub(crate) free_lists: [StaticLinkedList<BuddyBlock, MAX_BLOCKS_PER_LIST>; DEFAULT_MAX_ORDER + 1],
}

impl BuddySet {
    /// Create a new buddy set for a zone
    pub const fn new(base_addr: usize, size: usize, zone_id: usize) -> Self {
        Self {
            base_addr,
            end_addr: base_addr + size,
            total_pages: size / PAGE_SIZE,
            zone_id,
            free_lists: [const { StaticLinkedList::new() }; DEFAULT_MAX_ORDER + 1],
        }
    }

    /// Create an empty buddy set
    pub const fn empty() -> Self {
        Self::new(0, 0, 0)
    }

    pub const fn max_order(&self) -> usize {
        DEFAULT_MAX_ORDER
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
            self.zone_id, base_addr, base_addr + size
        );

        // Align to page boundaries
        let aligned_base = base_addr & !(PAGE_SIZE - 1);
        let end = base_addr + size;
        let aligned_end = (end + PAGE_SIZE - 1) & !(PAGE_SIZE - 1);
        let aligned_size = aligned_end - aligned_base;

        if aligned_size == 0 || aligned_size < PAGE_SIZE {
            panic!("Aligned size is too small: {:#x}", aligned_size);
        }

        info!(
            "zone {}: Adjusted region [{:#x}, {:#x}) (original [{:#x}, {:#x}))",
            self.zone_id, aligned_base, aligned_end, base_addr, end
        );

        self.base_addr = aligned_base;
        self.end_addr = aligned_end;
        self.total_pages = aligned_size / PAGE_SIZE;

        // Initialize all free lists
        for list in &mut self.free_lists {
            list.init();
        }

        // Linux-style initialization: release pages one by one
        // This naturally handles memory regions of any size
        for pfn in 0..self.total_pages {
            let page_addr = self.base_addr + pfn * PAGE_SIZE;
            self.dealloc_pages(page_addr, 1);
        }

        info!(
            "zone {}: {} pages initialized in buddy system",
            self.zone_id, self.total_pages
        );
    }

    /// Allocate pages using buddy system
    pub fn alloc_pages(&mut self, num_pages: usize, _align_pow2: usize) -> AllocResult<usize> {
        if num_pages == 0 {
            return Err(AllocError::InvalidParam);
        }

        // Find the required order (round up to next power of 2)
        let required_order = if num_pages.is_power_of_two() {
            num_pages.trailing_zeros() as usize
        } else {
            num_pages.next_power_of_two().trailing_zeros() as usize
        }
        .min(DEFAULT_MAX_ORDER);

        // Try to find a block of the required order or higher
        for order in required_order..=self.max_order() {
            if !self.free_lists[order].is_empty() {
                let mut block = self.free_lists[order].pop_front().unwrap();

                // Split down to required order
                while block.order > required_order {
                    block.order -= 1;
                    let split_size = (1 << block.order) * PAGE_SIZE;
                    let buddy_addr = block.addr + split_size;

                    // Push the second half back to free list (sorted!)
                    let success = self.free_lists[block.order].insert_sorted(BuddyBlock {
                        order: block.order,
                        addr: buddy_addr,
                    });
                    if !success {
                        warn!(
                            "Failed to push buddy block to free list during split at order {}",
                            block.order
                        );
                        // Put the original block back
                        self.free_lists[block.order + 1].push_back(block);
                        return Err(AllocError::NoMemory);
                    }
                }

                return Ok(block.addr);
            }
        }

        Err(AllocError::NoMemory)
    }

    /// Find a block with the given address in the free list of the given order
    fn find_block_in_free_list(&self, order: usize, addr: usize) -> Option<usize> {
        let list = &self.free_lists[order];
        let mut current_idx = list.head;
        let mut visited = 0;

        while let Some(idx) = current_idx {
            if visited > list.len() {
                warn!("Potential cycle detected in free list during search");
                return None;
            }

            if let Some(node) = &list.nodes[idx] {
                if node.data.addr == addr {
                    return Some(idx);
                }
                current_idx = node.next;
            } else {
                break;
            }
            visited += 1;
        }

        None
    }

    /// Remove a block from the free list at the given position
    fn remove_block_from_free_list(&mut self, order: usize, node_idx: usize) {
        if !self.node_exists_in_list(&self.free_lists[order], node_idx) {
            return;
        }

        let list = &mut self.free_lists[order];

        if node_idx >= MAX_BLOCKS_PER_LIST || list.nodes[node_idx].is_none() {
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
        }
    }

    /// Helper: check if a node exists in the list
    fn node_exists_in_list(
        &self,
        list: &StaticLinkedList<
            BuddyBlock,
            MAX_BLOCKS_PER_LIST,
        >,
        node_idx: usize,
    ) -> bool {
        let mut current_idx = list.head;
        let mut visited = 0;

        while let Some(idx) = current_idx {
            if visited > list.len {
                return false;
            }
            if idx == node_idx {
                return true;
            }
            if let Some(node) = &list.nodes[idx] {
                current_idx = node.next;
            } else {
                break;
            }
            visited += 1;
        }

        false
    }

    /// Deallocate pages back to buddy system with automatic merging
    pub fn dealloc_pages(&mut self, addr: usize, num_pages: usize) {
        if num_pages == 0 {
            return;
        }

        // Validate address belongs to this zone
        if !self.addr_in_zone(addr) {
            warn!(
                "zone {}: Address {:#x} not in zone [{:#x}, {:#x})",
                self.zone_id, addr, self.base_addr, self.end_addr
            );
            return;
        }

        // Convert address and pages to PFN (Page Frame Number)
        let pfn = addr / PAGE_SIZE;

        // Calculate the order for this deallocation
        let mut order = if num_pages.is_power_of_two() {
            num_pages.trailing_zeros() as usize
        } else {
            num_pages.next_power_of_two().trailing_zeros() as usize
        }
        .min(DEFAULT_MAX_ORDER);

        // Check alignment using PFN
        if pfn & ((1 << order) - 1) != 0 {
            warn!(
                "zone {}: Page PFN {} is not properly aligned for order {} (needs alignment to {} pages)",
                self.zone_id, pfn, order, 1 << order
            );
            return;
        }

        // Check page alignment
        if addr & (PAGE_SIZE - 1) != 0 {
            warn!(
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

            // Try to find buddy in free list
            if let Some(buddy_pos) = self.find_block_in_free_list(order, buddy_addr) {
                // Verify buddy has correct order and address
                if let Some(buddy_node) = &self.free_lists[order].nodes[buddy_pos] {
                    if buddy_node.data.order != order || buddy_node.data.addr != buddy_addr {
                        warn!(
                            "zone {}: Inconsistent buddy block found at PFN {}",
                            self.zone_id, buddy_pfn
                        );
                        break;
                    }
                }

                // Remove buddy from free list
                self.remove_block_from_free_list(order, buddy_pos);

                // Merge: use the aligned address (lower address)
                current_pfn = current_pfn & buddy_pfn;

                // Move to next order
                order += 1;

                trace!(
                    "zone {}: Merged blocks at PFN {} and {} to order {}",
                    self.zone_id, current_pfn, buddy_pfn, order
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

        let success = self.free_lists[order].insert_sorted(block);
        if !success {
            warn!(
                "zone {}: Failed to push block to free list: addr={:#x}, order={}, PFN={}",
                self.zone_id, final_addr, order, current_pfn
            );
        }
    }

    /// Get statistics for this zone
    pub fn get_stats(&self) -> super::stats::BuddyStats {
        let mut stats = super::stats::BuddyStats::new();
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

        for order in 0..=DEFAULT_MAX_ORDER {
            let count = self.free_lists[order].len();
            if count > 0 {
                let block_size = 1usize << order;
                let size_mb = (block_size * PAGE_SIZE) / (1024 * 1024);
                result.push_str(&alloc::format!(
                    "  Order {} ({} MB per block): {} free blocks\n",
                    order, size_mb, count
                ));
            }
        }

        result
    }

    /// Get free blocks of a specific order as an iterator
    pub fn get_free_blocks_by_order(&self, order: u32) -> impl Iterator<Item = &BuddyBlock> {
        self.free_lists[order as usize].iter()
    }
}

impl Default for BuddySet {
    fn default() -> Self {
        Self::empty()
    }
}
