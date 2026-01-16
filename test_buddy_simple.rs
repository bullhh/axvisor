use buddy_slab_allocator::BuddyPageAllocator;

fn main() {
    let mut allocator = BuddyPageAllocator::<4096>::new();

    // 使用与日志相同的参数
    let base_addr = 0x9400000;
    let size = 0xe4b00000;

    println!("=== Buddy Allocator Test ===\n");
    println!("Initializing with region [{:#x}, {:#x})", base_addr, base_addr + size);

    allocator.init(base_addr, size);

    let num_zones = allocator.get_zone_count();
    println!("\nNumber of zones: {}\n", num_zones);

    // 详细统计每个 order 的块数
    for zone_id in 0..num_zones {
        println!("Zone {} free blocks distribution:", zone_id);

        let mut total_free_pages = 0;
        for order in 0..=19 {
            if let Some(count) = allocator.get_free_blocks_by_order(zone_id, order as u32) {
                if count > 0 {
                    let block_pages = 1usize << order;
                    total_free_pages += count * block_pages;

                    println!(
                        "  Order {:2}: {:6} blocks ({} pages total, {} MB)",
                        order,
                        count,
                        count * block_pages,
                        (count * block_pages * 4096) / (1024 * 1024)
                    );
                }
            }
        }

        println!("\n  Total free pages: {} ({} MB)", total_free_pages,
                 (total_free_pages * 4096) / (1024 * 1024));
    }
}
