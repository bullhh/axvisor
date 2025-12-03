# Axvisor 内存嗅探流程详解

## 概述

本文档详细介绍了 Axvisor Hypervisor 在启用 `dyn-plat` 特性后的内存嗅探流程，从平台选择机制到具体的内存区域识别和初始化过程。本流程是 Axvisor 启动的核心环节，为后续的虚拟机管理和内存分配提供基础。

---

## 1. 平台选择机制：`dyn-plat` 特性的作用

### 1.1 特性定义与传递

在 `kernel/Cargo.toml` 中定义了 `dyn-plat` 特性：

```toml
[features]
dyn-plat = ["axstd/myplat", "axstd/driver-dyn", "axruntime/driver-dyn"]
```

**特性传递链：**
```
dyn-plat
    ├── axstd/myplat          # 启用自定义平台支持
    ├── axstd/driver-dyn      # 启用动态驱动支持  
    └── axruntime/driver-dyn # 启用运行时动态驱动
```

### 1.2 `axhal` 中的条件编译选择

#### 代码位置和连接机制

`axhal` 模块的源码位于 ArceOS 仓库的 `modules/axhal/` 目录，在 Axvisor 项目中通过以下方式引入：

```toml
# 在 /home/szy/work/hypervisor/axvisor/Cargo.toml 中
axhal = {git = "https://github.com/arceos-hypervisor/arceos.git", tag = "hv-0.4.1"}
```


#### 条件编译逻辑

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

---

## 2. 动态平台依赖注入：`axplat-aarch64-dyn` 的集成

### 2.1 条件依赖配置

在 `modules/axruntime/Cargo.toml` 中，通过条件依赖引入动态平台：

```toml
[target.'cfg(target_arch = "aarch64")'.dependencies]
axplat-aarch64-dyn = {git = "https://github.com/arceos-hypervisor/axplat-aarch64-dyn", tag = "v0.3.3", features = ["irq", "smp", "hv"]}
somehal = "0.4"
```

### 2.2 外部 crate 链接

在 `modules/axruntime/src/lib.rs` 中显式链接平台 crate：

```rust
#[cfg(target_arch = "aarch64")]
extern crate axplat_aarch64_dyn;
```

### 2.3 平台特性配置

`axplat-aarch64-dyn` 启用的特性：
- `irq`: 中断控制器支持（GICv2/GICv3）
- `smp`: 多核处理器支持
- `hv`: EL2 虚拟化支持（Hypervisor 环境）

这些特性确保平台提供 Hypervisor 所需的硬件功能。

---

## 3. 底层硬件抽象：`somehal` 的识别流程

### 3.1 `somehal` 的作用

`somehal` 是一个轻量级的硬件抽象层，为 `axplat-aarch64-dyn` 提供底层硬件访问能力。

### 3.2 CPU 数量识别

在 `axruntime/src/lib.rs` 中的 `cpu_count()` 函数展示了 `somehal` 的使用：

```rust
pub fn cpu_count() -> usize {
    let mut cpu_count;
    
    cfg_if::cfg_if! {
        if #[cfg(all(target_arch = "x86_64", target_os = "none"))] {
            cpu_count = axplat_x86_qemu_q35::cpu_count()
        } else if #[cfg(target_arch = "aarch64")] {
            // 使用 somehal 获取 CPU 列表
            cpu_count = somehal::mem::cpu_id_list().count()
        } else {
            cpu_count = 1;
        }
    }
    
    // 应用环境变量限制（如果设置了 AXVISOR_SMP）
    if let Some(smp) = smp() {
        cpu_count = smp.min(cpu_count);
    }
    
    cpu_count
}
```

### 3.3 `somehal` 提供的核心功能

#### 1. **CPU 信息获取**：`cpu_id_list()` 返回可用的 CPU ID 列表

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

#### 2. **设备树解析**：解析 bootloader 传递的设备树信息

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

#### 3. **内存区域识别**：识别 RAM、MMIO、保留区域等

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

#### 4. **硬件特性检测**：检测虚拟化支持、中断控制器类型等

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

#### `somehal` 的架构特点

```rust
// 顶层模块结构
pub mod common {
    pub mod fdt;     // 设备树处理
    pub mod mem {     // 内存管理
        pub mod mmu;  // MMU 配置
        pub mod stack; // 栈管理
    }
}

pub mod arch {
    #[cfg(target_arch = "aarch64")]
    pub mod aarch64 {
        pub mod el1;   // EL1 异常级别
        pub mod el2;   // EL2 虚拟化异常级别
        pub mod trap;  // 中断和异常处理
    }
}
```

`somehal` 通过这种分层设计，为上层提供了：
- **架构无关的通用接口**（common 模块）
- **架构特定的硬件抽象**（arch 模块）  
- **灵活的特性配置**（如虚拟化支持）
- **完整的启动和运行时支持**

---

## 4. 系统启动流程：从入口到 `main` 函数

### 4.1 启动入口点

系统启动的入口点定义在 `axruntime/src/lib.rs` 中：

```rust
#[cfg_attr(not(test), axplat::main)]
pub fn rust_main(cpu_id: usize, arg: usize) -> ! {
```

这个函数通过 `#[axplat::main]` 属性标记，会被平台特定的启动代码调用。

### 4.2 早期初始化阶段

```rust
pub fn rust_main(cpu_id: usize, arg: usize) -> ! {
    // 1. 清零 BSS 段
    unsafe { axhal::mem::clear_bss() };
    
    // 2. 初始化 CPU 本地数据
    axhal::init_percpu(cpu_id);
    
    // 3. 早期平台初始化
    axhal::init_early(cpu_id, arg);
    
    // 4. 打印启动信息
    ax_println!("{}", LOGO);
    ax_println!("smp = {}", cpu_count());
    
    // 5. 初始化日志系统
    axlog::init();
    log::set_max_level(log::LevelFilter::Trace);
    
    // 6. 关键：内存初始化
    axhal::mem::init();
```

### 4.3 内存嗅探核心流程

当调用 `axhal::mem::init()` 时，触发完整的内存嗅探过程：

```rust
// 在 axhal/src/mem.rs 中
pub fn init() {
    let mut all_regions = Vec::new();
    let mut push = |r: PhysMemRegion| {
        if r.size > 0 {
            all_regions.push(r).expect("too many memory regions");
        }
    };

    // Push regions in kernel image
    push(PhysMemRegion {
        paddr: virt_to_phys((_stext as usize).into()),
        size: _etext as usize - _stext as usize,
        flags: MemRegionFlags::RESERVED | MemRegionFlags::READ | MemRegionFlags::EXECUTE,
        name: ".text",
    });
    push(PhysMemRegion {
        paddr: virt_to_phys((_srodata as usize).into()),
        size: _erodata as usize - _srodata as usize,
        flags: MemRegionFlags::RESERVED | MemRegionFlags::READ,
        name: ".rodata",
    });
    push(PhysMemRegion {
        paddr: virt_to_phys((_sdata as usize).into()),
        size: _edata as usize - _sdata as usize,
        flags: MemRegionFlags::RESERVED | MemRegionFlags::READ | MemRegionFlags::WRITE,
        name: ".data .tdata .tbss .percpu",
    });
    push(PhysMemRegion {
        paddr: virt_to_phys((boot_stack as usize).into()),
        size: boot_stack_top as usize - boot_stack as usize,
        flags: MemRegionFlags::RESERVED | MemRegionFlags::READ | MemRegionFlags::WRITE,
        name: "boot stack",
    });
    push(PhysMemRegion {
        paddr: virt_to_phys((_sbss as usize).into()),
        size: _ebss as usize - _sbss as usize,
        flags: MemRegionFlags::RESERVED | MemRegionFlags::READ | MemRegionFlags::WRITE,
        name: ".bss",
    });

    // Push MMIO & reserved regions
    for &(start, size) in mmio_ranges() {
        push(PhysMemRegion::new_mmio(start, size, "mmio"));
    }
    for &(start, size) in reserved_phys_ram_ranges() {
        push(PhysMemRegion::new_reserved(start, size, "reserved"));
    }

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

    // Check overlapping
    all_regions.sort_unstable_by_key(|r| r.paddr);
    check_sorted_ranges_overlap(all_regions.iter().map(|r| (r.paddr.into(), r.size)))
        .inspect_err(|(a, b)| error!("Physical memory region {:#x?} overlaps with {:#x?}", a, b))
        .unwrap();

    ALL_MEM_REGIONS.init_once(all_regions);
}
```

---

## 5. 内存嗅探详细流程

### 5.1 函数调用链

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

### 5.2 内存区域识别过程

#### 5.2.1 MMIO 区域识别

**什么是 MMIO？**

MMIO（Memory-Mapped I/O，内存映射I/O）是一种硬件访问机制，通过将设备寄存器映射到内存地址空间，使 CPU 可以像访问内存一样访问硬件设备。

**MMIO 的原理：**

```
物理内存布局：
0x0000_0000 ┌─────────────────┐
           │     RAM        │  ← 普通内存，可读可写可执行
0x4000_0000 ├─────────────────┤
           │   MMIO Space   │  ← 设备寄存器，特殊访问语义
0x8000_0000 ├─────────────────┤
           │     RAM        │  ← 普通内存
0x_FFFF_FFFF └─────────────────┘
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

#### 5.2.2 RAM 区域识别

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

#### 5.2.3 保留区域识别

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

#### 5.2.4 内存区域的分类和标志

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

**内存区域类型总结：**

| 类型 | 地址范围 | 标志 | 用途 | 可否分配 |
|------|----------|------|------|----------|
| 内核代码 | 物理RAM | `RESERVED|READ|EXECUTE` | 存储内核指令 | ❌ |
| 内核数据 | 物理RAM | `RESERVED|READ|WRITE` | 存储内核数据 | ❌ |
| MMIO设备 | MMIO空间 | `MMIO` | 硬件寄存器访问 | ❌ |
| 保留区域 | 任意 | `RESERVED` | 固件/引导代码 | ❌ |
| 可用内存 | 物理RAM | `FREE` | 动态内存分配 | ✅ |

这种分类确保了：
- **安全性**：防止意外访问或修改关键系统区域
- **正确性**：MMIO 区域使用正确的访问语义
- **效率性**：可用内存可以高效地用于动态分配
- **虚拟化支持**：为虚拟机提供安全隔离的内存环境

### 5.3 内存区域构建流程

实际的内存区域构建是 `axhal::mem::init()` 函数的核心逻辑，主要步骤如下：

#### 步骤 1-6：添加各类内存区域

```rust
// 1-5. 添加内核镜像各段（.text, .rodata, .data, boot stack, .bss）
// 这些代码已在前面的 5.2 节中详细展示

// 6. 添加平台特定的内存区域
// Push MMIO & reserved regions
for &(start, size) in mmio_ranges() {
    push(PhysMemRegion::new_mmio(start, size, "mmio"));
}
for &(start, size) in reserved_phys_ram_ranges() {
    push(PhysMemRegion::new_reserved(start, size, "reserved"));
}
```

#### 步骤 7：计算可用内存区域

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

#### 步骤 8：验证和完成

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

## 6. 进入 Axvisor 主程序

### 6.1 内存管理器初始化

内存嗅探完成后，初始化内存管理器：

```rust
// 在 rust_main 中继续
axhal::mem::init();

// 打印发现的内存区域
info!("Found physical memory regions:");
for r in axhal::mem::memory_regions() {
    info!("  [{:x?}, {:x?}) {} ({:?})",
          r.paddr,
          r.paddr + r.size,
          r.name,
          r.flags);
}

// 初始化内存分配器
#[cfg(feature = "alloc")]
init_allocator();
```

### 6.2 内存分配器初始化

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

### 6.3 进入用户主程序

```rust
// 继续其他初始化...
axhal::init_later(cpu_id, arg);  // 平台后期初始化
#[cfg(feature = "multitask")]
axtask::init_scheduler();       // 任务调度器初始化

// 调用用户的主程序
unsafe { main() };
```

---

## 7. 关键技术点总结

### 7.1 符号解析机制

通过 Rust 的链接器符号解析，实现了平台无关的接口调用：

1. **接口定义**：`axplat` 定义统一的硬件抽象接口
2. **符号导出**：`axplat-aarch64-dyn` 通过宏导出特定名称的符号
3. **运行时绑定**：链接器将接口调用绑定到具体实现

### 7.2 条件编译和特性系统

- **`dyn-plat` 特性**：启用自定义平台支持
- **`myplat` 特性**：跳过默认平台选择
- **架构条件依赖**：根据目标架构选择合适的平台实现

### 7.3 内存区域管理

内存区域分为以下几类：

| 类型 | 标志 | 用途 |
|------|------|------|
| 内核镜像 | `RESERVED` | 系统代码和数据，不可分配 |
| MMIO | `MMIO` | 设备寄存器映射，用于硬件访问 |
| 保留 | `RESERVED` | 固件、引导加载器占用 |
| 可用 | `FREE` | 可用于内存分配 |

### 7.4 虚拟化支持

通过 `hv` 特性启用 EL2 虚拟化支持：

- **Stage-2 页表**：支持客户机物理地址到主机物理地址的转换
- **虚拟化中断**：支持虚拟中断注入和处理
- **内存虚拟化**：支持虚拟机的内存隔离和保护

---

## 8. 调试和故障排查

### 8.1 常见问题

1. **内存区域重叠**：检查设备树或平台配置中的内存定义
2. **平台符号未找到**：确认 `extern crate` 声明正确
3. **特性未生效**：检查 Cargo 特性传递链

### 8.2 调试输出

启用详细日志来跟踪内存嗅探过程：

```rust
// 在 rust_main 中
log::set_max_level(log::LevelFilter::Debug);

// 内存区域详细信息
for r in axhal::mem::memory_regions() {
    debug!("Memory region: {:x?}", r);
}
```

### 8.3 验证内存布局

```rust
// 验证地址转换
fn test_address_translation() {
    let test_va = axhal::mem::phys_to_virt(pa!(0x4000_0000));
    let test_pa = axhal::mem::virt_to_phys(test_va);
    assert_eq!(test_pa, pa!(0x4000_0000));
}
```

---

## 9. 总结

Axvisor 的内存嗅探流程通过精心设计的分层架构实现了：

1. **平台无关性**：通过 `axplat` 接口抽象，上层代码无需关心具体硬件
2. **灵活的平台支持**：通过特性系统和条件依赖，支持多种硬件平台
3. **虚拟化优化**：专门为 Hypervisor 场景优化的内存管理
4. **类型安全**：利用 Rust 的类型系统确保内存操作的安全性

这个流程为后续的虚拟机管理、内存分配和设备管理奠定了坚实的基础，是理解 Axvisor 内部工作机制的关键环节。

---

## 附录：相关文件和配置

### 关键配置文件
- `kernel/Cargo.toml` - 主程序特性定义
- `modules/axruntime/Cargo.toml` - 运行时依赖配置
- `configs/board/` - 各平台配置文件

### 核心源码文件
- `modules/axruntime/src/lib.rs` - 系统启动入口
- `axhal/src/mem.rs` - 内存管理核心逻辑
- `axhal/src/lib.rs` - 硬件抽象层入口

### 外部依赖
- `axplat-aarch64-dyn` - AArch64 动态平台实现
- `somehal` - 底层硬件抽象
- `axplat` - 硬件抽象接口定义