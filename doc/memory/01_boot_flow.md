# Axvisor 启动流程详解

## 概述

Axvisor 的启动是一个精心设计的多阶段过程，从引导加载程序到完整的 Hypervisor 运行时环境。

### 启动流程架构图

```
┌─────────────────────────────────────────────────────────────────┐
│                     Axvisor 启动时序                          │
├─────────────────────────────────────────────────────────────────┤
│ 1. 硬件引导 → 启动代码 → rust_main()                         │
│ 2. 早期初始化：BSS清理、CPU初始化、平台配置                   │
│ 3. 内存发现：内存嗅探、区域分类、安全验证                     │
│ 4. 分配器初始化：TLSF设置、区域选择、堆管理                   │
│ 5. 虚拟化建立：MMU配置、Stage-2页表、vCPU设置               │
│ 6. 运行就绪：调度器启动、虚拟机加载                          │
└─────────────────────────────────────────────────────────────────┘
```

## 启动入口和早期初始化

### 1. 入口点与平台抽象

#### 启动入口的原理机制

Axvisor 通过 `#[axplat::main]` 属性实现平台无关的启动入口：

```rust
// axruntime/src/lib.rs
#[axplat::main]  // ← 编译时生成特殊符号，供平台启动代码调用
pub fn rust_main(cpu_id: usize, arg: usize) -> ! {
    // 启动主逻辑
}
```

**核心原理**：
- **符号生成**：`axplat::main` 宏在编译时创建名为 `_start` 的导出符号
- **平台绑定**：不同平台的启动汇编（如 `start.S`）直接调用 `_start`
- **参数传递**：`cpu_id` 和 `arg` 由启动代码传递，标识CPU核心和启动参数

### 2. 早期平台初始化原理

#### 硬件抽象层初始化

`axhal::init_early()` 负责平台特定的早期硬件配置：

```rust
// axruntime/src/lib.rs  
pub fn rust_main(cpu_id: usize, arg: usize) -> ! {
    unsafe { axhal::mem::clear_bss() };
    axhal::init_percpu(cpu_id);
    
    // 3. 早期平台初始化
    axhal::init_early(cpu_id, arg);
    
    // 4. 启动信息输出
    ax_println!("{}", LOGO);
    ax_println!("smp = {}", cpu_count());
    
    // 5. 日志系统初始化
    axlog::init();
    log::set_max_level(log::LevelFilter::Trace);
}
```

**早期初始化的核心任务**：

1. **CPU 模式设置**：
   - 配置异常级别（EL2 for Hypervisor）
   - 设置栈指针和中断向量
   - 启用基本特权模式

2. **基础页表映射**：
   - 建立最小化的虚拟地址映射
   - 确保内核代码可执行
   - 为后续内存管理做准备

3. **串口配置**：
   - 初始化调试输出设备
   - 设置波特率和通信参数
   - 提供早期调试能力

## 内存发现和初始化

### 3. 内存嗅探的核心原理

#### 内存嗅探的动机与目的

内存嗅探是 Hypervisor 启动的关键环节，其原理是通过分析设备树和硬件配置，识别所有可用的物理内存区域，为后续的内存管理和虚拟化建立基础。

```rust
// axruntime/src/lib.rs
#[cfg(feature = "alloc")]
fn init_allocator() {
    info!("Initialize global memory allocator...");
    info!("  use {} allocator.", axalloc::global_allocator().name());
    
    // 关键：执行完整的内存嗅探
    axhal::mem::init();
    
    // 输出发现的内存区域信息
    info!("Found physical memory regions:");
    for r in axhal::mem::memory_regions() {
        info!("  [{:#x?}, {:#x?}) {} ({:?})",
              r.paddr,
              r.paddr + r.size,
              r.name,
              r.flags);
    }
}
```

#### 内存嗅探的详细算法

```rust
// arceos/axhal/src/mem.rs 中的内存嗅探实现
pub fn init() {
    let mut all_regions = Vec::new();
    let mut push = |r: PhysMemRegion| {
        if r.size > 0 {
            all_regions.push(r).expect("too many memory regions");
        }
    };

    // 第一步：识别内核镜像占用的内存区域
    // 1.1 代码段（.text）- 可执行、只读
    push(PhysMemRegion {
        paddr: virt_to_phys((_stext as usize).into()),
        size: _etext as usize - _stext as usize,
        flags: MemRegionFlags::RESERVED | MemRegionFlags::READ | MemRegionFlags::EXECUTE,
        name: ".text",
    });
    
    // 1.2 只读数据段（.rodata）- 只读
    push(PhysMemRegion {
        paddr: virt_to_phys((_srodata as usize).into()),
        size: _erodata as usize - _srodata as usize,
        flags: MemRegionFlags::RESERVED | MemRegionFlags::READ,
        name: ".rodata",
    });
    
    // 1.3 数据段（.data）- 可读写
    push(PhysMemRegion {
        paddr: virt_to_phys((_sdata as usize).into()),
        size: _edata as usize - _sdata as usize,
        flags: MemRegionFlags::RESERVED | MemRegionFlags::READ | MemRegionFlags::WRITE,
        name: ".data",
    });
    
    // 1.4 BSS段（.bss）- 可读写，零初始化
    push(PhysMemRegion {
        paddr: virt_to_phys((_sbss as usize).into()),
        size: _ebss as usize - _sbss as usize,
        flags: MemRegionFlags::RESERVED | MemRegionFlags::READ | MemRegionFlags::WRITE,
        name: ".bss",
    });

    // 第二步：识别MMIO和硬件保留区域
    for &(start, size) in mmio_ranges() {
        push(PhysMemRegion::new_mmio(start, size, "mmio"));
    }
    
    // 第三步：计算可用内存区域
    // 3.1 收集所有保留区域（内核+硬件）
    let mut reserved_ranges = reserved_phys_ram_ranges()
        .iter()
        .cloned()
        .collect::<Vec<_>>();
    
    // 3.2 从总RAM区域中减去保留区域，得到可用内存
    ranges_difference(phys_ram_ranges(), &reserved_ranges, |(start, size)| {
        push(PhysMemRegion::new_ram(start, size, "free memory"));
    }).unwrap();
    
    // 第四步：安全验证和初始化
    all_regions.sort_unstable_by_key(|r| r.paddr);
    check_sorted_ranges_overlap(all_regions.iter().map(|r| (r.paddr.into(), r.size))).unwrap();
    
    ALL_MEM_REGIONS.init_once(all_regions);
}
```

**内存嗅探的算法原理**：

1. **区域分类**：将内存分为四类
   - **保留区域**：内核镜像、引导代码、特殊硬件区域
   - **MMIO区域**：设备寄存器映射区域
   - **可用RAM**：可用于动态分配的物理内存
   - **保留RAM**：固件或硬件预留的RAM区域

2. **地址计算**：
   - `virt_to_phys()`：将内核虚拟地址转换为物理地址
   - `ranges_difference()`：计算集合差集，得到可用区域
   - 地址对齐：确保所有区域满足页对齐要求

3. **安全检查**：
   - 重叠检测：防止内存区域冲突
   - 对齐验证：确保地址符合硬件要求
   - 完整性校验：验证内存区域的有效性

### 4. 内存分配器初始化原理

#### 分配器选择的智能算法

Axvisor 使用 TLSF (Two-Level Segregated Fit) 分配器，采用 Level-1 单级架构：

```rust
// axvisor/modules/axruntime/src/lib.rs
#[cfg(feature = "alloc")]
fn init_allocator() {
    let mut max_region_size = 0;
    let mut max_region_paddr = 0.into();
    let mut use_next_free = false;

    // 智能选择算法：寻找最佳的主堆区域
    for r in memory_regions() {
        // 特殊规则：避免使用.bss段所在的内存区域
        if r.name == ".bss" {
            use_next_free = true;  // 强制使用下一个FREE区域
        } 
        // 寻找FREE（可用）内存区域
        else if r.flags.contains(MemRegionFlags::FREE) {
            if use_next_free {
                // 如果遇到.bss段，使用下一个FREE区域
                max_region_paddr = r.paddr;
                break;
            } else if r.size > max_region_size {
                // 否则选择最大的FREE区域
                max_region_size = r.size;
                max_region_paddr = r.paddr;
            }
        }
    }

    // TLSF分配器初始化
    for r in memory_regions() {
        if r.flags.contains(MemRegionFlags::FREE) && r.paddr == max_region_paddr {
            // 将物理地址转换为虚拟地址后初始化
            axalloc::global_init(
                phys_to_virt(r.paddr).as_usize(),
                r.size
            );
            break;
        }
    }

    // 添加其他可用内存区域到TLSF管理
    for r in memory_regions() {
        if r.flags.contains(MemRegionFlags::FREE) && r.paddr != max_region_paddr {
            axalloc::global_add_memory(
                phys_to_virt(r.paddr).as_usize(),
                r.size
            ).expect("add heap memory region failed");
        }
    }
}
```

**分配器选择策略的原理**：

1. **BSS段避免**：
   - .bss段在启动时被清零，可能与内存管理器产生冲突
   - 使用 `use_next_free` 标志强制跳过.bss段所在区域

2. **最大优先**：
   - 选择最大的连续内存区域作为主堆
   - 减少内存碎片，提高分配效率

3. **多区域支持**：
   - TLSF支持动态添加多个内存区域
   - 充分利用系统中的所有可用内存

## 虚拟化环境建立

### 5. 虚拟化硬件初始化

#### 后期平台初始化

内存管理就绪后，进入虚拟化环境的建立：

```rust
// axvisor/modules/axruntime/src/lib.rs
pub fn rust_main(cpu_id: usize, arg: usize) -> ! {
    // 早期初始化完成
    axhal::init_early(cpu_id, arg);
    axlog::init();
    
    // 内存管理初始化
    #[cfg(feature = "alloc")]
    init_allocator();
    
    // 关键：后期平台初始化，启用虚拟化特性
    axhal::init_later(cpu_id, arg);
    
    // 其他组件初始化...
}
```

`axhal::init_later()` 完成关键的虚拟化硬件配置：

1. **MMU 完整配置**：
   - 建立完整的页表映射
   - 启用多级页表支持
   - 配置缓存属性和内存权限

2. **虚拟化特性启用**：
   - 设置 HCR_EL2 寄存器，启用 Stage-2 翻译
   - 配置 VTCR_EL2，设置虚拟地址空间参数
   - 初始化虚拟化异常处理

3. **中断控制器**：
   - 完成 GIC（Generic Interrupt Controller）配置
   - 设置虚拟中断支持
   - 配置中断路由和优先级

### 6. 虚拟化组件初始化

#### 高级虚拟化组件

```rust
// axvisor/modules/axruntime/src/lib.rs
pub fn rust_main(cpu_id: usize, arg: usize) -> ! {
    // ... 硬件初始化完成 ...
    
    // 任务调度器初始化
    #[cfg(feature = "multitask")]
    axtask::init_scheduler();
    
    // 虚拟化内存管理初始化
    axmm::init_memory_management();
    
    // 调用用户的主程序
    unsafe { main() };
}
```

**虚拟化组件的核心功能**：

1. **axmm::init_memory_management()**：
   - 建立 Stage-2 页表框架
   - 初始化虚拟机地址空间管理
   - 配置 EPT/NPT 机制

2. **axtask::init_scheduler()**：
   - 初始化多任务调度器
   - 支持 vCPU 的调度和管理
   - 建立任务切换机制


## 启动流程的完整时序图

```
时间轴    │ 启动阶段            │ 核心操作                      │ 硬件状态
─────────┼─────────────────────┼──────────────────────────────┼─────────────────
T1-T10   │ 硬件引导             │ BIOS/UEFI → 引导加载程序      │ CPU实模式
T11-T20  │ 内核加载             │ 加载axvisor镜像到内存         │ 保护模式
T21-T30  │ rust_main入口       │ BSS清理、CPU初始化             │ 进入EL2
T31-T40  │ 早期平台初始化       │ 基础页表、串口配置             │ 基础MMU启用
T41-T60  │ 内存嗅探            │ 解析设备树、内存区域识别        │ 完整内存映射
T61-T70  │ 分配器初始化        │ TLSF设置、多区域管理           │ 内存管理就绪
T71-T80  │ 虚拟化建立          │ Stage-2页表、HCR_EL2配置      │ 虚拟化启用
T81-T90  │ 运行就绪            │ 调度器启动、虚拟机加载         │ Hypervisor运行
```

## 总结

Axvisor 的启动流程体现了现代 Hypervisor 设计的核心原则：

1. **分层初始化架构**：
   - 从硬件到软件的渐进式初始化
   - 每层依赖前层的完整性
   - 失败可快速定位和恢复

2. **内存中心设计**：
   - 内存管理是所有功能的基础
   - 智能的内存嗅探和分配策略
   - 针对虚拟化优化的分配器选择

这个启动流程为 Axvisor 的稳定运行和虚拟机的安全隔离提供了坚实的基础，是实现高效、安全 Hypervisor 的关键技术支撑。