//! Test bitmap state

use buddy_slab_allocator::{BuddyPageAllocator, PageAllocator};

const MEMORY_SIZE: usize = 0x100000; // 1 MB = 256 pages
static mut MEMORY: [u8; MEMORY_SIZE] = [0u8; MEMORY_SIZE];

fn main() {
    let start = unsafe { (&raw mut MEMORY) as usize };
    let mut allocator = BuddyPageAllocator::<4096>::new();

    allocator.init(start, MEMORY_SIZE);

    println!("Init: [{:#x}, {:#x})", start, start + MEMORY_SIZE);
    println!("Total pages: {}, Free pages: {}", allocator.total_pages(), allocator.available_pages());

    // Alloc 4 pages
    let a1 = allocator.alloc_pages(4, 0x1000).unwrap();
    println!("\nAllocated 4 pages at {:#x}", a1);
    println!("Total: {}, Free: {}, Used: {}", allocator.total_pages(), allocator.available_pages(), allocator.used_pages());

    // Dealloc
    println!("\nDeallocating...");
    allocator.dealloc_pages(a1, 4);
    println!("After dealloc - Total: {}, Free: {}, Used: {}", allocator.total_pages(), allocator.available_pages(), allocator.used_pages());

    // Alloc again
    let a2 = allocator.alloc_pages(4, 0x1000);
    println!("\nAlloc again: {:?}", a2);
    println!("Final - Total: {}, Free: {}, Used: {}", allocator.total_pages(), allocator.available_pages(), allocator.used_pages());

    // Try alloc with 16K alignment
    let a3 = allocator.alloc_pages(4, 0x4000);
    println!("\nAlloc 4 pages with 16K align: {:?}", a3);
}
