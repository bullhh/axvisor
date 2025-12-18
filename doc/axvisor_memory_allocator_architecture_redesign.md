# Axvisor内存分配器架构重设计

## 设计原则

1. **最大化allocator模块的封装性**：allocator应该是一个完全独立的内存分配器实现，提供所有必要的接口
2. **最小化axalloc模块的复杂性**：axalloc只负责Rust标准库集成和系统特定适配
3. **减少迁移工作量**：现有的allocator代码可以最小化修改直接使用

---

## 整体架构原理

### 内存分配器分层与职责

| 层次 | 组件 | 主要职责 | 核心算法 |
|------|------|----------|----------|
| 应用层 | VM管理、设备模拟、调度器等 | 使用内存分配服务 | - |
| 适配层 | axalloc模块 | 系统初始化、内存检测、标准库集成 | - |
| 全局分配器 | GlobalAllocator | 智能分配策略、统计信息、接口统一 | 智能路由算法 |
| 字节分配器 | SlabByteAllocator | 小对象分配（≤2KB） | Slab固定分配算法 |
| 页面分配器 | BuddyPageAllocator | 页面级分配（≥4KB）、大对象支持 | Buddy伙伴算法 |
| 抽象接口 | PageAllocator Trait | 统一页面分配接口、实现解耦 | 抽象接口规范 |

### 核心分配器原理详解

#### 1. Buddy页面分配器原理

Buddy分配器是一种基于二分伙伴关系的页面分配算法，其核心思想是：

1. **内存组织**：将物理内存按2的幂次方大小组织成块（4KB、8KB、16KB...）
2. **伙伴关系**：任何两个大小相同的块，如果它们的地址仅有一位不同，则互为伙伴
3. **分裂与合并**：
   - **分配**：找到大小合适的块，若过大则递归分裂
   - **释放**：将释放的块与伙伴合并成更大的块

```
Buddy分配过程示例：
初始状态: [空闲: 32KB]

分配8KB过程:
1. 分裂32KB → [分配: 8KB, 空闲: 8KB, 空闲: 16KB]
2. 分配8KB → [分配: 8KB, 分配: 8KB, 空闲: 16KB]

释放8KB过程(释放第一个8KB):
1. 释放8KB → [空闲: 8KB, 分配: 8KB, 空闲: 16KB]
2. 合并伙伴 → [空闲: 16KB, 分配: 8KB]
```

#### 2. Slab字节分配器原理

Slab分配器是一种针对小对象优化的固定大小分配器，其核心思想是：

1. **分类分配**：将小对象按大小分类（8B、16B、32B...2048B）
2. **页面容器**：每个Slab使用一个或多个页面，按固定槽位大小划分
3. **三级管理**：
   - **空Slab**：完全未分配
   - **部分Slab**：部分已分配
   - **满Slab**：完全分配

```
Slab分配过程示例(32字节对象):
初始化: 
┌─────────────────────┐
│ 32字节槽位1        │ ← 空闲
│ 32字节槽位2        │ ← 空闲
│ ...               │
│ 32字节槽位N        │ ← 空闲
└─────────────────────┘

分配后:
┌─────────────────────┐
│ 32字节槽位1        │ ← 已分配
│ 32字节槽位2        │ ← 空闲
│ ...               │
│ 32字节槽位N        │ ← 空闲
└─────────────────────┘
```

### PageAllocator Trait的作用与地位

PageAllocator Trait在内存分配器架构中起到关键的**抽象和解耦**作用：

1. **统一接口**：为所有页面分配器提供一致的API规范
2. **解耦设计**：使Slab分配器不依赖特定的页面分配实现
3. **可扩展性**：便于未来替换或添加新的页面分配算法

```
抽象接口设计模式:
┌─────────────────────┐
│   Slab分配器       │
└───────┬───────────┘
        │ 调用PageAllocator接口
        ↓
┌─────────────────────┐
│ PageAllocator Trait│ ← 抽象层
└───────┬───────────┘
        │ 由具体实现提供
        ↓
┌─────────────────────┐
│ BuddyPageAllocator │ ← 具体实现
└─────────────────────┘
```

### 分配器协作机制

#### 智能分配路由

全局分配器根据请求大小自动选择最优分配器：

```
请求大小 → 分配器选择流程:
┌─────────────┐    判断大小    ┌─────────────────┐
│  分配请求   │ ──────────→ │   大小分类     │
└─────────────┘             └────────┬────────┘
                               │
         ≤2KB? ──────┬───────┐     │
         是        │     否     │     │
           ↓        │         ↓     │
    ┌─────────────┐│  ┌─────────────┐ │
    │ Slab分配器 ││  │ Buddy分配器 │ │
    └─────────────┘│  └─────────────┘ │
                   │                │
                   └────────────────┘
```

#### 分配器间依赖关系

1. **Slab依赖Buddy**：Slab通过PageAllocator接口请求页面
2. **抽象解耦**：通过Trait实现而非直接依赖
3. **统一接口**：GlobalAllocator提供统一分配接口

```
依赖关系图:
┌─────────────────┐
│ SlabByteAllocator │
└───────┬───────┘
        │ PageAllocator Trait
        ↓
┌───────▼───────┐
│ BuddyPageAllocator│
└───────┬───────┘
        │ 物理内存
        ↓
┌───────▼───────┐
│   物理内存页面   │
└─────────────────┘
```

---

## 整体架构图

```
┌─────────────────────────────────────────────────────┐
│                  应用层                                │
│  VM管理、设备模拟、调度器等                          │
└─────────────────────┬───────────────────────────────────┘
                      │ 全局分配请求 (GlobalAlloc)
┌─────────────────────▼───────────────────────────────────┐
│              axalloc模块 (适配层)                     │
│  ┌─────────────────────────────────────────────┐         │
│  │         GlobalAlloc适配器                 │         │
│  │   ┌─────────────────────────────────────┐   │         │
│  │   │    系统初始化和内存检测           │   │         │
│  │   └─────────────────────────────────────┘   │         │
│  └─────────────────────────────────────────────┘         │
└─────────────────────┬───────────────────────────────────┘
                      │ 智能路由决策
┌─────────────────────▼───────────────────────────────────┐
│          allocator模块 (完整内存分配器)                │
│  ┌─────────────────────────────────────────────┐         │
│  │         GLOBAL_ALLOCATOR实例             │         │
│  │   ┌─────────────────────────────────────┐   │         │
│  │   │   完整的内存分配API             │   │         │
│  │   │ - 内存初始化                   │   │         │
│  │   │ - 内存添加                     │   │         │
│  │   │ - 字节分配                     │   │         │
│  │   │ - 页面分配                     │   │         │
│  │   │ - 统计信息                     │   │         │
│  │   │ - 大小判断路由                   │   │         │
│  │   └─────────────────┬───────────────────┘   │         │
│  │                   │                   │
│  │   ┌─────────────▼─────┐ ┌──────▼───────┐ │
│  │   │ SlabByteAllocator │ │BuddyPageAllocator│ │
│  │   │ - ≤2KB小对象     │ │ - ≥4KB大对象  │ │
│  │   │ - 固定大小分类    │ │ - 伙伴算法    │ │
│  │   └─────────────┬─────┘ └──────┬───────┘ │
│  │                 │              │         │
│  │   ┌─────────────▼───────┐        │         │
│  │   │ PageAllocator Trait│◄───────┘         │
│  │   │ - 抽象页面分配接口   │                  │
│  │   └─────────────────────┘                  │
│  └─────────────────────────────────────────────┘         │
└─────────────────────┬───────────────────────────────────┘
                      │ 物理页面分配
┌─────────────────────▼───────────────────────────────────┐
│                  物理内存页面                          │
│              (hypervisor管理的连续内存)                 │
└───────────────────────────────────────────────────────────┘
```

---

## Allocator模块设计

### 1. 全局分配器接口 (crate/allocator/src/lib.rs)

**完整封装的内存分配器接口**

```rust
/// 全局内存分配器 - 完全封装的实现
pub struct GlobalAllocator {
    palloc: SpinNoIrq<BuddyPageAllocator>,
    balloc: SpinNoIrq<SlabByteAllocator>,
}

/// 全局分配器实例
#[cfg_attr(all(target_os = "none", not(test)), global_allocator)]
static GLOBAL_ALLOCATOR: GlobalAllocator = GlobalAllocator::new();

impl GlobalAllocator {
    /// 创建新的全局分配器
    pub const fn new() -> Self {
        Self {
            palloc: SpinNoIrq::new(BuddyPageAllocator::new()),
            balloc: SpinNoIrq::new(SlabByteAllocator::new()),
        }
    }
    
    /// 初始化全局分配器（指定单个内存区域）
    pub fn init(&self, start_vaddr: usize, size: usize) {
        info!(
            "initialize global allocator at: [{:#x}, {:#x})",
            start_vaddr,
            start_vaddr + size
        );
        self.add_memory(start_vaddr, size).expect("Failed to initialize global allocator");
        
        // 建立字节分配器和页面分配器的关联
        self.balloc.lock().set_page_allocator(&mut *self.palloc.lock());
    }
    
    /// 添加内存区域（可多次调用）
    pub fn add_memory(&self, start_vaddr: usize, size: usize) -> AllocResult {
        debug!(
            "add a memory region to global allocator: [{:#x}, {:#x})",
            start_vaddr,
            start_vaddr + size
        );
        self.palloc.lock().add_memory(start_vaddr, size)
    }
    
    /// 智能分配内存（根据大小自动选择最优分配器）
    pub fn alloc(&self, layout: Layout) -> AllocResult<NonNull<u8>> {
        if let Some(_size_class) = SizeClass::from_layout(layout) {
            // 小对象使用Slab字节分配器
            self.balloc.lock().alloc(layout)
        } else {
            // 大对象直接使用Buddy页面分配器
            let addr = self.palloc.lock().alloc_for_byte_allocator(layout.size(), layout.align())?;
            Ok(NonNull::new(addr as *mut u8).unwrap())
        }
    }
    
    /// 释放内存
    pub fn dealloc(&self, ptr: NonNull<u8>, layout: Layout) -> AllocResult {
        // 根据布局选择释放策略
        if let Some(_size_class) = SizeClass::from_layout(layout) {
            // 小对象释放到Slab分配器
            self.balloc.lock().dealloc(ptr, layout)
        } else {
            // 大对象直接释放到页面分配器
            let addr = ptr.as_ptr() as usize;
            self.palloc.lock().dealloc_for_byte_allocator(addr, layout.size());
            Ok(())
        }
    }
    
    /// 重新分配内存
    pub fn realloc(&self, ptr: NonNull<u8>, old_layout: Layout, new_layout: Layout) -> AllocResult<NonNull<u8>> {
        // 简单实现：分配新内存，复制数据，释放旧内存
        let new_ptr = self.alloc(new_layout)?;
        unsafe {
            core::ptr::copy_nonoverlapping(
                ptr.as_ptr(),
                new_ptr.as_ptr(),
                core::cmp::min(old_layout.size(), new_layout.size())
            );
            self.dealloc(ptr, old_layout)?;
        }
        Ok(new_ptr)
    }
    
    /// 分配页面
    pub fn alloc_pages(&self, num_pages: usize, align_pow2: usize) -> AllocResult<usize> {
        self.palloc.lock().alloc_pages(num_pages, align_pow2)
    }
    
    /// 释放页面
    pub fn dealloc_pages(&self, pos: usize, num_pages: usize) {
        self.palloc.lock().dealloc_pages(pos, num_pages);
    }
    
    /// 获取内存统计信息
    pub fn get_memory_stats(&self) -> MemoryStats {
        let page_stats = self.palloc.lock().get_stats();
        let byte_stats = self.balloc.lock().alloc_stats();
        
        MemoryStats {
            total_memory: page_stats.total_capacity,
            used_memory: page_stats.used_capacity + byte_stats.used_bytes,
            free_memory: page_stats.available_capacity + byte_stats.available_bytes,
            total_pages: page_stats.total_pages,
            used_pages: page_stats.used_pages,
            free_pages: page_stats.available_pages,
            slab_stats: byte_stats,
        }
    }
}

// Rust核心库GlobalAlloc trait的实现
unsafe impl GlobalAlloc for GlobalAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        self.alloc(layout)
            .map(|ptr| ptr.as_ptr())
            .unwrap_or_else(|_| {
                panic!("Memory allocation failed for layout: {:?}", layout);
            })
    }
    
    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        let ptr = NonNull::new(ptr).expect("dealloc null ptr");
        if let Err(_) = self.dealloc(ptr, layout) {
            panic!("Heap deallocation error for ptr: {:p}, layout: {:?}", ptr, layout);
        }
    }
}

// 导出便利函数（供axalloc模块使用）
pub fn global_allocator() -> &'static GlobalAllocator {
    &GLOBAL_ALLOCATOR
}

/// 便利函数：初始化全局分配器
pub fn global_init(start_vaddr: usize, size: usize) {
    GLOBAL_ALLOCATOR.init(start_vaddr, size);
}

/// 便利函数：添加内存区域
pub fn global_add_memory(start_vaddr: usize, size: usize) -> AllocResult {
    GLOBAL_ALLOCATOR.add_memory(start_vaddr, size)
}

/// 便利函数：分配内存
pub fn alloc(layout: Layout) -> AllocResult<NonNull<u8>> {
    GLOBAL_ALLOCATOR.alloc(layout)
}

/// 便利函数：释放内存
pub fn dealloc(ptr: NonNull<u8>, layout: Layout) -> AllocResult {
    GLOBAL_ALLOCATOR.dealloc(ptr, layout)
}

/// 便利函数：重新分配内存
pub fn realloc(ptr: NonNull<u8>, old_layout: Layout, new_layout: Layout) -> AllocResult<NonNull<u8>> {
    GLOBAL_ALLOCATOR.realloc(ptr, old_layout, new_layout)
}

/// 便利函数：分配页面
pub fn alloc_pages(num_pages: usize, align_pow2: usize) -> AllocResult<usize> {
    GLOBAL_ALLOCATOR.alloc_pages(num_pages, align_pow2)
}

/// 便利函数：释放页面
pub fn dealloc_pages(pos: usize, num_pages: usize) {
    GLOBAL_ALLOCATOR.dealloc_pages(pos, num_pages);
}

/// 便利函数：获取内存统计信息
pub fn get_memory_stats() -> MemoryStats {
    GLOBAL_ALLOCATOR.get_memory_stats()
}
```

### 2. 底层算法实现 

- `BuddyPageAllocator`：实现`PageAllocator` trait
- `SlabByteAllocator`：实现`ByteAllocator` trait
- 各种辅助结构和工具

---

## Axalloc模块设计 (极简适配层)

### 1. 模块结构 (modules/axalloc/src/lib.rs)

```rust
//! Axvisor内存分配器适配层
//! 
//! 这个模块主要负责：
//! 1. 系统特定的内存检测
//! 2. 提供与现有axalloc代码兼容的接口
//! 3. 处理平台相关的内存管理

// 重新导出allocator模块的核心接口
pub use crate::allocator::{
    global_allocator, global_init, global_add_memory,
    alloc, dealloc, realloc,
    alloc_pages, dealloc_pages,
    get_memory_stats
};

/// 系统内存区域检测
pub fn detect_memory_regions() -> Vec<MemoryRegion> {
    // 这里实现具体的内存检测逻辑
    vec![
        MemoryRegion {
            start: 0x80000000,
            size: 0x10000000, // 256MB
            typ: MemoryType::Available,
        },
        // 其他内存区域...
    ]
}

/// 初始化系统内存（自动检测内存区域）
pub fn init_system_memory() -> AllocResult {
    let memory_regions = detect_memory_regions();
    
    if memory_regions.is_empty() {
        return Err(AllocError::NoMemory);
    }
    
    // 使用第一个内存区域初始化分配器（建立字节分配器和页面分配器的关联）
    let first_region = &memory_regions[0];
    global_init(first_region.start, first_region.size);
    
    // 添加剩余的内存区域
    for region in memory_regions.iter().skip(1) {
        global_add_memory(region.start, region.size)?;
    }
    
    info!("System memory initialized: {} regions", memory_regions.len());
    Ok(())
}

/// 系统内存区域信息
#[derive(Debug, Clone)]
pub struct MemoryRegion {
    pub start: usize,
    pub size: usize,
    pub typ: MemoryType,
}

#[derive(Debug, Clone)]
pub enum MemoryType {
    Available,
    Reserved,
    Reclaimable,
}

// 注意：不需要再次实现GlobalAlloc，因为已经在allocator模块中实现
```

### 2. Cargo.toml配置

```toml
[dependencies]
# 依赖allocator模块
axvisor_allocator = { path = "../../crates/allocator" }
```

---

## 使用方式

### 1. 直接使用allocator接口（推荐）

```rust
// 使用allocator模块的接口
use axvisor_allocator::{global_init, global_add_memory, init_system_memory};

// 初始化内存（三种方式）
// 方式1：指定单个内存区域
global_init(0x80000000, 0x10000000); // 初始化256MB内存区域

// 方式2：动态添加多个内存区域
global_add_memory(0x90000000, 0x8000000); // 添加128MB内存区域
global_add_memory(0xA0000000, 0x10000000); // 添加256MB内存区域

// 方式3：自动检测并初始化系统内存
init_system_memory()?; // 自动检测所有可用内存区域

// 直接使用分配器
let layout = Layout::from_size_align(1024, 8)?;
let ptr = axvisor_allocator::alloc(layout)?;

// 使用内存...

axvisor_allocator::dealloc(ptr, layout)?;
```

### 2. 通过axalloc适配层使用（兼容性）

```rust
// 通过axalloc模块使用（与现有代码兼容）
use axalloc::{global_init, global_add_memory, init_system_memory};

// 初始化内存（三种方式）
global_init(0x80000000, 0x10000000);
global_add_memory(0x90000000, 0x8000000);
init_system_memory()?;

// 使用分配器
use axalloc::{alloc, dealloc, alloc_pages, dealloc_pages};

let layout = Layout::from_size_align(1024, 8)?;
let ptr = alloc(layout)?;

// 使用内存...

dealloc(ptr, layout)?;

// 分配页面
let pages = alloc_pages(256, 0)?; // 分配1MB (256*4KB)
dealloc_pages(pages, 256);
```

### 3. 标准库容器自动集成

```rust
// 标准库容器自动使用我们的分配器
let large_vec: Vec<u8> = Vec::with_capacity(16 * 1024 * 1024); // 16MB
let large_box = Box::new([0u8; 1024 * 1024]); // 1MB数组

// 系统会自动调用GlobalAllocator的GlobalAlloc实现
```

---

## 总结

这种重设计实现了：

1. **allocator模块成为完全封装的内存分配器**，提供所有必要的接口
2. **axalloc模块简化为适配层**，主要负责系统特定的功能
3. **最小化迁移工作量**，现有代码可以最大程度复用
4. **提供灵活的使用方式**，满足不同场景的需求

这样的设计既保持了良好的模块化，又大大简化了实现复杂度和迁移工作量。