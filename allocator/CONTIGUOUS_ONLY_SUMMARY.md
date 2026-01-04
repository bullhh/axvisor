# 内存分配器优化总结

## 概述

移除了非连续分配功能，现在分配器保证所有分配的内存都是**物理连续**的。

## 架构分层

```
┌─────────────────────────────────────────────────┐
│         global_allocator.rs               │
│  - 协调 buddy 和 slab                │
│  - 暴露 API 给上层                   │
└──────────────┬──────────────────────────┘
               │
       ┌───────┴────────┐
       │                │
┌──────▼──────┐  ┌──▼─────────────┐
│page_allocator │  │ buddy/        │
│ - 策略优化   │  │ - 标准实现    │
└──────┬──────┘  └────────────────┘
       │
       │
┌──────▼─────────────┐
│ slab_byte_allocator│
│ - 小对象分配       │
└──────────────────┘
```

## 核心改进

### 1. 两层分配策略

所有分配都保证**物理连续**：

| 策略 | 方法 | 特点 | 速度 |
|------|------|------|------|
| **快路径** | `buddy.alloc_pages()` | 单个连续块 | O(1) |
| **慢路径** | `try_combine_contiguous_blocks()` | 多个连续块组合 | O(n) |

### 2. 连续块组合算法

```rust
fn try_combine_contiguous_blocks(&mut self, num_pages: usize, align_pow2: usize) -> Option<usize>
```

**算法步骤**：
1. 从大到小遍历 buddy 各阶的空闲链表（order 18 → 0）
2. 利用有序链表的特性，高效检查块是否连续
3. 双向检查：
   - `block_end == min_addr`：块在左侧连续
   - `block_start == max_addr`：块在右侧连续
4. 收集足够的连续页后，分配所有块
5. 如果分配失败，回滚所有已分配的块

**示例**：
```
请求：1536 页（6MB）

空闲链表：
  Order 10 (1024 页): [0x1000]
  Order 9 (512 页):  [0x501000]  ← 与 0x1000 连续

结果：
  0x1000      (1024 页)
  0x501000    (512 页)
  └──────────────┘
     总共 1536 页，物理连续 ✓
```

### 3. 失败时的详细统计

```rust
fn print_alloc_failure_stats(&self, num_pages: usize, align_pow2: usize)
```

独立于分配逻辑的统计打印函数，包括：
- 请求大小和对齐要求
- buddy 分配器统计信息
- 各阶空闲块分布
- 打印位置：在所有分配策略失败后

### 4. 分配后断言检查

在所有成功分配路径中添加 `debug_assert!`：
- 标准分配后
- 连续块组合后

```rust
debug_assert!(actual_pages >= num_pages,
             "Allocated pages {} < requested pages {}",
             actual_pages, num_pages);
```

## 移除的功能

### 非连续分配（已删除）

以下方法和结构已被完全移除：

1. **`alloc_composite()` 方法**
   - 功能：分解请求为多个 2 的幂次块，不检查连续性
   - 问题：可能返回物理不连续的内存

2. **`find_best_orders()` 方法**
   - 功能：贪心算法找到最优的块组合
   - 问题：仅服务于非连续分配

3. **`dealloc_composite()` 方法**
   - 功能：释放组合分配的所有块
   - 问题：已不需要，现在所有分配都通过 buddy 释放

4. **`is_composite_allocation()` 方法**
   - 功能：检查地址是否为组合分配
   - 问题：已不需要，现在所有分配都是连续的

5. **`CompositeAllocation` 结构体**
   - 功能：跟踪组合分配的元数据
   - 字段：
     - `base_addr`: 基地址
     - `total_pages`: 总页数
     - `parts`: 组成块列表 [(addr, order); MAX_PARTS_PER_ALLOC]
     - `num_parts`: 有效块数量
     - `used`: 是否使用中
   - 问题：已不需要，连续块分配不需要跟踪多个部分

6. **常量**
   - `MAX_COMPOSITE_ALLOCS = 64`（最大并发组合分配数）

## 更新的文件

| 文件 | 变更 |
|------|------|
| `page_allocator.rs` | 完全重写，移除所有非连续分配逻辑 |
| `global_allocator.rs` | 移除 `get_composite_stats()` 和 `CompositeStats` 导入 |
| `lib.rs` | 移除 `CompositeStats` 导出 |

## 保留的功能

### Buddy 分配器增强

在 `buddy/buddy_allocator.rs` 中添加：

```rust
/// 获取区域数量
pub fn get_zone_count(&self) -> usize

/// 获取指定区域和阶的空闲块迭代器
pub fn get_free_blocks_by_order(&self, zone_id: usize, order: u32)
    -> Option<impl Iterator<Item = &BuddyBlock>>
```

在 `buddy/buddy_set.rs` 中添加：

```rust
/// 获取指定阶的空闲块迭代器
pub fn get_free_blocks_by_order(&self, order: u32)
    -> impl Iterator<Item = &BuddyBlock>
```

### PageAllocator 特征

```rust
fn alloc_pages(&mut self, num_pages: usize, align_pow2: usize) -> AllocResult<usize>
fn dealloc_pages(&mut self, pos: usize, num_pages: usize)
fn alloc_pages_at(&mut self, base: usize, num_pages: usize, align_pow2: usize) -> AllocResult<usize>
fn total_pages(&self) -> usize
fn used_pages(&self) -> usize
fn available_pages(&self) -> usize
```

## API 变更

### 删除的 API

```rust
// 不再提供
pub struct CompositeStats { ... }
pub fn get_composite_stats(&self) -> CompositeStats
```

### 保留的 API

```rust
// Buddy 统计
pub struct BuddyStats { ... }
pub fn get_buddy_stats(&self) -> BuddyStats

// 全局统计
pub struct UsageStats { ... }
pub fn get_stats(&self) -> UsageStats

// 详细信息
pub fn get_free_lists_info(&self) -> alloc::string::String
```

## 编译状态

```bash
cargo check
```

**结果**：✅ 编译成功，仅有 2 个警告（与本次修改无关）

```
warning: unused variable: `start`
   --> allocator/src/slab_byte_allocator.rs:461:24
```

## 测试

更新了测试用例：

```rust
#[test]
fn test_contiguous_allocator_basic() {
    let mut allocator = CompositePageAllocator::new();
    allocator.init(0x80000000, 0x10000000); // 256MB

    // 测试标准分配（2 的幂次）
    let addr1 = allocator.alloc_pages(1024, PAGE_SIZE).unwrap();
    assert!(addr1 >= 0x80000000);

    allocator.dealloc_pages(addr1, 1024);
}

#[test]
fn test_allocator_stats() {
    let mut allocator = CompositePageAllocator::new();
    allocator.init(0x80000000, 0x10000000);

    let buddy_stats = allocator.get_buddy_stats();
    assert!(buddy_stats.total_pages > 0);
    assert!(buddy_stats.free_pages > 0);
}
```

## 总结

1. **保证连续性**：所有分配都保证物理连续
2. **简化代码**：移除了约 200 行非必要代码
3. **清晰分层**：buddy（标准）→ page（策略）→ global（适配）
4. **性能优先**：快路径 O(1)，慢路径 O(n) 但保证连续性
5. **易于维护**：职责分离，逻辑清晰
