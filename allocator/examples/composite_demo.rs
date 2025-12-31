//! Example and test for CompositePageAllocator
//!
//! This example demonstrates the composite allocation feature that allows
//! allocating non-power-of-2 sized memory blocks by combining
//! multiple buddy blocks.

use axvisor_allocator::CompositePageAllocator;

fn main() {
    // Initialize the allocator with 256MB of memory
    let mut allocator = CompositePageAllocator::new();
    allocator.init(0x80000000, 0x10000000); // [0x80000000, 0x90000000)

    println!("=== Composite Page Allocator Demo ===\n");

    // Get initial statistics
    println!("Initial state:");
    print_allocator_stats(&allocator);

    // Example 1: Standard buddy allocation (power of 2)
    println!("\n1. Allocating 1024 pages (4MB) - Standard buddy allocation");
    match allocator.alloc_pages(1024, 4096) {
        Ok(addr) => {
            println!("   Success: allocated at {:#x}", addr);
            print_allocator_stats(&allocator);
        }
        Err(e) => println!("   Failed: {:?}", e),
    }

    // Example 2: Composite allocation (non-power-of-2)
    // 1536 pages = 6MB = 1024 + 512 (not a power of 2)
    println!("\n2. Allocating 1536 pages (6MB) - Composite allocation");
    match allocator.alloc_pages(1536, 4096) {
        Ok(addr) => {
            println!("   Success: allocated at {:#x}", addr);
            print_allocator_stats(&allocator);
            print_composite_stats(&allocator);
        }
        Err(e) => println!("   Failed: {:?}", e),
    }

    // Example 3: Large composite allocation
    // 393216 pages = 1536MB = 1024 + 256 + 64 + ...
    println!("\n3. Allocating 393216 pages (1536MB) - Large composite allocation");
    match allocator.alloc_pages(393216, 4096) {
        Ok(addr) => {
            println!("   Success: allocated at {:#x}", addr);
            print_allocator_stats(&allocator);
            print_composite_stats(&allocator);
        }
        Err(e) => println!("   Failed: {:?}", e),
    }

    println!("\n=== Demo completed ===");
}

fn print_allocator_stats(allocator: &CompositePageAllocator) {
    let total = allocator.total_pages();
    let used = allocator.used_pages();
    let free = allocator.available_pages();
    
    println!("   Total pages: {} ({} MB)", total, pages_to_mb(total));
    println!("   Used pages:  {} ({} MB)", used, pages_to_mb(used));
    println!("   Free pages:  {} ({} MB)", free, pages_to_mb(free));
}

fn print_composite_stats(allocator: &CompositePageAllocator) {
    let stats = allocator.get_composite_stats();
    
    println!("   Composite allocations:");
    println!("     Active: {}", stats.active_allocations);
    println!("     Total parts: {}", stats.total_parts);
    println!("     Total pages in composite: {} ({} MB)", 
             stats.total_pages_in_composite, 
             pages_to_mb(stats.total_pages_in_composite));
    println!("     Slots used: {}/{}", stats.used_slots, stats.max_slots);
}

fn pages_to_mb(pages: usize) -> f64 {
    (pages as f64) * 4.0 / 1024.0
}
