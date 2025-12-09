# Axvisor MMU 初始化流程详解

## 概述

内存管理单元（MMU）初始化是 Axvisor 启动的核心环节，负责建立虚拟地址到物理地址的翻译机制。Axvisor 作为 Type-1 Hypervisor 运行在 ARM64 EL2 异常级别，需要配置双重 MMU：自身运行的 EL2 MMU 和为虚拟机服务的 Stage-2 MMU。

### MMU 基础原理

内存管理单元（Memory Management Unit，MMU）是计算机系统中的关键硬件组件，负责将虚拟地址转换为物理地址。在现代操作系统中，MMU提供了以下核心功能：

1. **地址翻译**：将CPU发出的虚拟地址转换为内存控制器可识别的物理地址
2. **内存保护**：通过权限检查确保进程只能访问授权的内存区域
3. **内存隔离**：为不同进程或虚拟机提供独立的地址空间
4. **缓存控制**：管理内存访问的缓存策略

### MMU 初始化的架构目标

```
┌─────────────────────────────────────────────────────────────────┐
│                    Axvisor MMU 架构                             │
├─────────────────────────────────────────────────────────────────┤
│  EL2 MMU (Hypervisor自身)                                       │
│  ┌─────────────────┐    ┌─────────────────────────────────┐     │
│  │   HVA → HPA     │◄──►│  Axvisor代码、数据、栈管理        │    │
│  └─────────────────┘    └─────────────────────────────────┘     │
├─────────────────────────────────────────────────────────────────┤
│  Stage-2 MMU (虚拟机服务)                                        │
│  ┌─────────────────┐    ┌─────────────────────────────────┐     │
│  │   GPA → HPA     │◄──►│  Guest物理地址隔离和翻译          │     │
│  └─────────────────┘    └─────────────────────────────────┘     │
├─────────────────────────────────────────────────────────────────┤
│  两阶段翻译机制 (Guest运行时)                                     │
│  ┌─────────────────┐    ┌─────────────────┐                     │
│  │ GVA → GPA       │    │ GPA → HPA       │                     │
│  │ (Guest OS)      │    │ (Axvisor)       │                     │
│  └─────────────────┘    └─────────────────┘                     │
└─────────────────────────────────────────────────────────────────┘
```

## ARM64 虚拟化 MMU 架构

### 异常级别与地址翻译原理

ARM64 架构支持 4 级页表结构，实现 48 位虚拟地址空间：

```
虚拟地址格式（48位）：
[47:39] [38:30] [29:21] [20:12] [11:0]
   L0索引   L1索引   L2索引   L3索引   页内偏移
    ↓        ↓        ↓        ↓        ↓
  L0表 →   L1表 →   L2表 →   L3表 →   物理页面
```

ARM64 定义了 4 个异常级别（EL0-EL3），Axvisor 主要运行在 EL2（Hypervisor 模式）：

```
EL3: Secure Monitor (安全监控)
EL2: Hypervisor (虚拟化管理器) ← axvisor运行在此级别
EL1: OS Kernel (操作系统内核)
EL0: User Applications (用户应用)
```

### 虚拟化环境下的双重翻译

在虚拟化环境中，地址翻译分为两个阶段：

```
Guest虚拟地址(GVA) → Guest物理地址(GPA) → Host物理地址(HPA)
        ↓                    ↓                    ↓
    Stage-1翻译          Stage-2翻译           最终物理地址
   (Guest OS管理)       (axvisor管理)         (硬件访问)
```

**实际示例：**
```
虚拟机中的程序访问地址0x4000_0000：
1. Guest OS将0x4000_0000翻译为GPA 0x8000_0000
2. axvisor将GPA 0x8000_0000翻译为HPA 0x2_0000_0000
3. 最终访问物理内存0x2_0000_0000
```

**异常级别的硬件机制**：
- **权限隔离**：每个级别有不同的权限和访问范围
- **状态保存**：异常切换时自动保存/恢复处理器状态
- **虚拟化支持**：EL2 专门用于虚拟化，提供额外的控制寄存器

## 两阶段 MMU 初始化策略

### 为什么采用两阶段设计？

Axvisor 采用两阶段 MMU 初始化策略是为了解决启动过程中的"鸡生蛋"问题：

**问题分析：**
1. 启动初期运行在物理地址模式，无 MMU 保护
2. 需要建立页表才能启用 MMU
3. 但建立页表本身就需要在虚拟地址空间进行操作
4. 需要一个过渡机制确保安全切换

**解决方案：两阶段 MMU 初始化**
```
启动流程：
物理地址运行 → 阶段1(临时MMU) → 虚拟地址运行 → 阶段2(完整MMU) → 生产环境
    ↓            ↓              ↓             ↓             ↓
 最小保护     基础映射       过渡状态      完整配置     高效运行
```

**实际启动流程**：
```
1. Boot Loader 阶段 (pie-boot-loader-aarch64)
   └── entry() → enable_mmu() → new_boot_table()  ← 启动页表建立

2. Hypervisor 主程序阶段 (axvisor)
   └── axhal::init_early() → axplat_aarch64_dyn::init_early() → mem::setup()
     └── axhal::init_later() → 完整页表建立
```

### 两阶段对比

| 特性 | 阶段1(Loader) | 阶段2(SomeHAL) |
|------|---------------|----------------|
| **运行环境** | 物理地址模式 | 虚拟地址模式 |
| **页表类型** | 临时页表 | 生产页表 |
| **映射范围** | 最小必要区域 | 完整内存空间 |
| **目标** | 安全跳转到虚拟地址 | 建立完整内存管理 |
| **初始化时机** | boot loader阶段 | 内核初始化阶段 |
| **代码位置** | `pie-boot-loader-aarch64` | `somehal` |

### 阶段 1：Loader 基础 MMU 建立

#### 核心目标

在物理地址运行阶段建立最基础的 MMU 配置，实现系统从物理地址模式到虚拟地址模式的安全跳转。这个阶段的关键是**最小化**和**安全性**。

#### 启动页表创建流程

**核心调用链：**
```rust
// 文件位置: somehal/loader/pie-boot-loader-aarch64/src/mmu.rs
pub fn new_boot_table<T, F>(args: &EarlyBootArgs, fdt: usize, new_pte: F) -> PhysAddr
where
    T: TableGeneric,
    F: Fn(CacheKind) -> T::PTE + Copy,
{
    // 1. 初始化内存分配器，用于页表存储
    let mut alloc = Ram {};
    let access = &mut alloc;
    
    // 2. 创建根页表
    let mut table = PageTableRef::<'_, T>::create_empty(access);
    
    unsafe {
        // 3. 内核代码段映射：优先1GB或2MB大页
        let align = if kcode_offset.is_aligned_to(GB) { GB } else { 2 * MB };
        let code_start_phys = args.kimage_addr_lma.align_down(align) as usize;
        let code_start = args.kimage_addr_vma as usize;
        let size = ((code_end - code_start).max(align));
        
        table.map(MapConfig {
            vaddr: code_start.into(),
            paddr: code_start_phys.into(),
            size,
            pte: new_pte(CacheKind::Normal),
            allow_huge: true,
            flush: false,
        }, access);
        
        // 4. RAM内存区域映射：从FDT解析内存布局
        add_rams(fdt, &mut table, access, new_pte);
        
        // 5. 调试设备映射：串口等调试设备
        if debug::reg_base() > 0 {
            table.map(MapConfig {
                vaddr: (debug::reg_base() + KLINER_OFFSET).into(),
                paddr: debug::reg_base().into(),
                size: PAGE_SIZE,
                pte: new_pte(CacheKind::Device),
                allow_huge: true,
                flush: false,
            }, access);
        }
    }
    
    table.paddr()
}
```

#### 设备树内存区域解析

```rust
// 解析FDT中的内存节点并建立映射
fn add_rams<T, F>(fdt: usize, table: &mut PageTableRef<'_, T>, 
                   access: &mut impl Access, new_pte: F) -> Result<(), &'static str> {
    let fdt = Fdt::from_ptr(fdt)?;
    
    // 遍历所有内存节点：memory@addr { reg = <addr size>; }
    for memory in fdt.memory().flat_map(|mem| mem.regions()) {
        if memory.size == 0 { continue; }
        
        let paddr = memory.address as usize;
        let vaddr = paddr + kliner_offset();
        
        unsafe {
            table.map(MapConfig {
                vaddr: vaddr.into(),
                paddr: paddr.into(), 
                size: memory.size,
                pte: new_pte(CacheKind::Normal),
                allow_huge: true,
                flush: false,
            }, access);
        }
    }
    Ok(())
}
```

### 阶段 2：SomeHAL 完善 MMU 配置

#### 核心目标

在虚拟地址运行环境下，建立完整的、生产级别的 MMU 配置。相比阶段1的最小化映射，阶段2提供精细的权限控制和优化的内存布局。

#### 内存区域配置生成

```rust
// 文件位置: somehal/somehal/src/common/mem/mod.rs
pub(crate) fn regions_to_map() -> alloc::vec::Vec<MapRangeConfig> {
    let mut map_ranges = alloc::vec::Vec::new();

    // 1. RAM和Reserved区域映射：系统内存基础
    for region in region_ram_and_rsv() {
        map_ranges.push(MapRangeConfig {
            vaddr: phys_to_virt(region.start),
            paddr: region.start,
            size: region.end - region.start,
            name: "ram",
            cache: CacheKind::Normal,
            access: AccessKind::ReadWrite,
            cpu_share: true,
        });
    }

    // 2. 调试控制台映射：启动调试支持
    if let Some(d) = &boot_info().debug_console {
        map_ranges.push(MapRangeConfig {
            vaddr: (d.base_phys + KLINER_OFFSET) as *mut u8,
            paddr: d.base_phys,
            size: PAGE_SIZE,
            name: "debug-con",
            cache: CacheKind::Device,
            access: AccessKind::ReadWrite,
            cpu_share: true,
        });
    }

    // 3. 内核各段映射：代码和数据的精细控制
    map_ranges.push(ld_range_to_map_config("text", ld::text, true, AccessKind::ReadExecute));
    map_ranges.push(ld_range_to_map_config("rodata", ld::rodata, true, AccessKind::Read));
    map_ranges.push(ld_range_to_map_config("data", ld::data, true, AccessKind::ReadWriteExecute));
    map_ranges.push(ld_range_to_map_config("bss", ld::bss, true, AccessKind::ReadWriteExecute));
    map_ranges.push(ld_range_to_map_config("stack0", ld::stack0, false, AccessKind::ReadWriteExecute));

    map_ranges
}
```

#### 页表项属性配置

```rust
// 文件位置: somehal/loader/pie-boot-loader-aarch64/src/el2.rs
impl Pte {
    pub fn new(cache: CacheKind) -> Self {
        let mut flags = PteFlags::empty() 
            | PteFlags::AF        // Access Flag: 必须设置
            | PteFlags::VALID     // Valid位: 标记页表项有效
            | PteFlags::NON_BLOCK; // Table位: 0表示页表项

        let idx = match cache {
            CacheKind::Device => 0,    // MAIR索引0：设备内存
            CacheKind::Normal => {     // MAIR索引1：普通内存
                flags |= PteFlags::INNER | PteFlags::SHAREABLE;
                1
            }
            CacheKind::NoCache => {    // MAIR索引2：无缓存内存
                flags |= PteFlags::SHAREABLE;
                2
            }
        };

        let mut s = Self(flags.bits());
        s.set_mair_idx(idx);
        s
    }
}
```

## 页表结构和地址翻译

### ARM64 4级页表结构

ARM64 使用 4 级页表结构，支持 48 位虚拟地址空间：

```
虚拟地址翻译流程：
虚拟地址 0x0000_0040_1234_5678
├── L0 索引 [47:39] = 0x000 → L0[0]
├── L1 索引 [38:30] = 0x100 → L1[256] 
├── L2 索引 [29:21] = 0x091 → L2[145]
├── L3 索引 [20:12] = 0x145 → L3[325]
└── 页内偏移 [11:0] = 0x678 → 最终物理地址
```

#### 大页映射策略

| 映射类型 | 翻译层级 | 地址对齐要求 | TLB效率 |
|----------|----------|--------------|----------|
| **4KB页** | L0→L1→L2→L3 | 4KB | 标准 |
| **2MB大页** | L0→L1→L2 | 2MB | 512倍提升 |
| **1GB巨页** | L0→L1 | 1GB | 262144倍提升 |

#### 智能大页选择算法

```rust
// 页表映射中的大页选择逻辑
while size > 0 {
    let level_depth = if config.allow_huge {
        let v_align = self.walk.detect_align_level(map_cfg.vaddr.raw(), size);
        let p_align = self.walk.detect_align_level(map_cfg.paddr.raw(), size);
        let arch_limit = T::MAX_BLOCK_LEVEL;
        v_align.min(p_align).min(arch_limit)  // 选择三者中最小值
    } else {
        1  // 强制使用4KB页
    };
    
    // 执行映射并更新进度
    let map_size = self.walk.copy_with_level(level_depth).level_entry_size();
    map_cfg.vaddr += map_size;
    map_cfg.paddr += map_size;
    size -= map_size;
}
```

## MMU 配置和优化策略

### 缓存属性配置

**内存类型分类**：
- **Device**：设备内存，强有序，无缓存（UART、GPIO 寄存器）
- **Normal**：普通内存，写回缓存（主内存、程序代码、数据结构）
- **NoCache**：非缓存内存，强一致性（帧缓冲、DMA 缓冲区）
- **WriteThrough**：写通缓存，平衡性能（预留扩展）

### MAIR（Memory Attribute Indirection Register）配置

MAIR 寄存器定义了 8 种内存属性组合，页表项通过 3 位索引引用这些属性：

```rust
// MAIR寄存器配置示例
pub fn setup_table_regs() {
    // Attr0: 设备内存 - 非缓存，强顺序访问
    let attr0 = MAIR_EL2::Attr0_Device::nonGathering_nonReordering_EarlyWriteAck;
    
    // Attr1: 普通内存 - 写回缓存，最高性能
    let attr1 = MAIR_EL2::Attr1_Normal_Inner::WriteBack_NonTransient_ReadWriteAlloc
        + MAIR_EL2::Attr1_Normal_Outer::WriteBack_NonTransient_ReadWriteAlloc;
    
    // Attr2: 无缓存内存 - 禁用缓存，强一致性
    let attr2 = MAIR_EL2::Attr2_Normal_Inner::NonCacheable 
        + MAIR_EL2::Attr2_Normal_Outer::NonCacheable;

    MAIR_EL2.write(attr0 + attr1 + attr2);
}
```

**属性索引使用场景**：
- **索引 0**：Device - MMIO 设备、串口寄存器
- **索引 1**：Normal - 主内存、程序代码、数据结构  
- **索引 2**：NoCache - 帧缓冲、DMA 缓冲区
- **索引 3**：WriteThrough - 预留扩展

## 地址翻译流程

### EL2 地址翻译

Axvisor 访问虚拟地址时的翻译过程：
1. **虚拟地址输入**：Axvisor 代码访问虚拟地址
2. **页表查询**：查询 EL2 页表（TTBR0_EL2）
3. **地址翻译**：将 HVA 翻译为 HPA
4. **物理访问**：最终访问物理内存

### Stage-2 地址翻译

虚拟机内存访问的硬件自动两阶段翻译：

```
Guest 访问: 0x4000_0000
1. Stage-1: Guest VA 0x4000_0000 → Guest IPA 0x8000_0000
2. Stage-2: Guest IPA 0x8000_0000 → Host PA 0x2000_0000
3. 硬件访问: 物理内存 0x2000_0000
```

**翻译特点**：
- **硬件自动**：两阶段翻译完全由硬件完成
- **性能高效**：Guest OS 无感知，延迟最小
- **安全隔离**：Hypervisor 控制最终内存访问权限

---

## 总结

Axvisor 的 MMU 初始化是一个精密而复杂的系统工程，体现了以下核心特点：

### 1. 分层设计原则
- **两阶段初始化**：快速启动建立基础环境，后期完善完整功能
- **双重 MMU 架构**：同时管理自身内存（EL2）和为虚拟机提供服务（Stage-2）
- **清晰的职责分离**：Boot Loader 建立基础映射，SomeHAL 完善生产配置

### 2. 硬件特性充分利用
- **ARM64 虚拟化扩展**：充分利用 EL2 异常级别的特殊能力
- **两阶段地址翻译**：硬件自动完成 Guest 到 Physical 的翻译
- **大页支持**：4KB、2MB、1GB 多种页表粒度，灵活配置

### 3. 性能优化策略
- **大页映射**：智能选择最优页表粒度，减少 TLB 压力
- **TLB 智能刷新**：按需精确刷新，减少性能开销
- **缓存属性优化**：MAIR 机制为不同内存类型选择合适缓存策略

这个 MMU 系统为 Axvisor 提供了高效、安全的内存管理基础，是实现可靠 Hypervisor 的核心技术支撑。通过精密的硬件配置和智能的软件管理，Axvisor 能够在保证安全性的同时，提供接近物理机的虚拟化性能。

