# Axvisor内存分配器设计文档

## 1. 引言

### 1.1 项目背景

axvisor作为基于ArceOS的Hypervisor，内存管理是其核心基础设施。当前内存分配器架构存在多内存区域支持不足、大页面分配效率不高等问题，需要设计新的架构以满足虚拟化环境的特殊需求。

### 1.2 设计目标

- **多内存区域管理**：支持动态添加和管理多个不连续内存区域
- **虚拟化性能优化**：优化虚拟机大页面内存分配效率
- **长期运行稳定性**：建立健康的内存生态系统，避免内存碎片和泄漏
- **架构可扩展性**：为未来NUMA、内存热插拔等功能预留扩展接口

## 2. 当前架构分析

### 2.1 Level-1单级TLSF分配器（当前默认）

axvisor通过`alloc-level-1`特性启用Level-1架构：

**工作流程**：
1. 所有内存请求（无论大小）都直接由TLSF分配器处理
2. TLSF直接管理全部free物理内存区域
3. 支持动态添加多个内存区域到TLSF内存池

**优势**：
- 架构简单，实现成本较低
- TLSF提供O(1)分配/释放性能
- 支持动态内存区域添加
- 适合实时系统场景

**局限性**：
- 大页面分配效率不是最优
- 长期运行可能存在内存碎片问题

### 2.2 Level-2两级分配器架构（理论存在但实际不可用）

虽然代码中存在Level-2的实现，但由于bitmap分配器的限制，实际上不可用：

```rust
// allocator/src/bitmap.rs
impl<const PAGE_SIZE: usize> BaseAllocator for BitmapPageAllocator<PAGE_SIZE> {
    fn add_memory(&mut self, _start: usize, _size: usize) -> AllocResult {
        Err(AllocError::NoMemory) // unsupported - 不支持动态添加内存
    }
}
```

**Bitmap分配器架构限制**：

1. **单一连续内存模型**：
   ```rust
   pub struct BitmapPageAllocator<const PAGE_SIZE: usize> {
       base: usize,           // 基址，1GB对齐
       total_pages: usize,    // 总页面数  
       used_pages: usize,     // 已使用页面数
       inner: BitAllocUsed,   // 内部bitmap分配器
   }
   ```

2. **单一基址对齐机制**：
   ```rust
   fn init(&mut self, start: usize, size: usize) {
       // 计算基址，强制1GB对齐
       self.base = crate::align_down(start, MAX_ALIGN_1GB); // 0x4000_0000
       
       // 计算在bitmap中的相对位置
       let start_idx = (start - self.base) / PAGE_SIZE;
       self.inner.insert(start_idx..start_idx + self.total_pages);
   }
   ```

**为什么`add_memory`难以实现**：

1. **多区域管理能力缺失**：bitmap分配器设计假设单一连续内存空间
2. **基址对齐约束限制**：1GB对齐要求限制了多区域支持
3. **内部算法的连续性假设**：底层bitmap-allocator库基于连续性假设

## 3. 主流操作系统内存分配器设计启示

### 3.1 通用操作系统的设计哲学

主流操作系统（如Linux、Windows）的需求与TLSF的设计目标存在根本差异，linux内存分配器实现为：

```
应用层 → glibc malloc (ptmalloc) → 系统调用 → Linux内核
    ↓
内核空间：Buddy System (物理页) + SLUB/SLAB (内核对象)
    ↓
硬件抽象层：NUMA感知、内存热插拔等
```

**设计特点**：
- **多级混合架构**：不同层次使用不同分配策略
- **吞吐量优先**：优化多核并发下的平均性能
- **碎片控制**：针对长期运行服务的碎片优化
- **通用性**：适应从嵌入式到数据中心的各类场景

### 3.2 为什么主流操作系统不采用TLSF作为核心分配器

| 特性 | TLSF (实时系统) | 通用操作系统 (Linux/Windows) |
|------|-----------------|-----------------------------|
| 首要目标 | 确定性、最坏情况性能 | 吞吐量、平均性能 |
| 锁策略 | 全局锁或简单锁机制 | 复杂的无锁/细粒度锁 |
| 内存视图 | 连续内存池管理 | 虚拟内存+物理页帧管理 |
| 工作负载 | 相对固定、可预测 | 极其多样、不可预测 |

**具体技术差异**：
1. **多核可扩展性**：TLSF的全局锁成为多核瓶颈，而Linux的SLUB使用每CPU缓存
2. **虚拟内存支持**：通用系统需要复杂的虚拟内存管理，TLSF设计为物理内存管理
3. **工作负载适应性**：通用系统需要适应从字节到GB的各种需求

## 4. Buddy+Slab vs Bitmap+TLSF架构对比

### 4.1 架构概述

#### 4.1.1 当前架构（Bitmap+TLSF）
```
内存请求分类：
├── 小内存(<4KB) ──→ TLSF字节分配器 (O(1)分配, O(1)释放)
└── 大内存(≥4KB) ──→ Bitmap页分配器 (O(n)分配, O(1)释放)
```

#### 4.1.2 目标架构（Buddy+Slab）
```
内存请求分类：
├── 小内存(<4KB) ──→ Slab字节分配器 (O(1)分配, O(1)释放)
└── 大内存(≥4KB) ──→ Buddy页分配器 (O(log n)分配, O(log n)释放)
└── 多区域支持   ──→ MultiRegionBuddyPageAllocator
```

### 4.2 Buddy系统的关键优势

#### 4.2.1 多区域支持
- **Buddy**: 完美匹配不连续内存布局，支持多内存区域
- **Bitmap**: 单一连续模型，不支持不连续内存区域，存在根本性架构限制

**Buddy System组织方式**：
```
内存块组织（以4KB页面为单位）：
Order 0: 1个4KB页面(4KB)     → 空闲链表0
Order 1: 2个4KB页面(8KB)     → 空闲链表1  
Order 2: 4个4KB页面(16KB)    → 空闲链表2
...
Order 9: 512个4KB页面(2MB)   → 空闲链表9
Order 10: 1024个4KB页面(4MB) → 空闲链表10
...
Order 18: 262,144个4KB页面(1GB) → 空闲链表18
```

**关键特征**：
- **页面数量**：Order n 包含 2^n 个4KB页面
- **实际大小**：Order n = 2^n × 4KB
- **伙伴关系**：相邻的Order n块可以合并为Order n+1块
- **快速分配**：直接根据所需页面数计算Order，从对应链表分配

#### 4.2.2 大块分配效率

**场景：分配1GB连续内存**

**Buddy方式**：
1. **计算Order**: `order = ceil(log2(1GB / 4KB)) = 18`
2. **查找链表**: 检查Order 18空闲链表是否有可用块
3. **分配策略**:
   - 有空闲块: 直接分配 O(log n)
   - 无空闲块: 向高阶查找并分裂 O(log n)

**Bitmap方式**：
- 扫描262,144个连续位，最坏O(n)时间
- 碎片化时性能急剧下降

**性能对比**：
- **Buddy**: 直接查找Order 18链表，O(log n)时间
- **Bitmap**: 扫描262,144个连续位，最坏O(n)时间

### 4.3 内存生态完整性对比

#### 4.3.1 Buddy+Slab：双向流动的健康生态

```
Slab层 ↔ Buddy层
    ↓     ↓
动态分配 → 动态回收
空Slab批量回收 → 页帧返还Buddy → 伙伴块合并 → 更大块可用
```

**特征**：
- **双向流动**：内存在不同层级间自由流动
- **自动修复**：通过合并机制维护大块内存可用性
- **动态平衡**：系统长期运行保持健康状态
- **资源优化**：释放的内存可立即被其他组件使用

#### 4.3.2 Bitmap+TLSF：单向流动的架构缺陷

```
TLSF层 → Bitmap层
    ↓
分配 ←→ 释放 ←→ 归还中断
    ↓
内存单向流动，无法有效归还
```

**问题**：
- 内存单向流动，导致内存分配效率下降
- **理论上可实现归还机制**

**结论**：TLSF和Slab在小内存分配上性能相当，Slab更适合通用操作系统。

## 5. Buddy System与Slab分配器详细原理

### 5.1 Buddy System详细原理

#### 5.1.1 核心管理机制

Buddy System将物理内存组织为2的幂次方大小的块，每个大小级别维护一个空闲链表：

**数据结构**：
```rust
struct BuddyAllocator {
    free_lists: [LinkedList; MAX_ORDER], // 各阶空闲链表
    memory_regions: Vec<MemoryRegion>,   // 多内存区域支持
}
```

#### 5.1.2 分配算法流程

1. **大小转换**：将请求大小转换为对应的Order（2^n ≥ 请求大小）
2. **链表查找**：在对应Order的空闲链表中查找可用块
3. **块分裂**：如果当前Order无可用块，向更高Order递归分裂
4. **分配完成**：返回合适大小的内存块

#### 5.1.3 释放算法流程

1. **块释放**：将释放的块加入对应Order的空闲链表
2. **伙伴检测**：检查"伙伴"块是否空闲
3. **块合并**：如果伙伴块空闲，合并为更大的块
4. **递归合并**：向上递归合并，直到无法合并为止

### 5.2 Slab分配器详细原理

Slab采用分层缓存设计，平衡速度与内存利用率：

#### 5.2.1 为何需要三级架构

这套架构的核心目标是最大限度地减少内存分配延迟，同时高效管理物理内存：
1. **CPU缓存局部性**：让一个CPU核心尽可能访问其独占的内存区域
2. **NUMA架构**：优先使用本地内存节点上的内存，减少跨节点访问的延迟

#### 5.2.2 三级缓存详解

1. **每CPU缓存 (Per-CPU Cache)**
   - **职责**：分配请求的快速路径，实现无锁分配
   - **关键字段**：
     - `freelist`：指向当前活跃Slab中下一个可用空闲对象
     - `page`：指向当前正在为此CPU服务的Slab
     - `partial`：每CPU部分空Slab链表
   - **工作方式**：LIFO策略，最大化利用CPU硬件缓存

2. **Slab节点缓存 (Node Cache)**
   - **职责**：NUMA节点内的内存协调
   - **关键字段**：
     - `partial`：管理此NUMA节点上所有部分被使用的Slab
     - `nr_partial`：partial链表上Slab的数量
   - **工作方式**：批量迁移Slab，减少跨节点访问

3. **全局Slab列表与伙伴系统**
   - **职责**：内存分配的最后防线
   - **工作方式**：向伙伴系统申请新的连续页框，创建全新Slab

#### 5.2.3 协同工作流程

1. **分配对象**：
   - 优先从每CPU缓存的freelist获取（最快）
   - 若空，检查每CPU的partial链表
   - 若仍空，从节点缓存的partial链表批量迁移Slab
   - 若节点缓存也空，向伙伴系统申请新页面创建Slab

2. **释放对象**：
   - 对象优先返回到每CPU缓存的freelist
   - 当Per-CPU缓存中空闲对象过多时，批量返回到节点缓存
   - 完全空闲的Slab在内存压力下会销毁，页面归还给伙伴系统

## 6. 设计方案

### 6.1 整体架构设计

```rust
// 新的内存分配器架构
pub struct AxvisorMemoryManager {
    // 页面级分配器 - Buddy System
    page_allocator: MultiRegionBuddyPageAllocator,
    
    // 字节级分配器 - Slab
    byte_allocator: SlabByteAllocator,
    
    // VM内存域管理
    vm_memory_domains: HashMap<VmId, VmMemoryDomain>,
    
    // 全局配置
    config: MemoryConfig,
}
```

### 6.2 核心组件设计

#### 6.2.1 MultiRegionBuddyPageAllocator

**设计目标**：
- 支持多个不连续内存区域
- 提供2^n从4KB开始的全Order支持
- 针对虚拟化场景优化的大页面分配

#### 6.2.2 三级缓存Slab分配器

**设计目标**：
- 基于现有slab.rs实现，重构为三级缓存
- 实现PerCpuCache无锁访问

#### 6.2.3 并发控制优化

**细粒度锁设计**：
- 每个MemoryRegion独立锁
- 每个VmMemoryDomain独立锁
- 每个PerCpuCache无锁访问

**无锁化优化**：
```rust
pub struct PerCpuCache {
    freelist: AtomicPtr<SlabObject>,     // 无锁freelist
    page: AtomicPtr<SlabPage>,           // 当前Slab页
    batch_size: usize,                   // 批量操作大小
}
```
通过这种针对性的架构优化，axvisor能够在保持现有稳定性的基础上，显著提升虚拟化环境下的内存管理效率和扩展性，为构建高性能Hypervisor平台奠定坚实基础。