//! Test buddy allocator initialization
//!
//! This test verifies that the buddy allocator correctly initializes
//! with proper free block distribution following Linux-style initialization.

use buddy_slab_allocator::{PageAllocator, BuddyPageAllocator};

fn main() {
    println!("=== Buddy Allocator Initialization Test ===\n");

    // Create a buddy allocator
    let mut allocator = BuddyPageAllocator::<4096>::new();

    // Initialize with a memory region similar to zone 2 from the log
    // Zone 2: [0x9400000, 0xedf00000), 936646 usable pages
    let base_addr = 0x9400000;
    let size = 0xe4b00000; // 3659 MB

    println!("Initializing allocator with region [{:#x}, {:#x})", base_addr, base_addr + size);
    println!("Total size: {} MB\n", size / (1024 * 1024));

    allocator.init(base_addr, size);

    // Get number of zones
    let num_zones = allocator.get_zone_count();
    println!("Number of zones: {}\n", num_zones);

    // Print free block distribution for each zone
    for zone_id in 0..num_zones {
        println!("Zone {} free blocks distribution:", zone_id);

        let mut total_free_pages = 0;
        let mut total_free_blocks = 0;

        for order in 0..=19 {
            if let Some(count) = allocator.get_free_blocks_by_order(zone_id, order) {
                if count > 0 {
                    let block_size = 1 << order; // in pages
                    let block_size_bytes = block_size * 4096;
                    let total_bytes = block_size_bytes * count;

                    println!("  Order {:2}: {} blocks (size {} bytes each, total {:#x})",
                             order, count, block_size_bytes, total_bytes);

                    total_free_pages += count * block_size;
                    total_free_blocks += count;
                }
            }
        }

        println!("\n  Total free blocks: {}", total_free_blocks);
        println!("  Total free pages: {} ({} MB)",
                 total_free_pages,
                 (total_free_pages * 4096) / (1024 * 1024));
        println!();
    }

    // Test allocation and deallocation
    println!("=== Testing allocation/deallocation ===\n");

    // Try to allocate some pages
    let alloc_sizes = vec![
        (1, 1),      // 1 page, alignment 1 page
        (2, 1),      // 2 pages
        (4, 1),      // 4 pages
        (8, 1),      // 8 pages
        (16, 1),     // 16 pages
        (32, 1),     // 32 pages
        (64, 1),     // 64 pages
        (128, 1),    // 128 pages
        (256, 1),    // 256 pages
        (512, 1),    // 512 pages
        (1024, 1),   // 1024 pages (4MB)
    ];

    let mut allocated_addrs = Vec::new();

    for (num_pages, align) in alloc_sizes {
        match allocator.alloc_pages(num_pages, align * 4096) {
            Ok(addr) => {
                println!("Allocated {} pages (alignment {} pages) at {:#x}",
                         num_pages, align, addr);
                allocated_addrs.push((addr, num_pages));
            }
            Err(e) => {
                println!("Failed to allocate {} pages: {:?}", num_pages, e);
            }
        }
    }

    println!("\n=== Free block distribution after allocation ===\n");
    for zone_id in 0..num_zones {
        println!("Zone {} free blocks distribution:", zone_id);
        let mut total_free_pages = 0;
        for order in 0..=19 {
            if let Some(count) = allocator.get_free_blocks_by_order(zone_id, order) {
                if count > 0 {
                    let block_size = 1 << order;
                    println!("  Order {:2}: {} blocks ({} pages total)",
                             order, count, count * block_size);
                    total_free_pages += count * block_size;
                }
            }
        }
        println!("  Total free pages: {}\n", total_free_pages);
    }

    // Deallocate all allocated pages
    println!("=== Deallocating all allocated pages ===\n");
    for (addr, num_pages) in allocated_addrs {
        println!("Deallocating {} pages at {:#x}", num_pages, addr);
        allocator.dealloc_pages(addr, num_pages);
    }

    println!("\n=== Free block distribution after deallocation ===\n");
    for zone_id in 0..num_zones {
        println!("Zone {} free blocks distribution:", zone_id);
        let mut total_free_pages = 0;
        for order in 0..=19 {
            if let Some(count) = allocator.get_free_blocks_by_order(zone_id, order) {
                if count > 0 {
                    let block_size = 1 << order;
                    println!("  Order {:2}: {} blocks ({} pages total)",
                             order, count, count * block_size);
                    total_free_pages += count * block_size;
                }
            }
        }
        println!("  Total free pages: {}\n", total_free_pages);
    }

    println!("=== Test completed ===");
}
