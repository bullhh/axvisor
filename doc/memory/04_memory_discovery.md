# 内存嗅探流程详解

## 1 概述

内存嗅探是 Axvisor 启动过程中最关键的环节之一，它负责识别、分类和组织物理内存资源。在虚拟化环境中，准确的内存嗅探不仅关系到 Hypervisor 自身的稳定运行，更直接影响后续虚拟机的内存分配和管理。本节将深入分析 Axvisor 内存嗅探的完整实现原理和关键机制。

---

### 1.1 平台选择机制：`dyn-plat` 特性的作用

### 1.1.1 特性定义与传递

在 `kernel/Cargo.toml` 中定义了 `dyn-plat` 特性：

```toml
[features]
dyn-plat = ["axstd/myplat", "axstd/driver-dyn", "axruntime/driver-dyn"]
```

### 1.1.2 条件编译逻辑

在 `axhal/src/lib.rs` 中，使用 `cfg_if!` 宏进行平台选择：

```rust
cfg_if::cfg_if! {
    if #[cfg(feature = "myplat")] {
        // 当启用 myplat 特性时，跳过默认平台选择
        // link the custom platform crate in your application.
    } else if #[cfg(target_os = "none")] {
        // 标准平台选择（当 myplat 未启用时）
        #[cfg(target_arch = "x86_64")]
        extern crate axplat_x86_pc;
        #[cfg(target_arch = "aarch64")]
        extern crate axplat_aarch64_qemu_virt;
        #[cfg(target_arch = "riscv64")]
        extern crate axplat_riscv64_qemu_virt;
        #[cfg(target_arch = "loongarch64")]
        extern crate axplat_loongarch64_qemu_virt;
    } else {
        // 测试环境使用 dummy 平台
        mod dummy;
    }
}
```

**关键机制说明：**

1. **特性传递链**：`dyn-plat` → `axstd/myplat` → `axhal/myplat`
2. **条件编译控制**：当 `myplat` 特性启用时，编译器会忽略默认平台选择
3. **外部注入机制**：通过 `axruntime` 的 `extern crate axplat_aarch64_dyn` 注入具体实现
4. **链接时绑定**：Rust 链接器在链接时将 `axplat` 接口调用绑定到具体平台实现

这种设计使得 `axhal` 既保持了接口的稳定性，又支持灵活的平台扩展。

## 2. 底层硬件抽象：`somehal` 的识别流程

### 2.1 `somehal` 的作用

`somehal` 是一个轻量级的硬件抽象层，为 `axplat-aarch64-dyn` 提供底层硬件访问能力。

### 2.2 `somehal` 提供的功能

#### 2.2.1 **CPU 信息获取**：`cpu_id_list()` 返回可用的 CPU ID 列表

**实现位置**：`src/common/mem/stack.rs`

```rust
pub fn cpu_id_list() -> impl Iterator<Item = usize> {
    let mut start = unsafe { STACK_START };
    let end = unsafe { STACK_END };
    let len = stack0().len().align_up(page_size());
    
    // 组合主 CPU 和次级 CPU ID
    [boot_info().cpu_id]
        .into_iter()
        .chain(core::iter::from_fn(move || {
            if start >= end {
                return None;
            }
            // 从栈底读取 CPU ID
            let id = unsafe { (phys_to_virt(start) as *const usize).read() };
            let ret = Some(id);
            start += len;
            ret
        }))
}
```

**工作原理**：
- 主 CPU ID 来自 `boot_info().cpu_id`
- 次级 CPU ID 存储在各自的栈底，通过遍历物理内存区域获取

#### 2.2.2 **设备树解析**：解析 bootloader 传递的设备树信息

**实现位置**：`src/common/fdt/mod.rs`

```rust
pub fn cpu_id_list() -> impl Iterator<Item = usize> {
    let fdt = fdt().expect("FDT not found");
    let nodes = fdt.find_nodes("/cpus/cpu");
    nodes
        .filter(|node| node.name().contains("cpu@"))
        .filter(|node| !matches!(node.status(), Some(Status::Disabled)))
        .map(|node| {
            let reg = node
                .reg()
                .unwrap_or_else(|| panic!("cpu {} reg not found", node.name()))
                .next()
                .unwrap();
            reg // 返回 CPU 的物理地址作为 ID
        })
}
```

**设备树功能**：
- 解析 `/cpus/cpu@*` 节点获取 CPU 信息
- 过滤禁用的 CPU 核心
- 提取 CPU 寄存器地址作为唯一标识

#### 2.2.3 **内存区域识别**：识别 RAM、MMIO、保留区域等

**实现位置**：`src/common/mem/mod.rs`

```rust
fn init_regions(args_regions: &[MemoryRegion]) {
    let mut regions = MEMORY_REGIONS.lock();
    regions.extend_from_slice(args_regions)
        .expect("Memory regions overflow");

    // 对齐所有区域到页边界
    for region in regions.iter_mut() {
        if !region.end.is_aligned_to(page_size()) {
            region.end = region.end.align_up(page_size());
        }
    }
    
    // 为主内存区域添加保留标记
    mainmem_start_rsv(&mut regions);
}
```

**内存管理功能**：
- 管理物理内存区域列表
- 处理内存区域对齐
- 区分 RAM、保留区域和设备内存
- 为内核镜像和栈空间预留内存

#### 2.2.4 **硬件特性检测**：检测虚拟化支持、中断控制器类型等

**实现位置**：`src/arch/aarch64/mod.rs`

```rust
// 根据 hv 特性选择不同的异常级别
#[cfg_attr(feature = "hv", path = "el2.rs")]
#[cfg_attr(not(feature = "hv"), path = "el1.rs")]
mod el;

// 启动时设置目标异常级别
el_value = const if cfg!(feature = "hv") { 2 } else { 1 },
```

**硬件检测机制**：
- **异常级别检测**：根据 `hv` 特性选择 EL1（普通模式）或 EL2（虚拟化模式）
- **MMU 初始化**：配置页表和内存管理单元
- **缓存管理**：处理指令和数据缓存的一致性
- **向量表设置**：配置异常和中断处理向量

---

## 3. 内存嗅探流程

### 3.1 函数调用链

```
axruntime::rust_main()
    ↓
axhal::mem::init()
    ↓
axplat::mem::mmio_ranges()          // 调用平台实现
    ↓
axplat-aarch64-dyn 提供的符号
    ↓
somehal 的底层硬件访问
```

### 3.2 内存区域识别过程

#### 3.2.1 MMIO 区域识别

**什么是 MMIO？**

MMIO（Memory-Mapped I/O，内存映射I/O）是一种硬件访问机制，通过将设备寄存器映射到内存地址空间，使 CPU 可以像访问内存一样访问硬件设备。

**MMIO 的原理：**

```
物理内存布局：
0x0000_0000 ┌─────────────────┐
            │     RAM         │  ← 普通内存，可读可写可执行
0x4000_0000 ├─────────────────┤
            │   MMIO Space    │  ← 设备寄存器，特殊访问语义
0x8000_0000 ├─────────────────┤
            │     RAM         │  ← 普通内存
0x_FFFF_FFFF└─────────────────┘
```

**MMIO 的特点和作用：**

1. **设备寄存器访问**：通过读写特定地址来控制硬件设备
2. **无缓存访问**：MMIO 区域通常配置为强一致性，禁用缓存
3. **特殊访问语义**：某些设备可能需要特定的访问大小或时序
4. **中断映射**：中断控制器的寄存器通常位于 MMIO 区域

**常见的 MMIO 设备区域：**

```rust
// axplat-aarch64-dyn 提供的 MMIO 范围示例
const MMIO_RANGES: &[RawRange] = &[
    (0x0900_0000, 0x1000_0000), // GIC 分布器 - 中断控制器
    (0x0a00_0000, 0x0010_0000), // GIC CPU 接口 - CPU本地中断接口
    (0x0c00_0000, 0x0200_0000), // UART 和串口设备 - 控制台输出
    (0x4000_0000, 0x4000_0000), // PCI Express ECAM - PCIe配置空间
    // ...
];
```

**MMIO 在 Hypervisor 中的重要性：**

- **设备虚拟化**：Hypervisor 需要拦截和管理虚拟机的 MMIO 访问
- **中断管理**：虚拟化中断控制器的配置和管理
- **设备分配**：将物理设备分配给特定虚拟机

---

#### 3.2.2 RAM 区域识别

**什么是 RAM 区域？**

RAM（Random Access Memory）区域是真正的物理内存，可用于数据存储、代码执行和内存分配。这些区域是系统的主要工作内存。

**RAM 的特点和属性：**

1. **可读写**：支持数据的读、写、修改操作
2. **可执行**：可以存储和执行机器代码
3. **缓存友好**：通常启用缓存以提高访问速度
4. **可分配**：可以用于动态内存分配

**RAM 区域识别过程：**

```rust
// 示例：从设备树解析出的内存区域
const RAM_RANGES: &[RawRange] = &[
    (0x4000_0000, 0x8000_0000), // 2GB RAM @ 1GB 物理地址
    (0x8_0000_0000, 0x8_0000_0000), // 2GB RAM @ 2GB 物理地址  
];
```

**RAM 在 Hypervisor 中的作用：**

- **虚拟机内存**：为每个虚拟机分配独立的内存空间
- **内核数据结构**：存储 Hypervisor 自身的数据结构
- **设备缓冲区**：用于网络、存储等设备的数据缓冲
- **页表存储**：虚拟化页表和内存映射表

---

#### 3.2.3 保留区域识别

**什么是保留区域？**

保留区域是指物理地址空间中不能用于通用内存分配的特殊区域，这些区域可能被固件、引导加载器、或特殊硬件结构占用。

**保留区域的类型和用途：**

1. **引导代码区域**：存储系统启动所需的固件代码
2. **设备树区域**：存储硬件配置信息的数据结构
3. **内核镜像区域**：存储操作系统内核的二进制代码
4. **硬件保留区域**：某些硬件设备需要的特殊内存区域

```rust
const RESERVED_RANGES: &[RawRange] = &[
    (0x0000_0000, 0x0010_0000), // 引导代码和向量表 - 系统启动代码
    (0x4000_0000, 0x0040_0000), // 设备树 - 硬件配置数据结构
    (0x8000_0000, 0x0080_0000), // 内核镜像 - 操作系统代码
];
```

**保留区域的重要性：**

- **系统稳定性**：防止意外修改关键系统代码和数据
- **启动保护**：确保引导代码不被覆盖
- **硬件兼容**：满足特定硬件的内存布局要求
- **安全隔离**：在虚拟化环境中隔离敏感系统区域

---

#### 3.2.4 内存区域的分类和标志

**内存区域标志系统：**

```rust
// 内存区域标志位定义
pub struct MemRegionFlags {
    const RESERVED: u8 = 0x01;    // 保留区域，不可分配
    const READ: u8 = 0x02;         // 可读
    const WRITE: u8 = 0x04;        // 可写  
    const EXECUTE: u8 = 0x08;      // 可执行
    const FREE: u8 = 0x10;         // 可用内存，可分配
    const MMIO: u8 = 0x20;         // MMIO设备区域
}
```

### 3.3 内存区域构建流程

实际的内存区域构建是 `axhal::mem::init()` 函数的核心逻辑，主要步骤如下：

#### 步骤 1：添加各类内存区域

```rust
// 1-5. 添加内核镜像各段（.text, .rodata, .data, boot stack, .bss）

// 6. 添加平台特定的内存区域
// Push MMIO & reserved regions
for &(start, size) in mmio_ranges() {
    push(PhysMemRegion::new_mmio(start, size, "mmio"));
}
for &(start, size) in reserved_phys_ram_ranges() {
    push(PhysMemRegion::new_reserved(start, size, "reserved"));
}
```

#### 步骤 2：计算可用内存区域

```rust
// Combine kernel image range and reserved ranges
let kernel_start = virt_to_phys(va!(_skernel as usize)).as_usize();
let kernel_size = _ekernel as usize - _skernel as usize;
let mut reserved_ranges = reserved_phys_ram_ranges()
    .iter()
    .cloned()
    .chain(core::iter::once((kernel_start, kernel_size))) // kernel image range is also reserved
    .collect::<Vec<_, MAX_REGIONS>>();

// Remove all reserved ranges from RAM ranges, and push the remaining as free memory
reserved_ranges.sort_unstable_by_key(|&(start, _size)| start);
ranges_difference(phys_ram_ranges(), &reserved_ranges, |(start, size)| {
    push(PhysMemRegion::new_ram(start, size, "free memory"));
})
.inspect_err(|(a, b)| error!("Reserved memory region {:#x?} overlaps with {:#x?}", a, b))
.unwrap();
```

#### 步骤 3：验证和完成

```rust
// Check overlapping
all_regions.sort_unstable_by_key(|r| r.paddr);
check_sorted_ranges_overlap(all_regions.iter().map(|r| (r.paddr.into(), r.size)))
    .inspect_err(|(a, b)| error!("Physical memory region {:#x?} overlaps with {:#x?}", a, b))
    .unwrap();

// 最终初始化
ALL_MEM_REGIONS.init_once(all_regions);
```

**核心函数说明：**
- `ranges_difference()`: 从 RAM 区域中移除保留区域，得到可用内存
- `check_sorted_ranges_overlap()`: 检查内存区域是否有重叠冲突
- `ALL_MEM_REGIONS.init_once()`: 将构建好的内存区域列表全局化

这种分步骤的构建方式确保了内存区域的正确性、完整性和无重叠性。

---

## 4. 进入 Axvisor 主程序

### 4.1 内存管理器初始化

内存嗅探完成后，初始化内存管理器：

```rust
// 在 rust_main 中继续
axhal::mem::init();

// 初始化内存分配器
#[cfg(feature = "alloc")]
init_allocator();
```

### 4.2 内存分配器初始化

```rust
#[cfg(feature = "alloc")]
fn init_allocator() {
    info!("Initialize global memory allocator...");
    
    // 查找最大的可用内存区域作为主堆
    let mut max_region_size = 0;
    let mut max_region_paddr = 0.into();
    
    for r in axhal::mem::memory_regions() {
        if r.flags.contains(MemRegionFlags::FREE) && r.size > max_region_size {
            max_region_size = r.size;
            max_region_paddr = r.paddr;
        }
    }
    
    // 初始化全局分配器
    for r in axhal::mem::memory_regions() {
        if r.flags.contains(MemRegionFlags::FREE) && r.paddr == max_region_paddr {
            axalloc::global_init(
                axhal::mem::phys_to_virt(r.paddr).as_usize(),
                r.size
            );
            break;
        }
    }
    
    // 添加其他可用内存区域
    for r in axhal::mem::memory_regions() {
        if r.flags.contains(MemRegionFlags::FREE) && r.paddr != max_region_paddr {
            axalloc::global_add_memory(
                axhal::mem::phys_to_virt(r.paddr).as_usize(),
                r.size
            ).expect("add heap memory region failed");
        }
    }
}
```
