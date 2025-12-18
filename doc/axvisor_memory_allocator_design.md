# Axvisor内存分配器架构设计

## 目录
1. [概述](#概述)
2. [现状分析](#现状分析)
3. [设计目标](#设计目标)
4. [整体架构](#整体架构)
5. [接口设计](#接口设计)
6. [Allocator模块实现](#allocator模块实现)
7. [Axalloc模块实现](#axalloc模块实现)
8. [实现计划](#实现计划)

---

## 概述

本文档为Axvisor虚拟机监控器设计一套全新的分层内存管理架构。新设计采用**分层内存管理架构**，结合Buddy System和Slab Allocator的优势，为Axvisor提供高效的内存管理服务。

### 核心设计理念
- **分层架构**：底层物理页面管理 + 上层堆对象管理
- **智能选择**：根据大小自动选择最优分配器
- **SMP优化**：CPU本地缓存 + 全局池
- **类型安全**：利用Rust类型系统保证内存安全

---

## 现状分析

### 当前axvisor内存分配器架构

#### 1. crate/allocator模块
```
crate/allocator/
├── lib.rs          # 统一接口定义
├── buddy.rs        # 基于buddy_system_allocator的封装
├── slab.rs         # 基于slab_allocator的封装
├── tlsf.rs         # TLSF分配器
└── bitmap.rs       # 位图页面分配器
```

#### 2. modules/axalloc模块
```
modules/axalloc/
├── lib.rs          # GlobalAllocator实现
├── page.rs         # GlobalPage RAII包装
└── tracking.rs     # 内存跟踪（可选）
```

**特点**：
- 简单的两层分配：字节分配器 + 页面分配器
- 字节分配器内存不足时向页面分配器请求
- 缺乏智能的分配器选择策略

### 主要改进需求

| 特性 | 当前axvisor | 新架构需求 | 改进方向 |
|------|-------------|-----------|----------|
| 架构层次 | 简单两层 | 分层优化 | 建立清晰的依赖关系 |
| SMP支持 | 无CPU本地缓存 | SMP优化 | 添加CPU本地缓存和负载均衡 |
| 分配策略 | 单一算法 | 智能选择 | 根据大小智能选择分配器 |
| 性能优化 | 基础优化 | 多级缓存 | 多级缓存和批量操作 |

---

## 整体架构

### 架构图
```
┌─────────────────────────────────────────────────────┐
│                  应用层                                │
│  VM管理、设备模拟、调度器等                          │
└─────────────────────┬───────────────────────────────────┘
                      │
┌─────────────────────▼───────────────────────────────────┐
│               系统分配器层(axalloc)                   │
│  ┌─────────────────────────────────────────────┐         │
│  │         SystemAllocator trait               │         │
│  │  ┌─────────────────────────────────────┐    │         │
│  │  │      AxSystemAllocator实现            │    │         │
│  │  │  ┌────────────────────────────────┐ │    │         │
│  │  │  │   AxAllocatorBridge(GlobalAlloc)│ │    │         │
│  │  │  └────────────────────────────────┘ │    │         │
│  │  └─────────────────────────────────────┘    │         │
│  └─────────────────────────────────────────────┘         │
└─────────────────────┬───────────────────────────────────┘
                      │
┌─────────────────────▼───────────────────────────────────┐
│               算法实现层(allocator)                   │
│  ┌─────────────────────────────────────────────┐         │
│  │         GlobalAllocator协调器               │         │
│  │   ┌─────────────────────────────────────┐   │         │
│  │   │       智能分发逻辑                    │   │         │
│  │   └─────────────────────────────────────┘   │         │
│  └─────────────────────────────────────────────┘         │
│  ┌─────────────────────────────────────────────┐         │
│  │           SlabByteAllocator                  │         │
│  │         8B-2048B小对象管理                   │         │
│  │         CPU本地缓存 + 全局池                │         │
│  └─────────────────────────────────────────────┘         │
│  ┌─────────────────────────────────────────────┐         │
│  │          BuddyPageAllocator                 │         │
│  │           物理页面管理                       │         │
│  │        CPU本地池 + 全局池                  │         │
│  └─────────────────────────────────────────────┘         │
└─────────────────────┬───────────────────────────────────┘
                      │
┌─────────────────────▼───────────────────────────────────┐
│                  物理内存页面                          │
│              (hypervisor管理的连续内存)                 │
└───────────────────────────────────────────────────────────┘
```

### 分层设计原则

1. **应用层**：使用axalloc提供的系统级分配接口
2. **系统层(axalloc)**：实现SystemAllocator trait，提供系统级分配服务
3. **算法层(allocator)**：实现具体的内存分配算法
4. **物理层**：实际的内存硬件资源

---

## 接口设计

### 1. 底层算法接口设计 (allocator模块)

#### 1.1 基础接口定义 (crate/allocator/src/traits.rs)

**基础分配器接口 - 所有分配器的公共基类**

```rust
/// 基础分配器接口
pub trait BaseAllocator {
    /// 初始化分配器
    fn init(&mut self, start: usize, size: usize);
    
    /// 添加内存区域
    fn add_memory(&mut self, start: usize, size: usize) -> AllocResult;
    
    /// 获取总容量
    fn total_capacity(&self) -> usize;
    
    /// 获取已用容量
    fn used_capacity(&self) -> usize;
    
    /// 获取可用容量
    fn available_capacity(&self) -> usize;
}

/// 页面分配器接口 - 物理内存管理
pub trait PageAllocator: BaseAllocator {
    /// 页面大小常量
    const PAGE_SIZE: usize = 4096;
    
    /// 分配连续页面
    fn alloc_pages(&mut self, num_pages: usize, align_pow2: usize) -> AllocResult<usize>;
    
    /// 释放页面
    fn dealloc_pages(&mut self, pos: usize, num_pages: usize);

    /// 获取总页面数
    fn total_pages(&self) -> usize;
    
    /// 获取已用页面数
    fn used_pages(&self) -> usize;
    
    /// 获取可用页面数
    fn available_pages(&self) -> usize;
    
    /// 从页面分配器获取内存给字节分配器
    fn alloc_for_byte_allocator(&mut self, size: usize, align: usize) -> AllocResult<usize>;
    
    /// 向页面分配器释放内存
    fn dealloc_for_byte_allocator(&mut self, addr: usize, size: usize);
}

/// 字节分配器接口 - 堆对象管理
pub trait ByteAllocator: BaseAllocator {
    /// 分配内存
    fn alloc(&mut self, layout: Layout) -> AllocResult<NonNull<u8>>;
    
    /// 释放内存
    fn dealloc(&mut self, pos: NonNull<u8>, layout: Layout);
    
    /// 重新分配内存
    fn realloc(&mut self, ptr: NonNull<u8>, old_layout: Layout, new_layout: Layout) -> AllocResult<NonNull<u8>>;
    
    /// 获取总字节数
    fn total_bytes(&self) -> usize;
    
    /// 获取已用字节数
    fn used_bytes(&self) -> usize;
    
    /// 获取可用字节数
    fn available_bytes(&self) -> usize;
    
    /// 获取分配统计信息
    fn alloc_stats(&self) -> AllocStats;
    
    /// 设置页面分配器支持
    fn set_page_allocator(&mut self, page_allocator: &mut dyn PageAllocator);
}

/// 全局协调器接口（由axalloc模块实现）
// 注意：这个trait在axalloc模块中定义，不在allocator模块中
```

#### 1.2 系统级接口需求 (axalloc模块需要实现的trait)

**全局协调器接口 - 由axalloc模块实现**

```rust
/// 全局分配器接口 - 由axalloc模块实现
pub trait GlobalAllocatorTrait {
    /// 智能分配内存（根据大小自动选择最优分配器）
    fn alloc(&self, layout: Layout) -> AllocResult<NonNull<u8>>;
    
    /// 释放内存
    fn dealloc(&self, ptr: NonNull<u8>, layout: Layout) -> AllocResult;
    
    /// 重新分配内存
    fn realloc(&self, ptr: NonNull<u8>, old_layout: Layout, new_layout: Layout) -> AllocResult<NonNull<u8>>;
    
    /// 分配页面
    fn alloc_pages(&self, num_pages: usize, align_pow2: usize) -> AllocResult<usize>;
    
    /// 释放页面
    fn dealloc_pages(&self, pos: usize, num_pages: usize);
    
    /// 获取内存统计信息
    fn get_memory_stats(&self) -> MemoryStats;
}

/// 系统初始化接口 - 由axalloc模块实现
pub trait SystemMemoryInitializer {
    /// 检测系统内存区域
    fn detect_memory_regions(&self) -> Vec<MemoryRegion>;
    
    /// 初始化系统内存
    fn init_system_memory(&mut self) -> AllocResult;
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
```

---

## Allocator模块实现

### 2. 底层算法模块接口 (crate/allocator/src/lib.rs)

**注意：allocator模块不再实现GlobalAllocator trait，只提供底层算法接口**

```rust
/// 内部分配器实例 - 供axalloc模块使用
pub struct AllocatorInstance {
    palloc: Spin<BuddyPageAllocator>,
    balloc: Spin<SlabByteAllocator>,
}

impl AllocatorInstance {
    /// 创建新的分配器实例
    pub fn new() -> Self {
        let mut palloc = BuddyPageAllocator::new();
        let mut balloc = SlabByteAllocator::new();
        
        // 建立层级关系
        balloc.set_page_allocator(&mut *palloc);
        
        Self {
            palloc: Spin::new(palloc),
            balloc: Spin::new(balloc),
        }
    }
    
    /// 添加内存区域
    pub fn add_memory(&mut self, start: usize, size: usize) -> AllocResult {
        self.palloc.lock().add_memory(start, size)
    }
    
    /// 智能分配内存
    pub fn alloc(&self, layout: Layout) -> AllocResult<NonNull<u8>> {
        if let Some(_size_class) = SizeClass::from_layout(layout) {
            self.balloc.lock().alloc(layout)
        } else {
            let addr = self.palloc.lock().alloc_for_byte_allocator(layout.size(), layout.align())?;
            Ok(NonNull::new(addr as *mut u8).unwrap())
        }
    }
    
    /// 释放内存
    pub fn dealloc(&self, ptr: NonNull<u8>, layout: Layout) -> AllocResult {
        if let Some(_size_class) = SizeClass::from_layout(layout) {
            self.balloc.lock().dealloc(ptr, layout)
        } else {
            let addr = ptr.as_ptr() as usize;
            self.palloc.lock().dealloc_for_byte_allocator(addr, layout.size());
            Ok(())
        }
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
```

### 3. 页面分配器实现 (crate/allocator/src/buddy.rs)

```rust
/// SMP友好的页面分配器
pub struct BuddyPageAllocator {
    /// 本地CPU缓存池（最大18阶，512MB）
    local_pools: CpuLocalArray<RefCell<BuddySet<18>>>,
    /// 全局内存池（最大32阶，8TB）
    global_pool: SpinLock<BuddySet<32>>,
    /// 按需全局锁（减少锁竞争）
    on_demand_lock: OnDemandGlobalLock,
    /// SMP统计计数器
    smp_stats: FastSmpCounter,
}

impl BuddyPageAllocator {
    /// 创建新的Buddy页面分配器
    pub fn new() -> Self {
        Self {
            local_pools: CpuLocalArray::new(|| RefCell::new(BuddySet::new())),
            global_pool: SpinLock::new(BuddySet::new()),
            on_demand_lock: OnDemandGlobalLock::new(),
            smp_stats: FastSmpCounter::new(),
        }
    }
    
    /// SMP优化的负载均衡
    fn balance_local_cache(&self) {
        let local_cpu = current_cpu_id();
        let mut local_pool = self.local_pools[local_cpu].borrow_mut();
        let mut global_pool = self.global_pool.lock();
        
        // 计算期望的本地缓存大小
        let global_size = global_pool.total_size();
        let expected_local_size = cache_expected_size(global_size);
        let minimal_local_size = cache_minimal_size(global_size);
        let maximal_local_size = cache_maximal_size(global_size);
        
        let local_size = local_pool.total_size();
        
        if local_size >= maximal_local_size {
            // 本地缓存过多，移至全局池
            balance_to_global(&mut local_pool, &mut *global_pool);
        } else if local_size < minimal_local_size {
            // 本地缓存不足，从全局池补充
            balance_from_global(&mut *global_pool, &mut local_pool);
        }
    }
}

impl PageAllocator for BuddyPageAllocator {
    fn init(&mut self, start: usize, size: usize) {
        self.add_memory(start, size).expect("Failed to initialize buddy allocator");
    }
    
    fn add_memory(&mut self, start: usize, size: usize) -> AllocResult {
        let mut global_pool = self.global_pool.lock();
        global_pool.add_free_memory(start, size);
        Ok(())
    }
    
    fn total_capacity(&self) -> usize {
        let global_pool = self.global_pool.lock();
        global_pool.total_size()
    }
    
    fn used_capacity(&self) -> usize {
        self.total_capacity() - self.available_capacity()
    }
    
    fn available_capacity(&self) -> usize {
        let mut total_available = 0;
        
        // 计算全局池可用内存
        {
            let global_pool = self.global_pool.lock();
            total_available += global_pool.available_size();
        }
        
        // 计算所有本地池可用内存
        for i in 0..num_cpus::get() {
            let local_pool = self.local_pools[i].borrow();
            total_available += local_pool.available_size();
        }
        
        total_available
    }
    
    fn alloc_pages(&mut self, num_pages: usize, align_pow2: usize) -> AllocResult<usize> {
        let required_size = num_pages * Self::PAGE_SIZE;
        let size_order = greater_order_of(required_size);
        let align_order = align_pow2.max(greater_order_of(Self::PAGE_SIZE));
        let order = size_order.max(align_order);
        
        let irq_guard = irq::disable_local();
        let local_cpu = current_cpu_id();
        let mut local_pool = self.local_pools[local_cpu].borrow_mut();
        
        // 1. 优先从本地缓存分配
        let chunk_addr = if order < 18 {
            local_pool.alloc_chunk(order)
        } else {
            None
        };
        
        // 2. 本地缓存不足时从全局池分配
        let chunk_addr = match chunk_addr {
            Some(addr) => addr,
            None => {
                let mut global_pool = self.on_demand_lock.lock(&self.global_pool);
                global_pool.alloc_chunk(order).ok_or(AllocError::NoMemory)?
            }
        };
        
        // 3. 更新统计并执行负载均衡
        self.smp_stats.inc();
        self.balance_local_cache();
        
        drop(irq_guard);
        Ok(chunk_addr)
    }
    
    fn dealloc_pages(&mut self, pos: usize, num_pages: usize) {
        let size = num_pages * Self::PAGE_SIZE;
        let irq_guard = irq::disable_local();
        let local_cpu = current_cpu_id();
        let mut local_pool = self.local_pools[local_cpu].borrow_mut();
        
        // 将内存块分解为标准大小的伙伴块并释放
        split_to_chunks(pos, size).for_each(|(addr, order)| {
            if order < 18 {
                local_pool.dealloc_chunk(addr, order);
            } else {
                let mut global_pool = self.on_demand_lock.lock(&self.global_pool);
                global_pool.dealloc_chunk(addr, order);
            }
        });
        
        self.balance_local_cache();
        
        drop(irq_guard);
    }
    
    fn total_pages(&self) -> usize {
        self.total_capacity() / Self::PAGE_SIZE
    }
    
    fn used_pages(&self) -> usize {
        self.used_capacity() / Self::PAGE_SIZE
    }
    
    fn available_pages(&self) -> usize {
        self.available_capacity() / Self::PAGE_SIZE
    }
    
    fn alloc_for_byte_allocator(&mut self, size: usize, align: usize) -> AllocResult<usize> {
        let num_pages = (size + Self::PAGE_SIZE - 1) / Self::PAGE_SIZE;
        let align_pow2 = align.trailing_zeros() as usize;
        self.alloc_pages(num_pages, align_pow2)
    }
    
    fn dealloc_for_byte_allocator(&mut self, addr: usize, size: usize) {
        let num_pages = (size + Self::PAGE_SIZE - 1) / Self::PAGE_SIZE;
        self.dealloc_pages(addr, num_pages);
    }
}
```

### 4. 字节分配器实现 (crate/allocator/src/slab.rs)

```rust
/// Slab字节分配器
pub struct SlabByteAllocator {
    /// 按大小分类的Slab缓存
    slab_caches: [
        SlabCache<8>, SlabCache<16>, SlabCache<32>, SlabCache<64>,
        SlabCache<128>, SlabCache<256>, SlabCache<512>, 
        SlabCache<1024>, SlabCache<2048>
    ],
    /// CPU本地缓存池
    local_caches: CpuLocalArray<RefCell<LocalHeapCache>>,
    /// 页面分配器引用
    page_allocator: Option<&'static mut dyn PageAllocator>,
    /// 全局统计
    global_stats: SlabGlobalStats,
}

impl SlabByteAllocator {
    /// 创建新的Slab字节分配器
    pub fn new() -> Self {
        Self {
            slab_caches: [
                SlabCache::new(), SlabCache::new(), SlabCache::new(), SlabCache::new(),
                SlabCache::new(), SlabCache::new(), SlabCache::new(), 
                SlabCache::new(), SlabCache::new()
            ],
            local_caches: CpuLocalArray::new(|| RefCell::new(LocalHeapCache::new())),
            page_allocator: None,
            global_stats: SlabGlobalStats::new(),
        }
    }
    
    /// 从页面分配器请求新Slab
    fn request_new_slab(&mut self, size_class: SizeClass) -> AllocResult {
        let page_allocator = self.page_allocator.ok_or(AllocError::NotInitialized)?;
        
        // 分配一个页面作为Slab
        let frame_addr = page_allocator.alloc_for_byte_allocator(PAGE_SIZE, PAGE_SIZE)?;
        let frame = unsafe { UniqueFrame::new(frame_addr) };
        
        // 添加到对应的Slab缓存
        let cache_index = size_class.cache_index();
        self.slab_caches[cache_index].create_slab(frame)?;
        
        self.global_stats.slabs_created += 1;
        
        Ok(())
    }
}

impl ByteAllocator for SlabByteAllocator {
    fn init(&mut self, start: usize, size: usize) {
        // 字节分配器不直接初始化内存，依赖页面分配器
    }
    
    fn add_memory(&mut self, start: usize, size: usize) -> AllocResult {
        // 字节分配器不直接管理内存，依赖页面分配器
        Ok(())
    }
    
    fn total_capacity(&self) -> usize {
        self.global_stats.total_slots * 64  // 估算值
    }
    
    fn used_capacity(&self) -> usize {
        self.global_stats.allocated_bytes
    }
    
    fn available_capacity(&self) -> usize {
        self.total_capacity() - self.used_capacity()
    }
    
    fn alloc(&mut self, layout: Layout) -> AllocResult<NonNull<u8>> {
        // 1. 根据布局选择大小分类
        let Some(size_class) = SizeClass::from_layout(layout) else {
            // 超过2048字节，回退到页面分配器
            return self.fallback_to_page_allocator(layout);
        };
        
        let cache_index = size_class.cache_index();
        let irq_guard = irq::disable_local();
        let local_cpu = current_cpu_id();
        let mut local_cache = self.local_caches[local_cpu].borrow_mut();
        
        // 2. 优先从CPU本地缓存分配
        let result = match size_class {
            SizeClass::Bytes8 => self.alloc_from_local_cache(&mut local_cache.cache8, cache_index),
            SizeClass::Bytes16 => self.alloc_from_local_cache(&mut local_cache.cache16, cache_index),
            SizeClass::Bytes32 => self.alloc_from_local_cache(&mut local_cache.cache32, cache_index),
            SizeClass::Bytes64 => self.alloc_from_local_cache(&mut local_cache.cache64, cache_index),
            SizeClass::Bytes128 => self.alloc_from_local_cache(&mut local_cache.cache128, cache_index),
            SizeClass::Bytes256 => self.alloc_from_local_cache(&mut local_cache.cache256, cache_index),
            SizeClass::Bytes512 => self.alloc_from_local_cache(&mut local_cache.cache512, cache_index),
            SizeClass::Bytes1024 => self.alloc_from_local_cache(&mut local_cache.cache1024, cache_index),
            SizeClass::Bytes2048 => self.alloc_from_local_cache(&mut local_cache.cache2048, cache_index),
        };
        
        drop(irq_guard);
        
        match result {
            Some(ptr) => {
                self.global_stats.alloc_count += 1;
                self.global_stats.allocated_bytes += layout.size();
                Ok(ptr)
            },
            None => {
                // 本地缓存不足，尝试批量补充
                self.batch_refill_local_cache(size_class)
            }
        }
    }
    
    fn dealloc(&mut self, ptr: NonNull<u8>, layout: Layout) {
        let Some(size_class) = SizeClass::from_layout(layout) else {
            // 大对象回退到页面分配器
            return self.fallback_dealloc_to_page_allocator(ptr, layout);
        };
        
        let irq_guard = irq::disable_local();
        let local_cpu = current_cpu_id();
        let mut local_cache = self.local_caches[local_cpu].borrow_mut();
        
        // 释放到CPU本地缓存
        match size_class {
            SizeClass::Bytes8 => local_cache.cache8.dealloc(ptr),
            SizeClass::Bytes16 => local_cache.cache16.dealloc(ptr),
            SizeClass::Bytes32 => local_cache.cache32.dealloc(ptr),
            SizeClass::Bytes64 => local_cache.cache64.dealloc(ptr),
            SizeClass::Bytes128 => local_cache.cache128.dealloc(ptr),
            SizeClass::Bytes256 => local_cache.cache256.dealloc(ptr),
            SizeClass::Bytes512 => local_cache.cache512.dealloc(ptr),
            SizeClass::Bytes1024 => local_cache.cache1024.dealloc(ptr),
            SizeClass::Bytes2048 => local_cache.cache2048.dealloc(ptr),
        }
        
        drop(irq_guard);
        
        self.global_stats.dealloc_count += 1;
        self.global_stats.allocated_bytes = self.global_stats.allocated_bytes.saturating_sub(layout.size());
        
        // 检查是否需要批量归还
        self.check_batch_return(size_class);
    }
    
    fn realloc(&mut self, ptr: NonNull<u8>, old_layout: Layout, new_layout: Layout) -> AllocResult<NonNull<u8>> {
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
    
    fn total_bytes(&self) -> usize {
        self.total_capacity()
    }
    
    fn used_bytes(&self) -> usize {
        self.used_capacity()
    }
    
    fn available_bytes(&self) -> usize {
        self.available_capacity()
    }
    
    fn alloc_stats(&self) -> AllocStats {
        AllocStats {
            alloc_count: self.global_stats.alloc_count,
            dealloc_count: self.global_stats.dealloc_count,
            used_bytes: self.global_stats.allocated_bytes,
            total_bytes: self.total_bytes(),
            available_bytes: self.available_bytes(),
        }
    }
    
    fn set_page_allocator(&mut self, page_allocator: &mut dyn PageAllocator) {
        self.page_allocator = Some(page_allocator);
    }
}
```

---

## Axalloc模块实现

### 5. 全局协调器实现 (modules/axalloc/src/lib.rs)

```rust
use crate::allocator::allocator;

/// 全局分配器实现 - 由axalloc模块实现
pub struct GlobalAllocator {
    /// 内部分配器实例（封装allocator模块的功能）
    inner: allocator::AllocatorInstance,
}

impl GlobalAllocator {
    /// 创建新的全局分配器
    pub fn new() -> Self {
        Self {
            inner: allocator::AllocatorInstance::new(),
        }
    }
    
    /// 初始化全局分配器（指定单个内存区域）
    pub fn init(&mut self, start_vaddr: usize, size: usize) {
        info!(
            "initialize global allocator at: [{:#x}, {:#x})",
            start_vaddr,
            start_vaddr + size
        );
        self.inner.add_memory(start_vaddr, size).expect("Failed to initialize global allocator");
    }
    
    /// 添加内存区域（可多次调用）
    pub fn add_memory(&mut self, start_vaddr: usize, size: usize) -> AllocResult {
        debug!(
            "add a memory region to global allocator: [{:#x}, {:#x})",
            start_vaddr,
            start_vaddr + size
        );
        self.inner.add_memory(start_vaddr, size)
    }
    
    /// 初始化系统内存（自动检测多个内存区域）
    pub fn init_system_memory(&mut self) -> AllocResult {
        let memory_regions = MEMORY_INITIALIZER.detect_memory_regions();
        
        for region in memory_regions {
            self.add_memory(region.start, region.size)?;
        }
        
        info!("System memory initialized: {} regions", memory_regions.len());
        Ok(())
    }
}

impl GlobalAllocatorTrait for GlobalAllocator {
    fn alloc(&self, layout: Layout) -> AllocResult<NonNull<u8>> {
        self.inner.alloc(layout)
    }
    
    fn dealloc(&self, ptr: NonNull<u8>, layout: Layout) -> AllocResult {
        self.inner.dealloc(ptr, layout)
    }
    
    fn realloc(&self, ptr: NonNull<u8>, old_layout: Layout, new_layout: Layout) -> AllocResult<NonNull<u8>> {
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
    
    fn alloc_pages(&self, num_pages: usize, align_pow2: usize) -> AllocResult<usize> {
        self.inner.alloc_pages(num_pages, align_pow2)
    }
    
    fn dealloc_pages(&self, pos: usize, num_pages: usize) {
        self.inner.dealloc_pages(pos, num_pages);
    }
    
    fn get_memory_stats(&self) -> MemoryStats {
        self.inner.get_memory_stats()
    }
}

/// 系统内存初始化器实现
pub struct AxMemoryInitializer;

impl SystemMemoryInitializer for AxMemoryInitializer {
    fn detect_memory_regions(&self) -> Vec<MemoryRegion> {
        detect_memory_regions_impl()
    }
    
    fn init_system_memory(&mut self) -> AllocResult {
        // 实际的初始化逻辑在AxGlobalAllocator::init_system_memory中
        Ok(())
    }
}

/// 全局分配器实例
#[cfg_attr(all(target_os = "none", not(test)), global_allocator)]
static GLOBAL_ALLOCATOR: GlobalAllocator = GlobalAllocator::new();

/// 返回全局分配器的引用
pub fn global_allocator() -> &'static GlobalAllocator {
    &GLOBAL_ALLOCATOR}

/// 系统内存初始化器实例
pub static MEMORY_INITIALIZER: AxMemoryInitializer = AxMemoryInitializer;

/// 初始化全局分配器（指定单个内存区域）
pub fn global_init(start_vaddr: usize, size: usize) {
    info!(
        "initialize global allocator at: [{:#x}, {:#x})",
        start_vaddr,
        start_vaddr + size
    );
    unsafe {
        // 注意：这里需要使用可变引用，但在静态上下文中需要特殊处理
        // 实际实现中可能需要使用内部可变性模式
        GLOBAL_ALLOCATOR.init(start_vaddr, size);
    }
}

/// 添加内存区域（可多次调用）
pub fn global_add_memory(start_vaddr: usize, size: usize) -> AllocResult {
    debug!(
        "add a memory region to global allocator: [{:#x}, {:#x})",
        start_vaddr,
        start_vaddr + size
    );
    unsafe {
        GLOBAL_ALLOCATOR.add_memory(start_vaddr, size)
    }
}

/// 初始化系统内存（自动检测多个内存区域）
pub fn init_system_memory() -> AllocResult {
    unsafe {
        GLOBAL_ALLOCATOR.init_system_memory()
    }
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

/// 实际的内存区域检测实现
fn detect_memory_regions_impl() -> Vec<MemoryRegion> {
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
```

### 6. 使用方式

#### 6.1 大内存页面分配

```rust
// 使用axalloc提供的便利函数
use axalloc::{alloc_pages, dealloc_pages};

let pages_16mb = 4096; // 4096 * 4KB = 16MB
let addr = alloc_pages(pages_16mb, 12)?; // 12 = 2^12 = 4096对齐

// 使用内存...
unsafe {
    let slice = std::slice::from_raw_parts_mut(addr as *mut u8, pages_16mb * 4096);
    slice.fill(0);
}

// 释放内存
dealloc_pages(addr, pages_16mb);
```

#### 6.2 标准库容器分配

```rust
// 自动使用我们的全局分配器
let large_vec: Vec<u8> = Vec::with_capacity(16 * 1024 * 1024); // 16MB
let large_box = Box::new([0u8; 1024 * 1024]); // 1MB数组

// 系统会自动调用：
// AxAllocatorBridge::alloc() -> GLOBAL_ALLOCATOR.alloc() -> allocator::alloc()
```

#### 6.3 直接使用GlobalAllocator接口

```rust
use axalloc::{global_allocator, GlobalAllocatorTrait};

let layout = Layout::from_size_align(1024, 8)?;
let ptr = global_allocator().alloc(layout)?;

// 使用内存...

global_allocator().dealloc(ptr, layout)?;
```

#### 6.4 初始化内存分配器

```rust
use axalloc::{global_init, global_add_memory, init_system_memory};

// 方式1：指定单个内存区域
global_init(0x80000000, 0x10000000); // 初始化256MB内存区域

// 方式2：动态添加多个内存区域
global_add_memory(0x90000000, 0x8000000); // 添加128MB内存区域
global_add_memory(0xA0000000, 0x10000000); // 添加256MB内存区域

// 方式3：自动检测并初始化系统内存
init_system_memory()?; // 自动检测所有可用内存区域
```

---

## 实现计划

### 阶段1：基础接口定义
1. 定义`crate/allocator/src/traits.rs`中的基础trait（BaseAllocator、PageAllocator、ByteAllocator）
2. 定义`modules/axalloc/src/lib.rs`中的系统级trait（GlobalAllocator、SystemMemoryInitializer）
3. 创建基本的数据结构和错误类型

### 阶段2：底层分配器实现
1. 实现`BuddyPageAllocator`（实现PageAllocator trait）
2. 实现`SlabByteAllocator`（实现ByteAllocator trait）
3. 实现`AllocatorInstance`内部实例（不对外暴露）
4. 在`allocator`模块中提供内部接口供axalloc调用

### 阶段3：系统集成层实现
1. 在`modules/axalloc`中实现`GlobalAllocator` trait（AxGlobalAllocator）
2. 实现`SystemMemoryInitializer` trait（AxMemoryInitializer）
3. 实现`AxAllocatorBridge`作为`GlobalAlloc`实现
4. 添加系统内存初始化功能和便利函数

### 阶段4：测试和优化
1. 编写单元测试和集成测试
2. 性能基准测试
3. SMP优化和缓存策略调整

---

## 总结

新设计的关键优势：

1. **清晰的职责分离**：
   - allocator模块：专注于底层算法实现（Buddy、Slab等）
   - axalloc模块：专注于系统级协调和Rust集成
   - 不再需要额外的SystemAllocator trait，直接使用GlobalAllocator

2. **简化的架构层次**：
   - 移除了不必要的中间层
   - GlobalAllocator trait直接在axalloc中实现
   - allocator模块只提供内部接口给axalloc调用

3. **良好的封装性**：
   - 上层模块通过GlobalAllocator接口访问内存分配
   - allocator模块的内部实现不对外暴露
   - axalloc模块作为系统级接口的唯一入口

4. **灵活的扩展性**：
   - 可以轻松替换底层算法实现而不影响系统级接口
   - GlobalAllocator trait保持稳定，提供一致的系统级API

5. **完整的集成**：
   - 同时支持直接内存分配和Rust标准库集成
   - 提供页面级和字节级分配接口
   - 通过GlobalAllocator trait统一管理所有内存操作

这种简化设计确保了代码的模块化、可维护性和可扩展性，同时减少了不必要的抽象层，为Axvisor提供了高效的内存管理能力。