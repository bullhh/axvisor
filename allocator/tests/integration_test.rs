//! Integration tests for the Axvisor memory allocator

use axvisor_allocator::{GlobalAllocator, AllocResult, BaseAllocator};
use core::alloc::Layout;

#[test]
fn test_basic_allocation() {
    let mut allocator = GlobalAllocator::new();
    
    // Initialize with 16MB memory
    let base_addr = 0x80000000;
    let size = 0x1000000; // 16MB
    
    assert!(allocator.init(base_addr, size).is_ok());
    
    // Test small allocation
    let layout = Layout::from_size_align(64, 8).unwrap();
    match allocator.alloc(layout) {
        Ok(ptr) => {
            println!("Small allocation successful at {:p}", ptr.as_ptr());
            allocator.dealloc(ptr, layout);
        }
        Err(e) => {
            println!("Small allocation failed: {:?}", e);
        }
    }
    
    // Test page allocation
    match allocator.alloc_pages(1, 0x1000) {
        Ok(page_addr) => {
            println!("Page allocation successful at 0x{:x}", page_addr);
            allocator.dealloc_pages(page_addr, 1);
        }
        Err(e) => {
            println!("Page allocation failed: {:?}", e);
        }
    }
    
    // Test statistics
    let stats = allocator.get_stats();
    println!("Memory stats: {:?}", stats);
    
    let buddy_stats = allocator.get_buddy_stats();
    println!("Buddy stats: {:?}", buddy_stats);
}

#[test]
fn test_multiple_allocations() {
    let mut allocator = GlobalAllocator::new();
    
    // Initialize with 16MB memory
    let base_addr = 0x80000000;
    let size = 0x1000000; // 16MB
    assert!(allocator.init(base_addr, size).is_ok());
    
    let mut allocations = Vec::new();
    
    // Test multiple small allocations
    for i in 0..10 {
        let layout = Layout::from_size_align(64 + i * 8, 8).unwrap();
        match allocator.alloc(layout) {
            Ok(ptr) => {
                allocations.push((ptr, layout));
                println!("Allocation {} successful at {:p}", i, ptr.as_ptr());
            }
            Err(e) => {
                println!("Allocation {} failed: {:?}", i, e);
                break;
            }
        }
    }
    
    // Deallocate all
    for (ptr, layout) in allocations {
        allocator.dealloc(ptr, layout);
    }
    
    let final_stats = allocator.get_stats();
    println!("Final stats: {:?}", final_stats);
}

#[test]
fn test_page_allocations() {
    let mut allocator = GlobalAllocator::new();
    
    // Initialize with 16MB memory
    let base_addr = 0x80000000;
    let size = 0x1000000; // 16MB
    assert!(allocator.init(base_addr, size).is_ok());
    
    let mut page_allocations = Vec::new();
    
    // Test multiple page allocations
    for i in 0..5 {
        match allocator.alloc_pages(1, 0x1000) {
            Ok(page_addr) => {
                page_allocations.push(page_addr);
                println!("Page allocation {} successful at 0x{:x}", i, page_addr);
            }
            Err(e) => {
                println!("Page allocation {} failed: {:?}", i, e);
                break;
            }
        }
    }
    
    // Deallocate all pages
    for page_addr in page_allocations {
        allocator.dealloc_pages(page_addr, 1);
    }
    
    let final_stats = allocator.get_stats();
    println!("Final page stats: {:?}", final_stats);
}

#[test]
fn test_memory_addition() {
    let mut allocator = GlobalAllocator::new();
    
    // Initialize with small memory first
    let base_addr = 0x80000000;
    let size = 0x100000; // 1MB
    assert!(allocator.init(base_addr, size).is_ok());
    
    let initial_stats = allocator.get_stats();
    println!("Initial stats: {:?}", initial_stats);
    
    // Add more memory
    let new_base = base_addr + size;
    let new_size = 0x200000; // 2MB
    assert!(allocator.add_memory(new_base, new_size).is_ok());
    
    let updated_stats = allocator.get_stats();
    println!("Updated stats: {:?}", updated_stats);
    
    // Should have more total memory now
    assert!(updated_stats.total_pages > initial_stats.total_pages);
}
