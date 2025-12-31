//! Test multi-zone buddy allocator
//!
//! This test demonstrates the multi-zone buddy allocator implementation
//! inspired by Linux kernel's zone-based memory management.

#![no_std]
#![no_main]

extern crate alloc;
extern crate axlog;

use axlog::info;
use axvisor_allocator::{BuddyPageAllocator, PageAllocator, BaseAllocator};

#[no_mangle]
pub extern "C" fn main() -> ! {
    axlog::init();
    info!("=== Multi-Zone Buddy Allocator Test ===\n");

    let mut allocator = BuddyPageAllocator::new();

    // Initialize zone 0 with first memory region
    info!("Initializing zone 0...");
    allocator.init(0x8000_0000, 0x0100_0000); // 16MB starting at 2GB
    info!("Zone 0 initialized\n");

    // Add zone 1 with second non-contiguous memory region
    info!("Adding zone 1...");
    if let Err(e) = allocator.add_memory(0x9000_0000, 0x0100_0000) { // 16MB starting at 2.25GB
        info!("Failed to add zone 1: {:?}\n", e);
    } else {
        info!("Zone 1 added successfully\n");
    }

    // Add zone 2 with third non-contiguous memory region
    info!("Adding zone 2...");
    if let Err(e) = allocator.add_memory(0xA000_0000, 0x0080_0000) { // 8MB starting at 2.5GB
        info!("Failed to add zone 2: {:?}\n", e);
    } else {
        info!("Zone 2 added successfully\n");
    }

    // Print allocator statistics
    info!("\n=== Allocator Statistics ===");
    let stats = allocator.get_stats();
    info!("Total pages: {}", stats.total_pages);
    info!("Free pages: {}", stats.free_pages);
    info!("Used pages: {}", stats.used_pages);

    // Print detailed free lists info
    info!("\n{}", allocator.get_free_lists_info());

    // Allocate some pages from different zones
    info!("\n=== Testing Allocation ===");
    info!("Allocating 1 page...");
    match allocator.alloc_pages(1, 4096) {
        Ok(addr) => info!("  Allocated at {:#x}\n", addr),
        Err(e) => info!("  Failed: {:?}\n", e),
    }

    info!("Allocating 8 pages...");
    match allocator.alloc_pages(8, 4096) {
        Ok(addr) => info!("  Allocated at {:#x}\n", addr),
        Err(e) => info!("  Failed: {:?}\n", e),
    }

    info!("Allocating 256 pages...");
    match allocator.alloc_pages(256, 4096) {
        Ok(addr) => info!("  Allocated at {:#x}\n", addr),
        Err(e) => info!("  Failed: {:?}\n", e),
    }

    // Test allocation that should span across zones (fallback)
    info!("\n=== Testing Zone Fallback ===");
    // Allocate from zone 0 until it's exhausted, then fallback to zone 1
    for i in 0..20 {
        match allocator.alloc_pages(1024, 4096) { // 4MB allocations
            Ok(addr) => {
                info!("Allocation {}: {:#x}\n", i, addr);
            }
            Err(e) => {
                info!("Allocation {} failed after zone exhaustion: {:?}\n", i, e);
                break;
            }
        }
    }

    // Print updated statistics
    info!("\n=== Updated Allocator Statistics ===");
    let stats = allocator.get_stats();
    info!("Total pages: {}", stats.total_pages);
    info!("Free pages: {}", stats.free_pages);
    info!("Used pages: {}", stats.used_pages);

    info!("\n{}", allocator.get_free_lists_info());

    info!("=== Test Complete ===");
    loop {}
}
