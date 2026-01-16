//! More detailed debug test

use buddy_slab_allocator::{BuddyPageAllocator, PageAllocator};

const MEMORY_SIZE: usize = 0x100000; // 1 MB
static mut MEMORY: [u8; MEMORY_SIZE] = [0u8; MEMORY_SIZE];

fn main() {
    let start = unsafe { (&raw mut MEMORY) as usize };
    let mut allocator = BuddyPageAllocator::<4096>::new();

    println!("Init: [{:#x}, {:#x})", start, start + MEMORY_SIZE);
    allocator.init(start, MEMORY_SIZE);
    println!("Total: {}, Free: {}", allocator.total_pages(), allocator.available_pages());

    // Alloc 4 pages, 4K alignment
    let addr1 = allocator.alloc_pages(4, 0x1000).unwrap();
    println!("\nAlloc 4 pages (4K align): {:#x}", addr1);
    println!("After alloc - Free: {}", allocator.available_pages());

    // Dealloc
    println!("\nDeallocating 4 pages at {:#x}...", addr1);
    allocator.dealloc_pages(addr1, 4);
    println!("After dealloc - Free: {}", allocator.available_pages());

    // Now alloc 4 pages, 16K alignment
    println!("\nTrying alloc 4 pages (16K align)...");
    let addr2 = allocator.alloc_pages(4, 0x4000);
    println!("Result: {:?}", addr2);

    if let Ok(addr) = addr2 {
        println!("✅ Success! {:#x}", addr);
    } else {
        println!("❌ Failed");
    }
}
