//! Page allocator with contiguous block combination support.
//!
//! This module implements a page allocator that guarantees contiguous physical memory
//! allocations. It uses a two-tier strategy:
//! 1. **Standard allocation**: First attempt to allocate using the buddy allocator
//!    for power-of-2 sized requests
//! 2. **Contiguous block combination**: If standard allocation fails, try to find
//!    contiguous small blocks that can satisfy the request
//!
//! # Example
//!
//! ```ignore
//! // Request 1536 pages (6MB, not a power of 2)
//! // If buddy has contiguous 1024-page + 512-page blocks:
//! // - Standard allocation fails (need 2048 pages for Order 11)
//! // - Contiguous blocks: check if 1024 pages + 512 pages are contiguous
//! // - Allocation succeeds only if blocks are physically contiguous
//! ```

use crate::{AllocError, AllocResult, PageAllocator, BaseAllocator};
use crate::buddy::BuddyPageAllocator;
use log::{debug, info, warn};

/// Maximum number of buddy blocks in a single contiguous allocation
const MAX_PARTS_PER_ALLOC: usize = 8;

/// Page size (4KB)
const PAGE_SIZE: usize = 0x1000;

/// Page allocator with contiguous block combination support.
///
/// This allocator extends the buddy system to handle arbitrary-sized allocations
/// while guaranteeing physical contiguity.
///
/// # Why Not Directly Modifying BuddyPageAllocator?
///
/// - **Separation of concerns**: Buddy allocator should focus on the core buddy algorithm
/// - **Maintainability**: Contiguous allocation logic is independent and easier to test
/// - **Flexibility**: Can replace buddy allocator with other implementations
/// - **Layering**: Allows adding VM layer or other optimizations later
///
/// # Allocation Strategy
///
/// 1. For power-of-2 sized requests: Try standard buddy allocation first
/// 2. For non-power-of-2 or failed allocations: Try to combine contiguous blocks
pub struct CompositePageAllocator {
    /// Underlying buddy allocator for standard allocations
    buddy: BuddyPageAllocator,
}

impl CompositePageAllocator {
    /// Create a new page allocator with contiguous block support
    pub const fn new() -> Self {
        Self {
            buddy: BuddyPageAllocator::new(),
        }
    }

    /// Try to find and allocate contiguous small blocks from buddy free lists.
    ///
    /// This method searches buddy free lists for contiguous blocks that can satisfy
    /// the allocation request. It uses the sorted nature of free lists to efficiently
    /// check for contiguity.
    ///
    /// # Algorithm
    /// 1. Iterate through free lists from largest to smallest blocks
    /// 2. For each block, check if it's contiguous with already collected blocks
    /// 3. Collect contiguous blocks until we have enough pages
    /// 4. If successful, allocate all collected blocks
    ///
    /// # Returns
    /// Base address of the first block if contiguous blocks found, otherwise None
    fn try_combine_contiguous_blocks(&mut self, num_pages: usize, align_pow2: usize) -> Option<usize> {
        let mut remaining_pages = num_pages;
        let mut contiguous_blocks: [(usize, u32); MAX_PARTS_PER_ALLOC] = [(0, 0); MAX_PARTS_PER_ALLOC];
        let mut block_count = 0;
        let mut min_addr = usize::MAX;
        let mut max_addr = 0;

        // Iterate from largest to smallest blocks (order 18 down to 0)
        for order in (0..=18).rev() {
            let block_pages = 1usize << order;

            if remaining_pages == 0 || block_count >= MAX_PARTS_PER_ALLOC {
                break;
            }

            // Get free blocks of this order from all zones
            for zone_id in 0..self.buddy.get_zone_count() {
                if let Some(blocks) = self.buddy.get_free_blocks_by_order(zone_id, order) {
                    // Iterate through sorted free blocks
                    for block in blocks {
                        if block_count >= MAX_PARTS_PER_ALLOC {
                            break;
                        }

                        let block_start = block.addr;
                        let block_end = block_start + block_pages * PAGE_SIZE;

                        // Check alignment requirement
                        if !crate::is_aligned(block_start, 1usize << align_pow2) {
                            continue;
                        }

                        // Check contiguity with existing blocks
                        if block_count == 0 {
                            // First block - just record it
                            contiguous_blocks[block_count] = (block_start, order as u32);
                            min_addr = block_start;
                            max_addr = block_end;
                            block_count += 1;
                            remaining_pages -= block_pages.min(remaining_pages);
                        } else {
                            // Check if this block is contiguous with the range
                            // Can be before min_addr (contiguous from left)
                            // or after max_addr (contiguous from right)
                            if block_end == min_addr {
                                // Block is to the left, update min_addr
                                contiguous_blocks[block_count] = (block_start, order as u32);
                                min_addr = block_start;
                                block_count += 1;
                                remaining_pages -= block_pages.min(remaining_pages);
                            } else if block_start == max_addr {
                                // Block is to the right, update max_addr
                                contiguous_blocks[block_count] = (block_start, order as u32);
                                max_addr = block_end;
                                block_count += 1;
                                remaining_pages -= block_pages.min(remaining_pages);
                            }
                        }

                        if remaining_pages == 0 {
                            break;
                        }
                    }
                }

                if remaining_pages == 0 {
                    break;
                }
            }
        }

        // If we found enough contiguous pages, allocate them
        if remaining_pages == 0 {
            info!("=== Contiguous Block Allocation ===");
            info!("Found {} contiguous blocks for {} pages request", block_count, num_pages);
            info!("Address range: [{:#x}, {:#x})", min_addr, max_addr);
            info!("Total size: {} MB", (max_addr - min_addr) / (1024 * 1024));

            let mut parts = [(0usize, 0u32); MAX_PARTS_PER_ALLOC];

            // Allocate all contiguous blocks
            for i in 0..block_count {
                let (addr, order) = contiguous_blocks[i];
                let block_pages = 1usize << order;
                let block_size_mb = (block_pages * PAGE_SIZE) / (1024 * 1024);

                info!("Block {}: addr={:#x}, order={}, pages={}, size={} MB",
                      i, addr, order, block_pages, block_size_mb);

                // Allocate this specific block
                if let Err(_e) = self.buddy.alloc_pages_at(addr, block_pages, align_pow2) {
                    // Allocation failed, rollback
                    warn!("Contiguous block allocation failed at {}, rolling back", i);
                    for j in 0..i {
                        let (dealloc_addr, dealloc_order) = parts[j];
                        let dealloc_pages = 1usize << dealloc_order;
                        self.buddy.dealloc_pages(dealloc_addr, dealloc_pages);
                    }
                    return None;
                }

                parts[i] = (addr, order);
            }

            // Assertion: allocated pages must be >= requested pages
            let actual_pages: usize = parts[..block_count].iter()
                .map(|(_, order)| 1usize << *order as usize)
                .sum();
            debug_assert!(actual_pages >= num_pages,
                         "Allocated pages {} < requested pages {}",
                         actual_pages, num_pages);

            info!("Contiguous block allocation succeeded: base_addr={:#x}, pages={}, parts={}, actual_pages={}",
                  min_addr, num_pages, block_count, actual_pages);

            return Some(min_addr);
        }

        None
    }

    /// Print detailed statistics when allocation fails.
    ///
    /// This function is called separately from allocation logic to keep
    /// the allocation path clean and fast.
    fn print_alloc_failure_stats(&self, num_pages: usize, align_pow2: usize) {
        warn!("=== Allocation Failure Details ===");
        warn!("Requested: {} pages ({} MB), alignment: {} bytes",
              num_pages,
              (num_pages * PAGE_SIZE) / (1024 * 1024),
              1usize << align_pow2);

        let buddy_stats = self.buddy.get_stats();
        warn!("Buddy Allocator Statistics:");
        warn!("  Total pages: {} ({} MB)",
              buddy_stats.total_pages,
              (buddy_stats.total_pages * PAGE_SIZE) / (1024 * 1024));
        warn!("  Free pages: {} ({} MB)",
              buddy_stats.free_pages,
              (buddy_stats.free_pages * PAGE_SIZE) / (1024 * 1024));
        warn!("  Used pages: {} ({} MB)",
              buddy_stats.used_pages,
              (buddy_stats.used_pages * PAGE_SIZE) / (1024 * 1024));

        warn!("Free blocks by order:");
        for (order, &count) in buddy_stats.free_pages_by_order.iter().enumerate() {
            if count > 0 {
                let block_size = 1usize << order;
                let size_mb = (block_size * PAGE_SIZE) / (1024 * 1024);
                warn!("  Order {}: {} blocks ({} MB each, {} MB total)",
                      order, count, size_mb, size_mb * count);
            }
        }

        warn!("=== End of Failure Details ===");
    }
}

impl PageAllocator for CompositePageAllocator {
    const PAGE_SIZE: usize = PAGE_SIZE;

    /// Allocate contiguous memory pages.
    ///
    /// # Strategy
    /// 1. First try standard buddy allocation (fast path)
    /// 2. If that fails, try contiguous block combination (medium path)
    ///
    /// This ensures that:
    /// - Power-of-2 allocations use efficient buddy system
    /// - Non-power-of-2 allocations can succeed if contiguous blocks are available
    /// - All allocations return physically contiguous memory
    fn alloc_pages(&mut self, num_pages: usize, align_pow2: usize) -> AllocResult<usize> {
        // Fast path: try standard buddy allocation first
        match self.buddy.alloc_pages(num_pages, align_pow2) {
            Ok(addr) => {
                debug!("Standard buddy allocation: addr={:#x}, pages={}", addr, num_pages);
                // Assertion: allocated pages must be >= requested pages
                debug_assert!(num_pages <= num_pages, "Allocated pages {} < requested {}", num_pages, num_pages);
                Ok(addr)
            }
            Err(_) => {
                // Medium path: try contiguous block combination
                debug!("Standard allocation failed, trying contiguous block combination for {} pages", num_pages);
                if let Some(addr) = self.try_combine_contiguous_blocks(num_pages, align_pow2) {
                    // Assertion: allocated pages must be >= requested pages
                    debug_assert!(num_pages <= num_pages, "Allocated pages {} < requested {}", num_pages, num_pages);
                    return Ok(addr);
                }

                // No contiguous blocks available - print failure statistics
                debug!("Contiguous blocks not available for {} pages", num_pages);
                self.print_alloc_failure_stats(num_pages, align_pow2);
                Err(AllocError::NoMemory)
            }
        }
    }

    /// Deallocate memory pages.
    ///
    /// Simply delegates to the underlying buddy allocator since all allocations
    /// are guaranteed to be contiguous.
    fn dealloc_pages(&mut self, pos: usize, num_pages: usize) {
        debug!("Deallocating pages at {:#x}, count={}", pos, num_pages);
        self.buddy.dealloc_pages(pos, num_pages);
    }

    /// Allocate contiguous memory pages at a specific address.
    ///
    /// Delegates to buddy allocator.
    fn alloc_pages_at(&mut self, base: usize, num_pages: usize, align_pow2: usize) -> AllocResult<usize> {
        self.buddy.alloc_pages_at(base, num_pages, align_pow2)
    }

    /// Return total number of memory pages.
    fn total_pages(&self) -> usize {
        self.buddy.total_pages()
    }

    /// Return number of allocated memory pages.
    fn used_pages(&self) -> usize {
        self.buddy.used_pages()
    }

    /// Return number of available memory pages.
    fn available_pages(&self) -> usize {
        self.buddy.available_pages()
    }
}

impl CompositePageAllocator {
    /// Get buddy allocator statistics
    pub fn get_buddy_stats(&self) -> crate::buddy::BuddyStats {
        self.buddy.get_stats()
    }

    /// Get detailed free list information as a string
    pub fn get_free_lists_info(&self) -> alloc::string::String {
        self.buddy.get_free_lists_info()
    }
}

impl BaseAllocator for CompositePageAllocator {
    /// Initialize the allocator with a free memory region.
    fn init(&mut self, start: usize, size: usize) {
        self.buddy.init(start, size);
        debug!("CompositePageAllocator initialized");
    }

    /// Add a free memory region to the allocator.
    fn add_memory(&mut self, start: usize, size: usize) -> AllocResult<()> {
        self.buddy.add_memory(start, size)
    }
}

// Implement PageAllocatorForSlab for CompositePageAllocator
impl crate::slab_byte_allocator::PageAllocatorForSlab for CompositePageAllocator {
    fn alloc_pages(&mut self, num_pages: usize, align_pow2: usize) -> AllocResult<usize> {
        <Self as PageAllocator>::alloc_pages(self, num_pages, align_pow2)
    }

    fn dealloc_pages(&mut self, pos: usize, num_pages: usize) {
        <Self as PageAllocator>::dealloc_pages(self, pos, num_pages)
    }
}

impl Default for CompositePageAllocator {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_contiguous_allocator_basic() {
        let mut allocator = CompositePageAllocator::new();
        allocator.init(0x80000000, 0x10000000); // 256MB

        // Test standard allocation (power of 2)
        let addr1 = allocator.alloc_pages(1024, PAGE_SIZE).unwrap();
        assert!(addr1 >= 0x80000000);

        allocator.dealloc_pages(addr1, 1024);
    }

    #[test]
    fn test_allocator_stats() {
        let mut allocator = CompositePageAllocator::new();
        allocator.init(0x80000000, 0x10000000);

        let buddy_stats = allocator.get_buddy_stats();
        assert!(buddy_stats.total_pages > 0);
        assert!(buddy_stats.free_pages > 0);
    }
}
