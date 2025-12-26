# Axvisor 内存分配器调试指南

本指南介绍如何查看和调试 Axvisor 内存分配器的内部状态，特别是 free_lists 和内存剩余情况。

## 查看内存状态的方法

### 1. Buddy 分配器调试

#### 获取 free_lists 信息
```rust
use axvisor_allocator::BuddyPageAllocator;

let mut buddy = BuddyPageAllocator::new();
buddy.init(base_addr, size);

// 获取详细的 free lists 信息
let info = buddy.get_free_lists_info();
println!("{}", info);
```

#### 获取统计信息
```rust
let stats = buddy.get_stats();
println!("总页数: {}", stats.total_pages);
println!("空闲页数: {}", stats.free_pages);
println!("已用页数: {}", stats.used_pages);

// 查看各阶空闲块
for (order, &count) in stats.free_pages_by_order.iter().enumerate() {
    if count > 0 {
        println!("阶{} ({}页): {} 个空闲块", order, 1 << order, count);
    }
}
```

### 2. 全局分配器调试

#### 获取整体统计
```rust
use axvisor_allocator::GlobalAllocator;

let global = GlobalAllocator::new();
global.init(base_addr, size).unwrap();

let stats = global.get_stats();
println!("总页数: {}", stats.total_pages);
println!("已用页数: {}", stats.used_pages);
println!("空闲页数: {}", stats.free_pages);
println!("Slab 字节数: {}", stats.slab_bytes);
println!("堆字节数: {}", stats.heap_bytes);
```

#### 获取 Buddy 分配器的 free_lists 信息
```rust
let info = global.get_free_lists_info();
println!("{}", info);
```

#### 获取 Buddy 分配器详细统计
```rust
let buddy_stats = global.get_buddy_stats();
for (order, &count) in buddy_stats.free_pages_by_order.iter().enumerate() {
    if count > 0 {
        let block_size = (1 << order) * 4096; // 4KB pages
        println!("阶{}: {} 个块 (每个 {} KB)", 
            order, count, block_size / 1024);
    }
}
```

## 输出信息解读

### free_lists 信息格式
```
=== Buddy Free Lists Info ===
Base Address: 0x80000000
Total Pages: 256
Order 0: 4 blocks, 4096 bytes each, 4 total pages (16 KB, 0 MB)
Order 1: 2 blocks, 8192 bytes each, 4 total pages (16 KB, 0 MB)
Order 2: 1 blocks, 16384 bytes each, 4 total pages (16 KB, 0 MB)

Summary:
  Free pages: 12 / 256
  Used pages: 244
  Free memory: 48 KB / 1024 KB
================================
```

### 各字段含义
- **Order N**: 表示 2^N 页的块
  - Order 0: 1 页 (4KB)
  - Order 1: 2 页 (8KB)  
  - Order 2: 4 页 (16KB)
  - 以此类推

- **blocks**: 当前该阶的空闲块数量
- **Block addresses**: 空闲块的起始地址（前8个）

## 实用调试场景

### 1. 检查内存碎片化
```rust
let buddy_stats = global.get_buddy_stats();
let mut total_blocks = 0;
let mut fragmentation_score = 0;

for (order, &count) in buddy_stats.free_pages_by_order.iter().enumerate() {
    if count > 0 {
        total_blocks += count;
        fragmentation_score += count * (1 << order);
        println!("阶{}: {} 个块", order, count);
    }
}

// 碎片化程度：块数量相对于总页数的比例
let fragmentation_ratio = total_blocks as f64 / buddy_stats.free_pages as f64;
println!("碎片化程度: {:.2} (越高越碎片化)", fragmentation_ratio);
```

### 2. 监控内存使用趋势
```rust
fn monitor_memory(global: &GlobalAllocator) {
    let stats = global.get_stats();
    let total_mb = (stats.total_pages * 4096) / (1024 * 1024);
    let used_mb = (stats.used_pages * 4096) / (1024 * 1024);
    let usage_percent = (stats.used_pages as f64 / stats.total_pages as f64) * 100.0;
    
    println!("内存使用: {} MB / {} MB ({:.1}%)", 
        used_mb, total_mb, usage_percent);
    
    if usage_percent > 80.0 {
        println!("警告：内存使用率过高！");
    }
}
```

### 3. 检查 Buddy 合并效果
```rust
// 分配然后释放，检查是否正确合并
let addr = buddy.alloc_pages(4, 0).unwrap();
buddy.dealloc_pages(addr, 4);

let stats_after = buddy.get_stats();
println!("释放后的空闲页数: {}", stats_after.free_pages);

// 检查高阶块是否增加（表示合并成功）
for order in (2..=10).rev() {
    if stats_after.free_pages_by_order[order] > 0 {
        println!("成功合并到阶{}块", order);
        break;
    }
}
```

## 运行示例

项目包含以下调试示例：

1. **basic_allocator_usage.rs**: 基础使用示例
2. **debug_memory_allocator.rs**: 完整调试功能演示
3. **memory_debug_demo.rs**: 简化的调试演示

运行示例：
```bash
# 在项目根目录
cargo run --example memory_debug_demo
```

## 性能分析建议

1. **监控碎片化**: 定期检查 `get_free_lists_info()` 输出
2. **内存使用率**: 使用 `get_stats()` 监控整体使用情况
3. **分配失败**: 检查是否有足够的连续大块可用
4. **泄漏检测**: 使用追踪功能检查未释放的分配

## 常见问题诊断

### 问题：分配大内存失败
- 检查目标阶的空闲块数量
- 查看是否需要从高阶块分割
- 确认总空闲内存足够

### 问题：内存碎片化严重
- 检查低阶块数量是否过多
- 观察释放后是否正确合并
- 考虑内存整理策略

### 问题：内存使用率异常
- 检查 Slab vs Buddy 的使用分布
- 验证统计信息的一致性
- 查看是否有泄漏的分配

通过这些调试方法，你可以深入了解内存分配器的内部状态，快速定位问题并优化内存使用。
