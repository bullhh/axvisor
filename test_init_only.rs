use buddy_slab_allocator::{BuddyPageAllocator, PageAllocator};

const BASE_ADDR: usize = 0x9400000;
const SIZE: usize = 0xe4b00000;

fn main() {
    let mut allocator = BuddyPageAllocator::<4096>::new();

    println!("=== Buddy Allocator Init Test ===");
    println!("Initializing with region [{:#x}, {:#x})", BASE_ADDR, BASE_ADDR + SIZE);

    allocator.init(BASE_ADDR, SIZE);

    println!("\nInit completed successfully!");
    println!("Total pages: {}", allocator.total_pages());
    println!("Free pages: {}", allocator.available_pages());
}
