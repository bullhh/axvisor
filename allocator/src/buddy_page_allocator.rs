//! Buddy page allocator implementation for Axvisor.
//! 
//! This module implements a simplified buddy system for page-level allocation
//! with support for memory regions and statistics.

extern crate alloc;

use crate::{AllocError, AllocResult, BaseAllocator, PageAllocator};
use super::slab_byte_allocator::PageAllocatorForSlab;

const PAGE_SIZE: usize = 0x1000;
pub const DEFAULT_MAX_ORDER: usize = 28; // Support up to 256GB allocations (2^28 * 4KB)

/// Buddy system statistics
#[derive(Debug, Clone, Copy)]
pub struct BuddyStats {
    pub total_pages: usize,
    pub free_pages: usize,
    pub used_pages: usize,
    pub free_pages_by_order: [usize; DEFAULT_MAX_ORDER + 1],
}

impl Default for BuddyStats {
    fn default() -> Self {
        Self {
            total_pages: 0,
            free_pages: 0,
            used_pages: 0,
            free_pages_by_order: [0; DEFAULT_MAX_ORDER + 1],
        }
    }
}

impl BuddyStats {
    pub const fn new() -> Self {
        Self {
            total_pages: 0,
            free_pages: 0,
            used_pages: 0,
            free_pages_by_order: [0; DEFAULT_MAX_ORDER + 1],
        }
    }
}

/// A memory region descriptor
#[derive(Debug, Clone)]
pub struct MemoryRegion {
    pub start_addr: usize,
    pub size: usize,
}

/// Buddy block metadata
#[derive(Debug, Clone, Copy)]
struct BuddyBlock {
    order: usize,
    addr: usize,
}

impl BuddyBlock {
    fn buddy_addr(&self, base_addr: usize) -> usize {
        self.addr ^ ((1 << self.order) * PAGE_SIZE)
    }

    fn is_first(&self, base_addr: usize) -> bool {
        ((self.addr - base_addr) / ((1 << self.order) * PAGE_SIZE)) % 2 == 0
    }
}

/// Buddy set implementation
#[derive(Debug)]
pub struct BuddySet {
    base_addr: usize,
    total_pages: usize,
    free_lists: [alloc::collections::LinkedList<BuddyBlock>; DEFAULT_MAX_ORDER + 1],
}

impl BuddySet {
    /// Create a new buddy set
    pub fn new(base_addr: usize, total_pages: usize) -> Self {
        let mut free_lists = [const { alloc::collections::LinkedList::new() }; DEFAULT_MAX_ORDER + 1];
        Self {
            base_addr,
            total_pages,
            free_lists,
        }
    }

    /// Create an empty buddy set
    pub const fn empty() -> Self {
        const EMPTY_LIST: alloc::collections::LinkedList<BuddyBlock> = alloc::collections::LinkedList::new();
        Self {
            base_addr: 0,
            total_pages: 0,
            free_lists: [EMPTY_LIST; DEFAULT_MAX_ORDER + 1],
        }
    }

    pub const fn max_order(&self) -> usize {
        DEFAULT_MAX_ORDER
    }

    pub fn init(&mut self, base_addr: usize, size: usize) {
        self.base_addr = base_addr;
        self.total_pages = size / PAGE_SIZE;
        
        // Simple initialization: add all pages as order 0
        // In a real implementation, we'd do proper merging
        for i in 0..self.total_pages {
            let addr = base_addr + i * PAGE_SIZE;
            self.free_lists[0].push_back(BuddyBlock { order: 0, addr });
        }
    }

    pub fn alloc_pages(&mut self, num_pages: usize, align_pow2: usize) -> AllocResult<usize> {
        self.alloc_pages_with_usage(num_pages, align_pow2, crate::global_allocator::UsageKind::Other)
    }

    pub fn alloc_pages_with_usage(&mut self, num_pages: usize, _align_pow2: usize, _usage: crate::global_allocator::UsageKind) -> AllocResult<usize> {
        if num_pages == 0 {
            return Err(AllocError::InvalidParam);
        }
        
        // Find the required order
        let required_order = if num_pages.is_power_of_two() {
            num_pages.trailing_zeros() as usize
        } else {
            num_pages.next_power_of_two().trailing_zeros() as usize
        }.min(DEFAULT_MAX_ORDER);
        
        // Try to find a block of the required order or higher
        for order in required_order..=DEFAULT_MAX_ORDER {
            if !self.free_lists[order].is_empty() {
                let mut block = self.free_lists[order].pop_front().unwrap();
                
                // Split down to required order
                while block.order > required_order {
                    block.order -= 1;
                    let buddy_addr = block.addr + (1 << block.order) * PAGE_SIZE;
                    self.free_lists[block.order].push_back(BuddyBlock {
                        order: block.order,
                        addr: buddy_addr,
                    });
                }
                
                return Ok(block.addr);
            }
        }
        
        Err(AllocError::NoMemory)
    }

    pub fn dealloc_pages(&mut self, addr: usize, num_pages: usize) {
        if num_pages == 0 {
            return;
        }
        
        let order = if num_pages.is_power_of_two() {
            num_pages.trailing_zeros() as usize
        } else {
            num_pages.next_power_of_two().trailing_zeros() as usize
        }.min(DEFAULT_MAX_ORDER);
        
        let block = BuddyBlock { order, addr };
        
        // Simple deallocation: just add back to the free list
        // In a real implementation, we'd try to merge with buddies
        self.free_lists[order].push_back(block);
    }

    pub fn dealloc_pages_with_usage(&mut self, addr: usize, num_pages: usize, _usage: crate::global_allocator::UsageKind) {
        self.dealloc_pages(addr, num_pages);
    }

    pub fn get_stats(&self) -> BuddyStats {
        let mut stats = BuddyStats::new();
        stats.total_pages = self.total_pages;
        
        for (order, list) in self.free_lists.iter().enumerate() {
            let pages_in_order = list.len() * (1 << order);
            stats.free_pages_by_order[order] = list.len();
            stats.free_pages += pages_in_order;
        }
        
        stats.used_pages = stats.total_pages.saturating_sub(stats.free_pages);
        stats
    }
}

impl Default for BuddySet {
    fn default() -> Self {
        Self::empty()
    }
}

/// Buddy page allocator
pub struct BuddyPageAllocator {
    global_pool: BuddySet,
    stats: BuddyStats,
}

impl BuddyPageAllocator {
    pub const fn new() -> Self {
        Self {
            global_pool: BuddySet::empty(),
            stats: BuddyStats::new(),
        }
    }

    pub fn bootstrap(&mut self, base_addr: usize, size: usize) {
        self.global_pool.init(base_addr, size);
        self.stats = self.global_pool.get_stats();
    }

    pub fn get_stats(&self) -> BuddyStats {
        self.global_pool.get_stats()
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

    fn add_memory(&mut self, start: usize, size: usize) -> AllocResult {
        // For simplicity, we'll just initialize with the new region
        self.global_pool.init(start, size);
        self.stats = self.global_pool.get_stats();
        Ok(())
    }
}

impl PageAllocator for BuddyPageAllocator {
    const PAGE_SIZE: usize = PAGE_SIZE;

    fn alloc_pages(&mut self, num_pages: usize, align_pow2: usize) -> AllocResult<usize> {
        let addr = self.global_pool.alloc_pages(num_pages, align_pow2)?;
        self.stats = self.global_pool.get_stats();
        Ok(addr)
    }

    fn dealloc_pages(&mut self, pos: usize, num_pages: usize) {
        self.global_pool.dealloc_pages(pos, num_pages);
        self.stats = self.global_pool.get_stats();
    }

    fn alloc_pages_with_usage(&mut self, num_pages: usize, align_pow2: usize, usage: crate::global_allocator::UsageKind) -> AllocResult<usize> {
        let addr = self.global_pool.alloc_pages_with_usage(num_pages, align_pow2, usage)?;
        self.stats = self.global_pool.get_stats();
        Ok(addr)
    }

    fn dealloc_pages_with_usage(&mut self, pos: usize, num_pages: usize, usage: crate::global_allocator::UsageKind) {
        self.global_pool.dealloc_pages_with_usage(pos, num_pages, usage);
        self.stats = self.global_pool.get_stats();
    }

    fn alloc_pages_at(
        &mut self,
        base: usize,
        num_pages: usize,
        align_pow2: usize,
    ) -> AllocResult<usize> {
        // For simplicity, we'll try to allocate and check if it matches base
        let addr = PageAllocator::alloc_pages(self, num_pages, align_pow2)?;
        if addr != base {
            // If not the exact address, deallocate and return error
            PageAllocator::dealloc_pages(self, addr, num_pages);
            Err(AllocError::InvalidParam)
        } else {
            Ok(addr)
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

impl PageAllocatorForSlab for BuddyPageAllocator {
    fn alloc_pages(&mut self, num_pages: usize, align_pow2: usize) -> AllocResult<usize> {
        self.global_pool.alloc_pages(num_pages, align_pow2)
    }

    fn dealloc_pages(&mut self, pos: usize, num_pages: usize) {
        self.global_pool.dealloc_pages(pos, num_pages)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_buddy_allocator_basic() {
        let mut allocator = BuddyPageAllocator::new();
        
        // Test initialization
        let base_addr = 0x80000000;
        let size = 0x100000; // 1MB
        allocator.init(base_addr, size);

        // Test allocation
        match PageAllocator::alloc_pages(&mut allocator, 1, PAGE_SIZE) {
            Ok(page_addr) => {
                PageAllocator::dealloc_pages(&mut allocator, page_addr, 1);
            }
            Err(_) => panic!("Page allocation failed"),
        }

        let stats = allocator.get_stats();
        assert!(stats.total_pages > 0);
    }
}
