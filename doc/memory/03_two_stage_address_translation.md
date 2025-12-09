# 两阶段地址翻译机制详解

## 1 概述

在虚拟化环境中，地址翻译需要两个阶段的转换过程。Axvisor 作为一个 Type-1 型 Hypervisor，完整实现了 ARM64 的两阶段地址翻译机制，实现了对 Guest 完全透明的内存虚拟化。

### 1.1 四种地址类型

在虚拟化环境中，内存地址有四种不同的"身份"：

```
┌─────────────────┐    ┌─────────────────┐    ┌─────────────────┐    ┌─────────────────┐
│   虚拟机程序     │   │   虚拟机OS       │    │   axvisor       │     │   物理内存      │
│                 │    │                 │    │                 │    │                 │
│ GVA: 0x80000000 │───▶│ GPA: 0x40000000 │───▶│ HPA: 0x20000000 │◀───│ 物理地址      │
│ (程序虚拟地址)   │    │ (虚拟机物理地址) │    │ (真实物理地址)   │     │ 0x20000000      │
└─────────────────┘    └─────────────────┘    └─────────────────┘    └─────────────────┘
        │                        │                       │
        └── Stage-1翻译 ─────────┘                        │
                                 └── Stage-2翻译 ────────┘
```

- **GVA** (Guest Virtual Address)：虚拟机中程序使用的虚拟地址
- **GPA** (Guest Physical Address)：虚拟机认为是"物理"的地址
- **HVA** (Host Virtual Address)：axvisor使用的虚拟地址  
- **HPA** (Host Physical Address)：真实的物理内存地址

### 1.2 为什么需要两阶段翻译

**问题**：如果只用一次翻译，虚拟机直接访问物理内存，就无法实现内存隔离。

**解决方案**：使用两阶段翻译，让axvisor居中管理：

```
阶段1：虚拟机内部翻译
GVA ──► GPA (由Guest OS的页表管理)

阶段2：axvisor翻译  
GPA ──► HPA (由axvisor的页表管理)

最终效果：每个虚拟机都有独立的"物理地址空间"
```

## 2 两阶段翻译原理详解

### 2.1 第一阶段翻译：GVA → GPA 的机制

第一阶段的地址翻译由 Guest OS 自己完成，整个过程对 Guest OS 完全透明。Guest OS 配置自己的页表，以为自己正在物理机上运行。

**Guest OS视角下的初始化**：
```rust
// Guest OS视角（虚拟机自身不知道自己被虚拟化）
// Guest OS配置自己的页表（认为是物理机的配置）
TTBR0_EL1 = guest_user_page_table;     // Guest用户空间页表基址
TTBR1_EL1 = guest_kernel_page_table;    // Guest内核空间页表基址
TCR_EL1  = guest_translation_config;    // Guest翻译控制寄存器
SCTLR_EL1.M = 1;                      // Guest启用MMU
```

**Stage-1 翻译的硬件流程**：
```
Guest 执行 ldr x0, [x1] (x1 = 0x80000000):
┌─────────────────────────────────────────────────────────────────┐
│                    ARM64 硬件自动执行                            │
├─────────────────────────────────────────────────────────────────┤
│ T1: 读取 TTBR1_EL1，获取 Guest 页表基址                           │
│ T2: 查询 L0 页表：使用地址 [47:39] 位                             │
│ T3: 查询 L1 页表：使用地址 [38:30] 位                             │
│ T4: 查询 L2 页表：使用地址 [29:21] 位                             │
│ T5: 查询 L3 页表：使用地址 [20:12] 位                             │
│ T6: 计算最终地址：页基址 + 偏移 [11:0]                            │
│ 结果：GVA 0x80000000 → GPA 0x40000000                           │
└─────────────────────────────────────────────────────────────────┘
```

### 2.2 第二阶段翻译：GPA → HPA 的硬件机制

第二阶段翻译由 ARM64 硬件自动完成，对 Guest 完全透明。Axvisor 设置 HCR_EL2.VM=1 启用 Stage-2 翻译，Guest 完全不知道这个操作的存在。

**axvisor的虚拟化寄存器配置**：
```rust
// arm_vcpu/src/vcpu.rs - axvisor在vCPU初始化时配置（Guest完全看不到这些寄存器）
fn init_vm_context(&mut self, config: Aarch64VCpuSetupConfig) {
    // ... 定时器配置代码 ...
    
    // Stage-2页表配置（关键！）
    #[cfg(feature = "4-level-ept")]
    {
        self.guest_system_regs.vtcr_el2 = (VTCR_EL2::PS::PA_48B_256TB
            + VTCR_EL2::TG0::Granule4KB
            + VTCR_EL2::SH0::Inner
            + VTCR_EL2::ORGN0::NormalWBRAWA
            + VTCR_EL2::IRGN0::NormalWBRAWA
            + VTCR_EL2::SL0.val(0b10) // 0b10 means start at level 0
            + VTCR_EL2::T0SZ.val(64 - 48))
        .into();
    }

    // 配置HCR_EL2（最关键的虚拟化控制寄存器）
    let mut hcr_el2 = HCR_EL2::VM::Enable
        + HCR_EL2::RW::EL1IsAarch64
        + HCR_EL2::FMO::EnableVirtualFIQ
        + HCR_EL2::TSC::EnableTrapEl1SmcToEl2;

    if !config.passthrough_interrupt {
        hcr_el2 += HCR_EL2::IMO::EnableVirtualIRQ;
    }

    // 设置HCR_EL2（Guest完全看不到这个寄存器！）
    self.guest_system_regs.hcr_el2 = hcr_el2.into();
    
    // ... VMPIDR_EL2设置 ...
}
```

**Stage-2页表根地址设置**：
```rust
// arm_vcpu/src/vcpu.rs - vCPU设置时将Stage-2页表根地址加载到VTTBR_EL2
fn set_ept_root(&mut self, ept_root: HostPhysAddr) -> AxResult {
    debug!("set vcpu ept root:{ept_root:#x}");
    // 关键：将Stage-2页表根地址存储到VTTBR_EL2
    self.guest_system_regs.vttbr_el2 = ept_root.as_usize() as u64;
    Ok(())
}
```

**完整的两阶段翻译硬件流程**：
```
Guest 执行 ldr x0, [x1] (x1 = 0x80000000):
┌─────────────────────────────────────────────────────────────────┐
│                    ARM64 硬件自动执行                            │
├─────────────────────────────────────────────────────────────────┤
│                                                                 │
│  Stage-1 翻译 (Guest OS 配置，Guest 知道这部分):                  │
│  ┌─────────────────────────────────────────────────────────┐    │
│  │ 读取 TTBR1_EL1: 0x40100000                           │   │
│  │ L0 查询: index = 0, entry = valid, next = 0x40101000 │   │
│  │ L1 查询: index = 0, entry = valid, next = 0x40102000 │   │
│  │ L2 查询: index = 1, entry = valid, next = 0x40103000 │   │
│  │ L3 查询: index = 0, entry = valid, page = 0x40104000 │   │
│  │ 计算偏移: offset = 0x00000, final = 0x40104000       │   │
│  └─────────────────────────────────────────────────────────┘   │
│  结果: GVA 0x80000000 → GPA 0x40000000                      │
│                                                             │
│  硬件检查 HCR_EL2.VM=1 (Guest 不知道这个检查)                 │
│                                                             │
│  Stage-2 翻译 (Axvisor 配置，Guest 不知道这部分):             │
│  ┌─────────────────────────────────────────────────────────┐   │
│  │ 读取 VTTBR_EL2: 0x50100000                           │   │
│  │ L0 查询: index = 1, entry = valid, next = 0x50101000 │   │
│  │ L1 查询: index = 0, entry = valid, next = 0x50102000 │   │
│  │ L2 查询: index = 0, entry = valid, next = 0x50103000 │   │
│  │ L3 查询: index = 1, entry = valid, page = 0x50104000 │   │
│  │ 计算偏移: offset = 0x00000, final = 0x50104000       │   │
│  └─────────────────────────────────────────────────────────┘   │
│  结果: GPA 0x40000000 → HPA 0x20000000                      │
│                                                             │
│  最终访问: 物理地址 0x20000000，返回数据给 Guest              │
│                                                             │
└─────────────────────────────────────────────────────────────────┘
```

### 2.3 Guest 的虚拟化无感知性

Guest 完全不知道自己是虚拟机，无法感知任何 EL2 寄存器的存在：

- **Guest 认为的流程**：设置页表 → 启用 MMU → 访问物理内存
- **实际发生的流程**：设置页表 → 启用 MMU → Axvisor 启用 Stage-2 → 完整的两阶段翻译

**Guest无法感知的虚拟化控制**：

```rust
// 这些寄存器和操作对Guest完全不可见：

1. HCR_EL2.VM = 1    // 在init_hv()中设置，启用Stage-2翻译
2. VTTBR_EL2         // 通过set_ept_root()设置，指向Stage-2页表  
3. VTCR_EL2          // 在init_vm_context()中设置，配置Stage-2翻译参数
4. Guest看到的寄存器  // Guest只能访问TTBR0_EL1/TTBR1_EL1，无法感知HCR_EL2的存在
```

**vCPU实际运行时的寄存器恢复**：

```rust
// arm_vcpu/src/vcpu.rs - Guest运行前的寄存器设置
unsafe fn restore_vm_system_regs(&mut self) {
    unsafe {
        // 清零CPTR_EL2（不拦截任何EL1系统寄存器访问）
        core::arch::asm!(
            "
                mov x3, xzr           // Trap nothing from EL1 to El2.
                msr cptr_el2, x3"
        );
        
        // 恢复所有Guest系统寄存器（包括HCR_EL2, VTTBR_EL2等）
        self.guest_system_regs.restore();
        
        // TLB刷新
        core::arch::asm!(
            "
                ic  iallu
                tlbi	alle2
                tlbi	alle1         // Flush tlb
                dsb	nsh
                isb"
        );
    }
}
```

这种透明性是虚拟化技术的核心优势，Guest OS 无需任何修改即可在虚拟化环境中运行。

## 3 ARM64 虚拟化硬件支持

### 3.1 关键寄存器

#### 3.1.1 HCR_EL2 (Hypervisor Configuration Register)

控制虚拟化行为的核心寄存器，Guest 完全无法访问。

**关键位域**：
- **VM (0)**: Stage-2 地址翻译启用位
- **RW (31)**: Guest 异常级别架构 (AArch64/AArch32)
- **TSC (19)**: SMC 指令拦截控制
- **IMO (4)**: 物理中断虚拟化控制

#### 3.1.2 VTTBR_EL2 (Virtualization Translation Table Base Register)

每个虚拟机都有独立的 Stage-2 页表基址。

**格式**：
```
┌─────────────────────────────────────────────────────────┐
│[63:48]│[47:0]                                           │
│ VMID   │ Stage-2 页表基址                                │
│ (8位)  │ (48位物理地址，12位对齐)                         │
└─────────────────────────────────────────────────────────┘
```

- **VMID**: 虚拟机标识符，支持 256 个虚拟机
- **页表基址**: Stage-2 L0 页表的物理地址，必须 4KB 对齐

#### 3.1.3 VTCR_EL2 (Virtualization Translation Control Register)

配置 Stage-2 翻译的所有参数。

**关键配置**：
- **PS**: 物理地址大小 (Axvisor 使用 48位，支持256TB)
- **TG0**: 页粒度 (4KB/64KB/16KB，Axvisor 使用 4KB)
- **SL0**: 起始级别 (Axvisor 使用 L0 开始，4级页表)
- **T0SZ**: IPA 大小 (48位 IPA 空间)

## 4 Stage-2 页表管理实现

### 4.1 Stage-2 页表项格式

Stage-2 页表项与普通页表项有重要区别：

**关键属性位**：
- **VALID**: 表项有效标志
- **NON_BLOCK**: 非4KB块标志 (页表指针)
- **AF**: 访问标志 (Stage-2 必须设置)
- **S2AP**: Stage-2 访问权限位
- **SH**: 共享属性
- **XN**: 执行禁止位
- **PHYS_ADDR_MASK**: 物理地址掩码

**Stage-2 页表项特点**：
- 支持 4KB、2MB、1GB 页面大小
- 必须设置 AF 位
- 提供独立的访问权限控制
- 可以覆盖 Guest OS 的权限设置

**ARM64 Stage-2页表项的代码实现**：
```rust
// axaddrspace/src/npt/arch/aarch64.rs -Stage-2页表项创建
impl A64PTEHV {
    fn new_page(paddr: HostPhysAddr, flags: MappingFlags, is_huge: bool) -> Self {
        let mut attr = DescriptorAttr::from(flags) | DescriptorAttr::AF;
        if !is_huge {
            attr |= DescriptorAttr::NON_BLOCK;  // 4KB页时设置为页表指针
        }
        
        // 组合物理地址和属性位
        Self(attr.bits() | (paddr.as_usize() as u64 & Self::PHYS_ADDR_MASK))
    }
}
```

### 4.2 映射策略的实现

Axvisor 支持三种不同的 Stage-2 映射策略，这些策略通过 axaddrspace 库的 Backend 机制实现：

#### 4.2.1 线性映射 (MapAlloc)

虚拟机指定期望的物理地址，axvisor 分配实际物理内存并建立映射。

**特点**：
- 虚拟机可以指定 GPA 地址
- axvisor 动态分配 HPA
- 固定偏移量：offset = GPA - HPA

**代码实现**：
```rust
// axvm/src/vm.rs - 内存分配和映射建立
pub fn alloc_memory_region(
    &self,
    layout: Layout,
    gpa: Option<GuestPhysAddr>,
) -> AxResult<&[u8]> {
    assert!(
        layout.size() > 0,
        "Cannot allocate zero-sized memory region"
    );

    // 1. 使用系统分配器分配零初始化内存
    let hva = unsafe { alloc::alloc::alloc_zeroed(layout) };
    let s = unsafe { core::slice::from_raw_parts_mut(hva, layout.size()) };
    let hva = HostVirtAddr::from_mut_ptr_of(hva);
    let hpa = H::virt_to_phys(hva);

    // 2. 确定最终的GPA（关键逻辑！）
    let gpa = gpa.unwrap_or_else(|| hpa.as_usize().into());
    // 解释：
    // - 如果gpa是Some(value)，使用指定的值（MapAlloc模式）
    // - 如果gpa是None，使用hpa转换的值（MapIdentical模式）
    
    // 3. 通过AddrSpace建立Stage-2映射：GPA → HPA
    let mut g = self.inner_mut.lock();
    g.address_space.map_linear(
        gpa,                              // Guest物理地址
        hpa,                               // Host物理地址  
        layout.size(),
        MappingFlags::READ | MappingFlags::WRITE | MappingFlags::EXECUTE | MappingFlags::USER,
    )?;
    
    Ok(s)
}

// axaddrspace/src/address_space/backend/linear.rs - Linear backend实现
pub(crate) fn map_linear(
    &self,
    start: GuestPhysAddr,           // GPA
    size: usize,
    flags: MappingFlags,
    pt: &mut PageTable<H>,          // NestedPageTable (Stage-2页表)
    pa_va_offset: usize,             // GPA到HPA的偏移量
) -> bool {
    // 根据偏移量计算HPA（线性映射公式）
    let pa_start = PhysAddr::from(start.as_usize() - pa_va_offset);
    debug!(
        "map_linear: [{:#x}, {:#x}) -> [{:#x}, {:#x}) {:?}",
        start, start + size, pa_start, pa_start + size, flags
    );
    
    // 调用NestedPageTable进行实际的页表映射
    pt.map_region(
        start,                              // GPA
        |va| PhysAddr::from(va.as_usize() - pa_va_offset),  // GPA→HPA转换函数
        size,
        flags,
        true,                                // 允许大页
        true,                                // 强制刷新TLB
    ).is_ok()
}
```

**示例**：
- 虚拟机期望：GPA 0x40000000
- axvisor 分配：HPA 0x20000000
- 偏移量：0x20000000
- 映射关系：GPA → HPA = GPA - 0x20000000

#### 4.2.2 恒等映射 (MapIdentical)

最简单的映射方式，GPA 和 HPA 数值相等。

**特点**：
- 偏移量为 0
- GPA = HPA
- 便于调试和管理

**示例**：
- axvisor 分配：HPA 0x30000000
- 设置 GPA：0x30000000
- 偏移量：0
- 映射关系：GPA → HPA = GPA

#### 4.2.3 预留映射 (MapReserved)

使用预分配的物理内存区域，通常用于设备映射或特殊内存区域。

**代码实现**：
```rust
// axvm/src/vm.rs - 设备直通映射
for pt_device in inner_mut.config.pass_through_devices() {
    let pt_dev_region = (
        align_down_4k(pt_device.base_gpa),
        align_up_4k(pt_device.length),
    );
    
    // 直接映射: GPA → HPA (passthrough到相同的物理地址)
    inner_mut.address_space.map_linear(
        GuestPhysAddr::from(pt_dev_region.0),
        HostPhysAddr::from(pt_dev_region.0),  // GPA = HPA
        pt_dev_region.1,
        MappingFlags::DEVICE | MappingFlags::READ | MappingFlags::WRITE | MappingFlags::USER,
    )?;
}
```

**特点**：
- 使用指定的物理内存
- 适用于设备内存映射
- 支持特殊用途内存区域

**示例**：
- 预留 HPA：0x50000000
- 指定 GPA：0x80000000
- 建立映射：GPA 0x80000000 → HPA 0x50000000

### 4.3 Stage-2页表建立的完整调用链

**五层责任链的职责分工**：

1. **AddrSpace（第1环）**：
   ```rust
   // 参数验证和地址对齐检查
   if !start_vaddr.is_aligned_4k() || !start_paddr.is_aligned_4k() || !is_aligned_4k(size) {
       return ax_err!(InvalidInput, "address not aligned");
   }
   // 计算GPA→HPA偏移量
   let offset = start_vaddr.as_usize() - start_paddr.as_usize();
   ```

2. **MemorySet（第2环）**：
   ```rust
   // 重叠检测和冲突处理
   if self.overlaps(area.va_range()) {
       if unmap_overlap {
           self.unmap(area.start(), area.size(), page_table)?;
       } else {
           return Err(MappingError::AlreadyExists);
       }
   }
   ```

3. **MemoryArea（第3环）**：
   ```rust
   // 封装映射信息，调用Backend的map方法
   pub(crate) fn map_area(&self, page_table: &mut B::PageTable) -> MappingResult {
       self.backend.map(self.start(), self.size(), self.flags, page_table)
           .then_some(()).ok_or(MappingError::BadState)
   }
   ```

4. **Backend（第4环）**：
   ```rust
   // 实现具体的映射策略
   fn map(&self, start: GuestPhysAddr, size: usize, flags: MappingFlags, page_table: &mut PageTable<H>) -> bool {
       match *self {
           Self::Linear { pa_va_offset } => {
               self.map_linear(start, size, flags, page_table, pa_va_offset)
           }
           Self::Alloc { populate, .. } => {
               self.map_alloc(start, size, flags, page_table, populate)
           }
       }
   }
   ```

5. **NestedPageTable/PageTable64（第5-6环）**：
   ```rust
   // 页表项分配和初始化
   pub fn map(&mut self, vaddr: M::VirtAddr, target: PhysAddr, page_size: PageSize, flags: MappingFlags) -> PagingResult<TlbFlush<M>> {
       let entry = self.get_entry_mut_or_create(vaddr, page_size)?;
       *entry = GenericPTE::new_page(target.align_down(page_size), flags, page_size.is_huge());
       Ok(TlbFlush::new(vaddr))
   }
   ```


### 4.4 硬件加速

充分利用 ARM64 硬件特性：
- Stage-2 翻译完全由硬件自动完成
- 无需软件干预
- 确保虚拟化环境的性能

**硬件加速的充分利用**：
```rust
// arm_vcpu/src/vcpu.rs - Guest运行的硬件优化
impl<H: AxVCpuHal> axvcpu::AxArchVCpu for Aarch64VCpu<H> {
    fn run(&mut self) -> AxResult<AxVCpuExitReason> {
        // Guest运行的完全硬件路径
        let exit_reason = unsafe {
            save_host_sp_el0();
            self.restore_vm_system_regs();  // 硬件寄存器恢复
            
            // 硬件自动执行Guest - 无软件干预的两阶段翻译
            self.run_guest()  // 这部分完全由ARM64硬件处理
        };

        // 只有在Guest退出时才需要软件处理
        let trap_kind = TrapKind::try_from(exit_reason as u8)?;
        self.vmexit_handler(trap_kind)
    }
}
```

## 5 总结

两阶段地址翻译是现代虚拟化技术的核心机制，Axvisor 充分利用了 ARM64 硬件的虚拟化扩展，实现了高效、安全的地址翻译。这种设计既保证了虚拟机的隔离性，又维持了良好的性能表现，是虚拟化技术的重要创新。