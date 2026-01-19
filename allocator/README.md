# buddy-slab-allocator 内存分配器

内存分配器，提供高效的页级和字节级内存管理。

## 特性

- **Buddy 页分配器**: 页级内存分配
- **Slab 字节分配器**: 小对象分配
- **复合页分配器**: 统一的多区域页分配接口
- **全局分配器**: 协调页分配器和字节分配器，提供统一的分配接口
- **零 `std` 依赖**: 完全 `#![no_std]`，适合嵌入式和内核环境
- **条件日志**: 支持 `log` feature 启用日志，默认无依赖


## 快速开始

### 添加依赖

```toml
[dependencies]
buddy-slab-allocator = { path = "allocator" }

# 可选功能
buddy-slab-allocator = { path = "allocator", features = ["log"] }    # 启用日志
buddy-slab-allocator = { path = "allocator", features = ["tracking"] } # 启用追踪
```

### 基本使用

#### 使用全局分配器

```rust
use buddy_slab_allocator::GlobalAllocator;
use core::alloc::Layout;

// 创建全局分配器
let mut global = GlobalAllocator::new();

// 初始化内存池（添加第一个区域）
global.add_memory(0x80000000, 0x1000000).unwrap(); // 16MB

// 小对象分配 (自动使用 Slab)
let small_layout = Layout::from_size_align(64, 8).unwrap();
let small_ptr = global.alloc(small_layout).unwrap();

// 大对象分配 (自动使用页分配器)
let large_layout = Layout::from_size_align(0x1000, 0x1000).unwrap();
let large_ptr = global.alloc(large_layout).unwrap();

// 释放内存
global.dealloc(small_ptr, small_layout);
global.dealloc(large_ptr, large_layout);
```

#### 使用复合页分配器

```rust
use buddy_slab_allocator::CompositePageAllocator;

let mut allocator = CompositePageAllocator::new();

// 添加多个内存区域
allocator.add_memory(0x80000000, 0x1000000, 0x1000).unwrap(); // 区域1: 16MB, 4KB页
allocator.add_memory(0x90000000, 0x2000000, 0x200000).unwrap(); // 区域2: 32MB, 2MB页

// 分配页面
let page_addr = allocator.alloc_pages(1, 0x1000).unwrap();

// 释放页面
allocator.dealloc_pages(page_addr, 1);
```

#### 使用 Buddy 页分配器

```rust
use buddy_slab_allocator::BuddyPageAllocator;

let mut buddy = BuddyPageAllocator::new();

// 初始化
buddy.init(0x80000000, 0x100000); // 1MB

// 分配页面
let page_addr = buddy.alloc_pages(1, 0x1000).unwrap(); // 1页，4KB对齐

// 释放页面
buddy.dealloc_pages(page_addr, 1);
```

#### 使用 Slab 字节分配器

```rust
use buddy_slab_allocator::{SlabByteAllocator, PageAllocatorForSlab};
use core::alloc::Layout;

let mut slab = SlabByteAllocator::new();

// 设置页分配器（需要实现 PageAllocatorForSlab trait）
slab.set_page_allocator(&mut page_allocator as *mut dyn PageAllocatorForSlab);

// 分配小对象
let layout = Layout::from_size_align(64, 8).unwrap();
let ptr = slab.alloc(layout).unwrap();

// 释放对象
slab.dealloc(ptr, layout);
```

## API 参考

### 核心接口

#### GlobalAllocator

统一的全局分配接口，自动选择最优分配策略。

**方法**
- `new()` - 创建新的全局分配器
- `add_memory(start, size)` - 添加内存区域
- `alloc(layout)` - 分配内存（自动选择页/字节分配器）
- `dealloc(ptr, layout)` - 释放内存
- `alloc_pages(num_pages, alignment)` - 分配页面
- `dealloc_pages(pos, num_pages)` - 释放页面
- `get_stats()` - 获取使用统计（需 `tracking` feature）

#### CompositePageAllocator

支持多区域和可配置页面大小的复合页分配器。

**方法**
- `new()` - 创建新的复合分配器
- `add_memory(start, size, page_size)` - 添加内存区域（可指定页大小）
- `alloc_pages(num_pages, alignment)` - 分配页面
- `dealloc_pages(pos, num_pages)` - 释放页面
- `alloc_pages_at(base, num_pages, alignment)` - 在指定地址分配页面

#### BuddyPageAllocator

经典的 Buddy 算法页分配器。

**方法**
- `new()` - 创建新的 Buddy 分配器
- `init(start, size)` - 初始化内存池
- `add_memory(start, size)` - 添加内存区域
- `alloc_pages(num_pages, alignment)` - 分配页面
- `dealloc_pages(pos, num_pages)` - 释放页面
- `get_stats()` - 获取统计信息（需 `tracking` feature）

#### SlabByteAllocator

基于 Slab 算法的小对象分配器，支持多级大小类。

**方法**
- `new()` - 创建新的 Slab 分配器
- `set_page_allocator(page_allocator)` - 设置页分配器
- `alloc(layout)` - 分配小对象
- `dealloc(ptr, layout)` - 释放小对象

### Trait 定义

#### BaseAllocator

所有分配器的基础 trait。

```rust
pub trait BaseAllocator {
    fn init(&mut self, start: usize, size: usize);
    fn add_memory(&mut self, start: usize, size: usize) -> AllocResult;
}
```

#### ByteAllocator

字节粒度分配器 trait。

```rust
pub trait ByteAllocator {
    fn alloc(&mut self, layout: Layout) -> AllocResult<NonNull<u8>>;
    fn dealloc(&mut self, pos: NonNull<u8>, layout: Layout);
    fn total_bytes(&self) -> usize;
    fn used_bytes(&self) -> usize;
    fn available_bytes(&self) -> usize;
}
```

#### PageAllocator

页粒度分配器 trait。

```rust
pub trait PageAllocator: BaseAllocator {
    const PAGE_SIZE: usize;
    fn alloc_pages(&mut self, num_pages: usize, alignment: usize) -> AllocResult<usize>;
    fn dealloc_pages(&mut self, pos: usize, num_pages: usize);
    fn alloc_pages_at(&mut self, base: usize, num_pages: usize, alignment: usize) -> AllocResult<usize>;
    fn total_pages(&self) -> usize;
    fn used_pages(&self) -> usize;
    fn available_pages(&self) -> usize;
}
```

## 特性详解

### 条件日志

支持通过 feature gate 启用日志功能：

```toml
[features]
default = []
log = ["dep:log"]
```

启用后可使用标准 `log` crate 的宏：
```rust
log::info!("Allocated memory at {:x}", ptr);
log::error!("Allocation failed: {:?}", err);
```

未启用时，日志调用会被编译为空操作，零运行时开销。

### 内存追踪

启用 `tracking` feature 后，分配器会收集详细的统计信息：

```rust
#[cfg(feature = "tracking")]
{
    let stats = buddy.get_stats();
    println!("Total allocs: {}", stats.total_allocs);
    println!("Current usage: {} bytes", stats.used_bytes);
    println!("Free blocks: {}", stats.free_blocks);
}
```

### 多区域支持

`CompositePageAllocator` 支持添加多个内存区域，每个区域可配置不同的页面大小：

```rust
allocator.add_memory(0x80000000, 0x10000000, 0x1000).unwrap(); // 256MB, 4KB页
allocator.add_memory(0x90000000, 0x10000000, 0x200000).unwrap(); // 256MB, 2MB页
```

分配器会根据请求的对齐要求和大小自动选择最合适的区域。

## 配置选项

### 编译时常量

- `DEFAULT_PAGE_SIZE`: 默认页面大小（0x1000 = 4KB）
- `DEFAULT_MAX_ORDER`: Buddy 分配器最大阶数
- `MAX_ZONES`: 最大支持的区域数

### Slab 大小类

Slab 分配器支持多个预定义的大小类（参见 `SizeClass`）：
- 16B, 32B, 64B, 128B, 256B, 512B
- 1KB, 2KB, 4KB, 8KB, 16KB, 32KB, 64KB

## 性能特性

- **快速分配**: 小对象分配 O(1) 时间复杂度
- **内存效率**: Buddy 算法有效减少外部碎片
- **自动合并**: 释放的页面自动合并，减少碎片
- **缓存友好**: Slab 多级缓存提高局部性
- **零 `std` 依赖**: 完全 `no_std`，适合内核环境
- **无锁设计**（分配路径）：快速路径无需加锁

## 测试

运行测试套件：

```bash
# 运行所有测试
cargo test --package buddy-slab-allocator

# 运行特定测试
cargo test --package buddy-slab-allocator --test integration_test

# 启用日志运行测试
cargo test --package buddy-slab-allocator --features log
```

## 文档

详细的实现文档请查看 `doc/` 目录：

- `BUDDY_IMPLEMENTATION.md` - Buddy 分配器实现细节
- `MULTI_ZONE_IMPLEMENTATION.md` - 多区域实现
- `LIST_POOL_IMPLEMENTATION.md` - 池化链表实现
- `COMPOSITE_ALLOCATOR_DESIGN.md` - 复合分配器设计
- `STATS_ACCURACY_TEST.md` - 统计准确性测试

## 错误处理

所有分配操作返回 `AllocResult<T>`：

```rust
pub enum AllocError {
    InvalidParam,    // 无效参数
    MemoryOverlap,   // 内存重叠
    NoMemory,        // 内存不足
    NotAllocated,    // 释放未分配的内存
}
```

## 最佳实践

1. **大小选择**: ≤2KB 使用 Slab 分配器，>2KB 使用页分配器
2. **对齐要求**: 始终提供正确的 `Layout::align`
3. **内存释放**: 确保 `dealloc` 的 layout 与 `alloc` 时一致
4. **多区域**: 使用 `CompositePageAllocator` 管理非连续物理内存
5. **日志**: 生产环境编译时可禁用 `log` feature 减小二进制大小

## 贡献

欢迎提交 Issue 和 Pull Request。

## 许可证

GPL-3.0-or-later OR Apache-2.0 OR MulanPSL-2.0

## 参考资料

- Asterinas 内存分配器实现
- Linux Slab Allocator
- Buddy System 算法
