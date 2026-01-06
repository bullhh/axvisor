//! Multi-zone buddy allocator
//!
//! Provides buddy allocator with support for multiple memory zones and
//! contiguous memory allocation from multiple orders.

use crate::{AllocError, AllocResult, BaseAllocator, PageAllocator};
use alloc::vec::Vec;
use log::{debug, error, info, warn};

use super::{
    buddy_block::{BuddyBlock, MAX_BLOCKS_PER_LIST, MAX_ZONES},
    buddy_set::BuddySet,
    linked_list::StaticLinkedList,
    stats::{BuddyStats, MemoryStatsReporter},
    PAGE_SIZE,
};

/// Buddy page allocator with multi-zone support
pub struct BuddyPageAllocator {
    zones: [BuddySet; MAX_ZONES],
    num_zones: usize,
    stats: BuddyStats,
}

impl BuddyPageAllocator {
    pub const fn new() -> Self {
        Self {
            zones: [const { BuddySet::empty() }; MAX_ZONES],
            num_zones: 0,
            stats: BuddyStats::new(),
        }
    }

    /// Bootstrap with initial memory region
    pub fn bootstrap(&mut self, base_addr: usize, size: usize) {
        debug!(
            "buddy allocator: Bootstrap with region [{:#x}, {:#x})",
            base_addr,
            base_addr + size
        );

        if self.num_zones >= MAX_ZONES {
            panic!("Cannot bootstrap: maximum zones reached");
        }

        self.zones[0] = BuddySet::new(base_addr, size, 0);
        self.zones[0].init(base_addr, size);
        self.num_zones = 1;

        self.update_stats();
    }

    pub fn get_stats(&self) -> BuddyStats {
        self.stats
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
        Some(self.zones[zone_id].get_free_blocks_by_order(order))
    }

    /// Get detailed free list information as a string
    pub fn get_free_lists_info(&self) -> alloc::string::String {
        let mut result = alloc::string::String::new();
        result.push_str("=== Multi-Zone Buddy Allocator Info ===\n");
        result.push_str(&alloc::format!("Total Zones: {}\n", self.num_zones));
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
        result.push_str("====================================\n");

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
        self.zones[zone_id].init(aligned_start, aligned_size);
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
    pub fn print_alloc_failure_stats(&self, num_pages: usize, alignment: usize) {
        let mut zone_infos = Vec::new();
        let mut zone_stats = Vec::new();

        for i in 0..self.num_zones {
            zone_infos.push(self.zones[i].zone_info());
            zone_stats.push(self.zones[i].get_stats());
        }

        MemoryStatsReporter::print_alloc_failure_stats(
            self.num_zones,
            &self.stats,
            &zone_infos,
            &zone_stats,
            num_pages,
            alignment,
        );
    }

    /// Get a reference to a free list for contiguity checking
    pub fn get_free_list(
        &self,
        zone_idx: usize,
        order: usize,
    ) -> Option<&StaticLinkedList<BuddyBlock, MAX_BLOCKS_PER_LIST>> {
        if zone_idx < self.num_zones {
            Some(&self.zones[zone_idx].free_lists[order])
        } else {
            None
        }
    }

    /// Check if blocks are physically contiguous
    pub fn check_contiguity(&self, blocks: &[(usize, usize)]) -> bool {
        if blocks.len() <= 1 {
            return true;
        }

        // Sort blocks by address
        let mut sorted_blocks = blocks.to_vec();
        sorted_blocks.sort_by_key(|b| b.0);

        // Check if blocks are adjacent
        for i in 0..sorted_blocks.len() - 1 {
            let (addr, order) = sorted_blocks[i];
            let (next_addr, _) = sorted_blocks[i + 1];

            let block_size = (1usize << order) * PAGE_SIZE;
            let expected_next = addr + block_size;

            if next_addr != expected_next {
                return false;
            }
        }

        true
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
            match self.zones[i].alloc_pages(num_pages, alignment) {
                Ok(addr) => {
                    self.update_stats();
                    if num_pages > 10 {
                        info!(
                            "buddy allocator: Allocated {} pages at {:#x} from zone {}",
                            num_pages, addr, i
                        );
                    }
                    return Ok(addr);
                }
                Err(_) => {
                    continue;
                }
            }
        }
        info!(
            "buddy allocator: Allocation failure: {} MB, align {}",
            num_pages * PAGE_SIZE / 0x100000,
            alignment
        );
        self.print_alloc_failure_stats(num_pages, alignment);
        Err(AllocError::NoMemory)
    }

    fn dealloc_pages(&mut self, pos: usize, num_pages: usize) {
        if let Some(zone_idx) = self.find_zone_for_addr(pos) {
            self.zones[zone_idx].dealloc_pages(pos, num_pages);
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
            match self.zones[zone_idx].alloc_pages(num_pages, alignment) {
                Ok(addr) if addr == base => {
                    self.update_stats();
                    Ok(addr)
                }
                Ok(addr) => {
                    self.zones[zone_idx].dealloc_pages(addr, num_pages);
                    Err(AllocError::InvalidParam)
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
