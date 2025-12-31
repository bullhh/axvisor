# CompositePageAllocator 设计说明

## 概述

`CompositePageAllocator` 是一个页面级内存分配器，通过复合分配策略解决了传统 buddy 系统的局限性。

## 解决的问题

### 问题 1：Buddy 系统的幂次约束

传统 buddy 系统只能分配 2^n 大小的块：
- 请求 1536MB → 需要分配 2048MB (浪费 512MB)
- 即使有足够的总内存，也可能分配失败

### 问题 2：地址对齐限制

即使有连续的物理内存，如果不能形成单个 2^n 块（由于对齐限制），也无法分配：
- Zone 2 有 2 个 Order 18 (1024MB) 块
- 但没有 2GB 对齐地址
- 无法形成 Order 19 块

## 解决方案

### 复合分配策略

当标准 buddy 分配失败时，将请求分解为多个 2^n 块：

```
请求: 1536MB (393216 页)
分解: 1 × Order 18 (1024MB) + 1 × Order 17 (512MB)
结果: 分配成功，无浪费
```

### 分配流程

```
1. 尝试标准 buddy 分配（快速路径）
   ├─ 成功 → 返回
   └─ 失败 → 继续步骤 2

2. 复合分配（慢速路径）
   ├─ 贪婪分解：使用最大的可能块
   │  例如: 1536 = 1024 + 512
   ├─ 逐个分配 buddy 块
   │  ├─ 如果全部成功 → 记录并返回
   │  └─ 如果失败 → 回滚所有已分配的块
   └─ 返回第一个块的地址
```

### 释放流程

```
1. 查找复合分配记录（通过基地址）
   ├─ 找到 → 释放所有组成块
   └─ 未找到 → 调用 buddy 标准释放
```

## 设计特点

### 1. 无动态分配

所有元数据使用静态数组：

```rust
composite_allocs: [CompositeAllocation; 64]
```

- 最多 64 个并发复合分配
- 每个分配最多 8 个 buddy 块
- 完全在栈上分配，无堆内存需求

### 2. 自动回退

对用户透明，自动选择最优策略：

```rust
// 用户请求 1536MB
let addr = allocator.alloc_pages(393216, 4096);

// 内部自动：
// - 先尝试标准 buddy 分配（失败）
// - 自动回退到复合分配（成功）
```

### 3. 错误处理

部分失败时自动回滚：

```rust
for (i, order) in orders.iter().enumerate() {
    let addr = self.buddy.alloc_pages(...) // 可能失败
        .map_err(|e| {
            // 回滚已分配的所有块
            for j in 0..i {
                self.buddy.dealloc_pages(parts[j].0, ...);
            }
            e
        })?;
    parts[i] = (addr, order);
}
```

## 性能分析

### 优点

1. **提高分配成功率**
   - 非幂次大小的请求也能成功
   - 充分利用可用内存

2. **减少内部碎片**
   - 精确匹配请求大小
   - 避免向上取整到 2^n

3. **低复杂度**
   - 快速路径：标准 buddy 分配 (O(log n))
   - 慢速路径：分解 + 多次分配 (O(k log n), k 为块数)

### 缺点

1. **物理内存不连续**
   - 返回的是多个不连续的块
   - 用户只能通过第一个块的地址访问

2. **释放复杂**
   - 需要查找复合分配记录
   - 涉及多次 buddy 释放

3. **元数据开销**
   - 静态数组占用内存
   - 64 × (sizeof(CompositeAllocation)) ≈ 64 × 80 bytes = 5KB

## 使用建议

### 何时使用 CompositePageAllocator

✅ **适合场景**:
- 需要分配非 2^n 大小的内存
- 内存总量足够但无法形成单个大块
- 可以接受物理内存不连续
- 需要高内存利用率

❌ **不适合场景**:
- 必须要求物理内存连续
- 对性能极度敏感（复合分配较慢）
- 系统内存充足，无需优化

### 与 BuddyPageAllocator 对比

| 特性 | BuddyPageAllocator | CompositePageAllocator |
|------|-------------------|----------------------|
| 幂次约束 | 必须是 2^n | 无约束 |
| 物理连续 | 是 | 否 |
| 分配成功率 | 较低 | 高 |
| 内存利用率 | 中等 | 高 |
| 分配速度 | 快 | 快（标准）/ 中（复合） |
| 适用场景 | 通用 | 内存受限环境 |

## 未来优化方向

### 1. VM 层支持

```rust
// 虚拟内存连续，物理内存可以不连续
let vaddr = allocator.alloc_vaddr(num_pages);
// 内部映射多个物理块到连续虚拟空间
```

### 2. 智能分配策略

```rust
// 根据请求大小选择最优分解策略
if num_pages.is_power_of_two() {
    buddy_alloc();  // 标准分配
} else if num_pages % (1 << 17) == 0 {
    // 大量 512MB 块
    composite_alloc_large_blocks();
} else {
    // 其他情况
    composite_alloc_balanced();
}
```

### 3. CPU 本地缓存

参考 Asterinas 的三层架构：
- CPU 本地缓存：1-4 页
- CPU 本地池：≤18 阶
- 全局池：所有阶

## 示例代码

```rust
use axvisor_allocator::CompositePageAllocator;

// 初始化
let mut allocator = CompositePageAllocator::new();
allocator.init(0x80000000, 0x10000000); // 256MB

// 标准分配（幂次大小）
let addr1 = allocator.alloc_pages(1024, 4096).unwrap(); // 4MB

// 复合分配（非幂次大小）
let addr2 = allocator.alloc_pages(1536, 4096).unwrap(); // 6MB

// 释放（自动检测类型）
allocator.dealloc_pages(addr1, 1024);
allocator.dealloc_pages(addr2, 1536);

// 获取统计
let stats = allocator.get_composite_stats();
println!("Composite allocations: {}", stats.active_allocations);
```

## 实现文件

- `allocator/src/page_allocator.rs` - 核心实现
- `allocator/src/lib.rs` - 导出
- `allocator/examples/composite_demo.rs` - 示例程序

## 总结

`CompositePageAllocator` 通过在 PageAllocator 层实现复合分配策略，成功解决了传统 buddy 系统在分配非幂次大小内存时的局限性。它保持了对现有 `BuddyPageAllocator` 的兼容性，提供了透明的自动回退机制，并且完全避免了动态内存分配，适合虚拟化等内存受限环境。

对于需要物理内存连续的场景，建议未来实现 VM 层；对于需要高性能的场景，可以结合 Asterinas 的三层架构（CPU 本地缓存 + 本地池 + 全局池）。
