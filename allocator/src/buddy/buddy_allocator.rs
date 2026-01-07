//! Multi-zone buddy allocator using global node pool
//!
//! Provides buddy allocator with support for multiple memory zones and
//! a single shared global node pool for all zones and orders.
//!
//! # Architecture
//!
//! - **GlobalNodePool**: Stores linked-list nodes (NOT memory pages)
//! - **BuddySetPool**: Each zone's free lists, using nodes from GlobalNodePool
//! - **BuddyPageAllocator**: Coordinates multiple zones with shared node pool

use crate::{AllocError, AllocResult, BaseAllocator, PageAllocator};
use log::{debug, error, info, warn};

use super::{
    buddy_block::{BuddyBlock, MAX_ZONES, ZoneInfo},
    buddy_set::BuddySet,
    global_node_pool::GlobalNodePool,
    stats::{BuddyStats, MemoryStatsReporter},
    PAGE_SIZE,
};

/// Buddy page allocator with multi-zone support and global node pool
///
/// The `global_node_pool` stores linked-list nodes (ListNode<BuddyBlock>),
/// which are used to construct the free lists in each BuddySet.
/// Memory pages themselves are tracked by BuddyBlock values, not by this pool.
pub struct BuddyPageAllocator {
    zones: [BuddySet; MAX_ZONES],
    num_zones: usize,
    /// Global node pool - stores linked-list nodes (NOT memory pages)
    ///
    /// This pool is shared across all zones and orders. It allocates
    /// ListNode<BuddyBlock> nodes which form the structure of free lists.
    /// The actual BuddyBlock data (containing page addresses) is stored
    /// in these nodes, not separately in this pool.
    global_node_pool: GlobalNodePool,
    stats: BuddyStats,
}

impl BuddyPageAllocator {
    pub const fn new() -> Self {
        Self {
            zones: [const { BuddySet::empty() }; MAX_ZONES],
            num_zones: 0,
            global_node_pool: GlobalNodePool::new(),
            stats: BuddyStats::new(),
        }
    }

    /// Initialize the global node pool and bootstrap with initial memory region
    pub fn init(&mut self, base_addr: usize, size: usize) {
        self.bootstrap(base_addr, size);
    }

    /// Bootstrap allocator with initial memory region
    pub fn bootstrap(&mut self, base_addr: usize, size: usize) {
        info!(
            "buddy allocator: Bootstrap with region [{:#x}, {:#x})",
            base_addr,
            base_addr + size
        );

        if self.num_zones >= MAX_ZONES {
            panic!("Cannot bootstrap: maximum zones reached");
        }

        // Initialize global node pool if not already initialized
        if self.global_node_pool.get_stats().total_allocations == 0 {
            self.global_node_pool.init();
        }

        self.zones[0] = BuddySet::new(base_addr, size, 0);
        self.zones[0].init(&mut self.global_node_pool, base_addr, size);
        self.num_zones = 1;

        self.update_stats();
    }

    pub fn get_stats(&self) -> BuddyStats {
        self.stats
    }

    /// Get global node pool statistics
    pub fn get_node_pool_stats(&self) -> super::global_node_pool::GlobalPoolStats {
        self.global_node_pool.get_stats()
    }

    /// Get number of zones in the allocator
    pub fn get_zone_count(&self) -> usize {
        self.num_zones
    }

    /// Get free blocks of a specific order from a zone
    /// Returns None if zone doesn't exist
    pub fn get_free_blocks_by_order(
        &self,
        zone_id: usize,
        order: u32,
    ) -> Option<impl Iterator<Item = &BuddyBlock>> {
        if zone_id >= self.num_zones {
            return None;
        }
        Some(self.zones[zone_id].get_free_blocks_by_order(&self.global_node_pool, order))
    }

    /// Get detailed free list information as a string
    pub fn get_free_lists_info(&self) -> alloc::string::String {
        let mut result = alloc::string::String::new();
        result.push_str("=== Multi-Zone Buddy Allocator (Global Node Pool) ===\n");
        result.push_str(&alloc::format!("Total Zones: {}\n", self.num_zones));

        let pool_stats = self.get_node_pool_stats();
        result.push_str(&alloc::format!(
            "Node Pool: {}/{} nodes used ({} allocs, {} deallocs)\n",
            pool_stats.allocated_nodes,
            pool_stats.total_nodes,
            pool_stats.total_allocations,
            pool_stats.total_deallocations
        ));
        result.push_str("\n");

        for i in 0..self.num_zones {
            let zone_info = self.zones[i].zone_info();
            result.push_str(&alloc::format!("Zone {}:\n", i));
            result.push_str(&alloc::format!(
                "  Range: [{:#x}, {:#x})\n",
                zone_info.start_addr,
                zone_info.end_addr
            ));
            result.push_str(&alloc::format!(
                "  Total Pages: {}\n",
                zone_info.total_pages
            ));
            result.push_str(&self.zones[i].get_free_lists_info());
            result.push_str("\n");
        }

        let stats = self.get_stats();
        result.push_str("Overall Summary:\n");
        result.push_str(&alloc::format!("  Total pages: {}\n", stats.total_pages));
        result.push_str(&alloc::format!("  Free pages: {}\n", stats.free_pages));
        result.push_str(&alloc::format!("  Used pages: {}\n", stats.used_pages));
        result.push_str("========================================\n");

        result
    }

    /// Update aggregated statistics from all zones
    fn update_stats(&mut self) {
        let mut total_stats = BuddyStats::new();

        for i in 0..self.num_zones {
            let zone_stats = self.zones[i].get_stats();
            total_stats.add(&zone_stats);
        }

        self.stats = total_stats;
    }

    /// Add a new memory region as a new zone
    pub fn add_memory_region(&mut self, start: usize, size: usize) -> AllocResult<()> {
        info!(
            "buddy allocator: Adding region [{:#x}, {:#x})",
            start,
            start + size
        );

        if self.num_zones >= MAX_ZONES {
            error!(
                "buddy allocator: Cannot add region: maximum zones ({}) reached",
                MAX_ZONES
            );
            return Err(AllocError::NoMemory);
        }

        // Align to page boundaries
        let aligned_start = start & !(PAGE_SIZE - 1);
        let end = start + size;
        let aligned_end = (end + PAGE_SIZE - 1) & !(PAGE_SIZE - 1);
        let aligned_size = aligned_end - aligned_start;

        if aligned_size == 0 || aligned_size < PAGE_SIZE {
            warn!(
                "buddy allocator: Aligned size is too small: {:#x}, skipping region",
                aligned_size
            );
            return Err(AllocError::InvalidParam);
        }

        // Check for overlap with existing zones
        for i in 0..self.num_zones {
            let zone = &self.zones[i];
            if !(aligned_end <= zone.base_addr || aligned_start >= zone.end_addr) {
                error!(
                    "buddy allocator: Region [{:#x}, {:#x}) overlaps with zone {} [{:#x}, {:#x})",
                    aligned_start, aligned_end, i, zone.base_addr, zone.end_addr
                );
                return Err(AllocError::MemoryOverlap);
            }
        }

        let zone_id = self.num_zones;
        self.zones[zone_id] = BuddySet::new(aligned_start, aligned_size, zone_id);
        self.zones[zone_id].init(&mut self.global_node_pool, aligned_start, aligned_size);
        self.num_zones += 1;

        Ok(())
    }

    /// Find the zone that contains the given address
    pub fn find_zone_for_addr(&self, addr: usize) -> Option<usize> {
        for i in 0..self.num_zones {
            if self.zones[i].addr_in_zone(addr) {
                return Some(i);
            }
        }
        None
    }

    /// Print detailed allocation failure statistics
    ///
    /// This method is public to allow CompositePageAllocator to call it
    /// when allocation fails, providing detailed per-zone failure information.
    pub fn print_alloc_failure_stats(&self, num_pages: usize, alignment: usize) {
        let mut zone_infos: [Option<ZoneInfo>; MAX_ZONES] = [None; MAX_ZONES];
        let mut zone_stats: [Option<BuddyStats>; MAX_ZONES] = [None; MAX_ZONES];

        for i in 0..self.num_zones {
            zone_infos[i] = Some(self.zones[i].zone_info());
            zone_stats[i] = Some(self.zones[i].get_stats());
        }

        // Create slices from the initialized elements
        let zone_infos_slice: &[ZoneInfo] = unsafe {
            core::slice::from_raw_parts(
                zone_infos.as_ptr() as *const ZoneInfo,
                self.num_zones,
            )
        };
        let zone_stats_slice: &[BuddyStats] = unsafe {
            core::slice::from_raw_parts(
                zone_stats.as_ptr() as *const BuddyStats,
                self.num_zones,
            )
        };

        MemoryStatsReporter::print_alloc_failure_stats(
            self.num_zones,
            &self.stats,
            zone_infos_slice,
            zone_stats_slice,
            num_pages,
            alignment,
        );
    }
}

impl crate::slab_byte_allocator::PageAllocatorForSlab for BuddyPageAllocator {
    fn alloc_pages(&mut self, num_pages: usize, alignment: usize) -> AllocResult<usize> {
        <Self as PageAllocator>::alloc_pages(self, num_pages, alignment)
    }

    fn dealloc_pages(&mut self, pos: usize, num_pages: usize) {
        <Self as PageAllocator>::dealloc_pages(self, pos, num_pages);
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

    fn add_memory(&mut self, start: usize, size: usize) -> AllocResult<()> {
        self.add_memory_region(start, size)?;
        self.update_stats();
        Ok(())
    }
}

impl PageAllocator for BuddyPageAllocator {
    const PAGE_SIZE: usize = PAGE_SIZE;

    fn alloc_pages(&mut self, num_pages: usize, alignment: usize) -> AllocResult<usize> {
        for i in 0..self.num_zones {
            match self.zones[i].alloc_pages(&mut self.global_node_pool, num_pages, alignment) {
                Ok(addr) => {
                    self.update_stats();
                    return Ok(addr);
                }
                Err(_) => {
                    continue;
                }
            }
        }
        debug!(
            "buddy allocator: Allocation failure: {} Byte, align {}",
            num_pages * PAGE_SIZE,
            alignment
        );
        Err(AllocError::NoMemory)
    }

    fn dealloc_pages(&mut self, pos: usize, num_pages: usize) {
        if let Some(zone_idx) = self.find_zone_for_addr(pos) {
            self.zones[zone_idx].dealloc_pages(&mut self.global_node_pool, pos, num_pages);
            self.update_stats();
        } else {
            warn!(
                "buddy allocator: Dealloc pages at {:#x}: address not in any zone",
                pos
            );
        }
    }

    fn alloc_pages_at(
        &mut self,
        base: usize,
        num_pages: usize,
        alignment: usize,
    ) -> AllocResult<usize> {
        if let Some(zone_idx) = self.find_zone_for_addr(base) {
            match self.zones[zone_idx].alloc_pages_at(
                &mut self.global_node_pool,
                base,
                num_pages,
                alignment,
            ) {
                Ok(addr) => {
                    self.update_stats();
                    Ok(addr)
                }
                Err(e) => Err(e),
            }
        } else {
            warn!(
                "buddy allocator: alloc_pages_at: address {:#x} not in any zone",
                base
            );
            Err(AllocError::InvalidParam)
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
