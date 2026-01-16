fn main() {
    // 分析 dealloc_pages 的 PFN 计算问题

    let base_addr = 0x9400000usize;
    let metadata_size = 237568usize;
    let usable_start = base_addr + metadata_size;
    let total_pages = 936646usize;
    let page_size = 4096usize;

    println!("=== 分析 PFN 计算问题 ===\n");
    println!("base_addr = {:#x}", base_addr);
    println!("metadata_size = {}", metadata_size);
    println!("usable_start = {:#x}", usable_start);
    println!("total_pages = {}\n", total_pages);

    println!("=== 当前实现（相对 PFN）===");
    for i in 0..5 {
        let page_addr = usable_start + i * page_size;
        println!("\n第 {} 页:", i);
        println!("  绝对地址: {:#x}", page_addr);

        // 当前的计算方式
        let offset = page_addr - usable_start;
        let pfn = offset / page_size;
        println!("  offset = {:#x} ({} 页)", offset, offset / page_size);
        println!("  pfn = {} (相对 PFN)", pfn);

        // 计算 order 0 的 buddy
        let order = 0;
        let buddy_pfn = pfn ^ (1 << order);
        println!("  buddy_pfn = {} (order {})", buddy_pfn, order);

        // 转换回绝对地址
        let buddy_addr = usable_start + buddy_pfn * page_size;
        println!("  buddy_addr = {:#x}", buddy_addr);

        // 检查 buddy 是否在 zone 内
        if buddy_addr < usable_start || buddy_addr >= usable_start + total_pages * page_size {
            println!("  ❌ buddy 超出 zone 范围！");
        } else {
            println!("  ✓ buddy 在 zone 内");
        }
    }

    println!("\n=== 参考（绝对 PFN）===");
    for i in 0..5 {
        let page_addr = base_addr + i * page_size;
        println!("\n第 {} 页:", i);
        println!("  绝对地址: {:#x}", page_addr);

        // 参考的计算方式
        let pfn = page_addr / page_size;
        println!("  pfn = {} (绝对 PFN)", pfn);

        // 计算 order 0 的 buddy
        let order = 0;
        let buddy_pfn = pfn ^ (1 << order);
        println!("  buddy_pfn = {} (order {})", buddy_pfn, order);

        // 转换回绝对地址
        let buddy_addr = buddy_pfn * page_size;
        println!("  buddy_addr = {:#x}", buddy_addr);
    }

    println!("\n=== 关键问题 ===");
    println!("1. 两种实现使用不同的 PFN：");
    println!("   - 当前: 相对 PFN (offset from usable_start)");
    println!("   - 参考: 绝对 PFN (from 0)\n");

    println!("2. 但 buddy 计算基于 PFN 的 XOR，所以：");
    println!("   - PFN=0 和 PFN=1 是 order 0 的 buddy (当前实现)");
    println!("   - PFN=37888 和 PFN=37889 是 order 0 的 buddy (参考实现)\n");

    println!("3. 关键问题：");
    println!("   在 order 19 时，需要合并 524288 个页面");
    println!("   buddy 系统要求这些页面连续对齐");
    println!("   但当前实现的相对 PFN 计算可能导致错误");
}
