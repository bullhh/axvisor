//! Integration tests for the improved Axvisor memory allocator.
//! 
//! This test suite validates the functionality of the buddy allocator,
//! slab allocator, and tracking system working together.

#![no_std]

extern crate alloc;

use axvisor_allocator::{
    BuddyPageAllocator, SlabByteAllocator, GlobalAllocator,
    PageAllocator, ByteAllocator, BaseAllocator, PageAllocatorForSlab,
    enable_tracking, disable_tracking,
};
use core::alloc::Layout;

#[test]
fn test_buddy_allocator_basic() {
    let mut allocator = BuddyPageAllocator::new();
    
    // Initialize with 1MB memory
    let base_addr = 0x80000000;
    let size = 0x100000; // 1MB = 256 pages
    allocator.init(base_addr, size);
    
    // Test basic allocation
    let page1 = PageAllocator::alloc_pages(&mut allocator, 1, 12).unwrap();
    assert_eq!(page1, base_addr);
    
    let page2 = PageAllocator::alloc_pages(&mut allocator, 1, 12).unwrap();
    assert_eq!(page2, base_addr + 0x1000);
    
    // Test deallocation
    PageAllocator::dealloc_pages(&mut allocator, page1, 1);
    PageAllocator::dealloc_pages(&mut allocator, page2, 1);
    
    // After deallocation, we should be able to allocate again
    let page3 = PageAllocator::alloc_pages(&mut allocator, 2, 12).unwrap();
    assert_eq!(page3, base_addr); // Should merge back
    
    let stats = allocator.get_stats();
    assert_eq!(stats.used_pages, 2);
}

#[test]
fn test_buddy_merge_functionality() {
    let mut allocator = BuddyPageAllocator::new();
    
    let base_addr = 0x80000000;
    let size = 0x10000; // 64KB = 16 pages
    allocator.init(base_addr, size);
    
    // Allocate two adjacent single pages
    let addr1 = PageAllocator::alloc_pages(&mut allocator, 1, 12).unwrap();
    let addr2 = PageAllocator::alloc_pages(&mut allocator, 1, 12).unwrap();
    
    // Verify they are adjacent
    assert_eq!(addr2, addr1 + 0x1000);
    
    // Deallocate both - should merge
    PageAllocator::dealloc_pages(&mut allocator, addr1, 1);
    PageAllocator::dealloc_pages(&mut allocator, addr2, 1);
    
    // Now we should be able to allocate a 2-page block
    let merged_addr = PageAllocator::alloc_pages(&mut allocator, 2, 12).unwrap();
    assert_eq!(merged_addr, addr1); // Should get the merged block
}

#[test]
fn test_slab_size_classes() {
    let mut slab = SlabByteAllocator::new();
    let mut buddy = BuddyPageAllocator::new();
    
    // Initialize buddy allocator with enough pages
    buddy.init(0x80000000, 0x10000); // 64KB = 16 pages
    
    // Set up the slab allocator with buddy as page allocator
    slab.set_page_allocator(&mut buddy as *mut dyn PageAllocatorForSlab);
    
    // Test various size classes
    let layouts = [
        Layout::from_size_align(8, 8).unwrap(),
        Layout::from_size_align(16, 8).unwrap(),
        Layout::from_size_align(32, 8).unwrap(),
        Layout::from_size_align(64, 8).unwrap(),
        Layout::from_size_align(128, 8).unwrap(),
        Layout::from_size_align(256, 8).unwrap(),
        Layout::from_size_align(512, 8).unwrap(),
        Layout::from_size_align(1024, 8).unwrap(),
        Layout::from_size_align(2048, 8).unwrap(),
    ];
    
    for layout in layouts {
        let ptr = slab.alloc(layout).unwrap();
        slab.dealloc(ptr, layout);
    }
}

#[test]
fn test_slab_allocation_patterns() {
    let mut slab = SlabByteAllocator::new();
    let mut buddy = BuddyPageAllocator::new();
    
    // Initialize buddy allocator with enough pages
    buddy.init(0x80000000, 0x10000); // 64KB = 16 pages
    
    // Set up the slab allocator with buddy as page allocator
    slab.set_page_allocator(&mut buddy as *mut dyn PageAllocatorForSlab);
    
    let layout = Layout::from_size_align(64, 8).unwrap();
    
    // Allocate multiple objects
    let mut ptrs = alloc::vec::Vec::new();
    for _ in 0..10 {
        ptrs.push(slab.alloc(layout).unwrap());
    }
    
    // Deallocate in random order
    while let Some(ptr) = ptrs.pop() {
        slab.dealloc(ptr, layout);
    }
    
    // Should be able to allocate again
    let ptr = slab.alloc(layout).unwrap();
    slab.dealloc(ptr, layout);
}

#[test]
fn test_global_allocator_integration() {
    let mut global = GlobalAllocator::new();
    
    // Initialize with 2MB
    let base_addr = 0x80000000;
    let size = 0x200000; // 2MB
    global.init(base_addr, size).unwrap();
    
    // Test small allocations (should use slab)
    let small_layout = Layout::from_size_align(64, 8).unwrap();
    let small_ptr1 = global.alloc(small_layout).unwrap();
    let small_ptr2 = global.alloc(small_layout).unwrap();
    
    // Test large allocation (should use buddy)
    let large_layout = Layout::from_size_align(0x1000, 0x1000).unwrap();
    let large_ptr1 = global.alloc(large_layout).unwrap();
    let large_ptr2 = global.alloc(large_layout).unwrap();
    
    // Test deallocations
    global.dealloc(small_ptr1, small_layout);
    global.dealloc(small_ptr2, small_layout);
    global.dealloc(large_ptr1, large_layout);
    global.dealloc(large_ptr2, large_layout);
    
    // Check statistics
    let stats = global.get_stats();
    assert!(stats.used_pages < stats.total_pages);
}

#[test]
fn test_memory_tracking_functionality() {
    // Enable tracking
    enable_tracking();
    
    // Reset tracking state
    axvisor_allocator::reset_tracking();
    
    let mut global = GlobalAllocator::new();
    global.init(0x80000000, 0x100000).unwrap();
    
    // Track some allocations
    let layout1 = Layout::from_size_align(64, 8).unwrap();
    let ptr1 = global.alloc(layout1).unwrap();
    
    let layout2 = Layout::from_size_align(1024, 8).unwrap();
    let ptr2 = global.alloc(layout2).unwrap();
    
    // Get statistics
    let overall_stats = axvisor_allocator::get_overall_stats();
    assert!(overall_stats.total_allocations >= 2);
    assert!(overall_stats.current_memory_usage > 0);
    
    // Deallocate
    global.dealloc(ptr1, layout1);
    global.dealloc(ptr2, layout2);
    
    // Check for leaks (should be none)
    let leaks = axvisor_allocator::get_memory_leaks();
    assert_eq!(leaks.len(), 0, "Found {} memory leaks", leaks.len());
    
    // Disable tracking
    disable_tracking();
}

#[test]
fn test_allocation_failure_scenarios() {
    let mut allocator = BuddyPageAllocator::new();
    allocator.init(0x80000000, 0x1000); // Only 1 page
    
    // Try to allocate more than available
    let result = PageAllocator::alloc_pages(&mut allocator, 2, 12);
    assert!(result.is_err());
    
    // Try to allocate 0 pages
    let result = PageAllocator::alloc_pages(&mut allocator, 0, 12);
    assert!(result.is_err());
}

#[test]
fn test_memory_pressure_handling() {
    let mut global = GlobalAllocator::new();
    global.init(0x80000000, 0x10000).unwrap(); // 64KB
    
    let mut ptrs = alloc::vec::Vec::new();
    let layout = Layout::from_size_align(256, 8).unwrap();
    
    // Allocate until we run out
    loop {
        match global.alloc(layout) {
            Ok(ptr) => ptrs.push(ptr),
            Err(_) => break,
        }
    }
    
    // Deallocate everything
    while let Some(ptr) = ptrs.pop() {
        global.dealloc(ptr, layout);
    }
    
    // Should be able to allocate again
    let ptr = global.alloc(layout).unwrap();
    global.dealloc(ptr, layout);
}

#[test]
fn test_fragmentation_and_coalescing() {
    let mut allocator = BuddyPageAllocator::new();
    allocator.init(0x80000000, 0x10000); // 16 pages
    
    // Create fragmentation pattern with tracked sizes
    let mut allocations = alloc::vec::Vec::new();
    for i in 0..8 {
        if i % 2 == 0 {
            let ptr = PageAllocator::alloc_pages(&mut allocator, 1, 12).unwrap();
            allocations.push((ptr, 1));
        } else {
            let ptr = PageAllocator::alloc_pages(&mut allocator, 2, 12).unwrap();
            allocations.push((ptr, 2));
        }
    }
    
    // Deallocate everything with correct sizes
    for (ptr, size) in allocations {
        PageAllocator::dealloc_pages(&mut allocator, ptr, size);
    }
    
    // Should be able to allocate full size again
    let large_block = PageAllocator::alloc_pages(&mut allocator, 16, 12).unwrap();
    PageAllocator::dealloc_pages(&mut allocator, large_block, 16);
}

#[test]
fn test_concurrent_allocation_simulation() {
    let mut global = GlobalAllocator::new();
    global.init(0x80000000, 0x20000).unwrap(); // 128KB
    
    // Simulate concurrent access patterns
    let layouts = [
        Layout::from_size_align(32, 8).unwrap(),
        Layout::from_size_align(64, 8).unwrap(),
        Layout::from_size_align(128, 8).unwrap(),
        Layout::from_size_align(256, 8).unwrap(),
    ];
    
    let mut allocations = alloc::vec::Vec::new();
    
    // Mixed allocation pattern
    for i in 0..100 {
        let layout = layouts[i % layouts.len()];
        if let Ok(ptr) = global.alloc(layout) {
            allocations.push((ptr, layout));
        }
        
        // Randomly deallocate some
        if i % 3 == 0 && !allocations.is_empty() {
            let idx = i % allocations.len();
            let (ptr, layout) = allocations.swap_remove(idx);
            global.dealloc(ptr, layout);
        }
    }
    
    // Clean up remaining
    for (ptr, layout) in allocations {
        global.dealloc(ptr, layout);
    }
    
    let stats = global.get_stats();
    assert_eq!(stats.used_pages, 0); // All should be deallocated
}
