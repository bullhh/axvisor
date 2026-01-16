//! Minimal test to find the bug

use buddy_slab_allocator::{BuddyPageAllocator, PageAllocator};

const MEMORY_SIZE: usize = 0x100000; // 1 MB = 256 pages
static mut MEMORY: [u8; MEMORY_SIZE] = [0u8; MEMORY_SIZE];

fn main() {
    let start = unsafe { (&raw mut MEMORY) as usize };
    let mut allocator = BuddyPageAllocator::<4096>::new();

    allocator.init(start, MEMORY_SIZE);

    println!("Initial: Total={}, Free={}", allocator.total_pages(), allocator.available_pages());

    // Alloc 1 page
    let a1 = allocator.alloc_pages(1, 0x1000).unwrap();
    println!("After alloc 1: Total={}, Free={}, Used={}", allocator.total_pages(), allocator.available_pages(), allocator.used_pages());

    // Alloc another 1 page
    let a2 = allocator.alloc_pages(1, 0x1000).unwrap();
    println!("After alloc 2: Total={}, Free={}, Used={}", allocator.total_pages(), allocator.available_pages(), allocator.used_pages());

    // Dealloc first
    allocator.dealloc_pages(a1, 1);
    println!("After dealloc 1: Total={}, Free={}, Used={}", allocator.total_pages(), allocator.available_pages(), allocator.used_pages());

    // Dealloc second
    allocator.dealloc_pages(a2, 1);
    println!("After dealloc 2: Total={}, Free={}, Used={}", allocator.total_pages(), allocator.available_pages(), allocator.used_pages());

    // Alloc again
    let a3 = allocator.alloc_pages(1, 0x1000);
    println!("After re-alloc: {:?}", a3);
}
