//! Global allocator implementation for Axvisor.
//! 
//! This module implements a global allocator that coordinates between
//! buddy page allocator and slab byte allocator for optimal performance.

extern crate alloc;

use core::alloc::Layout;
use core::ptr::NonNull;
use core::sync::atomic::{AtomicBool, Ordering};
use crate::{AllocError, AllocResult, BaseAllocator, PageAllocator, ByteAllocator};

use super::buddy::BuddyStats;
use super::page_allocator::CompositePageAllocator;
use super::slab_byte_allocator::{SlabByteAllocator, PageAllocatorForSlab};
use super::tracking::{track_allocation, track_deallocation, AllocationTag};
use kspin::SpinNoIrq;
use log::info;


const PAGE_SIZE: usize = 0x1000;
const MIN_HEAP_SIZE: usize = 0x8000; // 32KB minimum heap

/// Memory usage statistics
#[derive(Debug, Clone, Copy)]
pub struct UsageStats {
    pub total_pages: usize,
    pub used_pages: usize,
    pub free_pages: usize,
    pub slab_bytes: usize,
    pub heap_bytes: usize,
}

impl Default for UsageStats {
    fn default() -> Self {
        Self {
            total_pages: 0,
            used_pages: 0,
            free_pages: 0,
            slab_bytes: 0,
            heap_bytes: 0,
        }
    }
}



/// Global allocator that coordinates composite and slab allocators
pub struct GlobalAllocator {
    page_allocator: SpinNoIrq<CompositePageAllocator>,
    slab_allocator: SpinNoIrq<SlabByteAllocator>,
    stats: SpinNoIrq<UsageStats>,
    initialized: AtomicBool,
}

impl GlobalAllocator {
    pub const fn new() -> Self {
        Self {
            page_allocator: SpinNoIrq::new(CompositePageAllocator::new()),
            slab_allocator: SpinNoIrq::new(SlabByteAllocator::new()),
            stats: SpinNoIrq::new(UsageStats {
                total_pages: 0,
                used_pages: 0,
                free_pages: 0,
                heap_bytes: 0,
                slab_bytes: 0,
            }),
            initialized: AtomicBool::new(false),
        }
    }

    /// Initialize allocator with given memory region
    pub fn init(&self, start_vaddr: usize, size: usize) -> AllocResult<()> {

        if size <= MIN_HEAP_SIZE {
            return Err(AllocError::InvalidParam);
        }
        info!("global allocator: Initialize with region [{:#x}, {:#x})", start_vaddr, start_vaddr + size);
        // Initialize composite allocator first
        self.page_allocator.lock().init(start_vaddr, size);
        info!("global allocator: Composite page allocator initialized");

        // Set up page allocator for slab
        {
            let page_alloc_ptr = &mut *self.page_allocator.lock() as *mut CompositePageAllocator;
            self.slab_allocator.lock().set_page_allocator(
                page_alloc_ptr as *mut dyn PageAllocatorForSlab
            );
        }
        info!("global allocator: Slab allocator initialized");

        // Update statistics
        {
            let page_alloc = self.page_allocator.lock();
            let mut stats = self.stats.lock();
            stats.total_pages = page_alloc.total_pages();
            stats.used_pages = page_alloc.used_pages();
            stats.free_pages = page_alloc.available_pages();
        }

        self.initialized.store(true, Ordering::SeqCst);
        info!("global allocator: Initialized");
        Ok(())
    }

    /// Dynamically add memory region to allocator
    pub fn add_memory(&self, start_vaddr: usize, size: usize) -> AllocResult<()> {
        self.page_allocator.lock().add_memory(start_vaddr, size)?;

        // Update statistics
        {
            let page_alloc = self.page_allocator.lock();
            let mut stats = self.stats.lock();
            stats.total_pages = page_alloc.total_pages();
            stats.free_pages = page_alloc.available_pages();
        }

        Ok(())
    }

    /// Smart allocation based on size
    pub fn alloc(&self, layout: Layout) -> AllocResult<NonNull<u8>> {
        if !self.initialized.load(Ordering::SeqCst) {
            return Err(AllocError::NoMemory);
        }
        
        if layout.size() <= 2048 {
            // Use slab allocator for small objects
            match self.slab_allocator.lock().alloc(layout) {
                Ok(ptr) => {
                    self.stats.lock().slab_bytes += layout.size();
                    track_allocation(ptr, layout, AllocationTag::Slab);
                    return Ok(ptr);
                }
                Err(_) => {
                    // Fall back to buddy allocator
                }
            }
        }

        // Use buddy allocator for large objects
        let pages_needed = (layout.size() + PAGE_SIZE - 1) / PAGE_SIZE;
        // info!("global allocator: Allocating {} bytes with alignment {}", layout.size(), layout.align());

        // Print memory state before allocation for large allocations
        if layout.size() > 1024 * 1024 { // > 1MB
            let stats_before = self.get_stats();
            info!("global allocator: Memory state before allocation:");
            info!("  Requested: {} bytes ({} MB, {} pages)",
                   layout.size(),
                   layout.size() / (1024 * 1024),
                   pages_needed);
            info!("  Total pages: {} ({} MB)",
                   stats_before.total_pages,
                   (stats_before.total_pages * PAGE_SIZE) / (1024 * 1024));
            info!("  Free pages: {} ({} MB)",
                   stats_before.free_pages,
                   (stats_before.free_pages * PAGE_SIZE) / (1024 * 1024));
            info!("  Used pages: {} ({} MB)",
                   stats_before.used_pages,
                   (stats_before.used_pages * PAGE_SIZE) / (1024 * 1024));

            let buddy_stats = self.get_buddy_stats();
            info!("  Buddy free blocks by order:");
            for (order, &count) in buddy_stats.free_pages_by_order.iter().enumerate() {
                if count > 0 {
                    let size_mb = ((1 << order) * PAGE_SIZE) / (1024 * 1024);
                    info!("    Order {}: {} blocks ({} MB each, {} MB total)",
                           order, count, size_mb, size_mb * count);
                }
            }
        }

        let addr = PageAllocator::alloc_pages(&mut *self.page_allocator.lock(), pages_needed, layout.align())?;
        let ptr = unsafe { NonNull::new_unchecked(addr as *mut u8) };

        {
            let mut stats = self.stats.lock();
            stats.used_pages += pages_needed;
            stats.free_pages -= pages_needed;
            stats.heap_bytes += layout.size();
        }

        // Print allocation success info for large allocations
        if layout.size() > 1024 * 1024 { // > 1MB
            info!("global allocator: Successfully allocated {} pages at {:#x} ({} MB)",
                   pages_needed, addr, (pages_needed * PAGE_SIZE) / (1024 * 1024));
        }

        track_allocation(ptr, layout, AllocationTag::Buddy);
        Ok(ptr)
    }

    /// Allocate pages
    pub fn alloc_pages(&self, num_pages: usize, align_pow2: usize) -> AllocResult<usize> {
        if !self.initialized.load(Ordering::SeqCst) {
            return Err(AllocError::NoMemory);
        }
        
        // info!("global allocator: Allocating {} pages with alignment {}", num_pages, align_pow2);
        
        let addr = PageAllocator::alloc_pages(&mut *self.page_allocator.lock(), num_pages, align_pow2)?;
        
        // Update statistics
        {
            let mut stats = self.stats.lock();
            stats.used_pages += num_pages;
            stats.free_pages -= num_pages;
        }

        Ok(addr)
    }

    /// Deallocate memory
    pub fn dealloc(&self, ptr: NonNull<u8>, layout: Layout) {
        if !self.initialized.load(Ordering::SeqCst) {
            return;
        }
        
        if layout.size() <= 2048 {
            // Try slab deallocation first
            self.slab_allocator.lock().dealloc(ptr, layout);
            {
                let mut stats = self.stats.lock();
                stats.slab_bytes = stats.slab_bytes.saturating_sub(layout.size());
            }
            track_deallocation(ptr);
            return;
        }
        // info!("global allocator: Deallocating {:?} with alignment {}", ptr, layout.align());
        // Fall back to page deallocation
        let pages_needed = (layout.size() + PAGE_SIZE - 1) / PAGE_SIZE;
        PageAllocator::dealloc_pages(&mut *self.page_allocator.lock(), ptr.as_ptr() as usize, pages_needed);
        {
            let mut stats = self.stats.lock();
            stats.used_pages = stats.used_pages.saturating_sub(pages_needed);
            stats.free_pages += pages_needed;
            stats.heap_bytes = stats.heap_bytes.saturating_sub(layout.size());
        }
        track_deallocation(ptr);
    }

    /// Deallocate pages
    pub fn dealloc_pages(&self, pos: usize, num_pages: usize) {
        if !self.initialized.load(Ordering::SeqCst) {
            return;
        }
        // info!("global allocator: Deallocating {} pages at address {:#x}", num_pages, pos);

        PageAllocator::dealloc_pages(&mut *self.page_allocator.lock(), pos, num_pages);

        // Update statistics
        {
            let mut stats = self.stats.lock();
            stats.used_pages = stats.used_pages.saturating_sub(num_pages);
            stats.free_pages += num_pages;
        }
    }


}



impl GlobalAllocator {
    /// Get memory statistics
    pub fn get_stats(&self) -> UsageStats {
        *self.stats.lock()
    }

    /// Get buddy allocator statistics
    pub fn get_buddy_stats(&self) -> BuddyStats {
        self.page_allocator.lock().get_buddy_stats()
    }

    /// Get detailed free list information as a string
    pub fn get_free_lists_info(&self) -> alloc::string::String {
        self.page_allocator.lock().get_free_lists_info()
    }
}

impl Default for GlobalAllocator {
    fn default() -> Self {
        Self::new()
    }
}

impl BaseAllocator for GlobalAllocator {
    fn init(&mut self, start: usize, size: usize) {
        self.page_allocator.lock().init(start, size);
    }

    fn add_memory(&mut self, start: usize, size: usize) -> AllocResult {
        self.page_allocator.lock().add_memory(start, size)
    }
}

impl PageAllocator for GlobalAllocator {
    const PAGE_SIZE: usize = 0x1000; // 4KB page size

    fn alloc_pages(&mut self, num_pages: usize, align_pow2: usize) -> AllocResult<usize> {
        let mut allocator = self.page_allocator.lock();
        <CompositePageAllocator as PageAllocator>::alloc_pages(&mut allocator, num_pages, align_pow2)
    }

    fn dealloc_pages(&mut self, pos: usize, num_pages: usize) {
        let mut allocator = self.page_allocator.lock();
        <CompositePageAllocator as PageAllocator>::dealloc_pages(&mut allocator, pos, num_pages)
    }

    fn alloc_pages_at(
        &mut self,
        base: usize,
        num_pages: usize,
        align_pow2: usize,
    ) -> AllocResult<usize> {
        let mut allocator = self.page_allocator.lock();
        <CompositePageAllocator as PageAllocator>::alloc_pages_at(&mut allocator, base, num_pages, align_pow2)
    }

    fn total_pages(&self) -> usize {
        self.page_allocator.lock().total_pages()
    }

    fn used_pages(&self) -> usize {
        self.page_allocator.lock().used_pages()
    }

    fn available_pages(&self) -> usize {
        self.page_allocator.lock().available_pages()
    }
}



unsafe impl core::alloc::GlobalAlloc for GlobalAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        if !self.initialized.load(Ordering::SeqCst) {
            return core::ptr::null_mut();
        }

        if layout.size() <= 2048 {
              // Use slab allocator for small objects
              match self.slab_allocator.lock().alloc(layout) {
                  Ok(ptr) => {
                      {
                          let mut stats = self.stats.lock();
                          stats.slab_bytes += layout.size();
                      }
                      return ptr.as_ptr();
                  }
                  Err(_) => {
                      // Fall back to buddy allocator
                  }
              }
          }

        // Use page allocator for large objects
        let pages_needed = (layout.size() + PAGE_SIZE - 1) / PAGE_SIZE;
        match PageAllocator::alloc_pages(&mut *self.page_allocator.lock(), pages_needed, layout.align()) {
            Ok(addr) => {
                let mut stats = self.stats.lock();
                stats.used_pages += pages_needed;
                stats.free_pages -= pages_needed;
                stats.heap_bytes += layout.size();
                addr as *mut u8
            }
            Err(_) => core::ptr::null_mut(),
        }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        if let Some(ptr) = NonNull::new(ptr) {
            self.dealloc(ptr, layout);
        }
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        if new_size == 0 {
            if let Some(ptr) = NonNull::new(ptr) {
                self.dealloc(ptr, layout);
            }
            return core::ptr::null_mut();
        }

        if ptr.is_null() {
            let new_layout = Layout::from_size_align(new_size, layout.align())
                .unwrap_or_else(|_| Layout::new::<u8>());
            return unsafe { core::alloc::GlobalAlloc::alloc(self, new_layout) };
        }

        let new_layout = Layout::from_size_align(new_size, layout.align())
            .unwrap_or_else(|_| Layout::new::<u8>());

        // If new size fits in old allocation, return old pointer
        if new_size <= layout.size() {
            return ptr;
        }

        // Allocate new memory and copy
        let new_ptr = unsafe { core::alloc::GlobalAlloc::alloc(self, new_layout) };
        if !new_ptr.is_null() {
            core::ptr::copy_nonoverlapping(
                ptr,
                new_ptr,
                core::cmp::min(layout.size(), new_size),
            );
            if let Some(ptr) = NonNull::new(ptr) {
                self.dealloc(ptr, layout);
            }
            new_ptr
        } else {
            core::ptr::null_mut()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_global_allocator_basic() {
        let allocator = GlobalAllocator::new();
        
        // Test initialization
        let base_addr = 0x80000000;
        let size = 0x1000000; // 16MB
        assert!(allocator.init(base_addr, size).is_ok());

        // Test allocation
        let layout = Layout::from_size_align(64, 8).unwrap();
        match allocator.alloc(layout) {
            Ok(ptr) => {
                allocator.dealloc(ptr, layout);
            }
            Err(_) => panic!("Allocation failed"),
        }

        // Test page allocation
        match allocator.alloc_pages(1, PAGE_SIZE) {
            Ok(page_addr) => {
                allocator.dealloc_pages(page_addr, 1);
            }
            Err(_) => panic!("Page allocation failed"),
        }
    }

    #[test]
    fn test_stats() {
        let allocator = GlobalAllocator::new();
        
        let base_addr = 0x80000000;
        let size = 0x1000000; // 16MB
        assert!(allocator.init(base_addr, size).is_ok());

        let stats = allocator.get_stats();
        assert!(stats.total_pages > 0);
        assert!(stats.free_pages > 0);
    }
}
