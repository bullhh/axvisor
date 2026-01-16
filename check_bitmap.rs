fn main() {
    let total_pages = 936646usize;

    println!("=== Check bitmap size calculation ===");
    println!("total_pages = {}\n", total_pages);

    for order in 0..=19 {
        let blocks = (total_pages + (1 << order) - 1) >> order;
        let u64s_needed = (blocks + 63) / 64;

        println!(
            "Order {:2}: blocks = {:7}, u64s = {:6}, bytes = {:7}",
            order, blocks, u64s_needed, u64s_needed * 8
        );
    }

    // Calculate total memory needed
    let mut total_u64s = 0usize;
    for order in 0..=19 {
        let blocks = (total_pages + (1 << order) - 1) >> order;
        let u64s_needed = (blocks + 63) / 64;
        total_u64s += u64s_needed;
    }

    println!("\nTotal u64s: {}", total_u64s);
    println!("Total bytes: {}", total_u64s * 8);
    println!("Total pages: {}", (total_u64s * 8) / 4096);
    println!("Expected metadata pages: {}", 237568 / 4096);
}
