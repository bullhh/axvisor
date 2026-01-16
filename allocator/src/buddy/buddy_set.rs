//! Bitmap-based single-zone buddy allocator
//!
//! Implements the core buddy system for a single memory zone using
//! bitmaps stored in the zone's metadata region for efficient management.
//!
//! # Design
//!
//! - Each order has a bitmap tracking free/used status
//! - Metadata (bitmaps) stored at the beginning of the zone
//! - No external node pool needed - self-contained

use crate::{AllocError, AllocResult};

#[cfg(feature = "log")]
use log::{error, warn, info};

use super::buddy_block::{ZoneInfo, DEFAULT_MAX_ORDER};

/// A bitmap-based buddy set implementation - represents a single zone
///
/// Uses bitmaps to track free/used blocks at each order.
/// All metadata is stored within the zone itself.
pub struct BuddySet<const PAGE_SIZE: usize = { crate::DEFAULT_PAGE_SIZE }> {
    pub(crate) base_addr: usize,
    pub(crate) end_addr: usize,
    total_pages: usize,
    zone_id: usize,
    /// Pointer to the bitmap array for each order
    /// bitmaps[order] points to the bitmap for that order
    bitmaps: [Option<&'static mut [u64]>; DEFAULT_MAX_ORDER + 1],
    /// Number of blocks for each order
    blocks_per_order: [usize; DEFAULT_MAX_ORDER + 1],
    /// Number of free blocks for each order
    free_blocks_per_order: [usize; DEFAULT_MAX_ORDER + 1],
    /// Start address of usable memory (after metadata region)
    usable_start: usize,
}

impl<const PAGE_SIZE: usize> BuddySet<PAGE_SIZE> {
    /// Create a new buddy set for a zone (uninitialized, must call init())
    pub const fn new(base_addr: usize, size: usize, zone_id: usize) -> Self {
        Self {
            base_addr,
            end_addr: base_addr + size,
            total_pages: size / PAGE_SIZE,
            zone_id,
            bitmaps: [const { None }; DEFAULT_MAX_ORDER + 1],
            blocks_per_order: [0; DEFAULT_MAX_ORDER + 1],
            free_blocks_per_order: [0; DEFAULT_MAX_ORDER + 1],
            usable_start: base_addr,
        }
    }

    /// Create an empty buddy set
    pub const fn empty() -> Self {
        Self::new(0, 0, 0)
    }

    pub const fn max_order(&self) -> usize {
        DEFAULT_MAX_ORDER
    }

    /// Calculate bitmap size needed for a zone
    ///
    /// Formula:
    /// - N = zone_total_pages
    /// - max_order m = floor(log2(N))
    /// - blocks_k = ceil(N / 2^k) for order k
    /// - total_bits = sum(blocks_k) for k from 0 to m
    /// - total_bits <= 2N - 1
    fn calculate_metadata_size(total_pages: usize) -> usize {
        if total_pages == 0 {
            return 0;
        }

        let max_order = (total_pages.ilog2() as usize).min(DEFAULT_MAX_ORDER);
        let mut total_u64s = 0;

        for order in 0..=max_order {
            let blocks = (total_pages + (1 << order) - 1) >> order; // ceil(N / 2^k)
            let u64s_needed = (blocks + 63) / 64; // ceil(blocks / 64)
            total_u64s += u64s_needed;
        }

        // Each u64 is 8 bytes
        let bytes_needed = total_u64s * 8;
        
        // Align to page boundary
        (bytes_needed + PAGE_SIZE - 1) & !(PAGE_SIZE - 1)
    }

    /// Check if an address belongs to this zone
    pub fn addr_in_zone(&self, addr: usize) -> bool {
        addr >= self.usable_start && addr < self.end_addr
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
            "zone {}: init started - base={:#x}, size={:#x}",
            self.zone_id, base_addr, size
        );

        let aligned_base = base_addr & !(PAGE_SIZE - 1);
        let end = base_addr + size;
        let aligned_end = (end + PAGE_SIZE - 1) & !(PAGE_SIZE - 1);
        let aligned_size = aligned_end - aligned_base;

        if aligned_size == 0 || aligned_size < PAGE_SIZE {
            panic!("Aligned size is too small: {:#x}", aligned_size);
        }

        self.base_addr = aligned_base;
        self.end_addr = aligned_end;
        let initial_pages = aligned_size / PAGE_SIZE;
        
        // Calculate metadata size
        let metadata_size = Self::calculate_metadata_size(initial_pages);
        let metadata_pages = metadata_size / PAGE_SIZE;
        
        if metadata_pages >= initial_pages {
            panic!("Zone too small to hold metadata: {} pages needed, {} total", 
                   metadata_pages, initial_pages);
        }

        // Metadata region at the start
        self.usable_start = aligned_base + metadata_size;
        self.total_pages = initial_pages - metadata_pages;

        info!(
            "zone {}: metadata_size={} bytes ({} pages), usable_pages={}",
            self.zone_id, metadata_size, metadata_pages, self.total_pages
        );

        // Initialize bitmaps in the metadata region
        let metadata_ptr = aligned_base as *mut u64;
        let mut offset = 0;

        let max_order = (self.total_pages.ilog2() as usize).min(DEFAULT_MAX_ORDER);

        for order in 0..=max_order {
            let blocks = (self.total_pages + (1 << order) - 1) >> order;
            let u64s_needed = (blocks + 63) / 64;
            
            self.blocks_per_order[order] = blocks;
            
            // Safety: We've allocated this memory region for metadata
            let bitmap_slice = unsafe {
                core::slice::from_raw_parts_mut(
                    metadata_ptr.add(offset),
                    u64s_needed
                )
            };
            
            // Initialize all bits to 0 (not free)
            for i in 0..u64s_needed {
                bitmap_slice[i] = 0;
            }
            
            self.bitmaps[order] = Some(bitmap_slice);
            offset += u64s_needed;
        }

        // Release pages to build initial free lists
        // Use Linux-style initialization: add largest possible aligned blocks first
        info!(
            "zone {}: building free lists for {} pages, usable_start={:#x}",
            self.zone_id, self.total_pages, self.usable_start
        );

        let mut pfn = 0;
        while pfn < self.total_pages {
            // Find the largest order that:
            // 1. The remaining pages can accommodate
            // 2. The starting pfn is properly aligned for
            // 3. The physical address is properly aligned for the order
            let mut order = 0;
            while order <= max_order {
                let size = 1 << order;
                let phys_addr = self.usable_start + pfn * PAGE_SIZE;
                let align_requirement = size * PAGE_SIZE;
                
                // Check if we have enough pages, pfn is aligned, AND physical address is aligned
                if pfn + size <= self.total_pages 
                    && (pfn & (size - 1)) == 0 
                    && (phys_addr & (align_requirement - 1)) == 0 {
                    order += 1;
                } else {
                    break;
                }
            }
            order = order.saturating_sub(1);

            let block_size = 1 << order;
            let block_addr = self.usable_start + pfn * PAGE_SIZE;

            // Mark this block as free
            let block_idx = pfn >> order;
            self.set_bit(order, block_idx);
            self.free_blocks_per_order[order] += 1;

            pfn += block_size;
        }

        // Print final free block statistics
        info!("zone {}: initialization complete, free block distribution:", self.zone_id);
        for order in 0..=max_order {
            if self.free_blocks_per_order[order] > 0 {
                info!(
                    "zone {}:   order {}: {} blocks ({} pages total)",
                    self.zone_id, order, self.free_blocks_per_order[order],
                    self.free_blocks_per_order[order] * (1 << order)
                );
            }
        }
    }

    /// Set a bit in the bitmap
    #[inline]
    fn set_bit(&mut self, order: usize, block_idx: usize) {
        if let Some(bitmap) = &mut self.bitmaps[order] {
            let u64_idx = block_idx / 64;
            let bit_idx = block_idx % 64;
            if u64_idx < bitmap.len() {
                bitmap[u64_idx] |= 1u64 << bit_idx;
            }
        }
    }

    /// Clear a bit in the bitmap
    #[inline]
    fn clear_bit(&mut self, order: usize, block_idx: usize) {
        if let Some(bitmap) = &mut self.bitmaps[order] {
            let u64_idx = block_idx / 64;
            let bit_idx = block_idx % 64;
            if u64_idx < bitmap.len() {
                bitmap[u64_idx] &= !(1u64 << bit_idx);
            }
        }
    }

    /// Check if a bit is set in the bitmap
    #[inline]
    fn is_bit_set(&self, order: usize, block_idx: usize) -> bool {
        if let Some(bitmap) = &self.bitmaps[order] {
            let u64_idx = block_idx / 64;
            let bit_idx = block_idx % 64;
            if u64_idx < bitmap.len() {
                return (bitmap[u64_idx] & (1u64 << bit_idx)) != 0;
            }
        }
        false
    }

    /// Find the first set bit in the bitmap (first free block)
    fn find_first_free(&self, order: usize) -> Option<usize> {
        if let Some(bitmap) = &self.bitmaps[order] {
            for (u64_idx, &word) in bitmap.iter().enumerate() {
                if word != 0 {
                    let bit_idx = word.trailing_zeros() as usize;
                    return Some(u64_idx * 64 + bit_idx);
                }
            }
        }
        None
    }

    /// Convert address to block index for a given order
    #[inline]
    fn addr_to_block_idx(&self, addr: usize, order: usize) -> usize {
        let offset = addr - self.usable_start;
        let pfn = offset / PAGE_SIZE;
        pfn >> order
    }

    /// Convert block index to address for a given order
    #[inline]
    fn block_idx_to_addr(&self, block_idx: usize, order: usize) -> usize {
        let pfn = block_idx << order;
        self.usable_start + pfn * PAGE_SIZE
    }

    /// Allocate pages using buddy system
    pub fn alloc_pages(
        &mut self,
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
            error!(
                "zone {}: required_order {} > max_order {}",
                self.zone_id, required_order, self.max_order()
            );
            return Err(AllocError::NoMemory);
        }

        let align_pages = (alignment + PAGE_SIZE - 1) / PAGE_SIZE;
        let align_order = if align_pages > 0 {
            align_pages.next_power_of_two().trailing_zeros() as usize
        } else {
            0
        };
        let order_needed = required_order.max(align_order);



        // Try to find a block of the required order or higher
        for order in order_needed..=self.max_order() {
            if self.free_blocks_per_order[order] == 0 {
                continue;
            }

            if let Some(block_idx) = self.find_first_free(order) {
       

                // Remove this block from free list
                self.clear_bit(order, block_idx);
                self.free_blocks_per_order[order] -= 1;

                let current_addr = self.block_idx_to_addr(block_idx, order);
                let mut current_order = order;

                // Split down to required order
                while current_order > order_needed {
                    current_order -= 1;
                    let split_size = (1 << current_order) * PAGE_SIZE;
                    let buddy_addr = current_addr + split_size;
                    
      

                    // Mark buddy as free
                    let buddy_idx = self.addr_to_block_idx(buddy_addr, current_order);
                    self.set_bit(current_order, buddy_idx);
                    self.free_blocks_per_order[current_order] += 1;
                }

                return Ok(current_addr);
            }
        }

        error!(
            "zone {}: alloc_pages failed: no suitable block found for {} pages (order {})",
            self.zone_id, num_pages, order_needed
        );
        for order in 0..=self.max_order() {
            if self.free_blocks_per_order[order] > 0 {
                info!(
                    "zone {}: order {} has {} free blocks",
                    self.zone_id, order, self.free_blocks_per_order[order]
                );
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



        // Validate address
        if addr < self.usable_start || addr >= self.end_addr {
            error!(
                "zone {}: Address {:#x} not in usable zone [{:#x}, {:#x})",
                self.zone_id, addr, self.usable_start, self.end_addr
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

        let mut order = num_pages.trailing_zeros() as usize;
        if order > DEFAULT_MAX_ORDER {
            error!(
                "zone {}: Order {} exceeds maximum supported order {}",
                self.zone_id, order, DEFAULT_MAX_ORDER
            );
            return;
        }

        // Calculate PFN
        let offset = addr - self.usable_start;
        let pfn = offset / PAGE_SIZE;

        // Check alignment
        if pfn & ((1 << order) - 1) != 0 {
            error!(
                "zone {}: PFN {} is not properly aligned for order {}",
                self.zone_id, pfn, order
            );
            return;
        }

        let mut current_pfn = pfn;

        // Try to merge with buddy blocks
        while order < self.max_order() {
            let buddy_pfn = current_pfn ^ (1 << order);
            let buddy_addr = self.usable_start + buddy_pfn * PAGE_SIZE;

            // Check if buddy is within the zone
            if buddy_addr < self.usable_start || buddy_addr >= self.end_addr {
                break;
            }

            let buddy_idx = buddy_pfn >> order;
            
            // Check if buddy is free
            if self.is_bit_set(order, buddy_idx) {
       

                // Remove buddy from free list
                self.clear_bit(order, buddy_idx);
                self.free_blocks_per_order[order] -= 1;

                // Merge: use the lower address
                current_pfn = current_pfn & buddy_pfn;
                order += 1;
            } else {
                // Buddy not free, cannot merge
                break;
            }
        }

        // Add the final merged block to free list
        let final_idx = current_pfn >> order;
        self.set_bit(order, final_idx);
        self.free_blocks_per_order[order] += 1;

   
    }

    /// Get statistics for this zone
    #[cfg(feature = "tracking")]
    pub fn get_stats(&self) -> super::stats::BuddyStats {
        let mut stats = super::stats::BuddyStats::new();
        stats.total_pages = self.total_pages;

        for order in 0..=DEFAULT_MAX_ORDER {
            let block_count = self.free_blocks_per_order[order];
            stats.free_pages_by_order[order] = block_count;
            stats.free_pages += block_count * (1 << order);
        }

        stats.used_pages = stats.total_pages.saturating_sub(stats.free_pages);
        stats
    }

    /// Get the number of free blocks of a specific order
    pub fn get_free_blocks_by_order(&self, order: usize) -> usize {
        if order <= DEFAULT_MAX_ORDER {
            self.free_blocks_per_order[order]
        } else {
            0
        }
    }

    /// Allocate pages at a specific address
    pub fn alloc_pages_at(
        &mut self,
        base: usize,
        num_pages: usize,
        alignment: usize,
    ) -> AllocResult<usize> {
        if num_pages == 0 {
            return Err(AllocError::InvalidParam);
        }

        // Check if address belongs to this zone
        if base < self.usable_start || base >= self.end_addr {
            return Err(AllocError::InvalidParam);
        }

        // Check page alignment
        if base & (PAGE_SIZE - 1) != 0 {
            return Err(AllocError::InvalidParam);
        }

        // Check alignment requirement
        if base % alignment != 0 {
            return Err(AllocError::InvalidParam);
        }

        // Check if range fits in zone
        let size = num_pages * PAGE_SIZE;
        if base + size > self.end_addr {
            return Err(AllocError::InvalidParam);
        }

        // Calculate required order
        if !num_pages.is_power_of_two() {
            return Err(AllocError::InvalidParam);
        }

        let required_order = num_pages.trailing_zeros() as usize;
        let offset = base - self.usable_start;
        let pfn = offset / PAGE_SIZE;
        
        // Check alignment
        if pfn & ((1 << required_order) - 1) != 0 {
            return Err(AllocError::InvalidParam);
        }

        // Try to find a free block that contains this address
        for order in required_order..=self.max_order() {
            let block_pfn = pfn & !((1 << order) - 1);
            let block_idx = block_pfn >> order;
            let block_addr = self.usable_start + block_pfn * PAGE_SIZE;

            // Check if this block is free
            if self.is_bit_set(order, block_idx) {
                // Remove from free list
                self.clear_bit(order, block_idx);
                self.free_blocks_per_order[order] -= 1;

                let current_addr = block_addr;
                let mut current_order = order;

                // Split down to required order
                while current_order > required_order {
                    current_order -= 1;
                    let split_size = (1 << current_order) * PAGE_SIZE;
                    let buddy_addr = current_addr + split_size;
                    
                    let request_end = base + size;
                    
                    // If buddy is outside the requested region, add it back
                    if buddy_addr < base || buddy_addr >= request_end {
                        let buddy_idx = self.addr_to_block_idx(buddy_addr, current_order);
                        self.set_bit(current_order, buddy_idx);
                        self.free_blocks_per_order[current_order] += 1;
                    }
                }

                return Ok(base);
            }
        }

        Err(AllocError::NoMemory)
    }
}

impl Default for BuddySet {
    fn default() -> Self {
        Self::empty()
    }
}
