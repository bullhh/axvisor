//! Test with more details about deallocation

use buddy_slab_allocator::{BuddyPageAllocator, PageAllocator};

const MEMORY_SIZE: usize = 0x100000; // 1 MB = 256 pages
static mut MEMORY: [u8; MEMORY_SIZE] = [0u8; MEMORY_SIZE];

fn main() {
    let start = unsafe { (&raw mut MEMORY) as usize };
    let mut allocator = BuddyPageAllocator::<4096>::new();

    allocator.init(start, MEMORY_SIZE);

    // Alloc 1 page
    let a1 = allocator.alloc_pages(1, 0x1000).unwrap();
    println!("Alloc 1: {:#x}, Free={}, Used={}", a1, allocator.available_pages(), allocator.used_pages());

    // Dealloc
    println!("\nDeallocing {:#x}...", a1);
    allocator.dealloc_pages(a1, 1);
    println!("After dealloc: Free={}, Used={}", allocator.available_pages(), allocator.used_pages());

    // Try alloc again (should work)
    let a2 = allocator.alloc_pages(1, 0x1000);
    println!("\nRe-alloc: {:?}", a2);
}
