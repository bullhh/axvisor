//! Test 4 pages allocation/deallocation

use buddy_slab_allocator::{BuddyPageAllocator, PageAllocator};

const MEMORY_SIZE: usize = 0x100000; // 1 MB = 256 pages
static mut MEMORY: [u8; MEMORY_SIZE] = [0u8; MEMORY_SIZE];

fn main() {
    let start = unsafe { (&raw mut MEMORY) as usize };
    let mut allocator = BuddyPageAllocator::<4096>::new();

    allocator.init(start, MEMORY_SIZE);

    println!("Initial: Total={}, Free={}", allocator.total_pages(), allocator.available_pages());

    // Alloc 4 pages
    let a1 = allocator.alloc_pages(4, 0x1000).unwrap();
    println!("\nAlloc 4: {:#x}, Free={}, Used={}", a1, allocator.available_pages(), allocator.used_pages());

    // Dealloc 4 pages
    println!("\nDeallocing {:#x}...", a1);
    allocator.dealloc_pages(a1, 4);
    println!("After dealloc: Free={}, Used={}", allocator.available_pages(), allocator.used_pages());

    // Alloc 4 pages again (should work)
    let a2 = allocator.alloc_pages(4, 0x1000);
    println!("\nRe-alloc: {:?}", a2);
}
