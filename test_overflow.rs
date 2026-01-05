//! Test overflow handling in CompositePageAllocator
//!
//! This test demonstrates how the page allocator returns excess memory
//! allocated by buddy system back to the free pool.

#![no_std]
#![no_main]

extern crate alloc;
extern crate axlog;

use axlog::info;
use axvisor_allocator::{CompositePageAllocator, PageAllocator, BaseAllocator};

#[no_mangle]
pub extern "C" fn main() -> ! {
    axlog::init();
    info!("=== CompositePageAllocator Overflow Handling Test ===\n");

    let mut allocator = CompositePageAllocator::new();

    // Initialize with a large memory region (2GB)
    info!("Initializing allocator with 2GB memory...");
    allocator.init(0x8000_0000, 0x8000_0000); // 2GB starting at 2GB
    info!("Allocator initialized\n");

    // Print initial statistics
    info!("\n=== Initial Statistics ===");
    info!("{}", allocator.get_buddy_stats());

    // Test 1: Power-of-2 allocation (no overflow)
    info!("\n=== Test 1: Power-of-2 Allocation (1024 pages = 4MB) ===");
    match allocator.alloc_pages(1024, 4096) {
        Ok(addr) => {
            info!("Allocated at {:#x}", addr);
            let stats = allocator.get_buddy_stats();
            info!("Used pages: {}, Free pages: {}", stats.used_pages, stats.free_pages);
        }
        Err(e) => info!("Failed: {:?}", e),
    }

    // Test 2: Non-power-of-2 allocation (will trigger overflow handling)
    info!("\n=== Test 2: Non-Power-of-2 Allocation (1540 pages ≈ 6MB) ===");
    info!("This should allocate 2048 pages from buddy, then return 508 pages back");
    match allocator.alloc_pages(1540, 4096) {
        Ok(addr) => {
            info!("Allocated at {:#x}", addr);
            let stats = allocator.get_buddy_stats();
            info!("Used pages: {}, Free pages: {}", stats.used_pages, stats.free_pages);
        }
        Err(e) => info!("Failed: {:?}", e),
    }

    // Test 3: Another non-power-of-2 allocation
    info!("\n=== Test 3: Non-Power-of-2 Allocation (3000 pages ≈ 11.7MB) ===");
    info!("This should allocate 4096 pages from buddy, then return 1096 pages back");
    match allocator.alloc_pages(3000, 4096) {
        Ok(addr) => {
            info!("Allocated at {:#x}", addr);
            let stats = allocator.get_buddy_stats();
            info!("Used pages: {}, Free pages: {}", stats.used_pages, stats.free_pages);
        }
        Err(e) => info!("Failed: {:?}", e),
    }

    // Test 4: Large non-power-of-2 allocation
    info!("\n=== Test 4: Large Non-Power-of-2 Allocation (394240 pages = 1540MB) ===");
    info!("This should allocate 524288 pages (2GB) from buddy, then return 130048 pages (508MB) back");
    match allocator.alloc_pages(394240, 4096) {
        Ok(addr) => {
            info!("Allocated at {:#x}", addr);
            let stats = allocator.get_buddy_stats();
            info!("Used pages: {}, Free pages: {}", stats.used_pages, stats.free_pages);
        }
        Err(e) => info!("Failed: {:?}", e),
    }

    // Print final statistics
    info!("\n=== Final Statistics ===");
    let stats = allocator.get_buddy_stats();
    info!("Total pages: {}", stats.total_pages);
    info!("Used pages: {}", stats.used_pages);
    info!("Free pages: {}", stats.free_pages);

    info!("\n{}", allocator.get_free_lists_info());

    info!("=== Test Complete ===");
    loop {}
}
