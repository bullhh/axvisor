use buddy_slab_allocator::BuddyPageAllocator;

fn main() {
    let mut allocator = BuddyPageAllocator::<4096>::new();

    let base_addr = 0x9400000;
    let size = 0xe4b00000;

    println!("Initializing with region [{:#x}, {:#x})", base_addr, base_addr + size);

    allocator.init(base_addr, size);
}
