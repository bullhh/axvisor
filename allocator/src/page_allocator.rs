//! Page allocator with contiguous block combination support.
//!
//! This module implements a page allocator that guarantees contiguous physical memory
//! allocations. It uses a two-tier strategy:
//! 1. **Standard allocation**: First attempt to allocate using the buddy allocator
//! 2. **Backward decomposition overflow**: If buddy allocates more than requested,
//!    use backward decomposition to return excess memory back to buddy system
//! 3. **Contiguous block combination**: If standard allocation fails, try to find
//!    contiguous small blocks that can satisfy the request
//!
//! # Example
//!
//! ```ignore
//! // Request 1536 pages (6MB, not a power of 2)
//! // Buddy allocates 2048 pages (8MB, Order 11)
//! // Page allocator uses backward decomposition:
//! //   - 1024 pages -> user (1024 <= 1536)
//! //   - 512 pages -> user (1024+512=1536 == 1536)
//! //   - Remaining 512 pages -> returned to buddy
//! // User receives exactly 1536 pages
//! ```
//!
//! # Backward Decomposition Strategy
//!
//! The buddy system always allocates power-of-2 sized blocks. When a user requests
//! a non-power-of-2 amount, we use backward decomposition:
//!
//! 1. Allocate next power-of-2 from buddy system (e.g., 2048 pages for 1536 request)
//! 2. The user gets the first `num_pages` pages (e.g., 1536 pages)
//! 3. Return the excess pages (e.g., 512 pages) back to buddy system
//!
//! # Why Backward Decomposition?
//!
//! We return excess memory by decomposing from the end (back to front):
//! - Start from end_addr = base_addr + buddy_pages * PAGE_SIZE (aligned)
//! - Decompose excess into power-of-2 chunks from back to front
//! - Each chunk is guaranteed to be aligned because we start from aligned boundary
//!
//! Example with 2048 pages allocated, 1537 requested (511 excess):
//! - End addr: 0x80080000 (2048 pages aligned)
//! - Release 256 pages at 0x80040000 (aligned to 256)
//! - Release 128 pages at 0x80020000 (aligned to 128)
//! - Release 64 pages at 0x80010000 (aligned to 64)
//! - ... and so on
//!
//! All chunks are properly aligned!

use crate::buddy::{BuddyPageAllocator, DEFAULT_MAX_ORDER};
use crate::{AllocError, AllocResult, BaseAllocator, PageAllocator};
use log::{debug, info, warn};

/// Maximum number of buddy blocks in a single contiguous allocation
const MAX_PARTS_PER_ALLOC: usize = 8;

/// Page size (4KB)
const PAGE_SIZE: usize = 0x1000;

/// Page allocator with overflow handling and contiguous block combination support.
///
/// This allocator extends the buddy system to:
/// 1. **Handle overflow**: Return excess memory allocated by buddy back to system
/// 2. **Combine contiguous blocks**: When standard allocation fails, try to find
///    contiguous small blocks that can satisfy the request
///
/// # Why Not Directly Modifying BuddyPageAllocator?
///
/// - **Separation of concerns**: Buddy allocator should focus on core buddy algorithm
/// - **Purity**: Buddy system should maintain standard behavior (always allocates power-of-2)
/// - **Flexibility**: Different page allocators can have different overflow strategies
/// - **Maintainability**: Overflow logic is independent and easier to test at this layer
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
    fn try_combine_contiguous_blocks(
        &mut self,
        num_pages: usize,
        alignment: usize,
    ) -> Option<usize> {
        let mut remaining_pages = num_pages;
        let mut contiguous_blocks: [(usize, u32); MAX_PARTS_PER_ALLOC] =
            [(0, 0); MAX_PARTS_PER_ALLOC];
        let mut block_count = 0;
        let mut min_addr = usize::MAX;
        let mut max_addr = 0;

        // Iterate from largest to smallest blocks
        for order in (0..=DEFAULT_MAX_ORDER).rev() {
            let block_pages = 1usize << order;

            if remaining_pages == 0 || block_count >= MAX_PARTS_PER_ALLOC {
                break;
            }

            // Get free blocks of this order from all zones
            for zone_id in 0..self.buddy.get_zone_count() {
                if let Some(blocks) = self.buddy.get_free_blocks_by_order(zone_id, order as u32) {
                    // Iterate through sorted free blocks
                    for block in blocks {
                        if block_count >= MAX_PARTS_PER_ALLOC {
                            break;
                        }

                        let block_start = block.addr;
                        let block_end = block_start + block_pages * PAGE_SIZE;

                        // Check alignment requirement
                        if !crate::is_aligned(block_start, alignment) {
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

            let mut parts = [(0usize, 0u32); MAX_PARTS_PER_ALLOC];

            // Allocate all contiguous blocks
            for i in 0..block_count {
                let (addr, order) = contiguous_blocks[i];
                let block_pages = 1usize << order;
                let block_size_mb = (block_pages * PAGE_SIZE) / (1024 * 1024);

                debug!(
                    "Block {}: addr={:#x}, order={}, pages={}, size={} MB",
                    i, addr, order, block_pages, block_size_mb
                );

                // Allocate this specific block
                if let Err(_e) = self.buddy.alloc_pages_at(addr, block_pages, alignment) {
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
            let actual_pages: usize = parts[..block_count]
                .iter()
                .map(|(_, order)| 1usize << *order as usize)
                .sum();
            debug_assert!(
                actual_pages >= num_pages,
                "Allocated pages {} < requested pages {}",
                actual_pages,
                num_pages
            );

            info!("Contiguous block allocation succeeded: base_addr={:#x}, pages={}, parts={}, actual_pages={}",
                  min_addr, num_pages, block_count, actual_pages);

            return Some(min_addr);
        }

        None
    }

    /// Backward decomposition: decompose excess memory back to buddy system.
    ///
    /// When buddy allocates more pages than requested (e.g., 2048 for 1536 request),
    /// this function returns excess memory back to buddy system.
    ///
    /// # Algorithm
    /// 1. Calculate excess pages: buddy_pages - num_pages
    /// 2. Start from end address (base_addr + buddy_pages * PAGE_SIZE), which is aligned
    /// 3. Decompose excess from back to front using largest power-of-2 chunks
    /// 4. Each chunk is guaranteed to be aligned because we start from aligned boundary
    fn backward_decompose_overflow(
        &mut self,
        base_addr: usize,
        num_pages: usize,
        buddy_pages: usize,
    ) -> AllocResult<()> {
        let excess = buddy_pages - num_pages;

        if excess == 0 {
            return Ok(());
        }

        // Start from end address (aligned to buddy_pages), work backwards
        let end_addr = base_addr + buddy_pages * PAGE_SIZE;
        let mut remaining = excess;
        let mut current_addr = end_addr;

        debug!(
            "Backward decomposing: base={:#x}, user_pages={}, buddy_pages={}, excess={}",
            base_addr, num_pages, buddy_pages, excess
        );

        // Decompose excess into power-of-2 chunks from back to front
        // This ensures all chunks are aligned because end_addr is aligned
        while remaining > 0 {
            // Find largest power of 2 that fits in remaining
            let highest_bit = remaining.ilog2();
            let chunk_pages = 1usize << highest_bit;

            // Move backward by chunk_pages
            current_addr -= chunk_pages * PAGE_SIZE;

            // Check alignment (should always pass if logic is correct)
            let pfn = current_addr / PAGE_SIZE;
            if pfn & (chunk_pages - 1) != 0 {
                warn!(
                    "  Address {:#x} not aligned for {} pages, pfn={}, mask={}",
                    current_addr, chunk_pages, pfn, chunk_pages - 1
                );
                return Err(AllocError::InvalidParam);
            }

            // Return this chunk to buddy
            debug!(
                "  Returning excess: addr={:#x}, pages={}, order={}, size={} MB",
                current_addr,
                chunk_pages,
                highest_bit,
                (chunk_pages * PAGE_SIZE) / (1024 * 1024)
            );
            self.buddy.dealloc_pages(current_addr, chunk_pages);

            remaining -= chunk_pages;
        }

        Ok(())
    }

    /// Decompose a non-power-of-2 page count into power-of-2 chunks.
    ///
    /// This is used when deallocating memory that wasn't a power-of-2 allocation.
    /// Each chunk is returned to the buddy separately.
    fn dealloc_non_power_of_two(&mut self, mut addr: usize, mut pages: usize) {
        debug!(
            "Deallocating non-power-of-2: {} pages at {:#x}",
            pages, addr
        );

        while pages > 0 {
            // Binary decomposition: find largest power of 2 <= pages
            let highest_bit = pages.ilog2();
            let chunk_pages = 1usize << highest_bit;

            self.buddy.dealloc_pages(addr, chunk_pages);

            // Move to next chunk
            addr += chunk_pages * PAGE_SIZE;
            pages -= chunk_pages;
        }
    }

    /// Print detailed statistics when allocation fails.
    ///
    /// This function delegates to buddy allocator's detailed statistics reporter.
    fn print_alloc_failure_stats(&self, num_pages: usize, alignment: usize) {
        self.buddy.print_alloc_failure_stats(num_pages, alignment);
    }
}

impl PageAllocator for CompositePageAllocator {
    const PAGE_SIZE: usize = PAGE_SIZE;

    fn alloc_pages(&mut self, num_pages: usize, alignment: usize) -> AllocResult<usize> {
        if num_pages == 0 {
            return Err(AllocError::InvalidParam);
        }

        let buddy_pages = if num_pages.is_power_of_two() {
            num_pages
        } else {
            num_pages.next_power_of_two()
        };

        // Try to allocate from buddy system first
        let base_addr = match self.buddy.alloc_pages(buddy_pages, alignment) {
            Ok(addr) => addr,
            Err(_) => {
                // Standard allocation failed, try contiguous block combination
                debug!(
                    "Standard allocation failed, trying contiguous block combination for {} pages",
                    num_pages
                );
                if let Some(addr) = self.try_combine_contiguous_blocks(num_pages, alignment) {
                    return Ok(addr);
                }
                self.print_alloc_failure_stats(num_pages, alignment);
                return Err(AllocError::NoMemory);
            }
        };

        // If buddy allocated exactly what was requested, no overflow handling needed
        if buddy_pages == num_pages {
            return Ok(base_addr);
        }

        // Backward decomposition: decompose excess memory back to buddy
        self.backward_decompose_overflow(base_addr, num_pages, buddy_pages)?;

        Ok(base_addr)
    }

    /// Deallocate memory pages.
    ///
    /// Handles both power-of-2 and non-power-of-2 allocations.
    /// - Power-of-2: Delegates directly to buddy system
    /// - Non-power-of-2: Decomposes into power-of-2 chunks, then delegates to buddy
    fn dealloc_pages(&mut self, pos: usize, num_pages: usize) {
        debug!("Deallocating pages at {:#x}, count={}", pos, num_pages);

        if num_pages == 0 {
            return;
        }

        // Check if we need to decompose the deallocation
        if num_pages.is_power_of_two() {
            // Power-of-2: can deallocate directly to buddy
            self.buddy.dealloc_pages(pos, num_pages);
        } else {
            // Non-power-of-2: decompose into power-of-2 chunks
            self.dealloc_non_power_of_two(pos, num_pages);
        }
    }

    /// Allocate contiguous memory pages at a specific address.
    ///
    /// Delegates to buddy allocator.
    fn alloc_pages_at(
        &mut self,
        base: usize,
        num_pages: usize,
        alignment: usize,
    ) -> AllocResult<usize> {
        self.buddy.alloc_pages_at(base, num_pages, alignment)
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
}

impl BaseAllocator for CompositePageAllocator {
    /// Initialize the allocator with a free memory region.
    fn init(&mut self, start: usize, size: usize) {
        self.buddy.init(start, size);
    }

    /// Add a free memory region to the allocator.
    fn add_memory(&mut self, start: usize, size: usize) -> AllocResult<()> {
        self.buddy.add_memory(start, size)
    }
}

// Implement PageAllocatorForSlab for CompositePageAllocator
impl crate::slab_byte_allocator::PageAllocatorForSlab for CompositePageAllocator {
    fn alloc_pages(&mut self, num_pages: usize, alignment: usize) -> AllocResult<usize> {
        <Self as PageAllocator>::alloc_pages(self, num_pages, alignment)
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

