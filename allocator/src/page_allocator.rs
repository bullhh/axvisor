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
//! 1. Allocate next power-of-2 from buddy system
//! 2. Start from the base address (which is aligned to power-of-2)
//! 3. Decompose the block from largest to smallest orders
//! 4. For each chunk, decide if it goes to the user or back to buddy
//! 5. Ensure all chunks are properly aligned
//!
//! # Why Backward?
//!
//! Forward decomposition (starting from user's end) causes alignment issues:
//! - Excess starts at base + user_pages, which may not be aligned
//! - Example: 0x80000000 (2^11 aligned) + 1540 pages = 0x8180D000
//! - 0x8180D000 / 4096 = 135949, 135949 % 256 = 61 ≠ 0
//! - Not aligned for order 8 (256 pages)!
//!
//! Backward decomposition solves this by:
//! - Starting from the base address (already aligned)
//! - Ensuring each chunk is checked for alignment before use

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

                        info!(
                            "block_start: {:#x}, block_end: {:#x}, alignment: {}",
                            block_start, block_end, alignment
                        );
                        // Check alignment requirement
                        if !crate::is_aligned(block_start, alignment) {
                            info!("block_start is not aligned");
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
            info!(
                "Found {} contiguous blocks for {} pages request",
                block_count, num_pages
            );
            info!("Address range: [{:#x}, {:#x})", min_addr, max_addr);
            info!("Total size: {} MB", (max_addr - min_addr) / (1024 * 1024));

            let mut parts = [(0usize, 0u32); MAX_PARTS_PER_ALLOC];

            // Allocate all contiguous blocks
            for i in 0..block_count {
                let (addr, order) = contiguous_blocks[i];
                let block_pages = 1usize << order;
                let block_size_mb = (block_pages * PAGE_SIZE) / (1024 * 1024);

                info!(
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

    /// Decompose a non-power-of-2 page count into power-of-2 chunks.
    ///
    /// This is used when deallocating memory that wasn't a power-of-2 allocation.
    /// Each chunk is returned to the buddy separately.
    fn dealloc_non_power_of_two(&mut self, mut addr: usize, mut pages: usize) {
        debug!(
            "Deallocating non-power-of-2: {} pages at {:#x}",
            pages, addr
        );

        let mut chunk_count = 0;

        while pages > 0 {
            // Binary decomposition: find largest power of 2 <= pages
            let highest_bit = pages.ilog2();
            let chunk_pages = 1usize << highest_bit;

            debug!(
                "Chunk {}: dealloc {} pages at {:#x}",
                chunk_count, chunk_pages, addr
            );

            self.buddy.dealloc_pages(addr, chunk_pages);

            // Move to next chunk
            addr += chunk_pages * PAGE_SIZE;
            pages -= chunk_pages;
            chunk_count += 1;
        }

        debug!("Deallocated {} chunks total", chunk_count);
    }

    /// Print detailed statistics when allocation fails.
    ///
    /// This function is called separately from allocation logic to keep
    /// the allocation path clean and fast.
    fn print_alloc_failure_stats(&self, num_pages: usize, alignment: usize) {
        warn!("=== Allocation Failure Details ===");
        warn!(
            "Requested: {} pages ({} MB), alignment: {} bytes",
            num_pages,
            (num_pages * PAGE_SIZE) / (1024 * 1024),
            alignment
        );

        let buddy_stats = self.buddy.get_stats();
        warn!("Buddy Allocator Statistics:");
        warn!(
            "  Total pages: {} ({} MB)",
            buddy_stats.total_pages,
            (buddy_stats.total_pages * PAGE_SIZE) / (1024 * 1024)
        );
        warn!(
            "  Free pages: {} ({} MB)",
            buddy_stats.free_pages,
            (buddy_stats.free_pages * PAGE_SIZE) / (1024 * 1024)
        );
        warn!(
            "  Used pages: {} ({} MB)",
            buddy_stats.used_pages,
            (buddy_stats.used_pages * PAGE_SIZE) / (1024 * 1024)
        );

        warn!("Free blocks by order:");
        for (order, &count) in buddy_stats.free_pages_by_order.iter().enumerate() {
            if count > 0 {
                let block_size = 1usize << order;
                let size_mb = (block_size * PAGE_SIZE) / (1024 * 1024);
                warn!(
                    "  Order {}: {} blocks ({} MB each, {} MB total)",
                    order,
                    count,
                    size_mb,
                    size_mb * count
                );
            }
        }

        warn!("=== End of Failure Details ===");
    }
}

impl PageAllocator for CompositePageAllocator {
    const PAGE_SIZE: usize = PAGE_SIZE;

    /// Allocate contiguous memory pages with backward decomposition overflow handling.
    ///
    /// # Backward Decomposition Strategy
    ///
    /// The buddy system always allocates power-of-2 sized blocks. When a user requests
    /// a non-power-of-2 amount, we use backward decomposition:
    ///
    /// 1. Allocate next power-of-2 from buddy system
    /// 2. Start from the base address (which is aligned to power-of-2)
    /// 3. Decompose the block from largest to smallest orders
    /// 4. For each chunk, decide if it goes to the user or back to buddy
    /// 5. Ensure all chunks are properly aligned
    ///
    /// # Example: Request 1540 pages
    /// - Buddy allocates: 2048 pages (2^11) at 0x80000000 (aligned to 2^11)
    /// - Decompose from base:
    ///   * Order 10 (1024 pages): Give to user (1024 <= 1540)
    ///   * Order 9 (512 pages): Give to user (1024+512=1536 <= 1540)
    ///   * Order 2 (4 pages): Give to user (1536+4=1540 == 1540) ✓
    ///   * Remaining: 2048-1540 = 508 pages
    ///   * Order 8 (256 pages): Return to buddy
    ///   * Order 7 (128 pages): Return to buddy
    ///   * Order 6 (64 pages): Return to buddy
    ///   * Order 5 (32 pages): Return to buddy
    ///   * Order 4 (16 pages): Return to buddy
    ///   * Order 3 (8 pages): Return to buddy
    ///   * Order 2 (4 pages): Return to buddy
    /// - All chunks are properly aligned! No alignment errors.
    ///
    /// # Why Backward?
    ///
    /// Forward decomposition (starting from user's end) causes alignment issues:
    /// - Excess starts at base + user_pages, which may not be aligned
    /// - Example: 0x80000000 (2^11 aligned) + 1540 pages = 0x8180D000
    /// - 0x8180D000 / 4096 = 135949, 135949 % 256 = 61 ≠ 0
    /// - Not aligned for order 8 (256 pages)!
    ///
    /// Backward decomposition solves this by:
    /// - Starting from the base address (already aligned)
    /// - Ensuring each chunk is checked for alignment before use
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
                info!(
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

        // Backward decomposition: decompose from base address
        info!("=== Backward Decomposition Allocation ===");
        info!(
            "Base addr: {:#x}, user needs: {} pages, buddy allocated: {} pages",
            base_addr, num_pages, buddy_pages
        );

        let mut current_addr = base_addr;
        let mut remaining_user = num_pages;
        let mut remaining_buddy = buddy_pages;
        let mut user_end_addr = base_addr;
        let mut chunk_count = 0;
        let mut user_chunks = 0;
        let mut excess_chunks = 0;

        // Start from the order of buddy allocation
        let mut order = if buddy_pages > 0 {
            (buddy_pages.ilog2() as u32).min(DEFAULT_MAX_ORDER as u32) as usize
        } else {
            0
        };

        // Phase 1: Allocate to user until their needs are met
        // Use binary decomposition: always use the largest block that fits
        while remaining_user > 0 && remaining_buddy > 0 {
            // Find the largest power of 2 that fits in remaining_user
            let max_order = if remaining_user > 0 {
                (remaining_user.ilog2() as u32).min(DEFAULT_MAX_ORDER as u32) as usize
            } else {
                0
            };

            // Start from the largest possible order
            order = max_order;

            let mut found_block = false;
            while order > 0 && !found_block {
                let block_pages = 1usize << order;

                if block_pages > remaining_buddy {
                    order -= 1;
                    continue;
                }

                // Check alignment at current_addr
                let pfn = current_addr / PAGE_SIZE;
                if pfn & ((1 << order) - 1) != 0 {
                    // Not aligned, try smaller order
                    order -= 1;
                    continue;
                }

                // Found a valid aligned block, give it to user
                info!(
                    "  User chunk #{}: addr={:#x}, pages={}, order={}, size={} MB",
                    user_chunks,
                    current_addr,
                    block_pages,
                    order,
                    (block_pages * PAGE_SIZE) / (1024 * 1024)
                );
                remaining_user -= block_pages;
                user_end_addr = current_addr + block_pages * PAGE_SIZE;
                current_addr += block_pages * PAGE_SIZE;
                remaining_buddy -= block_pages;
                user_chunks += 1;
                chunk_count += 1;
                found_block = true;
            }

            // If no aligned block found, try order 0 (1 page)
            if !found_block && remaining_user > 0 && remaining_buddy > 0 {
                let block_pages = 1usize;
                let pfn = current_addr / PAGE_SIZE;
                if pfn & ((1 << 0) - 1) == 0 && block_pages <= remaining_buddy {
                    info!(
                        "  User chunk #{}: addr={:#x}, pages={}, order={}, size={} MB",
                        user_chunks,
                        current_addr,
                        block_pages,
                        0,
                        (block_pages * PAGE_SIZE) / (1024 * 1024)
                    );
                    remaining_user -= block_pages;
                    user_end_addr = current_addr + block_pages * PAGE_SIZE;
                    current_addr += block_pages * PAGE_SIZE;
                    remaining_buddy -= block_pages;
                    user_chunks += 1;
                    chunk_count += 1;
                    found_block = true;
                }
            }

            // Safety check to avoid infinite loop
            if !found_block {
                warn!(
                    "Cannot find aligned block for remaining_user={}, remaining_buddy={}",
                    remaining_user, remaining_buddy
                );
                break;
            }
        }

        // Phase 2: Return remaining blocks back to buddy
        while remaining_buddy > 0 {
            let block_pages = 1usize << order;

            if block_pages > remaining_buddy {
                if order > 0 {
                    order -= 1;
                }
                continue;
            }

            // Check alignment
            let pfn = current_addr / PAGE_SIZE;
            if pfn & ((1 << order) - 1) != 0 {
                if order > 0 {
                    order -= 1;
                }
                continue;
            }

            // Return this chunk to buddy
            info!(
                "  Excess chunk #{}: addr={:#x}, pages={}, order={}, size={} MB",
                excess_chunks,
                current_addr,
                block_pages,
                order,
                (block_pages * PAGE_SIZE) / (1024 * 1024)
            );
            self.buddy.dealloc_pages(current_addr, block_pages);

            current_addr += block_pages * PAGE_SIZE;
            remaining_buddy -= block_pages;
            excess_chunks += 1;
            chunk_count += 1;

            // Reset order to try larger blocks
            order = if remaining_buddy > 0 {
                (remaining_buddy.ilog2() as u32).min(DEFAULT_MAX_ORDER as u32) as usize
            } else {
                0
            };
        }

        // Verify we allocated enough to the user
        debug_assert!(
            remaining_user == 0,
            "Failed to allocate all user pages: {} remaining",
            remaining_user
        );

        let total_excess = buddy_pages - num_pages;
        info!("=== Summary ===");
        info!(
            "User memory: {:#x} ~ {:#x} ({} pages = {} MB)",
            base_addr,
            user_end_addr,
            num_pages,
            (num_pages * PAGE_SIZE) / (1024 * 1024)
        );
        info!(
            "Excess returned: {} pages ({} MB) in {} chunks",
            total_excess,
            (total_excess * PAGE_SIZE) / (1024 * 1024),
            excess_chunks
        );
        info!("Total chunks processed: {}", chunk_count);
        info!("================");

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

    /// Get detailed free list information as a string
    pub fn get_free_lists_info(&self) -> alloc::string::String {
        self.buddy.get_free_lists_info()
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
