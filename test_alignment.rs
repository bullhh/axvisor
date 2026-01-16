fn main() {
    let usable_start = 0x943a000usize;
    let page_size = 4096usize;

    let total_pages = 936646usize;

    // Check alignment for each order
    println!("=== Order Alignment Analysis ===\n");
    println!("usable_start = {:#x}", usable_start);
    println!("total_pages = {}\n", total_pages);

    for order in [0, 1, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19].iter() {
        let order = *order as usize;
        let block_pages = 1usize << order;
        let block_size_bytes = block_pages * page_size;
        let alignment_requirement = block_size_bytes;

        let is_aligned = (usable_start % alignment_requirement) == 0;
        let num_blocks = total_pages / block_pages;

        println!(
            "Order {:2}: block_size={} pages={} bytes={:#x} aligned={} max_blocks={}",
            order, block_size_bytes, block_pages, block_size_bytes, is_aligned, num_blocks
        );
    }
}
