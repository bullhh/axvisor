# 内存分配器实现

## 1. 概述

Axvisor作为基于ArceOS的Type-1型Hypervisor，其内存分配器实现借鉴了ArceOS的组件，且针对虚拟化环境进行了专门的优化和定制。本文档详细分析Axvisor内存分配器的完整实现，从系统启动初始化到具体分配算法的底层原理。

---

## 2. 初始化流程

### 2.1 Axvisor启动时序

内存分配器的初始化是Axvisor系统启动的关键步骤，必须在其他需要内存的组件初始化之前完成。

```rust
// axvisor/modules/axruntime/src/lib.rs - Hypervisor启动流程
#[cfg(feature = "alloc")]
init_allocator();  // ← 内存分配器初始化入口
```

### 2.2 完整的初始化时序

```
Axvisor Hypervisor启动流程：
┌─────────────────────────────────────────────────────────────────┐
│ 1. axhal::mem::init()                                           │
│    ├── ARM64硬件虚拟化初始化                                     │
│    ├── 内存嗅探完成，识别所有内存区域                              │
│    └── 建立物理到虚拟地址映射关系                                 │
├─────────────────────────────────────────────────────────────────┤
│ 2. #[cfg(feature = "alloc")] init_allocator()                   │  ← 关键步骤
│    ├── BSS段感知的内存区域选择                                    │
│    ├── 初始化主堆（最大FREE区域）                                 │
│    └── 添加其他可用内存区域                                       │
├─────────────────────────────────────────────────────────────────┤
│ 3. 获取内核地址空间信息                                           │
├─────────────────────────────────────────────────────────────────┤
│ 4. axmm::init_memory_management()                               │
│    ├── Stage-2页表初始化                                         │
│    ├── 虚拟化内存管理建立                                         │
│    └── VM地址空间支持                                            │
├─────────────────────────────────────────────────────────────────┤
│ 5. 虚拟化组件初始化                                              │
│    ├── axvcpu初始化                                             │
│    ├── axvm初始化                                               │
│    └─— 设备虚拟化准备                                            │
└─────────────────────────────────────────────────────────────────┘
```

## 3 内存分配器详解

### 3.1 分配器初始化

Axvisor支持Level-1和Level-2两种架构模式，它们的初始化逻辑有所不同。

#### 3.1.1 GlobalAllocator的初始化接口

```rust
// arceos/modules/axalloc/src/lib.rs:50-66
impl GlobalAllocator {
    /// Initialize the global allocator.
    pub fn init(&self, start_vaddr: usize, size: usize) {
        assert!(size > MIN_HEAP_SIZE);
        
        #[cfg(not(feature = "level-1"))]
        {
            // Level-2模式：两级分配器架构
            let init_heap_size = MIN_HEAP_SIZE; // 32KB
            
            // 1. 初始化页分配器，管理全部物理内存
            self.palloc.lock().init(start_vaddr, size);
            
            // 2. 从页分配器分配初始堆内存
            let heap_ptr = self
                .alloc_pages(init_heap_size / PAGE_SIZE, PAGE_SIZE)
                .unwrap();
            
            // 3. 使用分配的页内存初始化字节分配器
            self.balloc.lock().init(heap_ptr, init_heap_size);
        }
        
        #[cfg(feature = "level-1")]
        {
            // Level-1模式：单级分配器架构
            self.balloc.lock().init(start_vaddr, size);
        }
    }
}
```

#### 3.1.2 Level-2模式初始化详解

**Level-2架构初始化流程：**
```
Level-2初始化时序：
┌─────────────────────────────────────────────────────────────────┐
│ 1. 物理内存区域识别                                              │
│    ├── start_vaddr: 虚拟起始地址                                 │
│    ├── size: 总内存大小                                          │
│    └─— MIN_HEAP_SIZE: 最小堆大小要求 (32KB)                      │
├─────────────────────────────────────────────────────────────────┤
│ 2. 页分配器初始化                                                │
│    ├── palloc.init(start_vaddr, size)                           │
│    ├── 将全部物理内存交给页分配器管理                              │
│    └─— 页分配器负责4KB页面的分配和回收                             │
├─────────────────────────────────────────────────────────────────┤
│ 3. 初始堆内存分配                                                │
│    ├── alloc_pages(init_heap_size / PAGE_SIZE, PAGE_SIZE)       │
│    ├── 从页分配器申请32KB初始堆内存                               │
│    └─— heap_ptr: 堆内存的虚拟地址                                │
├─────────────────────────────────────────────────────────────────┤
│ 4. 字节分配器初始化                                               │
│    ├── balloc.init(heap_ptr, init_heap_size)                    │
│    ├── TLSF分配器在32KB堆中初始化                                 │
│    └─— 后续可动态扩展堆内存                                       │
└─────────────────────────────────────────────────────────────────┘
```

**页分配器的初始化（Level-2专用）：**
```rust
// allocator/src/bitmap.rs (实际的BitmapPageAllocator初始化)
impl<const PAGE_SIZE: usize> BaseAllocator for BitmapPageAllocator<PAGE_SIZE> {
    fn init(&mut self, start: usize, size: usize) {
        assert!(PAGE_SIZE.is_power_of_two());

        // 1. 对齐内存边界：确保页对齐
        // Range for real:  [align_up(start, PAGE_SIZE), align_down(start + size, PAGE_SIZE))
        let end = crate::align_down(start + size, PAGE_SIZE);
        let start = crate::align_up(start, PAGE_SIZE);
        self.total_pages = (end - start) / PAGE_SIZE;

        // 2. 计算基址偏移：用于1GB对齐的bitmap管理
        self.base = crate::align_down(start, MAX_ALIGN_1GB); // MAX_ALIGN_1GB = 1GB

        // 3. 初始化bitmap：设置可用页面范围
        // Range in bitmap: [start - self.base, start - self.base + total_pages * PAGE_SIZE)
        let start = start - self.base;
        let start_idx = start / PAGE_SIZE;

        // 将页面范围插入bitmap，标记为可用
        self.inner.insert(start_idx..start_idx + self.total_pages);
    }
}
```

#### 3.1.3 Level-1模式初始化详解

**Level-1架构初始化流程：**
```
Level-1初始化时序：
┌─────────────────────────────────────────────────────────────────┐
│ 1. 物理内存区域识别                                              │
│    ├── start_vaddr: 虚拟起始地址                                 │
│    ├── size: 总内存大小                                          │
│    └─— MIN_HEAP_SIZE: 最小堆大小要求 (32KB)                      │
├─────────────────────────────────────────────────────────────────┤
│ 2. TLSF字节分配器直接初始化                                       │
│    ├── balloc.init(start_vaddr, size)                           │
│    ├── TLSF直接管理全部内存区域                                   │
│    └─— 无需页分配器中间层                                         │
├─────────────────────────────────────────────────────────────────┤
│ 3. TlsfByteAllocator内部初始化                                   │
│    ├── 创建原始内存池slice                                       │
│    ├── 调用TLSF insert_free_block_ptr()                         │
│    └─— 设置统计信息                                              │
├─────────────────────────────────────────────────────────────────┤
│ 4. TLSF核心算法初始化                                            │
│    ├── 地址对齐处理                                              │
│    ├── 块分割和链接                                              │
│    ├── 创建块头和哨兵块                                          │
│    └─— 链接到FL/SL空闲列表                                       │
└─────────────────────────────────────────────────────────────────┘
```

**TlsfByteAllocator的实际初始化实现：**
```rust
// allocator/src/tlsf.rs (真实的TlsfByteAllocator初始化)
impl BaseAllocator for TlsfByteAllocator {
    fn init(&mut self, start: usize, size: usize) {
        // 1. 创建原始内存池slice
        unsafe {
            let pool = core::slice::from_raw_parts_mut(start as *mut u8, size);
            
            // 2. 将内存池插入TLSF分配器
            self.inner
                .insert_free_block_ptr(NonNull::new(pool).unwrap())
                .unwrap();
        }
        
        // 3. 设置总内存大小统计
        self.total_bytes = size;
    }
}
```

**TlsfByteAllocator的数据结构：**
```rust
// allocator/src/tlsf.rs
pub struct TlsfByteAllocator {
    inner: Tlsf<'static, u32, u32, 28, 32>, // max pool size: 32 * 2^28 = 8G
    total_bytes: usize,    // 总字节数
    used_bytes: usize,     // 已使用字节数
}
```

**TLSF内部的insert_free_block_ptr()详细实现：**
```rust
// rlsf/crates/rlsf/src/tlsf.rs (核心TLSF初始化算法)
pub unsafe fn insert_free_block_ptr(&mut self, block: NonNull<[u8]>) -> Option<NonZeroUsize> {
    let len = nonnull_slice_len(block);

    // 1. 地址对齐处理：确保起始地址满足TLSF粒度要求
    let unaligned_start = block.as_ptr() as *mut u8 as usize;
    let start = unaligned_start.wrapping_add(GRANULARITY - 1) & !(GRANULARITY - 1);

    // 2. 大小调整：减去对齐造成的偏移，验证最小大小
    let len = if let Some(x) = len
        .checked_sub(start.wrapping_sub(unaligned_start))
        .filter(|&x| x >= GRANULARITY * 2)  // 最小2个粒度单位
    {
        // 3. 向下对齐到粒度边界
        x & !(GRANULARITY - 1)
    } else {
        // 内存块太小，无法使用
        return None;
    };

    // 4. 调用对齐版本的处理函数
    let pool_len = self.insert_free_block_ptr_aligned(NonNull::new_unchecked(
        core::ptr::slice_from_raw_parts_mut(start as *mut u8, len),
    ))?;

    // 5. 返回实际使用的内存大小（包括对齐损失）
    Some(NonZeroUsize::new_unchecked(
        pool_len.get() + start.wrapping_sub(unaligned_start),
    ))
}
```

**insert_free_block_ptr_aligned()详细实现：**
```rust
// rlsf/crates/rlsf/src/tlsf.rs (对齐内存块处理)
pub(crate) unsafe fn insert_free_block_ptr_aligned(
    &mut self,
    block: NonNull<[u8]>,
) -> Option<NonZeroUsize> {
    let start = block.as_ptr() as *mut u8 as usize;
    let mut size = nonnull_slice_len(block);
    let mut cursor = start;

    // 6. 将大内存块分割为适合TLSF管理的小块
    while size >= GRANULARITY * 2 {
        // 计算当前块的大小（受MAX_POOL_SIZE限制）
        let chunk_size = if let Some(max_pool_size) = Self::MAX_POOL_SIZE {
            size.min(max_pool_size)
        } else {
            size
        };

        debug_assert_eq!(chunk_size % GRANULARITY, 0);

        // 7. 创建新的空闲块头
        let block = NonNull::new_unchecked(cursor as *mut FreeBlockHdr);

        // 8. 初始化块头信息
        *nn_field!(block, common) = BlockHdr {
            size: chunk_size - GRANULARITY,  // 减去头部大小
            prev_phys_block: None,
        };

        // 9. 在块末尾创建哨兵块（防止越界）
        let sentinel_block = BlockHdr::next_phys_block(nn_field!(block, common)).cast::<UsedBlockHdr>();
        *nn_field!(sentinel_block, common) = BlockHdr {
            size: GRANULARITY | SIZE_USED | SIZE_SENTINEL,
            prev_phys_block: Some(block.cast()),
        };

        // 10. 将空闲块链接到相应的FL/SL列表
        self.link_free_block(block, chunk_size - GRANULARITY);

        // 11. 移动到下一个内存位置
        size -= chunk_size;
        cursor = cursor.wrapping_add(chunk_size);
    }

    // 12. 返回处理的内存大小
    NonZeroUsize::new(cursor.wrapping_sub(start))
}
```

**Level-1初始化的技术特点：**

1. **单级管理**：TLSF直接管理全部内存，无中间层
2. **内存池模式**：将连续内存区域作为TLSF的内存池
3. **对齐保证**：严格的对齐处理确保TLSF算法正确性
4. **块分割**：大内存块自动分割为适合管理的小块
5. **哨兵保护**：每个块的末尾都有哨兵块防止越界

**Level-1 vs Level-2初始化对比：**

| 特性 | Level-1模式 | Level-2模式 |
|------|------------|------------|
| **初始化目标** | TLSF直接初始化 | 页分配器→页分配→TLSF |
| **内存管理** | TLSF管理全部内存 | 页分配器管理物理页，TLSF管理堆 |
| **初始化步骤** | 1步：TLSF初始化 | 3步：分层初始化 |
| **对齐处理** | TLSF内部处理 | 页分配器和TLSF双重处理 |
| **内存开销** | 仅TLSF元数据 | 页分配器+TLSF双重元数据 |


### 3.2 TLSF算法原理概述

TLSF（Two-Level Segregated Fit）是一种高效的内存分配算法，Axvisor通过rlsf crate使用该算法。其核心思想是通过两级分离结构实现O(1)时间复杂度的分配和释放。

**算法核心原理：两级分离结构**
- **第一级(FL)**：按大小范围粗分类，基于2的幂次
- **第二级(SL)**：在每个FL范围内细分类，提供精确大小匹配

**Axvisor中的配置：**
```rust
// rlsf/crates/rlsf/src/tlsf.rs:96
pub const GRANULARITY: usize = core::mem::size_of::<usize>() * 4; // 32字节 (aarch64)
pub const MAX_POOL_SIZE: Option<usize> = {
    let shift = GRANULARITY_LOG2 + FLLEN as u32; // 5 + 28 = 33
    Some(1 << shift) // 8GB
};

// Axvisor使用的TLSF实例类型
pub struct TlsfByteAllocator {
    inner: Tlsf<'static, u32, u32, 28, 32>, // FLLEN=28, SLLEN=32
    total_bytes: usize,
    used_bytes: usize,
}
```


### 3.3 Axvisor中的TLSF分配实现

Axvisor通过axalloc模块调用rlsf crate提供的TLSF算法实现，本节重点介绍实际调用路径和关键实现。

#### 3.3.1 实际分配调用路径

**Axvisor中的分配流程：**
```rust
// 1. Axvisor应用层调用
let layout = Layout::from_size_align(size, align)?;
let ptr = axalloc::alloc(layout)?; // ← 实际调用入口

// 2. arceos/modules/axalloc/src/lib.rs
#[cfg(feature = "level-1")]
impl GlobalAllocator {
    pub fn alloc(&self, layout: Layout) -> AllocResult<NonNull<u8>> {
        let mut balloc = self.balloc.lock();  // 获取TLSF分配器
        balloc.alloc(layout)                  // 委托给TLSF实现
    }
}

// 3. allocator/src/tlsf.rs(TlsfByteAllocator实现)
impl ByteAllocator for TlsfByteAllocator {
    fn alloc(&mut self, layout: Layout) -> AllocResult<NonNull<u8>> {
        let ptr = self.inner.allocate(layout).ok_or(AllocError::NoMemory)?; // ← 调用rlsf
        self.used_bytes += layout.size();
        Ok(ptr)
    }
}

// 4. rlsf/crates/rlsf/src/tlsf.rs (底层TLSF算法)
impl<'a, FLBitmap, SLBitmap, const FLLEN: usize, const SLLEN: usize> 
    Tlsf<'a, FLBitmap, SLBitmap, FLLEN, SLLEN> {
    
    /// 实际的分配算法实现
    pub fn allocate(&mut self, layout: Layout) -> Option<NonNull<u8>> {
        // TLSF核心算法：FL/SL映射、块查找、分割等
        // 这是通用的TLSF算法，Axvisor直接使用
    }
}
```

#### 3.3.2 实际内存分配示例

**在Axvisor中分配1KB内存的完整过程：**
```
1. 应用请求：alloc(1024, 8) → Layout { size: 1024, align: 8 }

2. axalloc::alloc() → GlobalAllocator::alloc()
   - 获取TLSF分配器锁
   - 调用 TlsfByteAllocator::alloc()

3. TlsfByteAllocator::alloc()
   - 调用 self.inner.allocate(layout) → rlsf算法
   - 更新 used_bytes += 1024

4. rlsf TLSF算法执行：
   - 调整大小：max(1024, GRANULARITY) = 1024 (已对齐)
   - 计算FL/SL索引：
     * fl = 64 - 5 - 1 - leading_zeros(1024) = 6
     * sl = rotate操作得到值
   - 查找空闲块：first_free[fl][sl]
   - 块分割：如果找到的块过大，分割剩余部分
   - 返回用户指针

5. Axvisor收到：NonNull<u8> 指向可用内存
```


### 3.3 Axvisor中的内存释放实现

**Axvisor中的释放流程：**
```rust
// 1. Axvisor应用层调用
axalloc::dealloc(ptr, layout); // ← 实际调用入口

// 2. arceos/modules/axalloc/src/lib.rs:92-96 (Level-1实现)
#[cfg(feature = "level-1")]
impl GlobalAllocator {
    pub fn dealloc(&self, pos: NonNull<u8>, layout: Layout) {
        let mut balloc = self.balloc.lock();  // 获取TLSF分配器
        balloc.dealloc(pos, layout);          // 委托给TLSF实现
    }
}

// 3. allocator/src/tlsf.rs:22-27 (TlsfByteAllocator实现)
impl ByteAllocator for TlsfByteAllocator {
    fn dealloc(&mut self, ptr: NonNull<u8>, layout: Layout) {
        self.used_bytes -= layout.size();
        unsafe {
            self.inner.deallocate(ptr, layout); // ← 调用rlsf释放算法
        }
    }
}

// 4. rlsf/crates/rlsf/src/tlsf.rs (底层TLSF释放算法)
impl<'a, FLBitmap, SLBitmap, const FLLEN: usize, const SLLEN: usize> 
    Tlsf<'a, FLBitmap, SLBitmap, FLLEN, SLLEN> {
    
    /// 实际的释放算法实现
    pub fn deallocate(&mut self, ptr: NonNull<u8>, layout: Layout) {
        // TLSF核心释放算法：块合并、FL/SL更新等
        // 这是通用的TLSF算法，Axvisor直接使用
    }
}
```

**TLSF释放机制：**
1. **块定位**：通过指针回退计算块头位置
2. **合并检查**：检查前后相邻块是否空闲，如空闲则合并
3. **插入空闲列表**：将合并后的块插入相应的FL/SL分类
4. **位图更新**：更新FL/SL位图状态


### 3.4 块合并算法

```
块合并算法流程：
┌─────────────────────────────────────────────────────────────────┐
│                  内存块释放请求                                  │
├─────────────────────────────────────────────────────────────────┤
│ 1. 获取块头信息                                                  │
│    ├── 计算块头指针 = 释放指针 - HEADER_SIZE                      │
│    ├── 读取块大小和状态信息                                       │
│    └─— 验证块的完整性                                            │
├─────────────────────────────────────────────────────────────────┤
│ 2. 检查前一个块                                                  │
│    ├── 计算前一个块的位置                                         │
│    ├── 检查前一个块是否为空闲状态                                 │
│    └─— 如果空闲，与前一个块合并                                   │
├─────────────────────────────────────────────────────────────────┤
│ 3. 检查后一个块                                                  │
│    ├── 计算后一个块的位置                                         │
│    ├── 检查后一个块是否为空闲状态                                 │
│    └─— 如果空闲，与后一个块合并                                   │
├─────────────────────────────────────────────────────────────────┤
│ 4. 插入分离数组                                                  │
│    ├── 计算合并后块的大小分类                                     │
│    ├── 插入到对应的FL和SL条目                                    │
│    └─— 更新分离数组的链表                                        │
└─────────────────────────────────────────────────────────────────┘
```

---


## 4 配置选项

Axvisor的内存分配器特性依赖链如下：

**第一层：Axvisor应用层配置**
```toml
# axvisor/Cargo.toml (实际文件)
axstd = {git = "https://github.com/arceos-hypervisor/arceos.git", tag = "hv-0.4.1", features = [
  "alloc-level-1",    # 启用Level-1单级分配器
  "paging",          # 启用虚拟内存管理
  "irq",            # 中断支持
  "multitask",      # 多任务支持
  "smp",            # 多核支持
]}
```

**第二层：axstd特性传递**
```toml
# arceos/ulib/axstd/Cargo.toml (实际文件)
alloc-level-1 = ["axfeat/alloc-level-1", "alloc"]  # → 启用axalloc的level-1和基础alloc
```

**第三层：axfeat特性聚合**
```toml
# arceos/api/axfeat/Cargo.toml (实际文件)
alloc-level-1 = ["axalloc/level-1", "alloc"]       # → 启用axalloc的level-1特性
alloc = ["axalloc", "axruntime/alloc"]             # → 启用axalloc模块和运行时支持
```

**第四层：axalloc分配器配置**
```toml
# arceos/modules/axalloc/Cargo.toml (实际文件)
default = ["tlsf", "allocator/page-alloc-256m"]    # 默认：TLSF算法 + 256MB页分配
tlsf = ["allocator/tlsf"]                          # TLSF字节分配器特性
level-1 = []                                       # Level-1单级模式特性
```

