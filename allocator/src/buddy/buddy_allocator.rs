//! Bitmap-based multi-zone buddy allocator
//!
//! Provides buddy allocator with support for multiple memory zones and
//! a bitmap-based representation for efficient contiguity checking.
//!
//! # Architecture
//!
//! - **Bitmap arrays**: Track free/used status for each order
//! - **BuddySetPool**: Each zone's free lists, using bitmaps instead of linked lists
//! - **BuddyPageAllocator**: Coordinates multiple zones with bitmap-based representation

use crate::{AllocError, AllocResult, BaseAllocator, PageAllocator};

#[cfg(feature = "log")]
use log::{debug, error, info, warn};

use super::{
    buddy_block::MAX_ZONES,
    buddy_set::BuddySet,
};

#[cfg(feature = "tracking")]
use super::stats::{BuddyStats, MemoryStatsReporter};

/// Buddy page allocator with multi-zone support and bitmap-based representation
///
/// Uses bitmaps to track free/used status of memory blocks at each order
/// rather than linked lists, enabling faster allocation/deallocation.
pub struct BuddyPageAllocator<const PAGE_SIZE: usize = { crate::DEFAULT_PAGE_SIZE }> {
    zones: [BuddySet<PAGE_SIZE>; MAX_ZONES],
    num_zones: usize,
    #[cfg(feature = "tracking")]
    stats: BuddyStats,
}

impl<const PAGE_SIZE: usize> BuddyPageAllocator<PAGE_SIZE> {
    pub const fn new() -> Self {
        Self {
            zones: [const { BuddySet::<PAGE_SIZE>::empty() }; MAX_ZONES],
            num_zones: 0,
            #[cfg(feature = "tracking")]
            stats: BuddyStats::new(),
        }
    }

    /// Initialize the allocator with initial memory region
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

        self.zones[0] = BuddySet::new(base_addr, size, 0);
        self.zones[0].init(base_addr, size);
        self.num_zones = 1;

        #[cfg(feature = "tracking")]
        self.update_stats();
    }

    #[cfg(feature = "tracking")]
    pub fn get_stats(&self) -> BuddyStats {
        self.stats
    }

    /// Get number of zones in the allocator
    pub fn get_zone_count(&self) -> usize {
        self.num_zones
    }

    /// Get number of free blocks of a specific order from a zone
    /// Returns None if zone doesn't exist
    pub fn get_free_blocks_by_order(&self, zone_id: usize, order: u32) -> Option<usize> {
        if zone_id >= self.num_zones {
            return None;
        }
        Some(self.zones[zone_id].get_free_blocks_by_order(order as usize))
    }

    /// Update aggregated statistics from all zones
    #[cfg(feature = "tracking")]
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
    ///
    /// This method is public to allow CompositePageAllocator to call it
    /// when allocation fails, providing detailed per-zone failure information.
    #[cfg(feature = "tracking")]
    pub fn print_alloc_failure_stats(&self, num_pages: usize, alignment: usize) {
        let mut zone_infos: [Option<ZoneInfo>; MAX_ZONES] = [None; MAX_ZONES];
        let mut zone_stats: [Option<BuddyStats>; MAX_ZONES] = [None; MAX_ZONES];

        for i in 0..self.num_zones {
            zone_infos[i] = Some(self.zones[i].zone_info());
            zone_stats[i] = Some(self.zones[i].get_stats());
        }

        // Create slices from the initialized elements
        let zone_infos_slice: &[ZoneInfo] = unsafe {
            core::slice::from_raw_parts(zone_infos.as_ptr() as *const ZoneInfo, self.num_zones)
        };
        let zone_stats_slice: &[BuddyStats] = unsafe {
            core::slice::from_raw_parts(zone_stats.as_ptr() as *const BuddyStats, self.num_zones)
        };

        MemoryStatsReporter::print_alloc_failure_stats(
            PAGE_SIZE,
            self.num_zones,
            &self.stats,
            zone_infos_slice,
            zone_stats_slice,
            num_pages,
            alignment,
        );
    }

    #[cfg(not(feature = "tracking"))]
    pub fn print_alloc_failure_stats(&self, _num_pages: usize, _alignment: usize) {
        // No-op when tracking is disabled
    }
}

impl<const PAGE_SIZE: usize> crate::slab::PageAllocatorForSlab for BuddyPageAllocator<PAGE_SIZE> {
    fn alloc_pages(&mut self, num_pages: usize, alignment: usize) -> AllocResult<usize> {
        <Self as PageAllocator>::alloc_pages(self, num_pages, alignment)
    }

    fn dealloc_pages(&mut self, pos: usize, num_pages: usize) {
        <Self as PageAllocator>::dealloc_pages(self, pos, num_pages);
    }
}

impl<const PAGE_SIZE: usize> Default for BuddyPageAllocator<PAGE_SIZE> {
    fn default() -> Self {
        Self::new()
    }
}

impl<const PAGE_SIZE: usize> BaseAllocator for BuddyPageAllocator<PAGE_SIZE> {
    fn init(&mut self, start: usize, size: usize) {
        self.bootstrap(start, size);
    }

    fn add_memory(&mut self, start: usize, size: usize) -> AllocResult<()> {
        self.add_memory_region(start, size)?;
        #[cfg(feature = "tracking")]
        self.update_stats();
        Ok(())
    }
}

impl<const PAGE_SIZE: usize> PageAllocator for BuddyPageAllocator<PAGE_SIZE> {
    const PAGE_SIZE: usize = PAGE_SIZE;

    fn alloc_pages(&mut self, num_pages: usize, alignment: usize) -> AllocResult<usize> {
        for i in 0..self.num_zones {
            match self.zones[i].alloc_pages(num_pages, alignment) {
                Ok(addr) => {
                    #[cfg(feature = "tracking")]
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
            self.zones[zone_idx].dealloc_pages(pos, num_pages);
            #[cfg(feature = "tracking")]
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
            match self.zones[zone_idx].alloc_pages_at(base, num_pages, alignment) {
                Ok(addr) => {
                    #[cfg(feature = "tracking")]
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
        #[cfg(feature = "tracking")]
        return self.stats.total_pages;
        #[cfg(not(feature = "tracking"))]
        return 0;
    }

    fn used_pages(&self) -> usize {
        #[cfg(feature = "tracking")]
        return self.stats.used_pages;
        #[cfg(not(feature = "tracking"))]
        return 0;
    }

    fn available_pages(&self) -> usize {
        #[cfg(feature = "tracking")]
        return self.stats.free_pages;
        #[cfg(not(feature = "tracking"))]
        return 0;
    }
}
