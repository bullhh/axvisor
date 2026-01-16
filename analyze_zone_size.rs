fn main() {
    let base_addr = 0x9400000usize;
    let metadata_size = 237568usize;
    let usable_start = base_addr + metadata_size;
    let size = 0xe4b00000usize;
    let page_size = 4096usize;

    let aligned_size = size & !(page_size - 1);
    let total_pages = aligned_size / page_size;
    let metadata_pages = metadata_size / page_size;
    let usable_pages = total_pages - metadata_pages;

    println!("=== 分析 Zone 对齐问题 ===\n");
    println!("base_addr = {:#x}", base_addr);
    println!("metadata_size = {} bytes ({} pages)", metadata_size, metadata_pages);
    println!("usable_start = {:#x}", usable_start);
    println!("usable_pages = {}\n", usable_pages);

    println!("=== 检查 order 19 (524288 pages = 2GB) ===\n");

    // 使用相对 PFN 的当前实现
    println!("\n当前实现（相对 PFN）:");
    let rel_first_pfn = 0;
    let rel_order_19_addr = usable_start + rel_first_pfn * page_size;
    println!("  order 19 块地址 = {:#x}", rel_order_19_addr);
    println!("  相对 PFN = {}", rel_first_pfn);
    println!("  检查对齐: {:#x} % {} = {:#x}", rel_order_19_addr, 524288 * page_size, rel_order_19_addr % (524288 * page_size));

    // order 19 的 buddy
    let rel_buddy_pfn = rel_first_pfn ^ (1 << 19);
    let rel_buddy_addr = usable_start + rel_buddy_pfn * page_size;
    println!("  buddy 相对 PFN = {}", rel_buddy_pfn);
    println!("  buddy 地址 = {:#x}", rel_buddy_addr);

    if rel_buddy_pfn >= usable_pages {
        println!("  ❌ buddy PFN {} >= usable_pages {}", rel_buddy_pfn, usable_pages);
        println!("  ✓ 正确：zone 只有 {} 页，无法形成 order 19 的 buddy", usable_pages);
    } else {
        println!("  ⚠️  buddy 在 zone 内，可能错误合并！");
    }

    // 使用绝对 PFN 的参考实现
    println!("\n参考实现（绝对 PFN）:");
    let abs_first_pfn = base_addr / page_size;
    let abs_order_19_addr = base_addr + abs_first_pfn * page_size;
    println!("  order 19 块地址 = {:#x}", abs_order_19_addr);
    println!("  绝对 PFN = {}", abs_first_pfn);
    println!("  检查对齐: {:#x} % {} = {:#x}", abs_order_19_addr, 524288 * page_size, abs_order_19_addr % (524288 * page_size));

    // order 19 的 buddy
    let abs_buddy_pfn = abs_first_pfn ^ (1 << 19);
    let abs_buddy_addr = base_addr + abs_buddy_pfn * page_size;
    println!("  buddy 绝对 PFN = {}", abs_buddy_pfn);
    println!("  buddy 地址 = {:#x}", abs_buddy_addr);

    println!("\n=== 关键结论 ===");
    println!("1. 当前实现使用相对 PFN 是正确的（因为有 metadata）");
    println!("2. 问题不在 PFN 计算，而可能在其他地方");
    println!("3. 需要追踪实际的段错误位置");
}
