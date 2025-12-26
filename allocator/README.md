# Axvisor 内存分配器

这是一个为 Axvisor 虚拟机监控器设计的高性能内存分配器，参考了 asterinas 的内存分配器实现。

## 特性

- **Buddy 页分配器**: 高效的页级内存分配，支持自动合并
- **Slab 字节分配器**: 优化的小对象分配，支持多级缓存
- **全局分配器**: 协调页分配器和字节分配器，提供统一接口
- **内存追踪**: 完整的分配追踪和泄漏检测功能
- **线程安全**: 使用自旋锁保证多核环境下的安全性

## 架构

```
┌─────────────────────────────────────┐
│        GlobalAllocator              │
├─────────────────────────────────────┤
│  ┌─────────────┐  ┌─────────────┐  │
│  │SlabAllocator│  │BuddyAllocator│ │
│  │             │  │             │ │
│  │ Small objs  │  │   Pages     │ │
│  │ ≤2KB        │  │   4KB+      │ │
│  └─────────────┘  └─────────────┘  │
└─────────────────────────────────────┘
```

## 快速开始

### 基本使用

```rust
use axvisor_allocator::{GlobalAllocator, PageAllocator, ByteAllocator};
use core::alloc::Layout;

// 创建全局分配器
let global = GlobalAllocator::new();

// 初始化内存池
global.init(0x80000000, 0x1000000).unwrap(); // 16MB

// 小对象分配 (使用 Slab)
let small_layout = Layout::from_size_align(64, 8).unwrap();
let small_ptr = global.alloc(small_layout).unwrap();

// 大对象分配 (使用 Buddy)
let large_layout = Layout::from_size_align(0x1000, 0x1000).unwrap();
let large_ptr = global.alloc(large_layout).unwrap();

// 释放内存
global.dealloc(small_ptr, small_layout);
global.dealloc(large_ptr, large_layout);
```

### 页分配器直接使用

```rust
use axvisor_allocator::BuddyPageAllocator;
use core::alloc::Layout;

let mut buddy = BuddyPageAllocator::new();
buddy.init(0x80000000, 0x100000); // 1MB

// 分配页面
let page_addr = buddy.alloc_pages(1, 12).unwrap(); // 1页，4KB对齐

// 释放页面
buddy.dealloc_pages(page_addr, 1);
```

### Slab 分配器直接使用

```rust
use axvisor_allocator::{SlabByteAllocator, BuddyPageAllocator, PageAllocatorForSlab};
use core::alloc::Layout;

let mut slab = SlabByteAllocator::new();
let mut buddy = BuddyPageAllocator::new();
buddy.init(0x80000000, 0x10000); // 64KB

// 设置页分配器
slab.set_page_allocator(&mut buddy as *mut dyn PageAllocatorForSlab);

// 分配小对象
let layout = Layout::from_size_align(64, 8).unwrap();
let ptr = slab.alloc(layout).unwrap();

// 释放对象
slab.dealloc(ptr, layout);
```

## 内存追踪

启用内存追踪功能来监控分配情况：

```rust
use axvisor_allocator::{enable_tracking, disable_tracking, get_overall_stats, print_memory_report};

// 启用追踪
enable_tracking();

// ... 执行分配操作 ...

// 获取统计信息
let stats = get_overall_stats();
println!("总分配: {} 次", stats.total_allocations);
println!("当前内存使用: {} 字节", stats.current_memory_usage);

// 打印详细报告
print_memory_report();

// 禁用追踪
disable_tracking();
```

## API 参考

### GlobalAllocator

主要的分配器接口，自动选择最优的分配策略。

#### 方法

- `new()` - 创建新的全局分配器
- `init(start_vaddr, size)` - 初始化内存池
- `alloc(layout)` - 分配内存
- `dealloc(ptr, layout)` - 释放内存
- `alloc_pages(num_pages, align_pow2)` - 分配页面
- `dealloc_pages(pos, num_pages)` - 释放页面
- `get_stats()` - 获取使用统计

### BuddyPageAllocator

基于 Buddy 算法的页分配器。

#### 方法

- `new()` - 创建新的 Buddy 分配器
- `init(start, size)` - 初始化内存池
- `alloc_pages(num_pages, align_pow2)` - 分配页面
- `dealloc_pages(pos, num_pages)` - 释放页面
- `get_stats()` - 获取统计信息

### SlabByteAllocator

基于 Slab 算法的小对象分配器。

#### 方法

- `new()` - 创建新的 Slab 分配器
- `set_page_allocator(page_allocator)` - 设置页分配器
- `alloc(layout)` - 分配小对象
- `dealloc(ptr, layout)` - 释放小对象

## 性能特性

- **快速分配**: 小对象分配通常在 O(1) 时间内完成
- **内存效率**: Buddy 算法有效减少外部碎片
- **自动合并**: 释放的页面会自动合并，减少碎片
- **缓存友好**: Slab 分配器提供多级缓存，提高局部性

## 配置选项

- `SLAB_CACHE_SIZE`: Slab 缓存大小
- `MAX_ORDER`: Buddy 分配器最大阶数
- `PAGE_SIZE`: 页面大小 (默认 4KB)

## 测试

运行测试套件：

```bash
cargo test --test integration_test
```

## 示例

查看 `examples/` 目录中的示例代码：

- `basic_usage.rs` - 基本使用示例
- `memory_tracking.rs` - 内存追踪示例
- `performance_test.rs` - 性能测试示例

## 贡献

欢迎提交 Issue 和 Pull Request 来改进这个分配器。

## 许可证

本项目遵循与 Axvisor 相同的许可证。
