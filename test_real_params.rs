use buddy_slab_allocator::{BuddyPageAllocator, PageAllocator};

// 使用与日志相同的参数
const BASE_ADDR: usize = 0x9400000;
const SIZE: usize = 0xe4b00000;

fn main() {
    let mut allocator = BuddyPageAllocator::<4096>::new();

    println!("=== Buddy Allocator Init Test ===");
    println!("Initializing with region [{:#x}, {:#x})", BASE_ADDR, BASE_ADDR + SIZE);

    allocator.init(BASE_ADDR, SIZE);

    println!("\nTotal pages: {}", allocator.total_pages());
    println!("Free pages: {}", allocator.available_pages());
    println!("Used pages: {}\n", allocator.used_pages());

    // Try some allocations
    println!("=== Testing allocations ===\n");

    // Alloc 1 page
    match allocator.alloc_pages(1, 0x1000) {
        Ok(addr) => println!("Alloc 1 page at {:#x}", addr),
        Err(e) => println!("Alloc 1 page failed: {:?}", e),
    }

    // Alloc 4 pages
    match allocator.alloc_pages(4, 0x1000) {
        Ok(addr) => println!("Alloc 4 pages at {:#x}", addr),
        Err(e) => println!("Alloc 4 pages failed: {:?}", e),
    }

    // Alloc 16 pages
    match allocator.alloc_pages(16, 0x1000) {
        Ok(addr) => println!("Alloc 16 pages at {:#x}", addr),
        Err(e) => println!("Alloc 16 pages failed: {:?}", e),
    }

    println!("\nAfter allocations:");
    println!("Free pages: {}", allocator.available_pages());

    // Try large allocation
    match allocator.alloc_pages(524288, 0x1000) {
        Ok(addr) => println!("\nAlloc 524288 pages (order 19) at {:#x}", addr),
        Err(e) => println!("\nAlloc 524288 pages (order 19) failed: {:?}", e),
    }
}
