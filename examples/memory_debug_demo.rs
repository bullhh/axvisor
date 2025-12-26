//! 内存分配器调试演示
//! 
//! 这个示例展示如何使用调试功能查看内存分配器的内部状态

use axvisor_allocator::{GlobalAllocator, BuddyPageAllocator};

fn main() {
    println!("=== Axvisor 内存分配器调试演示 ===\n");
    
    // 演示 Buddy 分配器调试
    demo_buddy_debug();
    
    // 演示全局分配器调试
    demo_global_debug();
    
    println!("=== 演示完成 ===");
}

fn demo_buddy_debug() {
    println!("--- Buddy 分配器调试演示 ---");
    
    let mut buddy = BuddyPageAllocator::new();
    
    // 初始化 1MB 内存
    let base_addr = 0x80000000;
    let size = 0x100000; // 1MB = 256 pages
    buddy.init(base_addr, size);
    
    println!("初始化后的 Buddy 分配器状态:");
    print_free_lists_info(&buddy);
    
    // 分配一些页面
    println!("\n分配 4 页 (16KB)...");
    let page1 = buddy.alloc_pages(4, 0).unwrap();
    
    println!("分配 1 页 (4KB)...");
    let page2 = buddy.alloc_pages(1, 0).unwrap();
    
    println!("分配 8 页 (32KB)...");
    let page3 = buddy.alloc_pages(8, 0).unwrap();
    
    println!("\n分配后的状态:");
    print_free_lists_info(&buddy);
    
    // 释放一些页面观察合并
    println!("\n释放 4 页...");
    buddy.dealloc_pages(page1, 4);
    
    println!("释放 1 页...");
    buddy.dealloc_pages(page2, 1);
    
    println!("\n部分释放后的状态 (应该显示合并):");
    print_free_lists_info(&buddy);
    
    // 获取统计信息
    let stats = buddy.get_stats();
    println!("\n最终统计信息:");
    println!("  总页数: {}", stats.total_pages);
    println!("  空闲页数: {}", stats.free_pages);
    println!("  已用页数: {}", stats.used_pages);
    println!("  内存利用率: {:.2}%", 
        (stats.used_pages as f64 / stats.total_pages as f64) * 100.0
    );
    
    // 显示各阶空闲块
    println!("\n各阶空闲块数量:");
    for (order, &count) in stats.free_pages_by_order.iter().enumerate() {
        if *count > 0 {
            println!("  阶{} ({}页): {} 个空闲块", 
                order, 
                1 << order, 
                count
            );
        }
    }
    
    println!();
}

fn demo_global_debug() {
    println!("--- 全局分配器调试演示 ---");
    
    let global = GlobalAllocator::new();
    
    // 初始化 2MB 内存
    let base_addr = 0x90000000;
    let size = 0x200000; // 2MB
    global.init(base_addr, size).expect("初始化失败");
    
    println!("初始化后的全局分配器状态:");
    print_global_stats(&global);
    
    // 执行一些分配操作
    println!("\n执行分配操作...");
    
    // 小对象分配 (使用 Slab)
    let small_layout = core::alloc::Layout::from_size_align(64, 8).unwrap();
    let _small_ptr1 = global.alloc(small_layout).unwrap();
    let _small_ptr2 = global.alloc(small_layout).unwrap();
    let _small_ptr3 = global.alloc(small_layout).unwrap();
    
    // 大对象分配 (使用 Buddy)
    let large_layout = core::alloc::Layout::from_size_align(8192, 8).unwrap(); // 8KB
    let _large_ptr = global.alloc(large_layout).unwrap();
    
    println!("分配后:");
    print_global_stats(&global);
    
    // 获取 free lists 信息
    println!("\n当前 free lists 信息:");
    let info = global.get_free_lists_info();
    println!("{}", info);
    
    // 释放一些内存
    println!("\n释放部分内存...");
    global.dealloc(_small_ptr1, small_layout);
    global.dealloc(_large_ptr, large_layout);
    
    println!("释放后:");
    print_global_stats(&global);
    
    println!("\n最终的 free lists 信息:");
    let final_info = global.get_free_lists_info();
    println!("{}", final_info);
    
    println!();
}

fn print_free_lists_info(buddy: &BuddyPageAllocator) {
    let info = buddy.get_free_lists_info();
    println!("{}", info);
}

fn print_global_stats(global: &GlobalAllocator) {
    let stats = global.get_stats();
    println!("  总页数: {}", stats.total_pages);
    println!("  已用页数: {}", stats.used_pages);
    println!("  空闲页数: {}", stats.free_pages);
    println!("  Slab 字节数: {} bytes", stats.slab_bytes);
    println!("  堆字节数: {} bytes", stats.heap_bytes);
    
    // 计算 MB
    let total_mb = (stats.total_pages * 4096) / (1024 * 1024);
    let used_mb = (stats.used_pages * 4096) / (1024 * 1024);
    let free_mb = (stats.free_pages * 4096) / (1024 * 1024);
    let slab_mb = stats.slab_bytes / (1024 * 1024);
    let heap_mb = stats.heap_bytes / (1024 * 1024);
    
    println!("  内存使用: {} MB / {} MB ({}%)", 
        used_mb, total_mb, 
        (stats.used_pages as f64 / stats.total_pages as f64) * 100.0
    );
    println!("  详细分布: Slab {} MB, 堆 {} MB, 空闲 {} MB", 
        slab_mb, heap_mb, free_mb
    );
}
