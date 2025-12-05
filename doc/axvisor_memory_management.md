# axvisor虚拟机内存管理详解

## 虚拟化内存管理基础

### 什么是虚拟机内存管理？

想象一下，你有三台电脑需要放在一个房间里：
- **真实物理内存** = 房间的实际空间
- **虚拟机内存** = 每台电脑"以为"自己独占的房间
- **内存管理** = 房间管理员，确保每台电脑都能正常使用，互不干扰

axvisor就是这位"房间管理员"，负责：
1. **内存隔离**：每台虚拟机只能访问自己的"房间"
2. **地址翻译**：把虚拟机的"假地址"转换为真实的"物理地址"
3. **资源分配**：合理分配有限的物理内存给多个虚拟机

### 四种地址类型

在虚拟化环境中，内存地址有四种不同的"身份"：

```
┌─────────────────┐    ┌─────────────────┐    ┌─────────────────┐    ┌─────────────────┐
│   虚拟机程序     │   │   虚拟机OS       │    │   axvisor       │    │   物理内存      │
│                 │    │                 │    │                 │    │                 │
│ GVA: 0x80000000 │───▶│ GPA: 0x40000000 │───▶│ HPA: 0x20000000 │◀───│ 物理地址        │
│ (程序虚拟地址)   │    │ (虚拟机物理地址) │    │ (真实物理地址)   │    │ 0x20000000      │
└─────────────────┘    └─────────────────┘    └─────────────────┘    └─────────────────┘
        │                        │                        │
        └── Stage-1翻译 ────────┘                        │
                                 └── Stage-2翻译 ────────┘
```

- **GVA** (Guest Virtual Address)：虚拟机中程序使用的虚拟地址
- **GPA** (Guest Physical Address)：虚拟机认为是"物理"的地址
- **HVA** (Host Virtual Address)：axvisor使用的虚拟地址  
- **HPA** (Host Physical Address)：真实的物理内存地址

---

## 两阶段地址翻译机制

### 为什么需要两阶段翻译？

**问题**：如果只用一次翻译，虚拟机直接访问物理内存，就无法实现内存隔离。

**解决方案**：使用两阶段翻译，让axvisor居中管理：

```
阶段1：虚拟机内部翻译
GVA ──► GPA (由Guest OS的页表管理)

阶段2：axvisor翻译  
GPA ──► HPA (由axvisor的页表管理)

最终效果：每个虚拟机都有独立的"物理地址空间"
```

### MMU硬件的三种工作模式

ARM64处理器为虚拟化提供了三种MMU工作模式：

#### 1. Guest模式（虚拟机运行时 - EL1）
```rust
// Guest OS视角（虚拟机自身不知道自己被虚拟化）
// Guest OS配置自己的页表（认为是物理机的配置）
TTBR0_EL1 = guest_user_page_table;     // Guest用户空间页表基址
TTBR1_EL1 = guest_kernel_page_table;    // Guest内核空间页表基址
TCR_EL1  = guest_translation_config;    // Guest翻译控制寄存器
SCTLR_EL1.M = 1;                      // Guest启用MMU

// 实际情况（Hypervisor控制，Guest完全不知道）
HCR_EL2.VM = 1;                       // Hypervisor启用Stage-2翻译（Guest看不到）

// 完整翻译流程（对Guest透明）
Guest访问0x80000000:
├── Stage-1: GVA 0x80000000 ──► GPA 0x40000000 (Guest OS页表，Guest认为自己完成了翻译)
└── Stage-2: GPA 0x40000000 ──► HPA 0x20000000 (Hypervisor页表，硬件自动翻译，Guest不知道)
```

**关键理解：虚拟机不知道自己是虚拟机！**

虚拟机无法感知自己运行在EL1还是EL0，也不知道HCR_EL2的存在。虚拟机认为自己在物理机上运行，配置的页表是"最终"的地址翻译。但实际上：

#### ARM硬件虚拟化的透明性

**1. 异常级别的隔离**
```rust
// 虚拟机视角（EL1）：
Guest OS认为自己运行在最高异常级别（除了EL0）
Guest OS可以正常访问所有EL1寄存器
Guest OS看不到任何EL2寄存器的存在

// Hypervisor视角（EL2）：
axvisor运行在更高的异常级别
axvisor可以配置和控制Guest OS的运行环境
axvisor的寄存器对Guest完全不可见
```

**2. 硬件自动的两阶段翻译**
```rust
// Guest OS配置页表
mmu_init() {
    // Guest认为这个页表是GVA→HPA的最终翻译
    TTBR1_EL1.write(guest_page_table_paddr);
    TCR_EL1.write(guest_tcr_config);
    SCTLR_EL1.modify(SCTLR_EL1::M::Enable);  // Guest启用MMU
}

// 实际硬件行为（Guest不知道）：
Guest访问0x80000000时：
1. Guest MMU开始翻译（Guest知道这部分）
   GVA 0x80000000 → GPA 0x40000000

2. 硬件检查HCR_EL2.VM=1（Guest不知道这部分）
   自动启动Stage-2翻译

3. Hypervisor MMU继续翻译（Guest不知道这部分）  
   GPA 0x40000000 → HPA 0x20000000

4. 最终访问物理内存（Guest不知道这部分）
   访问真实的物理地址0x20000000
```

#### axvisor中的实际实现

axvisor使用分层架构来处理vCPU的创建和管理。实际的vCPU设置涉及多个层次的抽象和实现。

##### 架构概览

```
┌─────────────────────────────────────────────────────────────────────┐
│                    axvm                              │
│  ┌─────────────────────────────────────────────┐         │
│  │              AxVCpu<A>              │         │
│  │  ┌─────────────────────────────┐    │         │
│  │  │     AxArchVCpu trait     │    │         │
│  │  └─────────────────────────────┘    │         │
│  │           ▲                         │         │
│  │           │                         │         │
│  ▼           │                         │         │
│  Aarch64VCpu<H> (arm_vcpu crate)        │         │
└─────────────────────────────────────────────────────────────────────┘
```

##### 1. vCPU创建流程

**实际的vCPU创建代码（axvm/src/vm.rs）：**
```rust
// 在VM::init()函数中的vCPU创建
pub fn init(&self) -> AxResult {
    let mut inner_mut = self.inner_mut.lock();
    
    // 1. 创建vCPU列表（根据物理CPU亲和性配置）
    let vcpu_id_pcpu_sets = inner_mut.config.phys_cpu_ls.get_vcpu_affinities_pcpu_ids();
    let mut vcpu_list = Vec::with_capacity(vcpu_id_pcpu_sets.len());
    
    for (vcpu_id, phys_cpu_set, _pcpu_id) in vcpu_id_pcpu_sets {
        // 2. 创建Aarch64VCpuCreateConfig
        let create_config = axvcpu::Aarch64VCpuCreateConfig {
            mpidr_el1: _pcpu_id as u64,  // 设置MPIDR_EL1值
            dtb_addr: inner_mut.config.device_tree_blob(), // 设备树地址
        };
        
        // 3. 创建vCPU实例
        let vcpu = VCpu::new(
            self.inner_const.id,    // VM ID
            vcpu_id,               // vCPU ID
            0,                     // 目前未使用的参数
            phys_cpu_set,          // 物理CPU集合
            create_config,          // 架构特定配置
        )?;
        vcpu_list.push(vcpu);
    }
    
    // 4. vCPU设置在init函数的最后部分完成
    for vcpu in self.vcpu_list() {
        // 配置setup_config
        let setup_config = axvcpu::Aarch64VCpuSetupConfig {
            passthrough_interrupt: passthrough,
            passthrough_timer: passthrough,
        };

        // 确定entry地址
        let entry = if vcpu.id() == 0 {
            inner_mut.config.bsp_entry()  // BSP入口
        } else {
            inner_mut.config.ap_entry()   // AP入口
        };

        // 关键步骤：vCPU设置
        vcpu.setup(
            entry,                               // Guest入口地址
            inner_mut.address_space.page_table_root(),  // Stage-2页表根地址
            setup_config,                         // vCPU配置
        )?;
    }
}
```

##### 2. AxArchVCpu trait接口定义

**实际的trait定义（axvcpu/src/arch_vcpu.rs）：**
```rust
/// 架构特定的虚拟CPU trait定义
pub trait AxArchVCpu: Sized {
    /// vCPU创建时的架构特定配置
    type CreateConfig;
    
    /// vCPU设置时的架构特定配置  
    type SetupConfig;

    /// 创建新的架构特定vCPU实例
    fn new(vm_id: VMId, vcpu_id: VCpuId, config: Self::CreateConfig) -> AxResult<Self>;
    
    /// 设置Guest入口点
    fn set_entry(&mut self, entry: GuestPhysAddr) -> AxResult;
    
    /// 设置扩展页表(EPT)根地址（用于内存翻译）
    fn set_ept_root(&mut self, ept_root: HostPhysAddr) -> AxResult;
    
    /// 完成vCPU初始化并准备执行
    fn setup(&mut self, config: Self::SetupConfig) -> AxResult;
    
    /// 执行vCPU直到VM退出
    fn run(&mut self) -> AxResult<AxVCpuExitReason>;
    
    /// 绑定vCPU到当前物理CPU
    fn bind(&mut self) -> AxResult;
    
    /// 从当前物理CPU解绑vCPU
    fn unbind(&mut self) -> AxResult;
    
    /// 设置通用寄存器值
    fn set_gpr(&mut self, reg: usize, val: usize);
    
    /// 注入中断到vCPU
    fn inject_interrupt(&mut self, vector: usize) -> AxResult;
    
    /// 设置返回值
    fn set_return_value(&mut self, val: usize);
}
```

##### 3. Aarch64VCpu实际实现

**ARM64虚拟CPU的核心实现（arm_vcpu/src/vcpu.rs）：**
```rust
/// ARM64架构的虚拟CPU实现
#[repr(C)]
#[derive(Debug)]
pub struct Aarch64VCpu<H: AxVCpuHal> {
    ctx: TrapFrame,                           // Guest上下文（通用寄存器等）
    host_stack_top: u64,                      // Host栈顶地址
    guest_system_regs: GuestSystemRegisters,       // Guest系统寄存器
    mpidr: u64,                            // MPIDR_EL1值
    _phantom: PhantomData<H>,
}

/// vCPU创建配置
#[derive(Clone, Debug, Default)]
pub struct Aarch64VCpuCreateConfig {
    pub mpidr_el1: u64,     // MPIDR_EL1值，用于多处理器系统中的CPU识别
    pub dtb_addr: usize,     // 设备树blob地址
}

/// vCPU设置配置
#[derive(Clone, Debug, Default)]
pub struct Aarch64VCpuSetupConfig {
    pub passthrough_interrupt: bool,  // 是否直通中断到Guest
    pub passthrough_timer: bool,     // 是否直通定时器到Guest
}
```

##### 4. 关键的setup方法实现

**实际的setup方法（arm_vcpu/src/vcpu.rs）：**
```rust
impl<H: AxVCpuHal> axvcpu::AxArchVCpu for Aarch64VCpu<H> {
    fn setup(&mut self, config: Self::SetupConfig) -> AxResult {
        self.init_hv(config);  // 初始化Hypervisor相关配置
        Ok(())
    }
}

// 私有方法：初始化虚拟化环境
fn init_hv(&mut self, config: Aarch64VCpuSetupConfig) {
    // 1. 设置Guest异常返回状态
    self.ctx.spsr = (SPSR_EL1::M::EL1h          // EL1模式
        + SPSR_EL1::I::Masked               // 屏蔽IRQ
        + SPSR_EL1::F::Masked               // 屏蔽FIQ  
        + SPSR_EL1::A::Masked               // 屏蔽SError
        + SPSR_EL1::D::Masked)              // 屏蔽Debug
        .value;
    
    // 2. 初始化VM上下文
    self.init_vm_context(config);
}

fn init_vm_context(&mut self, config: Aarch64VCpuSetupConfig) {
    // 定时器配置
    self.guest_system_regs.cntvoff_el2 = 0;
    self.guest_system_regs.cntkctl_el1 = 0;
    self.guest_system_regs.cnthctl_el2 = if config.passthrough_timer {
        // 允许Guest访问物理定时器
        (CNTHCTL_EL2::EL1PCEN::SET + CNTHCTL_EL2::EL1PCTEN::SET).into()
    } else {
        // 拦截Guest定时器访问
        (CNTHCTL_EL2::EL1PCEN::CLEAR + CNTHCTL_EL2::EL1PCTEN::CLEAR).into()
    };

    // Guest系统控制寄存器初始值
    self.guest_system_regs.sctlr_el1 = 0x30C50830;
    self.guest_system_regs.pmcr_el0 = 0;

    // Stage-2页表配置（关键！）
    #[cfg(feature = "4-level-ept")] {
        // 4级页表配置
        self.guest_system_regs.vtcr_el2 = (
            VTCR_EL2::PS::PA_48B_256TB           // 48位物理地址
            + VTCR_EL2::TG0::Granule4KB           // 4KB页粒度
            + VTCR_EL2::SH0::Inner                 // 内部共享
            + VTCR_EL2::ORGN0::NormalWBRAWA       // 写回缓存
            + VTCR_EL2::IRGN0::NormalWBRAWA       // 写回缓存
            + VTCR_EL2::SL0.val(0b10)             // 从L0开始（4级页表）
            + VTCR_EL2::T0SZ.val(64 - 48)        // 48位IPA空间
        ).into();
    }

    // 3. 配置HCR_EL2（最关键的虚拟化控制寄存器）
    let mut hcr_el2 = HCR_EL2::VM::Enable              // 启用Stage-2翻译
        + HCR_EL2::RW::EL1IsAarch64              // Guest运行在AArch64 EL1
        + HCR_EL2::FMO::EnableVirtualFIQ            // 虚拟FIQ
        + HCR_EL2::TSC::EnableTrapEl1SmcToEl2;  // 拦截SMC指令

    // 中断处理配置
    if !config.passthrough_interrupt {
        // 启用虚拟中断并拦截物理中断到EL2
        hcr_el2 += HCR_EL2::IMO::EnableVirtualIRQ;
    }

    // 设置HCR_EL2（Guest完全看不到这个寄存器！）
    self.guest_system_regs.hcr_el2 = hcr_el2.into();

    // 4. 设置VMPIDR_EL2（虚拟化处理器ID）
    let mut vmpidr = 1 << 31;  // 设置位31为1
    vmpidr |= self.mpidr;     // 使用真实的MPIDR值
    self.guest_system_regs.vmpidr_el2 = vmpidr;
}
```

##### 5. Stage-2页表根地址设置

**EPT根地址的实际设置（arm_vcpu/src/vcpu.rs）：**
```rust
impl<H: AxVCpuHal> axvcpu::AxArchVCpu for Aarch64VCpu<H> {
    fn set_ept_root(&mut self, ept_root: HostPhysAddr) -> AxResult {
        debug!("set vcpu ept root:{ept_root:#x}");
        // 关键：将Stage-2页表根地址存储到VTTBR_EL2
        self.guest_system_regs.vttbr_el2 = ept_root.as_usize() as u64;
        Ok(())
    }
    
    fn set_entry(&mut self, entry: GuestPhysAddr) -> AxResult {
        debug!("set vcpu entry:{entry:?}");
        // 设置Guest入口地址到ELR_EL1
        self.set_elr(entry.as_usize());
        Ok(())
    }
}

// 私有方法：设置ELR_EL1
fn set_elr(&mut self, elr: usize) {
    self.ctx.set_exception_pc(elr);
}
```

##### 6. Guest上下文结构

**Guest寄存器上下文的实际定义（arm_vcpu/src/context_frame.rs）：**
```rust
/// Guest系统寄存器结构（包含虚拟化所需的所有寄存器）
#[repr(C)]
#[repr(align(16))]
#[derive(Debug, Clone, Copy, Default)]
pub struct GuestSystemRegisters {
    // 定时器寄存器
    pub cntvoff_el2: u64,           // 虚拟定时器偏移
    pub cntv_cval_el0: u64,          // 虚拟定时器比较值
    pub cntkctl_el1: u32,            // 定时器控制
    pub cnthctl_el2: u64,            // Hypervisor定时器控制
    
    // 虚拟化处理器ID
    vpidr_el2: u32,                // 虚拟处理器ID
    pub vmpidr_el2: u64,            // 虚拟化多处理器ID
    
    // Guest的EL1/EL0寄存器（Guest以为自己正在使用）
    pub sp_el0: u64,                 // Guest EL0栈指针
    sp_el1: u64,                   // Guest EL1栈指针
    elr_el1: u64,                  // Guest异常链接寄存器
    spsr_el1: u32,                 // Guest程序状态寄存器
    pub sctlr_el1: u32,             // Guest系统控制寄存器
    ttbr0_el1: u64,                // Guest用户空间页表基址
    ttbr1_el1: u64,                // Guest内核空间页表基址
    tcr_el1: u64,                  // Guest翻译控制寄存器
    // ... 其他Guest寄存器
    
    // Hypervisor寄存器（Guest完全不知道这些！）
    pub hcr_el2: u64,                // 虚拟化控制寄存器
    pub vttbr_el2: u64,              // Stage-2页表基址
    pub vtcr_el2: u64,              // Stage-2翻译控制
    // ... 其他Hypervisor寄存器
}

/// Guest通用寄存器上下文
#[repr(C)]
#[derive(Copy, Clone, Debug)]
pub struct Aarch64ContextFrame {
    pub gpr: [u64; 31],            // 31个通用寄存器
    pub sp_el0: u64,                 // EL0栈指针
    pub elr: u64,                   // 异常链接寄存器
    pub spsr: u64,                  // 程序状态寄存器
}
```

##### 7. 实际的vCPU运行流程

**实际的run方法（arm_vcpu/src/vcpu.rs）：**
```rust
impl<H: AxVCpuHal> axvcpu::AxArchVCpu for Aarch64VCpu<H> {
    fn run(&mut self) -> AxResult<AxVCpuExitReason> {
        // 运行Guest
        let exit_reason = unsafe {
            // 1. 保存Host SP_EL0（因为被用作当前任务指针）
            save_host_sp_el0();
            
            // 2. 恢复VM系统寄存器
            self.restore_vm_system_regs();
            
            // 3. 运行Guest（控制权转移给Guest）
            self.run_guest()
        };

        // 4. Guest退出后的处理
        let trap_kind = TrapKind::try_from(exit_reason as u8)
            .expect("Invalid TrapKind");
        self.vmexit_handler(trap_kind)  // 处理VM退出原因
    }

    // 私有方法：恢复VM系统寄存器
    unsafe fn restore_vm_system_regs(&mut self) {
        unsafe {
            // 清零CPTR_EL2（不拦截任何EL1系统寄存器访问）
            core::arch::asm!(
                "mov x3, xzr
                 msr cptr_el2, x3"
            );
            
            // 恢复所有Guest系统寄存器（包括HCR_EL2, VTTBR_EL2等）
            self.guest_system_regs.restore();
            
            // TLB刷新
            core::arch::asm!(
                "ic  iallu
                 tlbi    alle2
                 tlbi    alle1
                 dsb     nsh
                 isb"
            );
        }
    }
}
```

##### 8. 关键理解：Guest无感知的虚拟化

**Guest完全不知道的虚拟化控制：**

1. **HCR_EL2.VM = 1**：在init_hv()中设置，启用Stage-2翻译
2. **VTTBR_EL2**：通过set_ept_root()设置，指向Stage-2页表
3. **VTCR_EL2**：在init_vm_context()中设置，配置Stage-2翻译参数
4. **Guest看到的寄存器**：Guest只能访问TTBR0_EL1/TTBR1_EL1，无法感知HCR_EL2的存在

**Guest访问内存的实际流程：**

```
当Guest执行 ldr x0, [x1] (x1 = 0x80000000) 时：

T1: Guest MMU开始Stage-1翻译（Guest知道这部分）
    使用TTBR1_EL1查页表：GVA 0x80000000 → GPA 0x40000000
    
T2: 硬件检查HCR_EL2.VM=1（Guest不知道这部分）
    自动启动Stage-2翻译
    
T3: Hypervisor MMU执行Stage-2翻译（Guest不知道这部分）
    使用VTTBR_EL2查Stage-2页表：GPA 0x40000000 → HPA 0x20000000
    
T4: 最终访问物理内存（Guest不知道这部分）
    访问真实物理地址0x20000000，返回数据给Guest
```

**实际的抽象层次关系：**

```
axvm层：          vCPU.create() → vCPU.setup() → vCPU.run()
  ↓
axvcpu层：        AxArchVCpu trait接口调用
  ↓  
arm_vcpu层：      Aarch64VCpu具体实现 → 寄存器操作 → 汇编指令
  ↓
硬件层：          ARM64虚拟化硬件执行实际的Guest运行
```

这种分层设计的优势：
1. **架构无关性**：axvm和axvcpu提供抽象接口，支持多种架构
2. **安全性**：Guest完全无法感知Hypervisor的存在
3. **性能**：大部分翻译由硬件自动完成，无需软件干预
4. **可扩展性**：通过trait接口支持新的虚拟化特性

---


## 内存分配的三种模式

axvisor为虚拟机提供三种不同的内存分配模式：

### 1. MapAlloc模式 - 指定地址映射
```rust
// 配置示例
VmMemMappingType::MapAlloc => {
    vm.alloc_memory_region(
        Layout::from_size_align(512 * MB, 2 * MB).unwrap(),
        Some(GuestPhysAddr::from(0x40000000)),  // 指定GPA
    )
}

// 效果：
// 虚拟机期望的物理地址：0x40000000
// axvisor实际分配的物理地址：0x20000000（随机）
// 建立映射：GPA 0x40000000 ──► HPA 0x20000000
```

### 2. MapIdentical模式 - 恒等映射
```rust
// 配置示例
VmMemMappingType::MapIdentical => {
    vm.alloc_memory_region(
        Layout::from_size_align(256 * MB, 2 * MB).unwrap(),
        None,  // 不指定GPA，使用恒等映射
    )
}

// 效果：
// axvisor分配物理地址：0x30000000
// 建立恒等映射：GPA 0x30000000 ──► HPA 0x30000000
// GPA和HPA数值相同，便于调试和管理
```

### 3. MapReserved模式 - 预留内存映射
```rust
// 配置示例
VmMemMappingType::MapReserved => {
    vm.map_reserved_memory_region(
        Layout::from_size_align(128 * MB, 2 * MB).unwrap(),
        Some(GuestPhysAddr::from(0x80000000)),
    )
}

// 效果：
// 使用已预留的物理内存区域
// 通常用于设备映射或特殊内存区域
```

---

## MMU配置和页表建立

### axvisor的页表管理架构

#### 页表层次结构
```
┌─────────────────────────────────────────────────────────┐
│                   axvisor (EL2)                        │
├─────────────────────────────────────────────────────────┤
│  Hypervisor页表 (TTBR0_EL2)                           │
│  ┌─────────┐ ┌─────────┐ ┌─────────┐ ┌─────────┐      │
│  │  L0表    │ → │  L1表    │ → │  L2表    │ → │  L3表    │   │
│  └─────────┘ └─────────┘ └─────────┘ └─────────┘      │
│         │           │           │           │            │
│         └── HVA ──► HPA (axvisor自身内存翻译)         │
├─────────────────────────────────────────────────────────┤
│  Stage-2页表 (VTTBR_EL2)                             │
│  ┌─────────┐ ┌─────────┐ ┌─────────┐ ┌─────────┐      │
│  │  L0表    │ → │  L1表    │ → │  L2表    │ → │  L3表    │   │
│  └─────────┘ └─────────┘ └─────────┘ └─────────┘      │
│         │           │           │           │            │
│         └── GPA ──► HPA (虚拟机内存翻译)              │
├─────────────────────────────────────────────────────────┤
│  Guest页表 (TTBR0_EL1/TTBR1_EL1)                    │
│  ┌─────────┐ ┌─────────┐ ┌─────────┐ ┌─────────┐      │
│  │  L0表    │ → │  L1表    │ → │  L2表    │ → │  L3表    │   │
│  └─────────┘ └─────────┘ └─────────┘ └─────────┘      │
│         │           │           │           │            │
│         └── GVA ──► GPA (虚拟机内部地址翻译)           │
└─────────────────────────────────────────────────────────┘
```

### Hypervisor页表初始化（EL2）

#### 两阶段初始化策略
axvisor采用两阶段MMU初始化，解决"鸡生蛋"问题：

**阶段1：启动页表（物理地址模式）**
```rust
// 在pie-boot-loader中建立最小化映射
fn new_boot_table<T, F>(args: &EarlyBootArgs, fdt: usize, new_pte: F) -> PhysAddr {
    let mut table = PageTableRef::create_empty(access);
    
    unsafe {
        // 1. 内核代码段映射（最关键的映射）
        let code_size = (args.kimage_addr_lma + kcode_offset).align_up(2 * MB);
        table.map(MapConfig {
            vaddr: args.kimage_addr_vma.into(),
            paddr: args.kimage_addr_lma.into(),
            size: code_size,
            pte: new_pte(CacheKind::Normal),
            allow_huge: true,  // 使用2MB大页
            flush: false,
        }, access);
        
        // 2. RAM内存映射（从设备树解析）
        add_rams(fdt, &mut table, access, new_pte);
        
        // 3. 调试设备映射（串口等）
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
    
    table.paddr()  // 返回页表物理地址
}
```

**阶段2：完善页表（虚拟地址模式）**
```rust
// 在somehal中建立完整映射
fn regions_to_map() -> Vec<MapRangeConfig> {
    let mut map_ranges = Vec::new();
    
    // 1. RAM区域精细映射
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
    
    // 2. 内核各段精细权限映射
    map_ranges.push(ld_range_to_map_config("text", ld::text, true, AccessKind::ReadExecute));
    map_ranges.push(ld_range_to_map_config("rodata", ld::rodata, true, AccessKind::Read));
    map_ranges.push(ld_range_to_map_config("data", ld::data, true, AccessKind::ReadWriteExecute));
    map_ranges.push(ld_range_to_map_config("bss", ld::bss, true, AccessKind::ReadWriteExecute));
    map_ranges.push(ld_range_to_map_config("stack0", ld::stack0, false, AccessKind::ReadWriteExecute));
    
    map_ranges
}
```

### Stage-2页表建立（虚拟机内存管理）

#### 概述：axaddrspace架构下的Stage-2页表建立
axvisor使用axaddrspace库来管理虚拟机的Stage-2地址翻译。整个建立过程涉及虚拟机创建、内存分配、地址空间初始化、vCPU设置等多个关键步骤，最终建立完整的GPA→HPA映射机制。

**完整组件关系图：**
```
┌─────────────────────────────────────────────────────────────────┐
│                     AxVM (axvm)                      │
│  ┌─────────────────────────────────────────────┐         │
│  │        AddrSpace (axaddrspace)          │         │
│  │  ┌─────────────────────────────────┐      │         │
│  │  │    NestedPageTable (NPT)     │      │         │
│  │  │  ┌─────────────────────────┐  │      │         │
│  │  │  │  PageTable64<...>   │  │      │         │
│  │  │  │ ┌─────────────────┐ │  │      │         │
│  │  │  │ │   A64PTEHV    │ │  │      │         │
│  │  │  │ │ (Stage-2 PTE) │ │  │      │         │
│  │  │  │ └─────────────────┘ │  │      │         │
│  │  │  │  page_table_multiarch│  │      │         │
│  │  │  │     核心引擎       │  │      │         │
│  │  │  └─────────────────────────┘  │      │         │
│  │  └─────────────────────────────────┘      │         │
│  └─────────────────────────────────────────────┘         │
└─────────────────────────────────────────────────────────────────┘
                          │
                          ▼
        ┌─────────────────────────────────────────┐
        │    page_table_multiarch库           │
        │  ┌─────────────────────────────┐   │
        │  │    PageTable64<M,PTE,H>   │   │
        │  │  ┌─────────────────────┐   │   │
        │  │  │   算法核心         │   │   │
        │  │  │ - 页表遍历算法       │   │   │
        │  │  │ - 大页优化策略       │   │   │
        │  │  │ - TLB刷新机制       │   │   │
        │  │  │ - 架构抽象层       │   │   │
        │  │  └─────────────────────┘   │   │
        │  └─────────────────────────────┘   │
        └─────────────────────────────────────────┘
```

#### 完整的Stage-2页表建立流程

##### 第1步：VM对象创建和配置解析
```rust
// axvm/src/vm.rs - VM创建（第88-107行）
impl<H: AxVMHal, U: AxVCpuHal> AxVM<H, U> {
    /// Creates a new VM with the given configuration.
    /// Returns an error if the configuration is invalid.
    /// The VM is not started until `boot` is called.
    pub fn new(config: AxVMConfig) -> AxResult<AxVMRef<H, U>> {
        // 1. 立即创建地址空间（关键修正！）
        let address_space = AddrSpace::new_empty(
            GuestPhysAddr::from(VM_ASPACE_BASE), 
            VM_ASPACE_SIZE  // 2TB-4KB地址空间
        )?;

        // 2. 创建VM对象并包装为Arc
        let result = Arc::new(Self {
            id: config.id(),              // 使用配置中的ID，不是自动生成
            inner_const: Once::new(),     // 延迟初始化常量部分
            inner_mut: Mutex::new(AxVMInnerMut {
                address_space,           // 地址空间在new()时创建
                config,                 // 配置直接存储
                memory_regions: Vec::new(),
                vm_status: VMStatus::Loading,  // 初始状态为Loading
                _marker: core::marker::PhantomData,
            }),
        });

        // 3. 记录VM创建日志
        info!("VM created: id={}", result.id());

        // 4. 返回Arc包装的VM引用
        Ok(result)
    }
}
```

##### 第2步：VM初始化和vCPU创建
```rust
// axvm/src/vm.rs - VM初始化（第112行开始）
pub fn init(&self) -> AxResult {
    let mut inner_mut = self.inner_mut.lock();
    
    // 1. 获取设备树地址和vCPU亲和性配置
    let dtb_addr = inner_mut.config.image_config().dtb_load_gpa;
    let vcpu_id_pcpu_sets = inner_mut.config.phys_cpu_ls.get_vcpu_affinities_pcpu_ids();

    // 2. 创建vCPU列表（根据物理CPU亲和性配置）
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
    
    // 3. 创建vCPU实例并设置设备
    let mut vcpu_list = Vec::with_capacity(vcpu_id_pcpu_sets.len());
    for (vcpu_id, phys_cpu_set, _pcpu_id) in vcpu_id_pcpu_sets {
        // 创建架构特定配置
        #[cfg(target_arch = "aarch64")]
        let arch_config = AxVCpuCreateConfig {
            mpidr_el1: _pcpu_id as _,
            dtb_addr: dtb_addr.unwrap_or_default().as_usize(),
        };
        
        // 创建vCPU实例
        vcpu_list.push(Arc::new(VCpu::new(
            self.id(),
            vcpu_id,
            0, // 目前未使用的参数
            phys_cpu_set,
            arch_config,
        )?));
    }
    
    // 4. 确定中断模式
    let passthrough = inner_mut.config.interrupt_mode() == axvmconfig::VMInterruptMode::Passthrough;
    
    // 5. 通过call_once初始化常量部分
    let mut devices = axdevice::AxVmDevices::new(AxVmDeviceConfig {
        emu_configs: inner_mut.config.emu_devices().to_vec(),
    });
    
    // 6. 设置GIC设备
    #[cfg(target_arch = "aarch64")]
    {
        if passthrough {
            let spis = inner_mut.config.pass_through_spis();
            let cpu_id = self.id() - 1; // FIXME: get the real CPU id.
            let mut gicd_found = false;

            for device in devices.iter_mmio_dev() {
                if let Some(result) = axdevice_base::map_device_of_type(
                    device,
                    |gicd: &arm_vgic::v3::vgicd::VGicD| {
                        debug!("VGicD found, assigning SPIs...");

                        for spi in spis {
                            gicd.assign_irq(*spi + 32, cpu_id, (0, 0, 0, cpu_id as _))
                        }

                        AxResult::Ok(())
                    },
                ) {
                    result?;
                    gicd_found = true;
                    break;
                }
            }

            if !gicd_found {
                warn!("Failed to assign SPIs: No VGicD found in device list");
            }
        } else {
            // 添加系统寄存器设备
            for dev in get_sysreg_device() {
                devices.add_sys_reg_dev(dev);
            }
        }
    }
    
    // 7. 通过call_once初始化常量部分
    self.inner_const.call_once(|| AxVMInnerConst {
        phys_cpu_ls: inner_mut.config.phys_cpu_ls.clone(),
        vcpu_list: vcpu_list.into_boxed_slice(),
        devices,
    });
    
    // 8. 设置所有vCPU
    for vcpu in self.vcpu_list() {
        let setup_config = axvcpu::Aarch64VCpuSetupConfig {
            passthrough_interrupt: passthrough,
            passthrough_timer: passthrough,
        };

        let entry = if vcpu.id() == 0 {
            inner_mut.config.bsp_entry()  // BSP入口
        } else {
            inner_mut.config.ap_entry()   // AP入口
        };

        vcpu.setup(
            entry,
            inner_mut.address_space.page_table_root(),  // Stage-2页表根地址
            setup_config,
        )?;
    }
    
    // 6. 更新VM状态
    inner_mut.vm_status = VMStatus::Loaded;
    info!("VM setup: id={}", self.id());
    
    Ok(())
}
```

##### 第3步：虚拟机内存分配和Stage-2映射建立
```rust
// axvm/src/vm.rs - 内存分配核心函数
pub fn alloc_memory_region(
    &self,
    layout: Layout,
    gpa: Option<GuestPhysAddr>,
) -> AxResult<&[u8]> {
    assert!(layout.size() > 0, "Cannot allocate zero-sized memory region");

    // 1. 使用系统分配器分配零初始化内存
    let hva = unsafe { alloc::alloc::alloc_zeroed(layout) };
    if hva.is_null() {
        return Err(AxError::NoMemory);
    }
    
    // 2. 转换为HVA (Host Virtual Address) 并创建slice
    let s = unsafe { core::slice::from_raw_parts_mut(hva, layout.size()) };
    let hva = HostVirtAddr::from_mut_ptr_of(hva);
    
    // 3. 通过HAL将HVA转换为HPA (Host Physical Address)
    let hpa = H::virt_to_phys(hva);
    
    // 4. 确定最终的GPA（关键逻辑！）
    let gpa = gpa.unwrap_or_else(|| hpa.as_usize().into());
    // 解释：
    // - 如果gpa是Some(value)，使用指定的值（MapAlloc模式）
    // - 如果gpa是None，使用hpa转换的值（MapIdentical模式）
    
    // 5. 通过AddrSpace建立Stage-2映射：GPA → HPA
    let mut g = self.inner_mut.lock();
    g.address_space.map_linear(
        gpa,                              // Guest物理地址
        hpa,                               // Host物理地址  
        layout.size(),
        MappingFlags::READ | MappingFlags::WRITE | MappingFlags::EXECUTE | MappingFlags::USER,
    )?;
    
    // 6. 记录内存区域信息
    g.memory_regions.push(VMMemoryRegion {
        gpa,
        hva,
        layout,
        needs_dealloc: true, // 标记需要释放
    });
    
    Ok(s)  // 返回可变slice引用
}
```

##### 第4步：AddrSpace的map_linear实现
```rust
// axaddrspace/src/address_space/mod.rs
impl<H: PagingHandler> AddrSpace<H> {
    pub fn map_linear(
        &mut self,
        start_vaddr: GuestPhysAddr,    // GPA
        start_paddr: PhysAddr,        // HPA
        size: usize,
        flags: MappingFlags,
    ) -> AxResult {
        // 1. 地址范围检查
        if !self.contains_range(start_vaddr, size) {
            return ax_err!(InvalidInput, "address out of range");
        }
        if !start_vaddr.is_aligned_4k() || !start_paddr.is_aligned_4k() || !is_aligned_4k(size) {
            return ax_err!(InvalidInput, "address not aligned");
        }

        // 2. 计算偏移量（线性映射的核心）
        let offset = start_vaddr.as_usize() - start_paddr.as_usize();
        // MapAlloc模式：offset ≠ 0 (例如：GPA 0x40000000 - HPA 0x20000000 = 0x20000000)
        // MapIdentical模式：offset = 0 (例如：GPA 0x10000000 - HPA 0x10000000 = 0)
        
        // 3. 创建MemoryArea并映射
        let area = MemoryArea::new(start_vaddr, size, flags, Backend::new_linear(offset));
        self.areas.map(area, &mut self.pt, false)
            .map_err(mapping_err_to_ax_err)?;
        Ok(())
    }
}
```

##### 第4步详解：MemorySet::map方法调用链分析

`self.areas.map()`是连接AddrSpace和底层页表建立的关键桥梁。这个调用涉及五个层次的协作：

```rust
// AddrSpace中的调用
self.areas.map(area, &mut self.pt, false)  // memory_set库的MemorySet::map
    ↓
// memory_set库内部调用MemoryArea的map_area方法
area.map_area(page_table)                  // MemoryArea的map_area方法
    ↓
// MemoryArea通过backend调用MappingBackend trait
backend.map(start, size, flags, page_table)  // axaddrspace实现的MappingBackend::map  
    ↓
// Backend分派到具体的Linear实现
Backend::Linear { pa_va_offset }.map_linear(start, size, flags, pt, offset)
    ↓
// 最终调用NestedPageTable的页表建立方法
NestedPageTable::map_region(start, translate_fn, size, flags, allow_huge, force_flush)
```

#### 4.1 MemorySet::map方法的作用（memory_set库）

```rust
// memory_set库中MemorySet::map的实际实现
impl<B: MappingBackend> MemorySet<B> {
    /// Add a new memory mapping.
    ///
    /// The mapping is represented by a [`MemoryArea`].
    ///
    /// If new area overlaps with any existing area, behavior is
    /// determined by `unmap_overlap` parameter. If it is `true`,
    /// overlapped regions will be unmapped first. Otherwise, it returns an error.
    pub fn map(
        &mut self,
        area: MemoryArea<B>,              // MemoryArea包含地址范围、权限、Backend
        page_table: &mut B::PageTable,    // 页表引用
        unmap_overlap: bool,               // 是否取消重叠映射
    ) -> MappingResult {
        // 1. 检查内存区域有效性
        if area.va_range().is_empty() {
            return Err(MappingError::InvalidParam);
        }

        // 2. 检查重叠区域并处理
        if self.overlaps(area.va_range()) {
            if unmap_overlap {
                self.unmap(area.start(), area.size(), page_table)?;
            } else {
                return Err(MappingError::AlreadyExists);
            }
        }

        // 3. 调用Backend的map方法进行实际映射（通过MemoryArea的map_area方法）
        area.map_area(page_table)?;
        
        // 4. 将MemoryArea添加到BTreeMap中管理（以起始地址为key）
        assert!(self.areas.insert(area.start(), area).is_none());
        Ok(())
    }
}
```

#### 4.2 MappingBackend trait的定义（memory_set库）

```rust
// memory_set库中MappingBackend trait的定义
pub trait MappingBackend {
    type Addr: MemoryAddr;           // 地址类型
    type Flags: Copy + Debug;         // 权限标志类型
    type PageTable;                  // 页表类型

    /// Maps a memory area.
    fn map(&self, start: Self::Addr, size: usize, flags: Self::Flags, page_table: &mut Self::PageTable) -> bool;
    
    /// Unmaps a memory area.
    fn unmap(&self, start: Self::Addr, size: usize, page_table: &mut Self::PageTable) -> bool;
    
    /// Changes protection flags of a memory area.
    fn protect(&self, start: Self::Addr, size: usize, new_flags: Self::Flags, page_table: &mut Self::PageTable) -> bool;
}
```

#### 4.3 MemoryArea::map_area方法详解（memory_set库实现）

```rust
// memory_set库中MemoryArea的map_area方法实际实现
impl<B: MappingBackend> MemoryArea<B> {
    /// Maps the whole memory area in the page table.
    /// 
    /// 这个方法是连接MemoryArea和Backend的关键桥梁：
    /// 1. 提取MemoryArea的地址范围、权限等信息
    /// 2. 调用backend的map方法进行实际的页表映射
    /// 3. 将bool返回值转换为MappingResult
    pub(crate) fn map_area(&self, page_table: &mut B::PageTable) -> MappingResult {
        self.backend
            .map(
                self.start(),           // 起始地址 (GuestPhysAddr)
                self.size(),            // 映射大小
                self.flags,             // 映射权限
                page_table              // 页表引用
            )
            .then_some(())                          // 将bool转为Result
            .ok_or(MappingError::BadState)          // false时返回BadState错误
    }
}
```

**关键理解：**

- **责任分离**：MemoryArea负责存储映射信息，Backend负责具体映射实现
- **参数传递**：map_area方法将MemoryArea的内部状态（地址、大小、权限）传递给Backend
- **错误转换**：Backend的map方法返回bool，map_area将其转换为标准Result类型
- **类型安全**：通过泛型参数B确保Backend和PageTable类型的一致性

#### 4.4 Backend的具体实现（axaddrspace实现）

```rust
// axaddrspace/src/address_space/backend/mod.rs
impl<H: PagingHandler> MappingBackend for Backend<H> {
    type Addr = GuestPhysAddr;
    type Flags = MappingFlags;
    type PageTable = PageTable<H>;

    fn map(
        &self,
        start: GuestPhysAddr,           // GPA
        size: usize,                    // 大小
        flags: MappingFlags,            // 权限
        page_table: &mut PageTable<H>,  // NestedPageTable
    ) -> bool {
        match *self {
            Self::Linear { pa_va_offset } => {
                // 调用Linear backend的具体实现
                self.map_linear(start, size, flags, page_table, pa_va_offset)
            }
            Self::Alloc { populate, .. } => {
                // 调用Alloc backend的具体实现
                self.map_alloc(start, size, flags, page_table, populate)
            }
        }
    }

    fn unmap(&self, start: GuestPhysAddr, size: usize, page_table: &mut PageTable<H>) -> bool {
        match *self {
            Self::Linear { pa_va_offset } => {
                self.unmap_linear(start, size, page_table, pa_va_offset)
            }
            Self::Alloc { populate, .. } => {
                self.unmap_alloc(start, size, page_table, populate)
            }
        }
    }

    fn protect(&self, start: GuestPhysAddr, size: usize, new_flags: MappingFlags, page_table: &mut PageTable<H>) -> bool {
        // 通过NestedPageTable直接调用protect_region
        page_table.protect_region(start, size, new_flags, true).is_ok()
    }
}
```

#### 4.5 调用流程的关键参数传递

以MapAlloc模式为例（GPA 0x40000000 → HPA 0x20000000）：

```rust
// 第1层：AddrSpace::map_linear调用
let area = MemoryArea::new(
    start_vaddr,          // GuestPhysAddr(0x40000000)
    size,                 // 0x200000 (2MB)
    flags,                // READ|WRITE|EXECUTE|USER
    Backend::new_linear(offset)  // Backend::Linear { pa_va_offset: 0x20000000 }
);

// 第2层：MemorySet::map调用
self.areas.map(area, &mut self.pt, false)
// 内部处理流程：
// - area.va_range() = AddrRange { start: 0x40000000, end: 0x40200000 }
// - area.start() = GuestPhysAddr(0x40000000)
// - area.size() = 0x200000
// - area.flags() = READ|WRITE|EXECUTE|USER
// - page_table = &mut self.pt (NestedPageTable)
// - 调用area.map_area(page_table)

// 第3层：MemoryArea::map_area调用
area.map_area(page_table)
// 内部调用Backend::map方法：
self.backend.map(
    self.start(),           // GuestPhysAddr(0x40000000)
    self.size(),           // 0x200000
    self.flags,           // READ|WRITE|EXECUTE|USER
    page_table           // &mut self.pt
)

// 第4层：Backend::map调用（分派到Linear实现）
match self {
    Backend::Linear { pa_va_offset } => {
        self.map_linear(
            start: GuestPhysAddr(0x40000000),   // GPA
            size: 0x200000,                     // 2MB
            flags: READ|WRITE|EXECUTE|USER,      // 权限
            pt: &mut self.pt,                   // NestedPageTable
            pa_va_offset: 0x20000000            // GPA→HPA偏移量
        )
    }
}

// 第5层：最终调用NestedPageTable
// self.map_linear内部会调用：
pt.map_region(
    start: GuestPhysAddr(0x40000000),
    translate_fn: |va| PhysAddr::from(va.as_usize() - 0x20000000),
    size: 0x200000,
    flags: READ|WRITE|EXECUTE|USER,
    allow_huge: true,
    force_flush: true
)
```

#### 4.6 关键设计模式：策略模式+责任链

```rust
// 策略模式：Backend抽象了不同的映射策略
pub enum Backend<H: PagingHandler> {
    Linear { pa_va_offset: usize },      // 线性映射策略
    Alloc { populate: bool, _phantom },  // 动态分配策略
}

// 责任链：AddrSpace → MemorySet → MemoryArea → Backend → NestedPageTable → PageTable64
impl<H: PagingHandler> AddrSpace<H> {
    pub fn map_linear(&mut self, ...) -> AxResult {
        // 第1环：创建MemoryArea（包含策略对象）
        let area = MemoryArea::new(..., Backend::new_linear(offset));
        
        // 第2环：委托给MemorySet管理（处理重叠检测）
        self.areas.map(area, &mut self.pt, false)
            .map_err(mapping_err_to_ax_err)?;
        
        // 第3-6环：由MemoryArea、Backend、NestedPageTable、PageTable64继续处理
        Ok(())
    }
}
```

**五层责任链的职责分工：**

1. **AddrSpace（第1环）**：
   - 参数验证和地址对齐检查
   - 计算GPA→HPA偏移量
   - 创建MemoryArea对象

2. **MemorySet（第2环）**：
   - 重叠检测和冲突处理
   - 管理内存区域的生命周期
   - 使用BTreeMap高效存储和查找

3. **MemoryArea（第3环）**：
   - 封装映射信息（地址、大小、权限）
   - 调用Backend的map方法
   - 处理错误类型转换

4. **Backend（第4环）**：
   - 实现具体的映射策略（Linear/Alloc）
   - 计算物理地址转换
   - 调用底层页表接口

5. **NestedPageTable/PageTable64（第5-6环）**：
   - 页表项分配和初始化
   - 大页优化策略
   - TLB刷新操作

---

##### 第5步：Linear Backend的映射实现
```rust
// axaddrspace/src/address_space/backend/linear.rs
impl<H: PagingHandler> Backend<H> {
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
        
        debug!("map_linear: [{:#x}, {:#x}) -> [{:#x}, {:#x}) {:?}",
            start, start + size, pa_start, pa_start + size, flags);
            
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
}
```

##### 第6步：NestedPageTable的页表建立

#### map_region方法详解
```rust
// axaddrspace/src/npt/mod.rs
impl<H: PagingHandler> NestedPageTable<H> {
    pub fn map_region<F>(
        &mut self,
        start_vaddr: GuestPhysAddr,
        translate_fn: F,
        size: usize,
        flags: MappingFlags,
        allow_huge: bool,
        force_flush: bool,
    ) -> memory_set::MappingResult
    where
        F: Fn(GuestPhysAddr) -> PhysAddr,
    {
        match self {
            NestedPageTable::L4(pt) => {
                // 调用page_table_multiarch的map_region方法并处理TLB刷新
                pt.map_region(start_vaddr, translate_fn, size, flags, allow_huge, force_flush)
                    .map_err(|_| MappingError::BadState)?
                    .flush_all();
            }
        }
        Ok(())
    }
}
```

#### PageTable64::map方法详解

```rust
// page_table_multiarch/src/bits64.rs
impl<M: PagingMetaData, PTE: GenericPTE, H: PagingHandler> PageTable64<M, PTE, H> {
    /// 建立单个虚拟地址到物理地址的映射
    pub fn map(
        &mut self,
        vaddr: M::VirtAddr,
        target: PhysAddr,
        page_size: PageSize,
        flags: MappingFlags,
    ) -> PagingResult<TlbFlush<M>> {
        // 1. 获取或创建页表项
        let entry = self.get_entry_mut_or_create(vaddr, page_size)?;
        
        // 2. 检查页表项是否已被使用
        if !entry.is_unused() {
            return Err(PagingError::AlreadyMapped);
        }
        
        // 3. 创建新的页表项
        *entry = GenericPTE::new_page(
            target.align_down(page_size),
            flags,
            page_size.is_huge(),
        );
        
        // 4. 返回TLB刷新对象
        Ok(TlbFlush::new(vaddr))
    }
}
```

#### 页表遍历和动态分配

```rust
// 页表索引计算（ARM64 4级页表）
const fn p4_index(vaddr: usize) -> usize { (vaddr >> (12 + 27)) & 0x1FF }  // L0索引[38:47]
const fn p3_index(vaddr: usize) -> usize { (vaddr >> (12 + 18)) & 0x1FF }  // L1索引[29:37]  
const fn p2_index(vaddr: usize) -> usize { (vaddr >> (12 + 9))  & 0x1FF }  // L2索引[20:28]
const fn p1_index(vaddr: usize) -> usize { (vaddr >> 12)        & 0x1FF }  // L3索引[12:19]

// 动态页表分配
fn next_table_mut_or_create(&mut self, entry: &mut PTE) -> PagingResult<&'a mut [PTE]> {
    if entry.is_unused() {
        // 分配新的页表页（4KB物理页）
        let paddr = Self::alloc_table()?;
        *entry = GenericPTE::new_table(paddr);  // 设置VALID + 物理地址
        Ok(self.table_of_mut(paddr))
    } else {
        self.next_table_mut(entry)
    }
}
```

#### 页表映射工作示例

**示例：映射GPA 0x40000000 → HPA 0x20000000（2MB大页）**

1. **页表索引计算**：
   - L0索引：`(0x40000000 >> 39) & 0x1FF = 0`
   - L1索引：`(0x40000000 >> 30) & 0x1FF = 0` 
   - L2索引：`(0x40000000 >> 21) & 0x1FF = 0`（2MB大页在L2级别）

2. **页表项创建**：
   - L0表项：指向L1页表地址
   - L1表项：指向L2页表地址  
   - L2表项：2MB大页项，包含HPA 0x20000000
   - 属性：`VALID + NON_BLOCK(=0) + S2AP_RO + S2AP_WO + AF`

3. **TLB刷新**：
   - 返回`TlbFlushAll`标记，执行`tlbi alle2is`指令

##### 第7步：ARM64 Hypervisor页表项建立

```rust
// axaddrspace/src/npt/arch/aarch64.rs
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

**关键理解**：
- **NON_BLOCK位**：`is_huge=false`时设为1（表示页表指针），`is_huge=true`时设为0（表示大页）
- **物理地址掩码**：`PHYS_ADDR_MASK = 0x0000_ffff_ffff_f000`，存储bits[47:12]
- **AF位**：Stage-2中必须设为1（访问标志）

##### 第8步：vCPU设置和Stage-2页表激活

vCPU设置实际上在VM的`init()`函数中完成。实际流程如下：

```rust
// axvm/src/vm.rs - VM的init函数中的vCPU设置流程
// 注意：vCPU设置在init()函数的最后完成，没有独立的setup_vcpus函数

// 1. 获取地址空间引用
let g = self.inner_mut.lock();
let address_space = g.address_space.as_ref()
    .ok_or_else(|| AxError::BadState("VM not initialized".into()))?;

// 2. 获取Stage-2页表根地址
let page_table_root = address_space.page_table_root();

// 3. 遍历所有vCPU进行设置
for vcpu in self.vcpu_list() {
    // 配置setup_config
    let setup_config = axvcpu::Aarch64VCpuSetupConfig {
        passthrough_interrupt: passthrough,
        passthrough_timer: passthrough,
    };
    
    // 确定entry地址
    let entry = if vcpu.id() == 0 {
        g.config.bsp_entry()
    } else {
        g.config.ap_entry()
    };
    
    // 调用vCPU.setup()
    vcpu.setup(entry, page_table_root, setup_config)?;
}

// 获取页表根地址
impl<H: PagingHandler> AddrSpace<H> {
    pub fn page_table_root(&self) -> PhysAddr {
        self.pt.root_addr()  // 返回Stage-2页表的物理地址
    }
}

// vCPU设置的实际实现（通过AxArchVCpu trait调用）
impl<H: AxVCpuHal> axvcpu::AxArchVCpu for Aarch64VCpu<H> {
    fn set_entry(&mut self, entry: GuestPhysAddr) -> AxResult {
        debug!("set vcpu entry:{entry:?}");
        self.set_elr(entry.as_usize());  // 设置Guest入口地址到ELR_EL1
        Ok(())
    }
    
    fn set_ept_root(&mut self, ept_root: HostPhysAddr) -> AxResult {
        debug!("set vcpu ept root:{ept_root:#x}");
        // 关键：将Stage-2页表根地址存储到VTTBR_EL2
        self.guest_system_regs.vttbr_el2 = ept_root.as_usize() as u64;
        Ok(())
    }
    
    fn setup(&mut self, config: Self::SetupConfig) -> AxResult {
        self.init_hv(config);  // 初始化虚拟化环境
        Ok(())
    }
}

// init_hv方法：初始化虚拟化环境
fn init_hv(&mut self, config: Aarch64VCpuSetupConfig) {
    // 1. 设置Guest异常返回状态
    self.ctx.spsr = (SPSR_EL1::M::EL1h          // EL1模式
        + SPSR_EL1::I::Masked               // 屏蔽IRQ
        + SPSR_EL1::F::Masked               // 屏蔽FIQ  
        + SPSR_EL1::A::Masked               // 屏蔽SError
        + SPSR_EL1::D::Masked)              // 屏蔽Debug
        .value;
    
    // 2. 初始化VM上下文
    self.init_vm_context(config);
}

fn init_vm_context(&mut self, config: Aarch64VCpuSetupConfig) {
    // 定时器配置
    self.guest_system_regs.cntvoff_el2 = 0;
    self.guest_system_regs.cntkctl_el1 = 0;
    self.guest_system_regs.cnthctl_el2 = if config.passthrough_timer {
        (CNTHCTL_EL2::EL1PCEN::SET + CNTHCTL_EL2::EL1PCTEN::SET).into()
    } else {
        (CNTHCTL_EL2::EL1PCEN::CLEAR + CNTHCTL_EL2::EL1PCTEN::CLEAR).into()
    };

    // Guest系统控制寄存器初始值
    self.guest_system_regs.sctlr_el1 = 0x30C50830;
    self.guest_system_regs.pmcr_el0 = 0;

    // Stage-2页表配置（关键！）
    #[cfg(feature = "4-level-ept")] {
        self.guest_system_regs.vtcr_el2 = (
            VTCR_EL2::PS::PA_48B_256TB           // 48位物理地址
            + VTCR_EL2::TG0::Granule4KB           // 4KB页粒度
            + VTCR_EL2::SH0::Inner                 // 内部共享
            + VTCR_EL2::ORGN0::NormalWBRAWA       // 写回缓存
            + VTCR_EL2::IRGN0::NormalWBRAWA       // 写回缓存
            + VTCR_EL2::SL0.val(0b10)             // 从L0开始（4级页表）
            + VTCR_EL2::T0SZ.val(64 - 48)        // 48位IPA空间
        ).into();
    }

    // 3. 配置HCR_EL2（最关键的虚拟化控制寄存器）
    let mut hcr_el2 = HCR_EL2::VM::Enable              // 启用Stage-2翻译
        + HCR_EL2::RW::EL1IsAarch64              // Guest运行在AArch64 EL1
        + HCR_EL2::FMO::EnableVirtualFIQ            // 虚拟FIQ
        + HCR_EL2::TSC::EnableTrapEl1SmcToEl2;  // 拦截SMC指令

    // 中断处理配置
    if !config.passthrough_interrupt {
        hcr_el2 += HCR_EL2::IMO::EnableVirtualIRQ;  // 启用虚拟中断并拦截物理中断到EL2
    }

    // 设置HCR_EL2（Guest完全看不到这个寄存器！）
    self.guest_system_regs.hcr_el2 = hcr_el2.into();

    // 4. 设置VMPIDR_EL2（虚拟化处理器ID）
    let mut vmpidr = 1 << 31;  // 设置位31为1
    vmpidr |= self.mpidr;     // 使用真实的MPIDR值
    self.guest_system_regs.vmpidr_el2 = vmpidr;
}
```

#### Stage-2页表建立的完整时序图
```
时间轴  │ 流程步骤                    │ 数据结构变化                     │ 硬件状态
────────┼─────────────────────────┼────────────────────────────┼────────────
T1      │ VM::new()                │ 创建VM对象                    │ 无变化
T2      │ VM::init()               │ 创建AddrSpace和NestedPageTable│ 无变化
T3      │ passthrough设备映射       │ 填充页表项(GPA=HPA)           │ 无变化
T4      │ alloc_memory_region()    │ 分配物理内存，建立GPA→HPA映射 │ 无变化
T5      │ address_space.map_linear()│ 计算offset，创建MemoryArea   │ 无变化
T6      │ backend.map_linear()     │ 调用NestedPageTable.map_region│ 无变化
T7      │ NestedPageTable映射      │ 填充A64PTEHV页表项           │ 无变化
T8      │ vcpu.setup()             │ 获取page_table_root          │ 无变化
T9      │ 设置VTTBR_EL2            │ 无变化                       │ VTTBR_EL2=页表根地址
T10     │ 设置HCR_EL2.VM=1         │ 无变化                       │ 启用Stage-2翻译
T11     │ TLB刷新                  │ 无变化                       │ TLB同步完成
T12     │ vcpu.run()               │ 无变化                       │ 开始两阶段翻译
```

#### 内存区域的数据结构

**核心数据结构**：
- **VMMemoryRegion**：跟踪VM的内存分配（GPA、HVA、布局、释放标记）
- **AddrSpace**：管理Stage-2地址空间和内存区域集合
- **MemoryArea**：封装单个内存区域的地址、大小、权限和后端策略
- **Backend**：实现Linear/Alloc两种映射策略

**两种映射模式**：
- **MapAlloc**：GPA ≠ HPA，offset = GPA - HPA ≠ 0
- **MapIdentical**：GPA = HPA，offset = 0（恒等映射）

#### Stage-2页表激活和使用

**关键寄存器设置**：
- `VTTBR_EL2`：Stage-2页表基址（指向axvisor建立的页表）
- `VTCR_EL2`：48位IPA空间，4KB页粒度
- `HCR_EL2.VM=1`：启用Stage-2翻译

**运行时翻译流程**：
```
Guest访问0x80000000:
Stage-1: GVA 0x80000000 → GPA 0x80000000 (Guest OS页表)
Stage-2: GPA 0x80000000 → HPA 0x30000000 (axvisor页表)
```

**Guest完全透明**：Guest无法感知HCR_EL2、VTTBR_EL2等EL2寄存器的存在

---


## 示例分析

#### 虚拟机配置概览

假设我们创建一个运行Linux的虚拟机，其内存配置包含两种不同的映射模式：

**内存区域规划**：
- **指定映射区域**：512MB内存，虚拟机期望的物理地址从0x40000000开始
- **恒等映射区域**：128MB内存，地址由axvisor动态分配，GPA等于HPA

#### 核心建立流程

**阶段1：虚拟机对象创建**
axvisor首先创建VM对象，其中最重要的步骤是立即建立空的Stage-2地址空间。这个地址空间覆盖完整的2TB虚拟地址范围，为后续的内存映射做准备。此时地址空间是空的，没有任何实际的GPA→HPA映射。

**阶段2：内存分配和映射建立**
这是整个过程中最关键的阶段，涉及两种不同的映射策略：

1. **指定映射模式处理**：
   - axvisor首先从系统分配器获取512MB的连续物理内存
   - 假设分配到的物理地址为0x20000000-0x3FFFFFFF
   - 虚拟机配置期望这些内存位于GPA 0x40000000-0x5FFFFFFF
   - axvisor建立映射关系：GPA 0x40000000 → HPA 0x20000000
   - 计算出固定偏移量：0x40000000 - 0x20000000 = 0x20000000
   - 这个偏移量被存储在Linear Backend中，用于后续地址转换

2. **恒等映射模式处理**：
   - axvisor分配128MB物理内存，假设地址为0x10000000-0x1FFFFFFF
   - 由于采用恒等映射，直接将HPA作为GPA使用
   - 建立映射关系：GPA 0x10000000 → HPA 0x10000000
   - 偏移量为0，意味着GPA和HPA数值相等

**阶段3：Stage-2页表激活**
vCPU设置过程将Stage-2页表根地址加载到硬件寄存器VTTBR_EL2中，并通过HCR_EL2启用Stage-2翻译。此时虚拟机的内存管理架构完全就绪。

#### 地址翻译机制详解

**指定映射区域的地址翻译**：
当虚拟机访问0x48000000地址时：
1. Guest OS内部的Stage-1翻译将GVA转换为GPA 0x48000000
2. 硬件自动启动Stage-2翻译，查询axvisor的Stage-2页表
3. axvisor使用Linear Backend计算：HPA = GPA - offset = 0x48000000 - 0x20000000 = 0x28000000
4. 最终访问物理地址0x28000000

**恒等映射区域的地址翻译**：
当虚拟机访问0x18000000地址时：
1. Stage-1翻译产生GPA 0x18000000
2. Stage-2翻译中，由于offset=0，直接得到HPA 0x18000000
3. GPA和HPA数值相同，故称为"恒等映射"

---
