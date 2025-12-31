//! Page allocator with composite allocation support.
//!
//! This module implements a page allocator that can handle arbitrary-sized allocations
//! by combining multiple buddy blocks when standard buddy allocation is not possible.
//! This addresses the issue where a single large allocation cannot be satisfied
//! due to buddy system's power-of-2 constraint, even when sufficient total memory exists.
//!
//! # Design
//!
//! The allocator uses a two-tier strategy:
//! 1. **Standard allocation**: First attempt to allocate using the buddy allocator
//!    for power-of-2 sized requests
//! 2. **Composite allocation**: If standard allocation fails, decompose the request
//!    into multiple power-of-2 blocks and combine them
//!
//! # Example
//!
//! ```ignore
//! // Request 1536MB (not a power of 2)
//! // If buddy has 2x1024MB blocks:
//! // - Standard allocation fails (need 2048MB for Order 19)
//! // - Composite allocation succeeds: 1x1024MB + 1x512MB = 1536MB
//! ```

use crate::{AllocError, AllocResult, PageAllocator, BaseAllocator};
use crate::buddy_page_allocator::BuddyPageAllocator;
use log::{debug, info, warn};

/// Maximum number of concurrent composite allocations
const MAX_COMPOSITE_ALLOCS: usize = 64;

/// Maximum number of buddy blocks in a single composite allocation
const MAX_PARTS_PER_ALLOC: usize = 8;

/// Page size (4KB)
const PAGE_SIZE: usize = 0x1000;

/// Composite allocation metadata
///
/// Tracks a composite allocation that consists of multiple buddy blocks.
/// When deallocating, all constituent blocks must be freed.
#[derive(Clone, Copy)]
struct CompositeAllocation {
    /// Base address of the composite allocation
    base_addr: usize,
    /// Total number of pages in this composite allocation
    total_pages: usize,
    /// Constituent buddy blocks: (physical_address, order)
    parts: [(usize, u32); MAX_PARTS_PER_ALLOC],
    /// Number of valid entries in parts array
    num_parts: u8,
    /// Whether this slot is in use
    used: bool,
}

impl CompositeAllocation {
    /// Create a new unused composite allocation slot
    const fn new_unused() -> Self {
        Self {
            base_addr: 0,
            total_pages: 0,
            parts: [(0, 0); MAX_PARTS_PER_ALLOC],
            num_parts: 0,
            used: false,
        }
    }
}

/// Page allocator with composite allocation support.
///
/// This allocator extends the buddy system to handle arbitrary-sized allocations
/// by combining multiple buddy blocks when a single contiguous block of the
/// requested size is not available.
///
/// # Why Not Directly Modifying BuddyPageAllocator?
///
/// - **Separation of concerns**: Buddy allocator should focus on the core buddy algorithm
/// - **Maintainability**: Composite allocation logic is independent and easier to test
/// - **Flexibility**: Can replace buddy allocator with other implementations
/// - **Layering**: Allows adding VM layer or other optimizations later
///
/// # Allocation Strategy
///
/// 1. For power-of-2 sized requests: Try standard buddy allocation first
/// 2. For non-power-of-2 or failed allocations: Decompose into power-of-2 blocks
/// 3. Greedy algorithm: Use largest possible blocks first to minimize fragmentation
pub struct CompositePageAllocator {
    /// Underlying buddy allocator for standard allocations
    buddy: BuddyPageAllocator,
    /// Static array tracking composite allocations (no dynamic allocation)
    composite_allocs: [CompositeAllocation; MAX_COMPOSITE_ALLOCS],
}

impl CompositePageAllocator {
    /// Create a new composite page allocator
    pub const fn new() -> Self {
        Self {
            buddy: BuddyPageAllocator::new(),
            composite_allocs: [CompositeAllocation::new_unused(); MAX_COMPOSITE_ALLOCS],
        }
    }

    /// Initialize the composite allocator
    ///
    /// Initializes the buddy allocator and resets composite allocation tracking.
    /// The composite allocation slots are already initialized via default().
    fn init_composite(&mut self) {
        // Reset all composite allocation slots
        for slot in &mut self.composite_allocs {
            *slot = CompositeAllocation::new_unused();
        }
        debug!("CompositePageAllocator initialized");
    }

    /// Find a free slot in the composite allocation array
    fn find_free_slot(&self) -> Option<usize> {
        for i in 0..MAX_COMPOSITE_ALLOCS {
            if !self.composite_allocs[i].used {
                return Some(i);
            }
        }
        warn!("No free composite allocation slots available");
        None
    }

    /// Find a composite allocation slot by base address
    fn find_slot_by_addr(&self, base_addr: usize) -> Option<usize> {
        for i in 0..MAX_COMPOSITE_ALLOCS {
            if self.composite_allocs[i].used && self.composite_allocs[i].base_addr == base_addr {
                return Some(i);
            }
        }
        None
    }

    /// Find optimal buddy block orders to satisfy a page allocation request.
    ///
    /// Uses a greedy algorithm to find the largest possible blocks first,
    /// minimizing the number of parts and potential fragmentation.
    ///
    /// # Arguments
    /// * `num_pages` - Number of pages requested
    ///
    /// # Returns
    /// Array of buddy orders and the count of parts
    fn find_best_orders(&self, num_pages: usize) -> Result<([u32; MAX_PARTS_PER_ALLOC], usize), AllocError> {
        let mut orders = [0u32; MAX_PARTS_PER_ALLOC];
        let mut remaining = num_pages;
        let mut count = 0;

        // Greedy algorithm: try largest blocks first (from order 18 down to 0)
        // Order 18 = 512MB, Order 0 = 4KB
        for order in (0..=18).rev() {
            let block_pages = 1usize << order;
            
            while remaining >= block_pages && count < MAX_PARTS_PER_ALLOC {
                orders[count] = order as u32;
                remaining -= block_pages;
                count += 1;

                if remaining == 0 {
                    break;
                }
            }

            if remaining == 0 {
                break;
            }
        }

        if remaining > 0 {
            return Err(AllocError::NoMemory);
        }

        debug!("Decomposed {} pages into {} buddy blocks: {:?}", num_pages, count, &orders[..count]);
        Ok((orders, count))
    }

    /// Allocate using composite strategy.
    ///
    /// When standard buddy allocation fails, this method decomposes the request
    /// into multiple power-of-2 blocks and allocates them individually.
    ///
    /// # Algorithm
    /// 1. Find optimal buddy block orders using greedy decomposition
    /// 2. Allocate each block using buddy allocator
    /// 3. If any allocation fails, rollback all previously allocated blocks
    /// 4. Record the composite allocation for later deallocation
    ///
    /// # Arguments
    /// * `num_pages` - Number of pages requested
    /// * `align_pow2` - Alignment requirement (power of 2)
    ///
    /// # Returns
    /// Base address of the first allocated block on success
    fn alloc_composite(&mut self, num_pages: usize, align_pow2: usize) -> AllocResult<usize> {
        let (orders, num_parts) = self.find_best_orders(num_pages)?;

        // Find a free slot for tracking
        let slot_idx = self.find_free_slot()
            .ok_or(AllocError::NoMemory)?;

        let mut parts = [(0usize, 0u32); MAX_PARTS_PER_ALLOC];
        let mut base_addr = None;
        let mut allocated_count = 0;

        // Allocate all constituent blocks
        for i in 0..num_parts {
            let order = orders[i];
            let block_pages = 1usize << order;
            
            // Calculate alignment requirement for this block
            // The block must be aligned to its size (2^order * PAGE_SIZE)
            let block_align = block_pages * PAGE_SIZE;
            let required_align = align_pow2.max(block_align);
            let align_order = required_align.ilog2() as usize;

            // Allocate using buddy allocator
            let addr = self.buddy.alloc_pages(block_pages, align_order)
                .map_err(|e| {
                    // Allocation failed, rollback all previously allocated blocks
                    warn!("Composite allocation failed at part {}, rolling back", i);
                    for j in 0..allocated_count {
                        let (dealloc_addr, dealloc_order) = parts[j];
                        let dealloc_pages = 1usize << dealloc_order;
                        debug!("Rollback: deallocating addr={:#x}, pages={}", dealloc_addr, dealloc_pages);
                        self.buddy.dealloc_pages(dealloc_addr, dealloc_pages);
                    }
                    e
                })?;

            parts[allocated_count] = (addr, order);
            
            if base_addr.is_none() {
                base_addr = Some(addr);
            }

            debug!("Composite part {}: addr={:#x}, order={}, pages={}", 
                    allocated_count, addr, order, block_pages);
            
            allocated_count += 1;
        }

        // Save composite allocation metadata
        self.composite_allocs[slot_idx] = CompositeAllocation {
            base_addr: base_addr.unwrap(),
            total_pages: num_pages,
            parts,
            num_parts: allocated_count as u8,
            used: true,
        };

        info!("Composite allocation succeeded: base_addr={:#x}, pages={}, parts={}", 
              base_addr.unwrap(), num_pages, allocated_count);

        Ok(base_addr.unwrap())
    }

    /// Deallocate a composite allocation.
    ///
    /// Frees all constituent buddy blocks of a composite allocation.
    ///
    /// # Arguments
    /// * `base_addr` - Base address of the composite allocation
    ///
    /// # Returns
    /// Number of pages freed, or None if not found
    fn dealloc_composite(&mut self, base_addr: usize) -> Option<usize> {
        let slot_idx = self.find_slot_by_addr(base_addr)?;

        let comp = self.composite_allocs[slot_idx];
        debug!("Deallocating composite allocation at {:#x}, parts={}", 
                base_addr, comp.num_parts);

        // Free all constituent blocks
        for i in 0..comp.num_parts as usize {
            let (addr, order) = comp.parts[i];
            let pages = 1usize << order;
            
            debug!("  Freeing part {}: addr={:#x}, order={}, pages={}", 
                    i, addr, order, pages);
            self.buddy.dealloc_pages(addr, pages);
        }

        // Mark slot as free
        self.composite_allocs[slot_idx].used = false;

        info!("Composite deallocation completed: freed {} pages at {:#x}", 
              comp.total_pages, base_addr);

        Some(comp.total_pages)
    }

    /// Check if an address corresponds to a composite allocation.
    fn is_composite_allocation(&self, base_addr: usize) -> bool {
        self.find_slot_by_addr(base_addr).is_some()
    }

    /// Get statistics about composite allocations.
    pub fn get_composite_stats(&self) -> CompositeStats {
        let mut count = 0;
        let mut total_pages = 0;
        let mut total_parts = 0;

        for slot in &self.composite_allocs {
            if slot.used {
                count += 1;
                total_pages += slot.total_pages;
                total_parts += slot.num_parts as usize;
            }
        }

        CompositeStats {
            active_allocations: count,
            total_pages_in_composite: total_pages,
            total_parts: total_parts,
            max_slots: MAX_COMPOSITE_ALLOCS,
            used_slots: count,
        }
    }
}

/// Statistics for composite allocations
#[derive(Debug, Clone, Copy)]
pub struct CompositeStats {
    /// Number of active composite allocations
    pub active_allocations: usize,
    /// Total pages allocated via composite method
    pub total_pages_in_composite: usize,
    /// Total number of buddy blocks used in all composite allocations
    pub total_parts: usize,
    /// Maximum number of composite allocations that can be tracked
    pub max_slots: usize,
    /// Number of slots currently in use
    pub used_slots: usize,
}

impl PageAllocator for CompositePageAllocator {
    const PAGE_SIZE: usize = PAGE_SIZE;

    /// Allocate contiguous memory pages.
    ///
    /// # Strategy
    /// 1. First try standard buddy allocation (fast path)
    /// 2. If that fails, fall back to composite allocation (slow path)
    ///
    /// This ensures that:
    /// - Power-of-2 allocations use efficient buddy system
    /// - Non-power-of-2 allocations can still succeed if memory is available
    /// - No fragmentation is introduced unnecessarily
    fn alloc_pages(&mut self, num_pages: usize, align_pow2: usize) -> AllocResult<usize> {
        // Fast path: try standard buddy allocation first
        match self.buddy.alloc_pages(num_pages, align_pow2) {
            Ok(addr) => {
                debug!("Standard buddy allocation: addr={:#x}, pages={}", addr, num_pages);
                Ok(addr)
            }
            Err(_) => {
                // Slow path: composite allocation
                debug!("Standard allocation failed, trying composite allocation for {} pages", num_pages);
                self.alloc_composite(num_pages, align_pow2)
            }
        }
    }

    /// Deallocate memory pages.
    ///
    /// Automatically detects whether the allocation was standard or composite
    /// and calls the appropriate deallocation method.
    fn dealloc_pages(&mut self, pos: usize, num_pages: usize) {
        if self.is_composite_allocation(pos) {
            debug!("Deallocating composite allocation at {:#x}", pos);
            self.dealloc_composite(pos);
        } else {
            debug!("Deallocating standard allocation at {:#x}", pos);
            self.buddy.dealloc_pages(pos, num_pages);
        }
    }

    /// Allocate contiguous memory pages at a specific address.
    ///
    /// Currently delegates to buddy allocator as composite allocation
    /// doesn't support fixed-address allocation.
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
    pub fn get_buddy_stats(&self) -> crate::buddy_page_allocator::BuddyStats {
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
        self.init_composite();
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
    fn test_composite_allocator_basic() {
        let mut allocator = CompositePageAllocator::new();
        allocator.init(0x80000000, 0x10000000); // 256MB

        // Test standard allocation (power of 2)
        let addr1 = allocator.alloc_pages(1024, PAGE_SIZE).unwrap();
        assert!(allocator.is_composite_allocation(addr1) == false);

        allocator.dealloc_pages(addr1, 1024);
    }

    #[test]
    fn test_composite_allocation_decomposition() {
        let allocator = CompositePageAllocator::new();
        
        // Test 1536 pages (6MB) = 1024 + 512
        let (orders, count) = allocator.find_best_orders(1536).unwrap();
        assert_eq!(count, 2);
        // Should use order 10 (1024) and order 9 (512)
        assert!(orders[0] >= orders[1]); // Largest first
    }

    #[test]
    fn test_composite_stats() {
        let mut allocator = CompositePageAllocator::new();
        allocator.init(0x80000000, 0x10000000);

        let stats = allocator.get_composite_stats();
        assert_eq!(stats.active_allocations, 0);
        assert_eq!(stats.max_slots, MAX_COMPOSITE_ALLOCS);
    }
}
