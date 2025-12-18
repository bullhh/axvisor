# Axvisor内存分配器详细设计与实现

## 设计原则

1. **最大化allocator模块的封装性**：allocator应该是一个完全独立的内存分配器实现，提供所有必要的接口
2. **最小化axalloc模块的复杂性**：axalloc只负责Rust标准库集成和系统特定适配
3. **高性能多核支持**：通过per-CPU缓存和负载均衡实现高效的SMP内存分配

---

## 整体架构原理

### 内存分配器层次与职责

| 层次 | 组件 | 主要职责 | 核心算法 |
|------|------|----------|----------|
| 适配层 | axalloc模块 | 系统初始化、内存检测、标准库集成 | - |
| 全局分配器 | GlobalAllocator | 智能分配策略、统计信息、接口统一 | 智能路由算法 |
| 字节分配器 | SlabByteAllocator | 小对象分配（≤2KB） | Slab固定分配算法 |
| 页面分配器 | BuddyPageAllocator | 页面级分配（≥4KB）、大对象支持 | Buddy伙伴算法 |
| 物理内存 | hypervisor管理的连续内存 | 提供物理内存资源 | - |

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

#### 3. Per-CPU缓存原理详解

Per-CPU缓存是提高多核系统性能的关键技术，它通过减少多核间的锁竞争来提升内存分配效率。在Axvisor中，Per-CPU缓存分为两个层次：Per-CPU页面缓存和Per-CPU对象缓存。

##### 3.1 Per-CPU页面缓存原理

**结构设计**:
```
每个CPU独立维护本地页面缓存:
CPU0: [LOCAL_POOL] → BuddySet<MAX_LOCAL_BUDDY_ORDER>
CPU1: [LOCAL_POOL] → BuddySet<MAX_LOCAL_BUDDY_ORDER>
CPU2: [LOCAL_POOL] → BuddySet<MAX_LOCAL_BUDDY_ORDER>
CPU3: [LOCAL_POOL] → BuddySet<MAX_LOCAL_BUDDY_ORDER>
   ↓
共享全局池:
[GLOBAL_POOL] → BuddySet<MAX_BUDDY_ORDER>
```

**工作原理**:
1. **本地优先策略**: 每个CPU首先尝试从自己的本地缓存分配页面
2. **批量交换机制**: 本地缓存与全局池之间采用批量交换，减少锁竞争
3. **动态容量调整**: 根据内存压力和全局池大小动态调整本地缓存容量
4. **大小限制策略**: 本地缓存只保留较小阶数的块(≤18阶，512MB)，大块直接由全局池管理

**详细分配流程**:
```
CPU0请求分配4个页面(order=2):
1. 检查CPU0本地缓存(order=2链表)
   └─ 有空闲块? → 是 → 直接返回地址
   └─ 有空闲块? → 否 → 继续步骤2

2. 检查CPU0本地缓存(更大阶数链表)
   └─ 找到order=3空闲块 → 分裂为两个order=2块
   └─ 一个返回给请求者，一个放入本地order=2链表
   └─ 无更大块 → 继续步骤3

3. 从全局池批量获取
   └─ 获取order=2块(或更大块分裂)
   └─ 更新本地缓存容量统计
   └─ 返回地址给请求者
   └─ 触发缓存平衡检查
```

##### 3.2 Per-CPU对象缓存原理

**结构设计**:
```
每个CPU独立维护9种大小的对象缓存:
CPU0: [LOCAL_OBJ_CACHES] → [8B][16B][32B][64B][128B][256B][512B][1KB][2KB]
CPU1: [LOCAL_OBJ_CACHES] → [8B][16B][32B][64B][128B][256B][512B][1KB][2KB]
CPU2: [LOCAL_OBJ_CACHES] → [8B][16B][32B][64B][128B][256B][512B][1KB][2KB]
CPU3: [LOCAL_OBJ_CACHES] → [8B][16B][32B][64B][128B][256B][512B][1KB][2KB]
               ↓                ↓                ↓
           全局Slab池 → [GLOBAL_SLAB_POOL] → 统一管理所有大小的Slab容器
```

**工作原理**:
1. **分类缓存策略**: 按对象大小(8B-2KB)分为9个独立的本地缓存
2. **批量填充机制**: 本地缓存为空时，从全局池批量获取16个对象
3. **溢出控制策略**: 本地缓存超过64KB时，批量释放部分对象到全局池
4. **最少保留策略**: 本地缓存至少保留1KB对象，避免完全清空

**详细分配流程**:
```
CPU0请求分配32字节对象:
1. 检查CPU0的32B本地缓存
   └─ 有空闲对象? → 是 → 直接返回对象
   └─ 有空闲对象? → 否 → 继续步骤2

2. 从全局池批量填充(16个32B对象)
   └─ 全局池检查32B Slab容器
      ├─ 有部分分配Slab? → 从中分配对象
      ├─ 有空Slab? → 使用空Slab分配
      └─ 无可用Slab? → 请求页面分配器创建新Slab
         └─ 通过PageAllocator Trait向Buddy分配器请求页面
         └─ 将页面划分为32B槽位，初始化Slab

3. 更新本地32B缓存
   └─ 将16个对象中的1个返回给请求者
   └─ 剩余15个保留在本地缓存供后续使用
```

##### 3.3 CPU缓存池与整体池的协作关系

**数据流向图**:
```
应用分配请求
    ↓
┌─────────────┐    判断大小     ┌─────────────────┐
│  智能路由器   │ ──────────→ │   大小分类判断    │
└─────────────┘             └────────┬────────┘
                                    │
                     ≤2KB? ──────┬─┴───┐
                      是        │     否
                        ↓        │     ↓
              ┌─────────▼─────┐ │ ┌───▼──────┐
              │ Per-CPU对象    │ │ │ Per-CPU   │
              │ 缓存系统       │ │ │ 页面缓存  │
              │               │ │ │ 系统      │
              │ ┌─8B─┐        │ │ │           │
              │ ├16B─┤        │ │ │ ┌─order0─┐│
              │ ├32B─┤        │ │ │ ├order1─┤│
              │ ├64B─┤        │ │ │ ├order2─┤│
              │ ├128B┤        │ │ │ ...      ││
              │ ├256B┤        │ │ │ └order18─┘│
              │ ├512B┤        │ │ │           │
              │ ├1KB─┤        │ │ │           │
              │ └2KB─┘        │ │ │           │
              └─────┬─────────┘ │ └─────┬─────┘
                    │           │       │
        本地缓存不足时↓           │本地不足时↓
                    │           │       │
              ┌─────▼─────┐     │  ┌────▼─────┐
              │ 全局Slab   │◄────┘  │ 全局Buddy│
              │ 池         │        │ 池        │
              │           │        │           │
              │ 三级Slab   │        │ 多阶空闲链 │
              │ 管理       │        │ 表        │
              └─────┬─────┘        └─────┬─────┘
                    │                    │
              需要页面时↓                 │
                    │                    │
              ┌─────▼────────────────────▼─────┐
              │      PageAllocator Trait       │
              │    (Slab与Buddy间的抽象桥梁)     │
              └─────────────────────────────────┘
```

**交互机制详解**:

1. **对象缓存与全局Slab池的交互**:
   - **批量填充**: 本地缓存为空时，从全局池获取16个对象
   - **溢出释放**: 本地缓存超过64KB时，释放16个对象回全局池
   - **延迟创建**: 全局池的Slab容器在需要时才创建，并向Buddy分配器请求页面

2. **页面缓存与全局Buddy池的交互**:
   - **本地优先**: 小阶数块(≤18阶)优先从本地缓存分配
   - **全局回退**: 本地缓存不足时，从全局池分配
   - **动态平衡**: 根据全局池大小动态调整本地缓存容量(1%-10%)
   - **大块直管**: 大阶数块(>18阶)直接由全局池管理

3. **Slab池与Buddy池的交互**:
   - **页面请求**: Slab池通过PageAllocator Trait向Buddy池请求页面
   - **页面释放**: 空Slab的页面可以释放回Buddy池
   - **大小适配**: Buddy池分配的页面大小总是2的幂次方，与Slab需求匹配

##### 3.4 整个内存池的层次关系

**内存池层次结构**:
```
┌───────────────────────────────────────────────────────┐
│                    物理内存资源                          │
│  [连续内存区域1][连续内存区域2][连续内存区域3]...        │
└──────────────────────┬────────────────────────────────┘
                       │ 初始化时添加
                       ↓
┌───────────────────────────────────────────────────────┐
│                全局Buddy页面池                         │
│  ┌─────┬─────┬─────┬─────┬─────┬─────┬─────┬─────┐      │
│  │order0│order1│order2│order3│order4│...  │order31│...  │  │
│  │ 4KB │ 8KB │16KB │32KB │64KB │     │ 8TB  │     │  │
│  └──┬──┴──┬──┴──┬──┴──┬──┴──┬──┴─────┴──┬──┴──┬──┘      │
│     │     │     │     │     │         │     │           │
└─────┼─────┼─────┼─────┼─────┼─────────┼─────┼───────────┘
      │     │     │     │     │         │     │
      ▼     ▼     ▼     ▼     ▼         ▼     ▼
┌───────────────────────────────────────────────────────┐
│              Per-CPU本地页面缓存                       │
│  CPU0    CPU1    CPU2    CPU3    ...                  │
│ [order0-18] [order0-18] [order0-18] [order0-18]      │
└──────────────────────┬────────────────────────────────┘
                       │ Slab页面请求
                       ↓
┌───────────────────────────────────────────────────────┐
│                全局Slab对象池                          │
│  ┌───┬───┬───┬───┬───┬───┬───┬───┬───┐              │
│  │8B │16B│32B│64B│128│256│512│1KB│2KB│              │
│  │[×]│[×]│[×]│[×]│[×]│[×]│[×]│[×]│[×]│              │
│  └┬─┬┴─┬─┴┬─┴┬─┴┬─┴┬─┴┬─┴┬─┴┬─┴┬─┘              │
│   │  │  │  │  │  │  │  │  │  │                   │
└───┼──┼──┼──┼──┼──┼──┼──┼──┼──┼───────────────────┘
    │  │  │  │  │  │  │  │  │  │
    ▼  ▼  ▼  ▼  ▼  ▼  ▼  ▼  ▼  ▼
┌───────────────────────────────────────────────────────┐
│             Per-CPU本地对象缓存                        │
│  CPU0     CPU1     CPU2     CPU3    ...                │
│ [8-2KB] [8-2KB] [8-2KB] [8-2KB]                       │
└──────────────────────┬────────────────────────────────┘
                       │ 应用分配请求
                       ↓
┌───────────────────────────────────────────────────────┐
│                 应用程序/系统组件                       │
│  VM管理、设备模拟、调度器、内核数据结构...              │
└───────────────────────────────────────────────────────┘
```

**层次关系特点**:

1. **物理层**: hypervisor管理的连续物理内存，是所有内存池的基础
2. **全局Buddy池**: 管理所有页面级内存，支持2的幂次方大小(4KB-8TB)
3. **Per-CPU页面缓存**: 每CPU独立的小阶数块缓存，减少全局锁竞争
4. **全局Slab池**: 管理小对象分配，使用Buddy池提供的页面构建Slab容器
5. **Per-CPU对象缓存**: 每CPU独立的9种大小对象缓存，快速响应小对象分配
6. **应用层**: 最终消费者，通过统一接口获取内存

**缓存层次间的内存流动**:
```
内存流向: 物理 → 全局Buddy → Per-CPU页面缓存/全局Slab → Per-CPU对象缓存 → 应用
回收流向: 应用 → Per-CPU对象缓存 → 全局Slab → Per-CPU页面缓存/全局Buddy → 物理
```

##### 3.5 多核性能优化原理

**锁竞争减少机制**:

1. **本地优先**: 每CPU优先使用自己的缓存，避免全局锁竞争
2. **批量操作**: 本地缓存与全局池之间采用批量交换，减少锁获取次数
3. **读写分离**: 本地缓存使用无锁数据结构，全局池使用细粒度锁

**NUMA友好设计**:

1. **内存亲和性**: 每CPU倾向于使用本地缓存的内存，提高访问速度
2. **负载均衡**: 通过动态缓存平衡，避免某些CPU内存不足
3. **扩展性**: 支持任意数量的CPU，每CPU独立缓存不影响其他CPU

**性能指标优化**:

1. **分配延迟**: Per-CPU缓存命中时，延迟仅为几纳秒级别
2. **吞吐量**: 通过并行分配，整体吞吐量随CPU数量线性增长
3. **内存利用率**: 动态平衡机制确保内存有效利用，避免浪费

### 分配器协作机制

#### 智能分配路由

全局分配器根据请求大小自动选择最优分配器：

```
请求大小 → 分配器选择流程:
┌─────────────┐    判断大小  ┌─────────────────┐
│  分配请求    │ ──────────→ │   大小分类       │
└─────────────┘             └────────┬────────┘
                                     │
          ≤2KB? ─────┬───────┐       │
          是        │     否  │      │
           ↓        │         ↓      │
    ┌─────────────┐ │ ┌─────────────┐ │
    │ Slab分配器  │ │  │ Buddy分配器 │ │
    └─────────────┘│  └─────────────┘ │
                   │                │
                   └────────────────┘
```

#### 分配器间依赖关系

1. **Slab依赖Buddy**：Slab需要页面来创建Slab容器
2. **全局协调**：通过PageAllocator Trait实现松耦合
3. **统一接口**：GlobalAllocator提供统一分配接口

```
依赖关系图:
┌─────────────────┐
│ SlabByteAllocator │
└───────┬───────┘
        │ alloc_pages()
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
┌─────────────────────────────────────┐
│              应用层                 │
│    VM管理、设备模拟、调度器等         │
└─────────────────────────────────────┘
                    ↓ 全局分配请求 (GlobalAlloc)
┌─────────────────────────────────────┐
│         axalloc适配层               │
│    - GlobalAlloc适配器              │
│    - 系统初始化和内存检测            │
└─────────────────────────────────────┘
                    ↓ 智能路由决策
┌─────────────────────────────────────┐
│        allocator内存分配器           │
│    ┌─────────────────────────────┐  │
│    │      全局分配器接口          │  │
│    │ - 内存初始化与添加           │  │
│    │ - 智能分配策略               │  │
│    │ - 统计信息                  │  │
│    │ - 大小判断路由               │ │
│    └───────────┬─────────────────┘ │
│                ↓                   │
│      ┌──────────▼──────────┐       │
│      │    分配路由判断      │       │
│      └───┬────────┬───────┘       │
│          │        │              │
│     ≤2KB │        │ >2KB         │
│          ↓        ↓              │
│  ┌───▼────┐ ┌───▼──────┐         │
│  │ Slab   │ │ Buddy    │        │
│  │ 字节   │ │ 页面      │        │
│  │ 分配器  │ │ 分配器   │        │
│  └───┬────┘ └───┬──────┘       │
│      │            │            │
│      │ 页面请求    │            │
│      ↓            │            │
│  ┌───▼────────────▼───────┐    │
│  │ PageAllocator Trait    │    │
│  │ (抽象页面分配接口)      │    │
│  │                        │    │
│  │ Slab调用此接口获取页面   │    │
│  └───────────┬───────────┘    │
│              │ 实现            │
│              ↓                │
│  ┌───────────▼───────────┐    │
│  │   BuddyPageAllocator │    │
│  │   (实现PageAllocator) │    │
│  │ - 伙伴算法实现        │    │
│  │ - 自由块链表管理      │    │
│  │ - Per-CPU页面缓存     │    │
│  └───────────┬───────────┘    │
│              ↓                │
└──────────────┼─────────────────┘
               ↓
┌──────────────▼──────────────┐
│        物理内存页面          │
│  (hypervisor管理的连续内存)  │
└─────────────────────────────┘

说明：
1. PageAllocator Trait是抽象接口，不是任何分配器的子组件
2. BuddyPageAllocator实现了PageAllocator Trait
3. Slab通过PageAllocator Trait请求页面，实际由BuddyPageAllocator处理
```

### 关键协作流程示例

#### 小对象分配流程 (≤2KB)

```
应用请求分配256字节
       ↓
全局分配器判断大小分类
       ↓
路由到Slab分配器
       ↓
检查Per-CPU本地缓存(256B类)
   命中? ────是──→ 直接返回对象
       │否
       ↓
从全局Slab池批量获取对象
       ↓
填充本地缓存
       ↓
返回对象给应用
```

#### 大对象分配流程 (>2KB)

```
应用请求分配16KB
       ↓
全局分配器判断大小分类
       ↓
路由到Buddy页面分配器
       ↓
计算所需阶数(order=2, 4页)
       ↓
检查Per-CPU本地页面缓存
   命中? ────是──→ 直接返回页面地址
       │否
       ↓
从全局Buddy池分配页面
       ↓
动态平衡本地缓存
       ↓
返回页面地址给应用
```

#### Slab向Buddy请求页面流程

```
Slab分配器需要新页面(创建新Slab)
       ↓
调用PageAllocator接口
       ↓
Buddy分配器分配页面(4KB)
       ↓
页面转换为Slab容器
       ↓
按固定槽位大小划分
       ↓
初始化空闲槽位链表
       ↓
加入Slab管理系统
```

---

## Allocator模块设计

### 1. 全局分配器接口 (crate/allocator/src/lib.rs)

**完整封装的内存分配器接口**

```rust
use core::alloc::{GlobalAlloc, Layout};
use core::ptr::NonNull;
use spin::SpinNoIrq;

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
        self.balloc.lock().set_page_allocator_raw(&mut *self.palloc.lock());
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

---

### 2. Buddy页面分配器实现

#### 2.1 核心数据结构

**自由块结构** (crate/allocator/src/buddy/chunk.rs):

```rust
/// 自由块表示
pub(crate) struct FreeChunk {
    head: UniqueFrame<Link<FreeHeadMeta>>,
}

impl FreeChunk {
    /// 创建一个未使用的自由块
    pub(crate) fn from_unused(addr: Paddr, order: BuddyOrder) -> Self {
        let frame = unsafe { Frame::from_unchecked(addr) };
        let head = frame.into_untyped_frame()
            .cast::<Link<FreeHeadMeta>>()
            .with_meta(Link::new(FreeHeadMeta { order }));
        Self { head }
    }
    
    /// 从已有自由块创建
    pub(crate) fn from_free_head(head: UniqueFrame<Link<FreeHeadMeta>>) -> Self {
        Self { head }
    }
    
    /// 获取块地址
    pub(crate) fn addr(&self) -> Paddr {
        self.head.start_paddr()
    }
    
    /// 获取块大小阶数
    pub(crate) fn order(&self) -> BuddyOrder {
        self.head.meta().order
    }
    
    /// 计算伙伴地址（关键算法）
    pub(crate) fn buddy(&self) -> Paddr {
        let addr = self.addr();
        let order = self.order();
        addr ^ size_of_order(order)  // XOR 计算伙伴地址
    }
    
    /// 分割自由块
    pub(crate) fn split_free(self) -> (FreeChunk, FreeChunk) {
        let order = self.order();
        debug_assert!(order > 0);
        let new_order = order - 1;
        let addr = self.addr();
        
        // 分割成两个更小的伙伴块
        let left_child_addr = addr;
        let right_child_addr = addr ^ size_of_order(new_order);
        
        let left_child = Self::from_unused(left_child_addr, new_order);
        let right_child = Self::from_unused(right_child_addr, new_order);
        
        (left_child, right_child)
    }
    
    /// 合并两个伙伴块
    pub(crate) fn merge_free(mut self, mut buddy: FreeChunk) -> FreeChunk {
        debug_assert_eq!(self.order(), buddy.order());
        debug_assert_eq!(self.buddy(), buddy.addr());
        
        // 合并两个伙伴块成更大的块
        let new_order = self.order() + 1;
        let addr = core::cmp::min(self.addr(), buddy.addr());
        
        Self::from_unused(addr, new_order)
    }
}

/// 自由块元数据
#[derive(Clone, Copy)]
pub(crate) struct FreeHeadMeta {
    pub order: BuddyOrder,
}
```

#### 2.2 Buddy集合管理

**BuddySet结构** (crate/allocator/src/buddy/set.rs):

```rust
/// Buddy集合 - 管理所有阶数的自由块
pub(crate) struct BuddySet<const MAX_ORDER: usize> {
    total_size: usize,
    lists: [LinkedList<FreeHeadMeta>; MAX_ORDER],  // 每个阶的空闲链表
}

impl<const MAX_ORDER: usize> BuddySet<MAX_ORDER> {
    /// 创建空的Buddy集合
    pub const fn new_empty() -> Self {
        const INIT: LinkedList<FreeHeadMeta> = LinkedList::new();
        Self {
            total_size: 0,
            lists: [INIT; MAX_ORDER],
        }
    }
    
    /// 插入自由块（自动合并）
    pub(crate) fn insert_chunk(&mut self, addr: Paddr, order: BuddyOrder) {
        debug_assert!(order < MAX_ORDER);
        let mut chunk = FreeChunk::from_unused(addr, order);
        let chunk_order = order;
        
        // 向上查找伙伴并合并（关键算法）
        let mut current_order = chunk_order;
        while current_order < MAX_ORDER - 1 {
            let buddy_addr = chunk.buddy();
            
            // 检查伙伴是否在当前阶的空闲列表中
            if let Some(buddy_chunk) = self.remove_chunk_at(buddy_addr, current_order) {
                // 找到伙伴，合并
                chunk = chunk.merge_free(buddy_chunk);
                current_order += 1;
            } else {
                // 没有找到伙伴，停止合并
                break;
            }
        }
        
        // 插入最终合并后的块
        let final_order = current_order;
        self.lists[final_order].push_front(chunk.into_unique_head());
        self.total_size += size_of_order(final_order);
    }
    
    /// 分配指定阶数的块（自动分割）
    pub(crate) fn alloc_chunk(&mut self, order: BuddyOrder) -> Option<Paddr> {
        debug_assert!(order < MAX_ORDER);
        
        // 找到最小的足够大的空闲块
        let non_empty = self.find_non_empty_order(order)?;
        
        // 从非空链表中取出一个块
        let chunk = self.lists[non_empty].pop_front()?;
        let mut chunk = FreeChunk::from_free_head(chunk);
        self.total_size -= size_of_order(non_empty);
        
        // 分割块直到达到目标大小
        for current_order in (order + 1..=non_empty).rev() {
            if current_order > order {
                let (left_child, right_child) = chunk.split_free();
                // 右子块放回空闲链表，左子块继续分割
                self.lists[current_order - 1].push_front(right_child.into_unique_head());
                self.total_size += size_of_order(current_order - 1);
                chunk = left_child;
            }
        }
        
        Some(chunk.addr())
    }
    
    /// 释放块
    pub(crate) fn dealloc_chunk(&mut self, addr: Paddr, order: BuddyOrder) {
        self.insert_chunk(addr, order);
    }
    
    /// 查找非空链表的最小阶数
    fn find_non_empty_order(&self, start_order: BuddyOrder) -> Option<BuddyOrder> {
        for (i, list) in self.lists.iter().enumerate().skip(start_order) {
            if !list.is_empty() {
                return Some(i);
            }
        }
        None
    }
    
    /// 移除指定地址的块
    fn remove_chunk_at(&mut self, addr: Paddr, order: BuddyOrder) -> Option<FreeChunk> {
        if let Some(mut cursor) = self.lists[order].cursor_mut_at(addr) {
            let chunk = cursor.take_current()?;
            self.total_size -= size_of_order(order);
            Some(FreeChunk::from_free_head(chunk))
        } else {
            None
        }
    }
    
    /// 获取总大小
    pub(crate) fn total_size(&self) -> usize {
        self.total_size
    }
    
    /// 获取统计信息
    pub(crate) fn get_stats(&self) -> PageAllocatorStats {
        let mut total_pages = 0;
        let mut free_pages = 0;
        let mut total_capacity = 0;
        let mut available_capacity = 0;
        
        for (order, list) in self.lists.iter().enumerate() {
            let chunk_size = size_of_order(order);
            let chunk_pages = chunk_size / PAGE_SIZE;
            let chunks_in_list = list.len();
            
            total_pages += chunk_pages * chunks_in_list;
            free_pages += chunk_pages * chunks_in_list;
            total_capacity += chunk_size * chunks_in_list;
            available_capacity += chunk_size * chunks_in_list;
        }
        
        PageAllocatorStats {
            total_pages,
            used_pages: 0,  // Buddy系统中，已分配的页面不在空闲链表中
            free_pages,
            total_capacity,
            used_capacity: 0,
            available_capacity,
        }
    }
}
```

#### 2.3 Per-CPU页面缓存

**CPU本地缓存** (crate/allocator/src/buddy/pools/mod.rs):

```rust
use core::cell::RefCell;
use crate::cpu_local;

// CPU本地自由页面缓存
cpu_local! {
    static LOCAL_POOL: RefCell<BuddySet<MAX_LOCAL_BUDDY_ORDER>> = 
        RefCell::new(BuddySet::new_empty());
}

// 全局页面池（延迟初始化）
static GLOBAL_POOL: OnDemandGlobalLock<BuddySet<MAX_BUDDY_ORDER>> = OnDemandGlobalLock::new();

// 本地缓存最大阶数（512MB）
const MAX_LOCAL_BUDDY_ORDER: BuddyOrder = 18;

// 全局池最大阶数（8TiB）
const MAX_BUDDY_ORDER: BuddyOrder = 32;

/// Per-CPU Buddy分配器
pub struct PerCpuBuddyAllocator;

impl PerCpuBuddyAllocator {
    /// 分配页面
    pub fn alloc_pages(order: BuddyOrder) -> Option<Paddr> {
        // 优先从本地缓存分配
        if order < MAX_LOCAL_BUDDY_ORDER {
            if let Some(addr) = LOCAL_POOL.get().borrow_mut().alloc_chunk(order) {
                return Some(addr);
            }
        }
        
        // 本地缓存不足，从全局池分配
        let addr = GLOBAL_POOL.get().lock().alloc_chunk(order)?;
        
        // 平衡缓存
        balance_local_cache();
        
        Some(addr)
    }
    
    /// 释放页面
    pub fn dealloc_pages(addr: Paddr, order: BuddyOrder) {
        // 优先释放到本地缓存
        if order < MAX_LOCAL_BUDDY_ORDER {
            LOCAL_POOL.get().borrow_mut().dealloc_chunk(addr, order);
            balance_local_cache();
        } else {
            // 大块直接释放到全局池
            GLOBAL_POOL.get().lock().dealloc_chunk(addr, order);
        }
    }
    
    /// 添加新内存到全局池
    pub fn add_memory(addr: Paddr, size: usize) {
        GLOBAL_POOL.get().lock().add_memory(addr, size);
        balance_local_cache();
    }
    
    /// 获取统计信息
    pub fn get_stats() -> PageAllocatorStats {
        let local_stats = LOCAL_POOL.get().borrow().get_stats();
        let global_stats = GLOBAL_POOL.get().lock().get_stats();
        
        PageAllocatorStats {
            total_pages: local_stats.total_pages + global_stats.total_pages,
            used_pages: local_stats.used_pages + global_stats.used_pages,
            free_pages: local_stats.free_pages + global_stats.free_pages,
            total_capacity: local_stats.total_capacity + global_stats.total_capacity,
            used_capacity: local_stats.used_capacity + global_stats.used_capacity,
            available_capacity: local_stats.available_capacity + global_stats.available_capacity,
        }
    }
}

/// 平衡本地缓存
fn balance_local_cache() {
    let mut local = LOCAL_POOL.get().borrow_mut();
    let mut global = GLOBAL_POOL.get().lock();
    
    balancing::balance(&mut local, &mut global);
}
```

#### 2.4 动态负载均衡

**缓存平衡策略** (crate/allocator/src/buddy/pools/balancing.rs):

```rust
/// 本地缓存动态平衡
pub fn balance(
    local: &mut BuddySet<MAX_LOCAL_BUDDY_ORDER>, 
    global: &mut BuddySet<MAX_BUDDY_ORDER>
) {
    let global_size = global.total_size();
    
    // 基于全局内存大小动态调整缓存大小
    let minimal_local_size = cache_minimal_size(global_size);
    let expected_local_size = cache_expected_size(global_size);
    let maximal_local_size = cache_maximal_size(global_size);
    
    let local_size = local.total_size();
    
    if local_size >= maximal_local_size {
        // 本地缓存过大，移动到全局池
        let excess_size = local_size - expected_local_size;
        let order = lesser_order_of(excess_size);
        
        if let Some(addr) = local.alloc_chunk(order) {
            global.dealloc_chunk(addr, order);
        }
    } else if local_size < minimal_local_size {
        // 本地缓存过小，从全局池补充
        let deficit_size = expected_local_size - local_size;
        let order = lesser_order_of(deficit_size);
        
        if let Some(addr) = global.alloc_chunk(order) {
            local.dealloc_chunk(addr, order);
        }
    }
}

/// 计算本地缓存最小大小
fn cache_minimal_size(global_size: usize) -> usize {
    // 至少保留全局内存的1%
    global_size / 100
}

/// 计算本地缓存期望大小
fn cache_expected_size(global_size: usize) -> usize {
    // 期望保留全局内存的5%
    global_size / 20
}

/// 计算本地缓存最大大小
fn cache_maximal_size(global_size: usize) -> usize {
    // 最多保留全局内存的10%
    global_size / 10
}

/// 计算小于等于指定大小的最大2的幂阶数
fn lesser_order_of(size: usize) -> BuddyOrder {
    if size == 0 {
        return 0;
    }
    let order = 64 - size.leading_zeros() as usize - 1;
    if size_of_order(order) > size {
        order - 1
    } else {
        order
    }
}
```

---

### 3. Slab字节分配器实现

#### 3.1 核心数据结构

**Slab结构** (crate/allocator/src/slab/slab.rs):

```rust
use core::ptr::NonNull;

/// Slab固定大小分配器
pub type Slab<const SLOT_SIZE: usize> = UniqueFrame<Link<SlabMeta<SLOT_SIZE>>>;

/// Slab元数据
pub struct SlabMeta<const SLOT_SIZE: usize> {
    free_list: SlabSlotList<SLOT_SIZE>,  // 空闲槽位链表
    nr_allocated: u16,                   // 已分配槽位数
}

impl<const SLOT_SIZE: usize> Slab<SLOT_SIZE> {
    /// 创建新的Slab
    pub fn new(page_alloc: &mut dyn PageAllocator) -> Result<Self, AllocError> {
        // 分配一个新的slab页面
        let frame = page_alloc.alloc_frame(PAGE_SIZE, PAGE_SIZE)
            .ok_or(AllocError)?;
            
        let mut slab: Slab<SLOT_SIZE> = frame.into_untyped_frame()
            .cast::<Link<SlabMeta<SLOT_SIZE>>>()
            .with_meta(Link::new(SlabMeta::<SLOT_SIZE> {
                free_list: SlabSlotList::new(),
                nr_allocated: 0,
            }));
        
        // 初始化所有槽位到空闲链表
        let head_vaddr = slab.vaddr();
        for slot_offset in (0..PAGE_SIZE).step_by(SLOT_SIZE) {
            let slot_ptr = unsafe { 
                NonNull::new_unchecked((head_vaddr + slot_offset) as *mut u8) 
            };
            slab.meta_mut().free_list.push(unsafe { 
                HeapSlot::new(slot_ptr, SlotInfo::SlabSlot(SLOT_SIZE)) 
            });
        }
        
        Ok(slab)
    }
    
    /// 从Slab分配一个槽位
    pub fn alloc(&mut self) -> Option<HeapSlot> {
        if let Some(slot) = self.meta_mut().free_list.pop() {
            self.meta_mut().nr_allocated += 1;
            Some(slot)
        } else {
            None
        }
    }
    
    /// 向Slab释放一个槽位
    pub fn dealloc(&mut self, slot: HeapSlot) -> Result<(), AllocError> {
        if slot.size() != SLOT_SIZE {
            return Err(AllocError);
        }
        
        self.meta_mut().free_list.push(slot);
        self.meta_mut().nr_allocated -= 1;
        Ok(())
    }
    
    /// 获取已分配槽位数
    pub fn nr_allocated(&self) -> u16 {
        self.meta().nr_allocated
    }
    
    /// 获取Slab容量
    pub fn capacity(&self) -> u16 {
        (PAGE_SIZE / SLOT_SIZE) as u16
    }
    
    /// 检查是否为满
    pub fn is_full(&self) -> bool {
        self.nr_allocated() == self.capacity()
    }
    
    /// 检查是否为空
    pub fn is_empty(&self) -> bool {
        self.nr_allocated() == 0
    }
}

impl<const SLOT_SIZE: usize> Slab<SLOT_SIZE> {
    /// 获取Slab元数据的不可变引用
    fn meta(&self) -> &SlabMeta<SLOT_SIZE> {
        self.head.meta()
    }
    
    /// 获取Slab元数据的可变引用
    fn meta_mut(&mut self) -> &mut SlabMeta<SLOT_SIZE> {
        self.head.meta_mut()
    }
}
```

#### 3.2 Slab缓存管理

**SlabCache结构** (crate/allocator/src/slab/slab_cache.rs):

```rust
use alloc::collections::LinkedList;

/// Slab缓存 - 管理多个相同大小的Slab
pub struct SlabCache<const SLOT_SIZE: usize> {
    empty: LinkedList<SlabMeta<SLOT_SIZE>>,    // 空 slab 链表
    partial: LinkedList<SlabMeta<SLOT_SIZE>>,  // 部分分配 slab 链表  
    full: LinkedList<SlabMeta<SLOT_SIZE>>,     // 满 slab 链表
    page_alloc: *mut dyn PageAllocator,       // 页面分配器引用
}

impl<const SLOT_SIZE: usize> SlabCache<SLOT_SIZE> {
    /// 创建新的Slab缓存
    pub fn new(page_alloc: *mut dyn PageAllocator) -> Self {
        Self {
            empty: LinkedList::new(),
            partial: LinkedList::new(),
            full: LinkedList::new(),
            page_alloc,
        }
    }
    
    /// 分配一个槽位
    pub fn alloc(&mut self) -> Result<HeapSlot, AllocError> {
        // 优先从部分分配的slab分配
        if let Some(slab) = self.alloc_from_partial() {
            return Ok(slab);
        }
        
        // 然后从空slab分配
        if let Some(slab) = self.alloc_from_empty() {
            return Ok(slab);
        }
        
        // 最后分配新slab
        self.alloc_from_new_slab()
    }
    
    /// 释放一个槽位
    pub fn dealloc(&mut self, slot: HeapSlot) -> Result<(), AllocError> {
        // 尝试从满slab中找到对应的slab
        if let Some(slab) = self.find_slab_in_full(&slot) {
            return self.dealloc_to_slab(slab, slot);
        }
        
        // 尝试从部分分配的slab中找到对应的slab
        if let Some(slab) = self.find_slab_in_partial(&slot) {
            return self.dealloc_to_slab(slab, slot);
        }
        
        Err(AllocError)
    }
    
    /// 从部分分配的slab分配槽位
    fn alloc_from_partial(&mut self) -> Option<HeapSlot> {
        if self.partial.is_empty() {
            return None;
        }
        
        let mut cursor = self.partial.cursor_back_mut();
        let slab = cursor.current_meta().unwrap();
        let allocated = slab.alloc().unwrap();
        
        if slab.is_full() {
            // 如果slab已满，移动到满链表
            let full_slab = cursor.take_current().unwrap();
            self.full.push_front(full_slab);
        }
        
        Some(allocated)
    }
    
    /// 从空slab分配槽位
    fn alloc_from_empty(&mut self) -> Option<HeapSlot> {
        if self.empty.is_empty() {
            return None;
        }
        
        let mut slab = self.empty.pop_front().unwrap();
        let allocated = slab.meta_mut().alloc().unwrap();
        self.add_slab(slab);
        
        Some(allocated)
    }
    
    /// 分配新slab并分配槽位
    fn alloc_from_new_slab(&mut self) -> Result<HeapSlot, AllocError> {
        let page_alloc = unsafe { &mut *self.page_alloc };
        let mut allocated_empty = Slab::new(page_alloc)?;
        let allocated = allocated_empty.meta_mut().alloc().unwrap();
        
        self.add_slab(allocated_empty);
        
        // 预分配更多空slab作为缓存
        for _ in 0..EXPECTED_EMPTY_SLABS {
            let page_alloc = unsafe { &mut *self.page_alloc };
            if let Ok(empty_slab) = Slab::new(page_alloc) {
                self.empty.push_front(empty_slab);
            }
        }
        
        Ok(allocated)
    }
    
    /// 向slab释放槽位
    fn dealloc_to_slab(&mut self, slab: Slab<SLOT_SIZE>, slot: HeapSlot) -> Result<(), AllocError> {
        let was_full = slab.is_full();
        slab.meta_mut().dealloc(slot)?;
        
        if was_full {
            // 从满slab移到部分分配链表
            self.remove_slab_from_full(&slab);
            self.partial.push_front(slab);
        } else if slab.is_empty() {
            // 从部分分配链表移到空链表
            self.remove_slab_from_partial(&slab);
            self.empty.push_front(slab);
        }
        
        Ok(())
    }
    
    /// 添加slab到适当的链表
    fn add_slab(&mut self, mut slab: Slab<SLOT_SIZE>) {
        if slab.is_full() {
            self.full.push_front(slab);
        } else if slab.is_empty() {
            self.empty.push_front(slab);
        } else {
            self.partial.push_front(slab);
        }
    }
    
    // 其他辅助方法...
}

// 预期空slab数量
const EXPECTED_EMPTY_SLABS: usize = 2;
```

#### 3.3 大小分类系统

**固定大小分类** (crate/allocator/src/slab/size_class.rs):

```rust
use core::alloc::Layout;

/// 常用大小分类
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
#[repr(usize)]
pub enum CommonSizeClass {
    Bytes8 = 8,
    Bytes16 = 16,
    Bytes32 = 32,
    Bytes64 = 64,
    Bytes128 = 128,
    Bytes256 = 256,
    Bytes512 = 512,
    Bytes1024 = 1024,
    Bytes2048 = 2048,
}

impl CommonSizeClass {
    /// 根据布局确定大小分类
    pub fn from_layout(layout: Layout) -> Option<Self> {
        let size = layout.size();
        let align = layout.align();
        
        // 根据大小确定基本分类
        let size_class = match size {
            0 => return None,
            1..=8 => CommonSizeClass::Bytes8,
            9..=16 => CommonSizeClass::Bytes16,
            17..=32 => CommonSizeClass::Bytes32,
            33..=64 => CommonSizeClass::Bytes64,
            65..=128 => CommonSizeClass::Bytes128,
            129..=256 => CommonSizeClass::Bytes256,
            257..=512 => CommonSizeClass::Bytes512,
            513..=1024 => CommonSizeClass::Bytes1024,
            1025..=2048 => CommonSizeClass::Bytes2048,
            _ => return None,  // 超出Slab处理范围
        };
        
        // 根据对齐要求调整分类
        let align_class = match align {
            0 | 1 | 2 | 4 | 8 => CommonSizeClass::Bytes8,
            16 => CommonSizeClass::Bytes16,
            32 => CommonSizeClass::Bytes32,
            64 => CommonSizeClass::Bytes64,
            128 => CommonSizeClass::Bytes128,
            256 => CommonSizeClass::Bytes256,
            512 => CommonSizeClass::Bytes512,
            1024 => CommonSizeClass::Bytes1024,
            2048 => CommonSizeClass::Bytes2048,
            _ => return None,  // 超出Slab处理范围
        };
        
        // 选择更大的满足对齐要求的分类
        Some(if size_class > align_class {
            size_class
        } else {
            align_class
        })
    }
    
    /// 获取分类大小
    pub fn size(&self) -> usize {
        *self as usize
    }
    
    /// 从大小获取分类
    pub fn from_size(size: usize) -> Option<Self> {
        let layout = Layout::from_size_align(size, 1).ok()?;
        Self::from_layout(layout)
    }
}

/// 大小分类集合
#[derive(Debug)]
pub struct SizeClass {
    class: CommonSizeClass,
}

impl SizeClass {
    /// 创建新的大小分类
    pub fn new(class: CommonSizeClass) -> Self {
        Self { class }
    }
    
    /// 从布局创建大小分类
    pub fn from_layout(layout: Layout) -> Option<Self> {
        CommonSizeClass::from_layout(layout).map(|class| Self { class })
    }
    
    /// 获取分类大小
    pub fn size(&self) -> usize {
        self.class.size()
    }
    
    /// 获取分类枚举
    pub fn class(&self) -> CommonSizeClass {
        self.class
    }
}

impl From<CommonSizeClass> for SizeClass {
    fn from(class: CommonSizeClass) -> Self {
        Self { class }
    }
}
```

#### 3.4 Per-CPU对象缓存

**对象缓存结构** (crate/allocator/src/slab/object_cache.rs):

```rust
use core::cell::RefCell;

/// Per-CPU对象缓存
pub struct ObjectCache<const SLOT_SIZE: usize> {
    list: SlabSlotList<SLOT_SIZE>,
    list_size: usize,
}

impl<const SLOT_SIZE: usize> ObjectCache<SLOT_SIZE> {
    /// 创建新的对象缓存
    pub fn new() -> Self {
        Self {
            list: SlabSlotList::new(),
            list_size: 0,
        }
    }
    
    /// 从本地缓存分配对象
    pub fn alloc(&mut self) -> Result<HeapSlot, AllocError> {
        // 优先从本地缓存分配
        if let Some(slot) = self.list.pop() {
            self.list_size -= SLOT_SIZE;
            return Ok(slot);
        }
        
        // 本地缓存为空，从全局池批量填充
        self.refill_from_global()?;
        
        // 再次尝试从本地缓存分配
        if let Some(slot) = self.list.pop() {
            self.list_size -= SLOT_SIZE;
            Ok(slot)
        } else {
            Err(AllocError)
        }
    }
    
    /// 释放对象到本地缓存
    pub fn dealloc(&mut self, slot: HeapSlot) -> Result<(), AllocError> {
        if slot.size() != SLOT_SIZE {
            return Err(AllocError);
        }
        
        // 检查本地缓存是否过大
        if self.list_size >= OBJ_CACHE_MAX_SIZE {
            // 本地缓存过大，批量释放到全局池
            self.flush_to_global()?;
        }
        
        self.list.push(slot);
        self.list_size += SLOT_SIZE;
        Ok(())
    }
    
    /// 从全局池填充本地缓存
    fn refill_from_global(&mut self) -> Result<(), AllocError> {
        let size_class = SizeClass::from_size(SLOT_SIZE)
            .ok_or(AllocError)?;
        let class = size_class.class();
        
        // 从全局池批量获取对象
        let mut global_pool = GLOBAL_SLAB_POOL.lock();
        for _ in 0..OBJ_CACHE_REFILL_COUNT {
            if let Ok(slot) = global_pool.alloc(class) {
                self.list.push(slot);
                self.list_size += SLOT_SIZE;
            } else {
                break;
            }
        }
        
        Ok(())
    }
    
    /// 批量释放到全局池
    fn flush_to_global(&mut self) -> Result<(), AllocError> {
        if self.list.is_empty() {
            return Ok(());
        }
        
        let size_class = SizeClass::from_size(SLOT_SIZE)
            .ok_or(AllocError)?;
        let class = size_class.class();
        
        let mut global_pool = GLOBAL_SLAB_POOL.lock();
        let mut flush_count = 0;
        
        // 批量释放到全局池，但保留一些对象在本地缓存
        while self.list_size > OBJ_CACHE_MIN_SIZE && flush_count < OBJ_CACHE_FLUSH_COUNT {
            if let Some(slot) = self.list.pop() {
                global_pool.dealloc(slot, class).ok();
                self.list_size -= SLOT_SIZE;
                flush_count += 1;
            } else {
                break;
            }
        }
        
        Ok(())
    }
    
    /// 获取缓存大小
    pub fn size(&self) -> usize {
        self.list_size
    }
}

// 全局Slab池
static GLOBAL_SLAB_POOL: spin::SpinNoIrq<GlobalSlabPool> = spin::SpinNoIrq::new(GlobalSlabPool::new());

// 对象缓存配置常量
const OBJ_CACHE_MIN_SIZE: usize = 1024;      // 最小缓存大小
const OBJ_CACHE_MAX_SIZE: usize = 64 * 1024; // 最大缓存大小
const OBJ_CACHE_REFILL_COUNT: usize = 16;    // 填充对象数量
const OBJ_CACHE_FLUSH_COUNT: usize = 16;    // 刷新对象数量

/// CPU本地对象缓存
cpu_local! {
    static LOCAL_OBJ_CACHES: RefCell<[ObjectCache<256>; 9]> = 
        RefCell::new([
            ObjectCache::<8>::new(),
            ObjectCache::<16>::new(),
            ObjectCache::<32>::new(),
            ObjectCache::<64>::new(),
            ObjectCache::<128>::new(),
            ObjectCache::<256>::new(),
            ObjectCache::<512>::new(),
            ObjectCache::<1024>::new(),
            ObjectCache::<2048>::new(),
        ]);
}

/// Per-CPU Slab分配器
pub struct PerCpuSlabAllocator;

impl PerCpuSlabAllocator {
    /// 分配对象
    pub fn alloc(layout: Layout) -> Result<HeapSlot, AllocError> {
        let size_class = SizeClass::from_layout(layout)
            .ok_or(AllocError)?;
        let slot_size = size_class.size();
        
        // 确定使用哪个本地缓存
        let cache_index = match slot_size {
            8 => 0,
            16 => 1,
            32 => 2,
            64 => 3,
            128 => 4,
            256 => 5,
            512 => 6,
            1024 => 7,
            2048 => 8,
            _ => return Err(AllocError),
        };
        
        // 从本地缓存分配
        LOCAL_OBJ_CACHES.get().borrow_mut()[cache_index].alloc()
    }
    
    /// 释放对象
    pub fn dealloc(slot: HeapSlot) -> Result<(), AllocError> {
        let slot_size = slot.size();
        
        // 确定使用哪个本地缓存
        let cache_index = match slot_size {
            8 => 0,
            16 => 1,
            32 => 2,
            64 => 3,
            128 => 4,
            256 => 5,
            512 => 6,
            1024 => 7,
            2048 => 8,
            _ => return Err(AllocError),
        };
        
        // 释放到本地缓存
        LOCAL_OBJ_CACHES.get().borrow_mut()[cache_index].dealloc(slot)
    }
    
    /// 初始化Per-CPU分配器
    pub fn init(page_alloc: *mut dyn PageAllocator) {
        let mut global_pool = GLOBAL_SLAB_POOL.lock();
        global_pool.init(page_alloc);
    }
}
```

---

### 4. 页面分配器抽象接口

#### 4.1 页面分配器Trait

**PageAllocator Trait** (crate/allocator/src/page_allocator.rs):

```rust
/// 页面分配器抽象接口
pub trait PageAllocator: Send + Sync {
    /// 分配连续的物理页面
    fn alloc_frame(&self, size: usize, align: usize) -> Option<Frame>;
    
    /// 释放连续的物理页面
    fn dealloc_frame(&self, frame: Frame);
    
    /// 分配连续页面并返回物理地址
    fn alloc(&self, size: usize, align: usize) -> Option<usize> {
        self.alloc_frame(size, align).map(|frame| frame.start_paddr())
    }
    
    /// 释放连续页面
    fn dealloc(&self, addr: usize, size: usize) {
        let frame = unsafe { Frame::from_unchecked(addr) };
        self.dealloc_frame(frame);
    }
    
    /// 分配指定数量的页面
    fn alloc_pages(&self, num_pages: usize, align_pow2: usize) -> Option<usize> {
        let size = num_pages * PAGE_SIZE;
        let align = if align_pow2 > 0 { 1 << align_pow2 } else PAGE_SIZE;
        self.alloc(size, align)
    }
    
    /// 释放指定数量的页面
    fn dealloc_pages(&self, pos: usize, num_pages: usize) {
        let size = num_pages * PAGE_SIZE;
        self.dealloc(pos, size);
    }
    
    /// 添加空闲内存区域
    fn add_memory(&self, start: usize, size: usize);
    
    /// 获取分配器统计信息
    fn get_stats(&self) -> PageAllocatorStats;
    
    /// 为字节分配器分配页面
    fn alloc_for_byte_allocator(&self, size: usize, align: usize) -> Option<usize> {
        self.alloc(size, align)
    }
    
    /// 为字节分配器释放页面
    fn dealloc_for_byte_allocator(&self, addr: usize, size: usize) {
        self.dealloc(addr, size);
    }
}

/// 页面分配器统计信息
#[derive(Debug, Clone)]
pub struct PageAllocatorStats {
    pub total_pages: usize,      // 总页面数
    pub used_pages: usize,       // 已使用页面数
    pub free_pages: usize,       // 空闲页面数
    pub total_capacity: usize,   // 总容量（字节）
    pub used_capacity: usize,   // 已使用容量（字节）
    pub available_capacity: usize, // 可用容量（字节）
}

impl PageAllocatorStats {
    /// 创建新的统计信息
    pub fn new() -> Self {
        Self {
            total_pages: 0,
            used_pages: 0,
            free_pages: 0,
            total_capacity: 0,
            used_capacity: 0,
            available_capacity: 0,
        }
    }
}
```

#### 4.2 Buddy页面分配器实现

**BuddyPageAllocator结构** (crate/allocator/src/buddy/mod.rs):

```rust
use spin::SpinNoIrq;

/// Buddy页面分配器
pub struct BuddyPageAllocator {
    pools: PerCpuBuddyAllocator,
}

impl BuddyPageAllocator {
    /// 创建新的Buddy页面分配器
    pub const fn new() -> Self {
        Self {
            pools: PerCpuBuddyAllocator,
        }
    }
    
    /// 初始化分配器
    pub fn init(&self, start_vaddr: usize, size: usize) {
        self.pools.add_memory(start_vaddr, size);
    }
    
    /// 添加内存区域
    pub fn add_memory(&self, start_vaddr: usize, size: usize) {
        self.pools.add_memory(start_vaddr, size);
    }
    
    /// 分配连续的物理页面
    pub fn alloc_pages(&self, num_pages: usize, align_pow2: usize) -> Option<usize> {
        // 计算所需的最小阶数
        let required_size = num_pages * PAGE_SIZE;
        let order = if required_size.is_power_of_two() {
            required_size.trailing_zeros() as usize - PAGE_SHIFT
        } else {
            required_size.next_power_of_two().trailing_zeros() as usize - PAGE_SHIFT
        };
        
        self.pools.alloc_pages(order as BuddyOrder)
    }
    
    /// 释放连续的物理页面
    pub fn dealloc_pages(&self, pos: usize, num_pages: usize) {
        // 计算对应的阶数
        let size = num_pages * PAGE_SIZE;
        let order = if size.is_power_of_two() {
            size.trailing_zeros() as usize - PAGE_SHIFT
        } else {
            size.next_power_of_two().trailing_zeros() as usize - PAGE_SHIFT
        };
        
        self.pools.dealloc_pages(pos, order as BuddyOrder);
    }
    
    /// 为字节分配器分配页面
    pub fn alloc_for_byte_allocator(&self, size: usize, align: usize) -> Option<usize> {
        // 计算对齐后的实际大小
        let aligned_size = if size <= align {
            align.max(PAGE_SIZE)
        } else {
            ((size + align - 1) / align) * align
        };
        
        // 确保至少是一个页面
        let alloc_size = aligned_size.max(PAGE_SIZE);
        
        // 计算对应的阶数
        let order = if alloc_size.is_power_of_two() {
            alloc_size.trailing_zeros() as usize - PAGE_SHIFT
        } else {
            alloc_size.next_power_of_two().trailing_zeros() as usize - PAGE_SHIFT
        };
        
        self.pools.alloc_pages(order as BuddyOrder)
    }
    
    /// 为字节分配器释放页面
    pub fn dealloc_for_byte_allocator(&self, addr: usize, size: usize) {
        // 计算对齐后的实际大小
        let aligned_size = if size <= PAGE_SIZE {
            PAGE_SIZE
        } else {
            size.next_power_of_two()
        };
        
        // 确保至少是一个页面
        let dealloc_size = aligned_size.max(PAGE_SIZE);
        
        // 计算对应的阶数
        let order = if dealloc_size.is_power_of_two() {
            dealloc_size.trailing_zeros() as usize - PAGE_SHIFT
        } else {
            dealloc_size.next_power_of_two().trailing_zeros() as usize - PAGE_SHIFT
        };
        
        self.pools.dealloc_pages(addr, order as BuddyOrder);
    }
    
    /// 获取分配器统计信息
    pub fn get_stats(&self) -> PageAllocatorStats {
        self.pools.get_stats()
    }
}

impl PageAllocator for BuddyPageAllocator {
    fn alloc_frame(&self, size: usize, align: usize) -> Option<Frame> {
        let addr = self.alloc(size, align)?;
        Some(unsafe { Frame::from_unchecked(addr) })
    }
    
    fn dealloc_frame(&self, frame: Frame) {
        self.dealloc(frame.start_paddr(), frame.size());
    }
    
    fn alloc(&self, size: usize, align: usize) -> Option<usize> {
        // 确保大小和对齐是页面大小的整数倍
        let aligned_size = (size + PAGE_SIZE - 1) & !(PAGE_SIZE - 1);
        let aligned_align = (align + PAGE_SIZE - 1) & !(PAGE_SIZE - 1);
        
        // 计算所需的最小阶数
        let alloc_size = aligned_size.max(aligned_align);
        let order = if alloc_size.is_power_of_two() {
            alloc_size.trailing_zeros() as usize - PAGE_SHIFT
        } else {
            alloc_size.next_power_of_two().trailing_zeros() as usize - PAGE_SHIFT
        };
        
        self.pools.alloc_pages(order as BuddyOrder)
    }
    
    fn dealloc(&self, addr: usize, size: usize) {
        // 计算对应的阶数
        let aligned_size = (size + PAGE_SIZE - 1) & !(PAGE_SIZE - 1);
        let dealloc_size = aligned_size.max(PAGE_SIZE);
        
        let order = if dealloc_size.is_power_of_two() {
            dealloc_size.trailing_zeros() as usize - PAGE_SHIFT
        } else {
            dealloc_size.next_power_of_two().trailing_zeros() as usize - PAGE_SHIFT
        };
        
        self.pools.dealloc_pages(addr, order as BuddyOrder);
    }
    
    fn alloc_pages(&self, num_pages: usize, align_pow2: usize) -> Option<usize> {
        self.alloc_pages(num_pages, align_pow2)
    }
    
    fn dealloc_pages(&self, pos: usize, num_pages: usize) {
        self.dealloc_pages(pos, num_pages);
    }
    
    fn add_memory(&self, start: usize, size: usize) {
        self.add_memory(start, size);
    }
    
    fn get_stats(&self) -> PageAllocatorStats {
        self.get_stats()
    }
}

// 页面大小相关常量
const PAGE_SIZE: usize = 4096;
const PAGE_SHIFT: usize = 12;
pub type BuddyOrder = usize;
```

---

### 5. 字节分配器实现

#### 5.1 字节分配器抽象接口

**ByteAllocator Trait** (crate/allocator/src/byte_allocator.rs):

```rust
use core::alloc::Layout;
use core::ptr::NonNull;

/// 字节分配器抽象接口
pub trait ByteAllocator: Send + Sync {
    /// 分配内存
    fn alloc(&self, layout: Layout) -> AllocResult<NonNull<u8>>;
    
    /// 释放内存
    fn dealloc(&self, ptr: NonNull<u8>, layout: Layout) -> AllocResult;
    
    /// 重新分配内存
    fn realloc(&self, ptr: NonNull<u8>, old_layout: Layout, new_layout: Layout) -> AllocResult<NonNull<u8>> {
        // 默认实现：分配新内存，复制数据，释放旧内存
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
    
    /// 设置页面分配器引用
    fn set_page_allocator_raw(&mut self, page_alloc: *mut dyn PageAllocator);
    
    /// 获取分配器统计信息
    fn alloc_stats(&self) -> ByteAllocatorStats;
}

/// 字节分配器统计信息
#[derive(Debug, Clone)]
pub struct ByteAllocatorStats {
    pub used_bytes: usize,         // 已使用字节数
    pub available_bytes: usize,    // 可用字节数
    pub total_bytes: usize,        // 总字节数
    pub allocations: usize,        // 分配次数
    pub deallocations: usize,      // 释放次数
    pub slab_stats: SlabStats,     // Slab统计信息
}

impl ByteAllocatorStats {
    /// 创建新的统计信息
    pub fn new() -> Self {
        Self {
            used_bytes: 0,
            available_bytes: 0,
            total_bytes: 0,
            allocations: 0,
            deallocations: 0,
            slab_stats: SlabStats::new(),
        }
    }
}

/// Slab统计信息
#[derive(Debug, Clone)]
pub struct SlabStats {
    pub active_slabs: usize,       // 活跃slab数量
    pub empty_slabs: usize,         // 空slab数量
    pub partial_slabs: usize,       // 部分分配slab数量
    pub full_slabs: usize,          // 满slab数量
    pub total_slots: usize,         // 总槽位数
    pub used_slots: usize,          // 已使用槽位数
}

impl SlabStats {
    /// 创建新的统计信息
    pub fn new() -> Self {
        Self {
            active_slabs: 0,
            empty_slabs: 0,
            partial_slabs: 0,
            full_slabs: 0,
            total_slots: 0,
            used_slots: 0,
        }
    }
}
```

#### 5.2 Slab字节分配器实现

**SlabByteAllocator结构** (crate/allocator/src/slab/mod.rs):

```rust
use spin::SpinNoIrq;

/// Slab字节分配器
pub struct SlabByteAllocator {
    caches: [SpinNoIrq<SlabCacheCache>; 9],  // 不同大小的Slab缓存
    page_alloc: *mut dyn PageAllocator,       // 页面分配器引用
    stats: SpinNoIrq<ByteAllocatorStats>,    // 统计信息
}

impl SlabByteAllocator {
    /// 创建新的Slab字节分配器
    pub const fn new() -> Self {
        const INIT_CACHE: SpinNoIrq<SlabCacheCache> = SpinNoIrq::new(SlabCacheCache::new());
        Self {
            caches: [INIT_CACHE; 9],
            page_alloc: core::ptr::null_mut(),
            stats: SpinNoIrq::new(ByteAllocatorStats::new()),
        }
    }
    
    /// 初始化分配器
    pub fn init(&mut self, page_alloc: *mut dyn PageAllocator) {
        self.page_alloc = page_alloc;
        
        // 初始化所有Slab缓存
        for (i, cache) in self.caches.iter_mut().enumerate() {
            let size = 8 << i;  // 8, 16, 32, 64, 128, 256, 512, 1024, 2048
            cache.lock().init(size, page_alloc);
        }
    }
    
    /// 分配内存
    pub fn alloc(&self, layout: Layout) -> AllocResult<NonNull<u8>> {
        let size_class = SizeClass::from_layout(layout)
            .ok_or(AllocError)?;
        let slot_size = size_class.size();
        
        // 确定使用哪个Slab缓存
        let cache_index = match slot_size {
            8 => 0,
            16 => 1,
            32 => 2,
            64 => 3,
            128 => 4,
            256 => 5,
            512 => 6,
            1024 => 7,
            2048 => 8,
            _ => return Err(AllocError),
        };
        
        // 从对应的Slab缓存分配
        let slot = self.caches[cache_index].lock().alloc()?;
        
        // 更新统计信息
        self.update_stats_on_alloc(slot_size);
        
        Ok(slot.ptr())
    }
    
    /// 释放内存
    pub fn dealloc(&self, ptr: NonNull<u8>, layout: Layout) -> AllocResult {
        let size_class = SizeClass::from_layout(layout)
            .ok_or(AllocError)?;
        let slot_size = size_class.size();
        
        // 创建HeapSlot对象
        let slot = unsafe { HeapSlot::new(ptr, SlotInfo::SlabSlot(slot_size)) };
        
        // 确定使用哪个Slab缓存
        let cache_index = match slot_size {
            8 => 0,
            16 => 1,
            32 => 2,
            64 => 3,
            128 => 4,
            256 => 5,
            512 => 6,
            1024 => 7,
            2048 => 8,
            _ => return Err(AllocError),
        };
        
        // 释放到对应的Slab缓存
        self.caches[cache_index].lock().dealloc(slot)?;
        
        // 更新统计信息
        self.update_stats_on_dealloc(slot_size);
        
        Ok(())
    }
    
    /// 重新分配内存
    pub fn realloc(&self, ptr: NonNull<u8>, old_layout: Layout, new_layout: Layout) -> AllocResult<NonNull<u8>> {
        // 获取旧的大小分类
        let old_size_class = SizeClass::from_layout(old_layout)
            .ok_or(AllocError)?;
        let old_slot_size = old_size_class.size();
        
        // 获取新的大小分类
        let new_size_class = SizeClass::from_layout(new_layout)
            .ok_or(AllocError)?;
        let new_slot_size = new_size_class.size();
        
        // 如果大小分类相同，直接返回原指针
        if old_slot_size == new_slot_size {
            return Ok(ptr);
        }
        
        // 分配新内存
        let new_ptr = self.alloc(new_layout)?;
        
        // 复制数据
        unsafe {
            let copy_size = core::cmp::min(old_layout.size(), new_layout.size());
            core::ptr::copy_nonoverlapping(
                ptr.as_ptr(),
                new_ptr.as_ptr(),
                copy_size
            );
        }
        
        // 释放旧内存
        self.dealloc(ptr, old_layout)?;
        
        Ok(new_ptr)
    }
    
    /// 设置页面分配器引用
    pub fn set_page_allocator_raw(&mut self, page_alloc: *mut dyn PageAllocator) {
        self.page_alloc = page_alloc;
        
        // 更新所有Slab缓存的页面分配器引用
        for cache in self.caches.iter_mut() {
            cache.lock().set_page_allocator(page_alloc);
        }
    }
    
    /// 获取分配器统计信息
    pub fn alloc_stats(&self) -> ByteAllocatorStats {
        let mut stats = self.stats.lock().clone();
        
        // 汇总所有Slab缓存的统计信息
        for cache in self.caches.iter() {
            let cache_stats = cache.lock().get_stats();
            stats.used_bytes += cache_stats.used_bytes;
            stats.available_bytes += cache_stats.available_bytes;
            stats.total_bytes += cache_stats.total_bytes;
            
            // 更新Slab统计信息
            stats.slab_stats.active_slabs += cache_stats.slab_stats.active_slabs;
            stats.slab_stats.empty_slabs += cache_stats.slab_stats.empty_slabs;
            stats.slab_stats.partial_slabs += cache_stats.slab_stats.partial_slabs;
            stats.slab_stats.full_slabs += cache_stats.slab_stats.full_slabs;
            stats.slab_stats.total_slots += cache_stats.slab_stats.total_slots;
            stats.slab_stats.used_slots += cache_stats.slab_stats.used_slots;
        }
        
        stats
    }
    
    /// 更新分配统计信息
    fn update_stats_on_alloc(&self, size: usize) {
        let mut stats = self.stats.lock();
        stats.used_bytes += size;
        stats.allocations += 1;
    }
    
    /// 更新释放统计信息
    fn update_stats_on_dealloc(&self, size: usize) {
        let mut stats = self.stats.lock();
        stats.used_bytes = stats.used_bytes.saturating_sub(size);
        stats.deallocations += 1;
    }
}

impl ByteAllocator for SlabByteAllocator {
    fn alloc(&self, layout: Layout) -> AllocResult<NonNull<u8>> {
        self.alloc(layout)
    }
    
    fn dealloc(&self, ptr: NonNull<u8>, layout: Layout) -> AllocResult {
        self.dealloc(ptr, layout)
    }
    
    fn realloc(&self, ptr: NonNull<u8>, old_layout: Layout, new_layout: Layout) -> AllocResult<NonNull<u8>> {
        self.realloc(ptr, old_layout, new_layout)
    }
    
    fn set_page_allocator_raw(&mut self, page_alloc: *mut dyn PageAllocator) {
        self.set_page_allocator_raw(page_alloc)
    }
    
    fn alloc_stats(&self) -> ByteAllocatorStats {
        self.alloc_stats()
    }
}

/// Slab缓存包装器
pub struct SlabCacheCache {
    cache: Option<SlabCache<256>>,  // 使用最大可能的槽位大小
    slot_size: usize,
}

impl SlabCacheCache {
    /// 创建新的Slab缓存包装器
    pub const fn new() -> Self {
        Self {
            cache: None,
            slot_size: 0,
        }
    }
    
    /// 初始化缓存
    pub fn init(&mut self, slot_size: usize, page_alloc: *mut dyn PageAllocator) {
        self.slot_size = slot_size;
        // 这里需要根据槽位大小创建对应的SlabCache
        // 由于Rust的泛型特性，需要使用宏或trait对象来处理不同大小的槽位
        // 这里简化处理，使用Option<SlabCache<256>>作为示例
    }
    
    /// 从缓存分配
    pub fn alloc(&mut self) -> Result<HeapSlot, AllocError> {
        if let Some(ref mut cache) = self.cache {
            cache.alloc()
        } else {
            Err(AllocError)
        }
    }
    
    /// 释放到缓存
    pub fn dealloc(&mut self, slot: HeapSlot) -> Result<(), AllocError> {
        if let Some(ref mut cache) = self.cache {
            cache.dealloc(slot)
        } else {
            Err(AllocError)
        }
    }
    
    /// 设置页面分配器
    pub fn set_page_allocator(&mut self, page_alloc: *mut dyn PageAllocator) {
        if let Some(ref mut cache) = self.cache {
            // 更新页分配器引用
        }
    }
    
    /// 获取统计信息
    pub fn get_stats(&self) -> SlabCacheStats {
        // 返回统计信息
        SlabCacheStats::new()
    }
}

/// Slab缓存统计信息
#[derive(Debug, Clone)]
pub struct SlabCacheStats {
    pub used_bytes: usize,
    pub available_bytes: usize,
    pub total_bytes: usize,
    pub slab_stats: SlabStats,
}

impl SlabCacheStats {
    pub fn new() -> Self {
        Self {
            used_bytes: 0,
            available_bytes: 0,
            total_bytes: 0,
            slab_stats: SlabStats::new(),
        }
    }
}
```

---

### 6. 统计信息与错误处理

#### 6.1 内存统计信息

**MemoryStats结构** (crate/allocator/src/stats.rs):

```rust
/// 内存分配器统计信息
#[derive(Debug, Clone)]
pub struct MemoryStats {
    pub total_memory: usize,        // 总内存量
    pub used_memory: usize,          // 已使用内存量
    pub free_memory: usize,          // 空闲内存量
    pub total_pages: usize,          // 总页面数
    pub used_pages: usize,           // 已使用页面数
    pub free_pages: usize,           // 空闲页面数
    pub slab_stats: ByteAllocatorStats, // Slab分配器统计
}

impl MemoryStats {
    /// 创建新的统计信息
    pub fn new() -> Self {
        Self {
            total_memory: 0,
            used_memory: 0,
            free_memory: 0,
            total_pages: 0,
            used_pages: 0,
            free_pages: 0,
            slab_stats: ByteAllocatorStats::new(),
        }
    }
    
    /// 打印统计信息
    pub fn print(&self) {
        info!("=== Memory Allocator Statistics ===");
        info!("Total Memory: {} MB", self.total_memory / (1024 * 1024));
        info!("Used Memory: {} MB", self.used_memory / (1024 * 1024));
        info!("Free Memory: {} MB", self.free_memory / (1024 * 1024));
        info!("Total Pages: {}", self.total_pages);
        info!("Used Pages: {}", self.used_pages);
        info!("Free Pages: {}", self.free_pages);
        info!("Slab Allocations: {}", self.slab_stats.allocations);
        info!("Slab Deallocations: {}", self.slab_stats.deallocations);
        info!("Active Slabs: {}", self.slab_stats.slab_stats.active_slabs);
        info!("Empty Slabs: {}", self.slab_stats.slab_stats.empty_slabs);
        info!("Partial Slabs: {}", self.slab_stats.slab_stats.partial_slabs);
        info!("Full Slabs: {}", self.slab_stats.slab_stats.full_slabs);
    }
}
```

#### 6.2 错误处理

**错误类型** (crate/allocator/src/error.rs):

```rust
/// 内存分配错误
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AllocError {
    /// 内存不足
    OutOfMemory,
    /// 无效参数
    InvalidArgument,
    /// 不支持的操作
    Unsupported,
    /// 没有内存
    NoMemory,
    /// 其他错误
    Other,
}

impl core::fmt::Display for AllocError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            AllocError::OutOfMemory => write!(f, "Out of memory"),
            AllocError::InvalidArgument => write!(f, "Invalid argument"),
            AllocError::Unsupported => write!(f, "Unsupported operation"),
            AllocError::NoMemory => write!(f, "No memory available"),
            AllocError::Other => write!(f, "Other error"),
        }
    }
}

/// 内存分配结果
pub type AllocResult<T> = Result<T, AllocError>;
```

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
    // 示例：硬编码内存区域（实际实现应该从设备树或其他来源获取）
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
[package]
name = "axalloc"
version = "0.1.0"
edition = "2021"

[dependencies]
# 依赖allocator模块
axvisor_allocator = { path = "../../crates/allocator" }

# 其他依赖
log = "0.4"
spin = "0.9"
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

## 性能优化机制

### 1. Per-CPU缓存优化

1. **CPU本地页面缓存**：每个CPU维护自己的Buddy页面缓存，减少锁竞争
2. **CPU本地对象缓存**：每个CPU维护自己的Slab对象缓存，加速小对象分配
3. **动态负载均衡**：根据内存压力自动调整缓存大小，保证公平性

### 2. 分层缓存机制

1. **三级缓存结构**：
   - CPU本地缓存（最快）
   - 全局共享缓存（中等）
   - 物理内存（最慢）

2. **批量操作**：
   - 预填充缓存减少系统调用
   - 批量释放提高效率

### 3. 智能分配策略

1. **大小分类**：固定大小槽位，避免碎片化
2. **自动选择**：根据对象大小自动选择最优分配器
3. **延迟分裂**：按需分割大块，减少内存浪费

---

## 迁移策略

### 1. 现有代码迁移

1. **allocator模块**：
   - 将现有的GlobalAllocator实现移动到allocator模块
   - 添加GlobalAlloc trait实现
   - 添加便利函数导出
   - 实现Per-CPU缓存和负载均衡

2. **axalloc模块**：
   - 简化为极简适配层
   - 重新导出allocator模块的接口
   - 保留系统特定的内存检测功能

### 2. 依赖更新

```toml
# 在使用内存分配器的模块中
[dependencies]
axvisor_allocator = { path = "../../crates/allocator" }  # 直接使用allocator模块
# 或者
axalloc = { path = "../../modules/axalloc" }        # 通过axalloc适配层使用
```

---

## 优势分析

### 1. 最小化迁移工作量

- allocator模块只需要添加GlobalAlloc实现和便利函数导出
- axalloc模块几乎不需要修改现有代码
- 应用代码可以继续使用现有接口

### 2. 最大化封装性

- allocator模块是一个完全独立的内存分配器
- 所有内存管理逻辑都在一个模块中
- 清晰的接口边界

### 3. 高性能多核支持

- Per-CPU缓存减少锁竞争
- 动态负载均衡保证公平性
- 分层缓存提高分配效率

### 4. 灵活的使用方式

- 可以直接使用allocator模块（更高效）
- 可以通过axalloc模块使用（更兼容）
- 支持标准库容器的自动集成

### 5. 简化的架构

- 消除了不必要的中间层
- 减少了模块间的复杂交互
- 清晰的职责分离

---

## 总结

这种重设计实现了：

1. **allocator模块成为完全封装的内存分配器**，提供所有必要的接口
2. **axalloc模块简化为适配层**，主要负责系统特定的功能
3. **最小化迁移工作量**，现有代码可以最大程度复用
4. **提供灵活的使用方式**，满足不同场景的需求
5. **高性能多核支持**，通过Per-CPU缓存和负载均衡实现高效分配

这样的设计既保持了良好的模块化，又大大简化了实现复杂度和迁移工作量，同时通过参考asterinas的实现，提供了高性能的内存分配机制。