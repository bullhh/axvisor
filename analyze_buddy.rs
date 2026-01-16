fn main() {
    // 模拟参数
    let base_addr = 0x9400000usize;
    let metadata_size = 237568usize;
    let usable_start = base_addr + metadata_size;
    let total_pages = 936646usize;
    let page_size = 4096usize;

    println!("=== 分析 bitmap vs 链表实现的 PFN 计算差异 ===\n");
    println!("base_addr = {:#x}", base_addr);
    println!("metadata_size = {} bytes", metadata_size);
    println!("usable_start = {:#x}", usable_start);
    println!("total_pages = {}\n", total_pages);

    // 链表实现（参考实现）
    println!("=== 链表实现（参考） ===");
    println!("pfn = addr / PAGE_SIZE (绝对 PFN)\n");

    for &addr in &[0x9400000, 0x943a000, 0x943b000] {
        let pfn = addr / page_size;
        println!("  addr {:#x}: pfn = {:#x} ({})", addr, pfn, pfn);
    }

    // Bitmap 实现（当前）
    println!("\n=== Bitmap 实现（当前） ===");
    println!("pfn = (addr - usable_start) / PAGE_SIZE (相对 PFN)\n");

    for &addr in &[0x9400000, 0x943a000, 0x943b000] {
        if addr >= usable_start {
            let offset = addr - usable_start;
            let pfn = offset / page_size;
            println!("  addr {:#x}: offset={:#x}, pfn = {:#x} ({})", addr, offset, pfn, pfn);
        } else {
            println!("  addr {:#x}: < usable_start, 无效", addr);
        }
    }

    // 分析 buddy 合并问题
    println!("\n=== Buddy 合并分析 ===");
    println!("对于 order 19 (524288 页 = 2GB):\n");

    // Bitmap 实现：第一页的绝对地址是 0x943a000
    let first_page_addr = usable_start;
    println!("Bitmap 实现：");
    println!("  第一页绝对地址 = {:#x}", first_page_addr);
    println!("  第一页相对 PFN = 0");
    println!("  Order 19 块的绝对地址 = {:#x}", usable_start);
    println!("  Order 19 需要对齐到 2GB = {:#x}", 0x80000000u64 as usize);
    println!("  {:#x} % 2GB = {:#x} (不是 0!)", usable_start, usable_start % (0x80000000u64 as usize));

    println!("\n链表实现：");
    println!("  第一页绝对地址 = {:#x}", base_addr);
    println!("  第一页绝对 PFN = {:#x} ({})", base_addr / page_size, base_addr / page_size);
    println!("  Order 19 块的绝对地址 = {:#x}", base_addr);
    println!("  Order 19 需要对齐到 2GB = {:#x}", 0x80000000u64 as usize);
    println!("  {:#x} % 2GB = {:#x} (不是 0!)", base_addr, base_addr % (0x80000000u64 as usize));

    // 关键问题
    println!("\n=== 关键问题 ===");
    println!("1. Buddy 系统要求：");
    println!("   - Order k 的块必须对齐到 2^k 页");
    println!("   - 但这是相对于 zone 的 PFN 对齐，不是绝对物理地址！\n");

    println!("2. Bitmap 实现：");
    println!("   - 使用相对 PFN (相对于 usable_start)");
    println!("   - PFN=0, PFN=524288 是 order 19 的 buddy");
    println!("   - 合并逻辑检查 buddy_addr = usable_start + buddy_pfn * PAGE_SIZE");
    println!("   - 但 buddy_pfn 是相对 PFN，所以逻辑正确！\n");

    println!("3. 实际检查：");
    // 检查初始化时会释放的页数
    println!("   - 初始化会释放 {} 页", total_pages);
    println!("   - 最多可以形成的 order 19 块数: {} 个", total_pages / 524288);
    let max_order_19 = total_pages / 524288;
    println!("   - 剩余页数: {} 页", total_pages % 524288);

    // 分析剩余页数能形成什么
    let remaining = total_pages % 524288;
    if remaining > 0 {
        println!("\n   剩余 {} 页能形成的 order:", remaining);
        for order in 0..=18 {
            let size = 1usize << order;
            let count = remaining / size;
            if count > 0 && size <= 65536 { // 只显示较大的
                println!("     Order {}: {} 个块 ({} 页)", order, count, count * size);
            }
        }
    }
}
