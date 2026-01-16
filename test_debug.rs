//! Simple debug test for buddy allocator

use buddy_slab_allocator::{BuddyPageAllocator, PageAllocator};

const MEMORY_SIZE: usize = 0x100000; // 1 MB
static mut MEMORY: [u8; MEMORY_SIZE] = [0u8; MEMORY_SIZE];

fn main() {
    let start = unsafe { (&raw mut MEMORY) as usize };
    let mut allocator = BuddyPageAllocator::<4096>::new();

    println!("Init: [{:#x}, {:#x}) ({})", start, start + MEMORY_SIZE, MEMORY_SIZE / 4096);
    allocator.init(start, MEMORY_SIZE);

    println!("Total: {}, Free: {}", allocator.total_pages(), allocator.available_pages());

    // Alloc 4 pages, 4K alignment
    let addr1 = allocator.alloc_pages(4, 0x1000).unwrap();
    println!("Alloc 1: {:#x} (align 4K), Free: {}", addr1, allocator.available_pages());

    // Dealloc
    allocator.dealloc_pages(addr1, 4);
    println!("Dealloc, Free: {}", allocator.available_pages());

    // Alloc 4 pages, 16K alignment
    let addr2 = allocator.alloc_pages(4, 0x4000);
    println!("Alloc 2: {:?}", addr2);

    if let Ok(addr) = addr2 {
        println!("Success! {:#x} (align 16K)", addr);
        println!("Free: {}", allocator.available_pages());
    }
}
