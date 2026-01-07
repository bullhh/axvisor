//! Single-zone buddy allocator using global node pool
//!
//! Implements the core buddy system for a single memory zone using
//! pooled linked lists that draw nodes from a shared global pool.

use crate::{AllocError, AllocResult};
use log::{debug, error, info, trace, warn};

use super::{
    buddy_block::{BuddyBlock, ZoneInfo, DEFAULT_MAX_ORDER},
    global_node_pool::GlobalNodePool,
    pooled_list::PooledLinkedList,
    PAGE_SIZE,
};

/// A buddy set implementation - represents a single zone
///
/// Uses pooled linked lists with global node pool for efficient memory usage.
/// All zones share the same global node pool.
pub struct BuddySetPool {
    pub(crate) base_addr: usize,
    pub(crate) end_addr: usize,
    total_pages: usize,
    zone_id: usize,
    /// Free lists for each order
    free_lists: [PooledLinkedList; DEFAULT_MAX_ORDER + 1],
}

impl BuddySetPool {
    /// Create a new buddy set for a zone (uninitialized, must call init())
    pub const fn new(base_addr: usize, size: usize, zone_id: usize) -> Self {
        Self {
            base_addr,
            end_addr: base_addr + size,
            total_pages: size / PAGE_SIZE,
            zone_id,
            free_lists: [const { PooledLinkedList::new() }; DEFAULT_MAX_ORDER + 1],
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
    fn add_block_to_order(
        &mut self,
        pool: &mut GlobalNodePool,
        order: usize,
        block: BuddyBlock,
    ) -> bool {
        if order > DEFAULT_MAX_ORDER {
            error!(
                "zone {}: Order {} exceeds maximum order {}",
                self.zone_id, order, DEFAULT_MAX_ORDER
            );
            return false;
        }

        self.free_lists[order].insert_sorted(pool, block)
    }

    /// Find a block with the given address in the free list for its order
    fn find_block_in_order(
        &self,
        pool: &GlobalNodePool,
        order: usize,
        addr: usize,
    ) -> Option<(usize, Option<usize>)> {
        if order > DEFAULT_MAX_ORDER {
            return None;
        }
        self.free_lists[order].find_by_addr(pool, addr)
    }

    /// Remove a block from its list
    fn remove_block_from_order(
        &mut self,
        pool: &mut GlobalNodePool,
        order: usize,
        node_idx: usize,
    ) -> bool {
        if order > DEFAULT_MAX_ORDER {
            error!("zone {}: Order {} exceeds maximum order {}", self.zone_id, order, DEFAULT_MAX_ORDER);
            return false;
        }
        self.free_lists[order].remove(pool, node_idx)
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
    pub fn init(&mut self, pool: &mut GlobalNodePool, base_addr: usize, size: usize) {
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

        info!("zone {}: Initialized with {} pages", self.zone_id, self.total_pages);

        // Reset free lists
        for list in &mut self.free_lists {
            list.clear(pool);
        }

        // Linux-style initialization: release pages one by one
        // This naturally handles memory regions of any size
        for pfn in 0..self.total_pages {
            let page_addr = self.base_addr + pfn * PAGE_SIZE;
            self.dealloc_pages(pool, page_addr, 1);
        }
    }

    /// Allocate pages using buddy system
    pub fn alloc_pages(
        &mut self,
        pool: &mut GlobalNodePool,
        num_pages: usize,
        alignment: usize,
    ) -> AllocResult<usize> {
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
            if !self.free_lists[order].is_empty() {
                let mut block = self.free_lists[order].pop_front(pool).unwrap();

                // Split down to required order
                while block.order > order_needed {
                    block.order -= 1;
                    let split_size = (1 << block.order) * PAGE_SIZE;
                    let buddy_addr = block.addr + split_size;

                    // Push the second half back to free list (sorted!)
                    let success = self.add_block_to_order(
                        pool,
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
                        self.add_block_to_order(pool, block.order + 1, block);
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

        Err(AllocError::NoMemory)
    }

    /// Deallocate pages back to buddy system with automatic merging
    pub fn dealloc_pages(&mut self, pool: &mut GlobalNodePool, addr: usize, num_pages: usize) {
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

            // Try to find buddy in free list
            if let Some((node_idx, _)) = self.find_block_in_order(pool, order, buddy_addr) {
                // Verify buddy has correct order and address
                let node = pool.get_node(node_idx).unwrap();
                if node.data.order != order || node.data.addr != buddy_addr {
                    warn!(
                        "zone {}: Inconsistent buddy block found at PFN {}",
                        self.zone_id, buddy_pfn
                    );
                    break;
                }

                // Remove buddy from free list
                self.remove_block_from_order(pool, order, node_idx);

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

        let success = self.add_block_to_order(pool, order, block);
        if !success {
            error!(
                "zone {}: Failed to push block to free list: addr={:#x}, order={}, PFN={}",
                self.zone_id, final_addr, order, current_pfn
            );
        }
    }

    /// Get statistics for this zone
    pub fn get_stats(&self, _pool: &GlobalNodePool) -> super::stats::BuddyStats {
        let mut stats = super::stats::BuddyStats::new();
        stats.total_pages = self.total_pages;

        for order in 0..=DEFAULT_MAX_ORDER {
            let block_count = self.free_lists[order].len();
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
    pub fn get_free_blocks_by_order<'a>(
        &'a self,
        pool: &'a GlobalNodePool,
        order: u32,
    ) -> impl Iterator<Item = &'a BuddyBlock> {
        self.free_lists[order as usize].iter(pool)
    }

    /// Get the number of blocks in a specific order
    pub fn get_order_block_count(&self, order: usize) -> usize {
        if order <= DEFAULT_MAX_ORDER {
            self.free_lists[order].len()
        } else {
            0
        }
    }
}

impl Default for BuddySetPool {
    fn default() -> Self {
        Self::empty()
    }
}
