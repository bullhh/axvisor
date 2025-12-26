//! Axvisor 内存分配器调试示例
//! 
//! 这个示例展示如何查看内存分配器的内部状态，包括 free_lists 和内存剩余情况

#![no_std]
#![no_main]

extern crate alloc;

use axvisor_allocator::{
    GlobalAllocator, BuddyPageAllocator,
    PageAllocator, ByteAllocator,
};

#[no_mangle]
pub fn main() {
    println!("=== Axvisor 内存分配器调试示例 ===");
    
    // 示例1: 查看 Buddy 分配器的 free_lists
    demo_buddy_debug_info();
    
    // 示例2: 查看全局分配器的内存状态
    demo_global_allocator_debug();
    
    println!("=== 调试示例完成 ===");
}

fn demo_buddy_debug_info() {
    println!("\n--- Buddy 分配器调试信息 ---");
    
    let mut buddy = BuddyPageAllocator::new();
    
    // 初始化 1MB 内存
    let base_addr = 0x80000000;
    let size = 0x100000; // 1MB = 256 pages
    buddy.init(base_addr, size);
    
    println!("初始化后的 Buddy 分配器状态:");
    
    // 使用新添加的调试方法
    // 注意：由于这是 no_std 环境，我们需要使用 get_free_lists_info
    let info = buddy.global_pool.get_free_lists_info();
    print!("{}", info);
    
    // 分配一些页面来观察变化
    println!("\n分配 4 页 (16KB)...");
    let page1 = buddy.alloc_pages(4, 0).unwrap();
    
    let info_after_alloc = buddy.global_pool.get_free_lists_info();
    println!("分配后的状态:");
    print!("{}", info_after_alloc);
    
    // 再分配一些不同大小的页面
    println!("\n分配 1 页 (4KB)...");
    let page2 = buddy.alloc_pages(1, 0).unwrap();
    
    println!("\n分配 8 页 (32KB)...");
    let page3 = buddy.alloc_pages(8, 0).unwrap();
    
    let info_after_more = buddy.global_pool.get_free_lists_info();
    println!("更多分配后的状态:");
    print!("{}", info_after_more);
    
    // 释放一些页面来观察合并
    println!("\n释放 4 页...");
    buddy.dealloc_pages(page1, 4);
    
    let info_after_dealloc = buddy.global_pool.get_free_lists_info();
    println!("释放后的状态 (应该显示合并):");
    print!("{}", info_after_dealloc);
    
    // 释放相邻页面观察进一步合并
    println!("\n释放 1 页...");
    buddy.dealloc_pages(page2, 1);
    
    let info_after_dealloc2 = buddy.global_pool.get_free_lists_info();
    println!("释放相邻页面后的状态:");
    print!("{}", info_after_dealloc2);
    
    // 获取统计信息
    let stats = buddy.get_stats();
    println!("\n最终统计信息:");
    println!("  总页数: {}", stats.total_pages);
    println!("  空闲页数: {}", stats.free_pages);
    println!("  已用页数: {}", stats.used_pages);
    println!("  内存利用率: {:.2}%", 
        (stats.used_pages as f64 / stats.total_pages as f64) * 100.0
    );
    
    println!("各阶空闲块数量:");
    for (order, &count) in stats.free_pages_by_order.iter().enumerate() {
        if *count > 0 {
            println!("  阶{} ({}页): {} 个空闲块", 
                order, 
                1 << order, 
                count
            );
        }
    }
}

fn demo_global_allocator_debug() {
    println!("\n--- 全局分配器调试信息 ---");
    
    let global = GlobalAllocator::new();
    
    // 初始化 2MB 内存
    let base_addr = 0x90000000;
    let size = 0x200000; // 2MB
    global.init(base_addr, size).expect("初始化失败");
    
    println!("初始化后的全局分配器状态:");
    
    // 获取内存统计
    let stats = global.get_stats();
    println!("  总页数: {}", stats.total_pages);
    println!("  已用页数: {}", stats.used_pages);
    println!("  空闲页数: {}", stats.free_pages);
    println!("  Slab 字节数: {}", stats.slab_bytes);
    println!("  堆字节数: {}", stats.heap_bytes);
    
    // 获取 Buddy 统计
    let buddy_stats = global.get_buddy_stats();
    println!("\nBuddy 分配器详细统计:");
    for (order, &count) in buddy_stats.free_pages_by_order.iter().enumerate() {
        if *count > 0 {
            let block_size = (1 << order) * 4096; // 4KB pages
            println!("  阶{}: {} 个块 (每个 {} 字节 = {} KB)", 
                order, 
                count, 
                block_size,
                block_size / 1024
            );
        }
    }
    
    // 执行一些分配操作来观察变化
    println!("\n执行一些分配操作...");
    
    // 小对象分配 (使用 Slab)
    let small_layout = core::alloc::Layout::from_size_align(64, 8).unwrap();
    let _small_ptr1 = global.alloc(small_layout).unwrap();
    let _small_ptr2 = global.alloc(small_layout).unwrap();
    let _small_ptr3 = global.alloc(small_layout).unwrap();
    
    // 大对象分配 (使用 Buddy)
    let large_layout = core::alloc::Layout::from_size_align(8192, 8).unwrap(); // 8KB
    let _large_ptr = global.alloc(large_layout).unwrap();
    
    println!("分配后:");
    let stats_after = global.get_stats();
    println!("  已用页数: {} (增加 {})", 
        stats_after.used_pages, 
        stats_after.used_pages - stats.used_pages
    );
    println!("  Slab 字节数: {} (增加 {})", 
        stats_after.slab_bytes, 
        stats_after.slab_bytes - stats.slab_bytes
    );
    println!("  堆字节数: {} (增加 {})", 
        stats_after.heap_bytes, 
        stats_after.heap_bytes - stats.heap_bytes
    );
    
    // 获取最新的 free lists 信息
    println!("\n最新的 free lists 信息:");
    let free_lists_info = global.get_free_lists_info();
    print!("{}", free_lists_info);
    
    // 释放一些内存观察变化
    println!("\n释放一些内存...");
    
    // 释放小对象
    global.dealloc(_small_ptr1, small_layout);
    global.dealloc(_small_ptr2, small_layout);
    
    println!("释放部分小对象后:");
    let stats_after_partial = global.get_stats();
    println!("  Slab 字节数: {} (减少 {})", 
        stats_after_partial.slab_bytes, 
        stats_after.slab_bytes - stats_after_partial.slab_bytes
    );
    
    // 释放大对象
    global.dealloc(_large_ptr, large_layout);
    
    println!("释放大对象后:");
    let stats_after_all = global.get_stats();
    println!("  已用页数: {} (减少 {})", 
        stats_after_all.used_pages, 
        stats_after_partial.used_pages - stats_after_all.used_pages
    );
    println!("  堆字节数: {} (减少 {})", 
        stats_after_all.heap_bytes, 
        stats_after_partial.heap_bytes - stats_after_all.heap_bytes
    );
    
    // 最终的 free lists 状态
    println!("\n最终的 free lists 信息:");
    let final_info = global.get_free_lists_info();
    print!("{}", final_info);
    
    println!("\n内存使用总结:");
    println!("  总内存: {} MB", size / (1024 * 1024));
    println!("  已用内存: {} MB", 
        (stats_after_all.used_pages * 4096 + stats_after_all.slab_bytes + stats_after_all.heap_bytes) / (1024 * 1024)
    );
    println!("  空闲内存: {} MB", 
        (stats_after_all.free_pages * 4096) / (1024 * 1024)
    );
}

/// 辅助函数：简单的 println 替代（用于 no_std 环境）
#[macro_export]
macro_rules! debug_print {
    ($($arg:tt)*) => {
        // 在真实环境中，这里应该使用适当的日志输出
        // 现在只是作为占位符
    };
}
