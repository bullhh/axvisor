//! Axvisor 内存分配器基本使用示例
//! 
//! 这个示例展示了如何使用新的内存分配器进行基本的内存分配操作。

#![no_std]
#![no_main]

extern crate alloc;

use axvisor_allocator::{
    GlobalAllocator, BuddyPageAllocator, SlabByteAllocator,
    PageAllocator, ByteAllocator, BaseAllocator, PageAllocatorForSlab,
    enable_tracking, disable_tracking, print_memory_report,
};
use core::alloc::Layout;

#[no_mangle]
pub fn main() {
    println!("=== Axvisor 内存分配器基本使用示例 ===");
    
    // 示例1: 使用全局分配器
    demo_global_allocator();
    
    // 示例2: 使用页分配器
    demo_page_allocator();
    
    // 示例3: 使用 Slab 分配器
    demo_slab_allocator();
    
    // 示例4: 内存追踪功能
    demo_memory_tracking();
    
    println!("=== 示例完成 ===");
}

fn demo_global_allocator() {
    println!("\n--- 全局分配器演示 ---");
    
    let mut global = GlobalAllocator::new();
    
    // 初始化内存池 (16MB)
    global.init(0x80000000, 0x1000000).expect("初始化失败");
    
    // 小对象分配 (使用 Slab)
    let small_layout = Layout::from_size_align(64, 8).unwrap();
    let small_ptr1 = global.alloc(small_layout).expect("小对象分配失败");
    let small_ptr2 = global.alloc(small_layout).expect("小对象分配失败");
    
    println!("分配了两个 64 字节的小对象");
    println!("指针1: {:p}", small_ptr1);
    println!("指针2: {:p}", small_ptr2);
    
    // 大对象分配 (使用 Buddy)
    let large_layout = Layout::from_size_align(0x1000, 0x1000).unwrap();
    let large_ptr = global.alloc(large_layout).expect("大对象分配失败");
    
    println!("分配了一个 4KB 的大对象");
    println!("指针3: {:p}", large_ptr);
    
    // 获取统计信息
    let stats = global.get_stats();
    println!("内存统计:");
    println!("  总页数: {}", stats.total_pages);
    println!("  已用页数: {}", stats.used_pages);
    println!("  空闲页数: {}", stats.free_pages);
    println!("  Slab 字节: {}", stats.slab_bytes);
    println!("  堆字节: {}", stats.heap_bytes);
    
    // 释放内存
    global.dealloc(small_ptr1, small_layout);
    global.dealloc(small_ptr2, small_layout);
    global.dealloc(large_ptr, large_layout);
    
    println!("已释放所有分配的内存");
}

fn demo_page_allocator() {
    println!("\n--- 页分配器演示 ---");
    
    let mut buddy = BuddyPageAllocator::new();
    
    // 初始化 1MB 内存
    buddy.init(0x90000000, 0x100000);
    
    // 分配不同大小的页面块
    let page1 = buddy.alloc_pages(1, 12).expect("分配1页失败");
    let page2 = buddy.alloc_pages(2, 12).expect("分配2页失败");
    let page4 = buddy.alloc_pages(4, 12).expect("分配4页失败");
    
    println!("分配了 1+2+4 = 7 页");
    println!("页面1地址: {:#x}", page1);
    println!("页面2地址: {:#x}", page2);
    println!("页面4地址: {:#x}", page4);
    
    // 获取 Buddy 统计
    let buddy_stats = buddy.get_stats();
    println!("Buddy 统计:");
    for (order, count) in buddy_stats.free_blocks.iter().enumerate() {
        if *count > 0 {
            println!("  阶{}: {} 个空闲块", order, count);
        }
    }
    
    // 释放页面（注意：应该按分配时的实际大小释放）
    buddy.dealloc_pages(page1, 1);
    buddy.dealloc_pages(page2, 2);
    buddy.dealloc_pages(page4, 4);
    
    println!("已释放所有页面");
}

fn demo_slab_allocator() {
    println!("\n--- Slab 分配器演示 ---");
    
    let mut slab = SlabByteAllocator::new();
    let mut buddy = BuddyPageAllocator::new();
    
    // 初始化 Buddy 分配器作为 Slab 的页分配器
    buddy.init(0xA0000000, 0x10000); // 64KB
    
    // 设置页分配器
    slab.set_page_allocator(&mut buddy as *mut dyn PageAllocatorForSlab);
    
    // 测试不同大小的对象分配
    let sizes = [8, 16, 32, 64, 128, 256, 512, 1024, 2048];
    let mut allocated_ptrs = alloc::vec::Vec::new();
    
    for &size in &sizes {
        let layout = Layout::from_size_align(size, 8).unwrap();
        match slab.alloc(layout) {
            Ok(ptr) => {
                allocated_ptrs.push((ptr, layout));
                println!("分配了 {} 字节的对象: {:p}", size, ptr);
            }
            Err(e) => {
                println!("分配 {} 字节失败: {:?}", size, e);
            }
        }
    }
    
    // 释放所有分配的对象
    for (ptr, layout) in allocated_ptrs {
        slab.dealloc(ptr, layout);
        println!("释放了 {} 字节的对象", layout.size());
    }
    
    println!("已释放所有 Slab 对象");
}

fn demo_memory_tracking() {
    println!("\n--- 内存追踪演示 ---");
    
    // 启用追踪
    enable_tracking();
    println!("已启用内存追踪");
    
    let mut global = GlobalAllocator::new();
    global.init(0xB0000000, 0x100000).expect("初始化失败");
    
    // 执行一些分配操作
    let layout1 = Layout::from_size_align(64, 8).unwrap();
    let layout2 = Layout::from_size_align(1024, 8).unwrap();
    
    let ptr1 = global.alloc(layout1).expect("分配失败");
    let ptr2 = global.alloc(layout2).expect("分配失败");
    
    println!("分配了两个对象用于追踪演示");
    
    // 获取追踪统计
    let overall_stats = axvisor_allocator::get_overall_stats();
    println!("追踪统计:");
    println!("  总分配次数: {}", overall_stats.total_allocations);
    println!("  当前内存使用: {} 字节", overall_stats.current_memory_usage);
    println!("  峰值内存使用: {} 字节", overall_stats.peak_memory_usage);
    
    // 释放一个对象
    global.dealloc(ptr1, layout1);
    
    let updated_stats = axvisor_allocator::get_overall_stats();
    println!("释放一个对象后:");
    println!("  总分配次数: {}", updated_stats.total_allocations);
    println!("  当前内存使用: {} 字节", updated_stats.current_memory_usage);
    
    // 打印详细报告
    print_memory_report();
    
    // 释放剩余对象
    global.dealloc(ptr2, layout2);
    
    // 禁用追踪
    disable_tracking();
    println!("已禁用内存追踪");
}
