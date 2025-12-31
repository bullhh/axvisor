# 多内存区域Buddy分配器

## 快速开始

### 基本用法

```rust
use axvisor_allocator::{BuddyPageAllocator, PageAllocator, BaseAllocator};

let mut allocator = BuddyPageAllocator::new();

// 初始化第一个内存区域 (zone 0)
allocator.init(0x8000_0000, 0x0100_0000);  // 16MB

// 添加额外的非连续内存区域
allocator.add_memory(0x9000_0000, 0x0100_0000)?;  // zone 1: 16MB
allocator.add_memory(0xA000_0000, 0x0080_0000)?;  // zone 2: 8MB

// 分配 - 自动选择合适的zone
let addr = allocator.alloc_pages(1024, 4096)?;

// 释放 - 自动定位正确的zone
allocator.dealloc_pages(addr, 1024);
```

### 获取统计信息

```rust
// 获取总体统计
let stats = allocator.get_stats();
println!("Total: {} pages", stats.total_pages);
println!("Free: {} pages", stats.free_pages);
println!("Used: {} pages", stats.used_pages);

// 获取详细的zone信息
println!("{}", allocator.get_free_lists_info());
```

## 设计特点

1. **多zone支持**: 最多支持8个独立的内存区域
2. **自动zone管理**: 分配/释放时自动处理zone选择和定位
3. **Linux风格**: 参考Linux内核ZONE机制的成熟实现
4. **强验证**: 地址验证、重叠检测、zone边界检查
5. **完整统计**: 每个zone独立统计 + 全局汇总

## 文档

- `MULTI_ZONE_DESIGN.md`: 详细设计文档
- `MULTI_ZONE_IMPLEMENTATION.md`: 实现总结

## 测试

运行测试：
```bash
cargo test -p axvisor-allocator
```

## 架构概览

```
BuddyPageAllocator (多zone管理器)
├── Zone 0 (BuddySet)
│   ├── base_addr: 0x80000000
│   ├── end_addr: 0x81000000
│   └── free_lists[0..28]
├── Zone 1 (BuddySet)
│   ├── base_addr: 0x90000000
│   ├── end_addr: 0x91000000
│   └── free_lists[0..28]
└── Zone 2 (BuddySet)
    ├── base_addr: 0xA0000000
    ├── end_addr: 0xA0800000
    └── free_lists[0..28]
```
