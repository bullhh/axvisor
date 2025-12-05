# axvisor 启动流程详细分析

## 目录
1. [概述](#概述)
2. [多架构支持](#多架构支持)
3. [ARM64基础概念](#arm64基础概念)
4. [启动流程概览](#启动流程概览)
5. [详细启动流程](#详细启动流程)
   - [5.1 U-Boot到Loader阶段](#51-uboot到loader阶段)
   - [5.2 Loader阶段初始化](#52-loader阶段初始化)
   - [5.3 SomeHAL层初始化](#53-somehal层初始化)
   - [5.4 Axvisor主体启动](#54-axvisor主体启动)
6. [架构特定的启动差异](#架构特定的启动差异)
7. [总结](#总结)

## 概述

axvisor是一个跨架构的Type-1 Hypervisor，支持ARM64、x86_64等多种架构。其启动流程是一个精心设计的多阶段过程，从引导加载器开始，经过多个抽象层的初始化，最终建立起完整的虚拟化环境。

### 技术架构特点

1. **跨架构支持**：统一的上层接口，架构特定的底层实现
2. **分层抽象设计**：从硬件抽象层到虚拟化管理层的清晰分层
3. **两阶段MMU启用**：先在物理地址运行建立基础设施，再启用MMU进入虚拟地址空间
4. **硬件虚拟化支持**：充分利用各架构的虚拟化扩展
5. **动态设备发现**：通过设备树(ARM64)或ACPI(x86_64)动态获取硬件配置信息

### 关键技术概念

- **异常级别/特权级别**：ARM64的EL0-EL3，x86的Ring0-3
- **虚拟地址空间**：不同架构的地址空间布局
- **页表管理**：各架构的页表结构和大页支持
- **硬件虚拟化扩展**：ARM64的EL2，x86的VMX

## 多架构支持

axvisor通过条件编译和架构抽象层实现跨架构支持：

### 支持的架构

| 架构 | 虚拟化扩展 | 引导方式 | 设备描述 | 当前状态 |
|------|-------------|----------|----------|----------|
| ARM64 | EL2/Virtualization | U-Boot + DTB | Device Tree | 完整支持 |
| x86_64 | Intel VT-x/AMD-V | BIOS/UEFI + GRUB | ACPI | 部分支持 |
| RISC-V | H扩展 | U-Boot + DTB | Device Tree | 计划支持 |
| LoongArch | Hypervisor扩展 | U-Boot + DTB | Device Tree | 计划支持 |

### 架构抽象层设计

**调用链关系：**
```
上层通用代码 → axplat层 → arch层 → 硬件特定实现
     ↓            ↓          ↓           ↓
统一接口   → 平台抽象 → 架构适配 → 硬件访问
```

**代码组织：**
```rust
// axvisor/kernel/src/hal/mod.rs
#[cfg_attr(target_arch = "aarch64", path = "arch/aarch64/mod.rs")]
#[cfg_attr(target_arch = "x86_64", path = "arch/x86_64/mod.rs")]
// #[cfg_attr(target_arch = "riscv64", path = "arch/riscv/mod.rs")] // 未来支持
pub mod arch;
```

### 统一的HAL接口

```rust
// 硬件检查接口
pub fn hardware_check();  // 各架构实现不同的检查逻辑

// 中断注入接口  
pub fn inject_interrupt(vector: usize);  // 架构特定的中断机制

// 缓存操作接口
pub enum CacheOp { Clean, Invalidate, CleanAndInvalidate }
```

## ARM64基础概念

本文档以ARM64架构为主要示例，因为其虚拟化支持最为完整。其他架构的启动流程类似，但实现细节有所不同。

### 异常级别(Exception Levels)

ARM64架构定义了四个异常级别，提供不同的权限隔离：

```
EL3 - Secure Monitor (安全监控)
│
EL2 - Hypervisor (虚拟化管理器) ← axvisor运行在此级别
│
EL1 - OS Kernel (操作系统内核)
│
EL0 - Applications (应用程序)
```

**关键特性：**
- **EL2**：提供虚拟化扩展，支持Stage-2页表转换
- **TTBR0/TTBR1**：分别指向用户空间和内核空间的页表
- **VTCR/HTCR**：虚拟化和hypervisor翻译控制寄存器

### 地址空间布局

ARM64使用48位虚拟地址，地址空间布局如下：

```
0x0000_0000_0000_0000 ──┐
                        ├── TTBR0: 用户空间/低地址空间 (256TB)
0x0000_ffff_ffff_ffff ──┘
                        └─── 内核空洞 ────┐
0xffff_0000_0000_0000 ──┐                ├── TTBR1: 内核空间 (256TB)
                        └─────────────────┘
0xffff_ffff_ffff_ffff ──┘
```

## 启动流程概览

### 整体启动时序

axvisor的启动流程遵循从低到高的抽象层次，逐步建立运行环境：

```
时间轴 →
┌─────────┬────────────────┬─────────────────┬──────────────┬─────────────┐
│ U-Boot │ SomeHAL Loader │ SomeHAL Core   │ Platform     │ Axvisor     │
│ 加载    │ MMU建立        │ 完整初始化     │ 抽象层       │ Hypervisor  │
└─────────┴────────────────┴─────────────────┴──────────────┴─────────────┘
    ↓           ↓              ↓           ↓           ↓
  EL3 →    EL2/EL1 Setup →   MMU Init →  HAL Init →  Hypervisor Init
```

### 关键跳转机制

1. **汇编入口** → **Rust函数**：通过`extern "C"`和`naked_asm!`宏实现
2. **物理地址** → **虚拟地址**：MMU启用后的地址空间切换
3. **EL3** → **EL2/EL1**：通过`eret`指令和状态寄存器配置
4. **Loader** → **HAL Core**：通过函数指针跳转到虚拟入口

### 代码组织结构

```
启动阶段代码组织：
├── somehal/somehal/src/arch/aarch64/mod.rs        # 汇编入口，异常级别设置
├── somehal/loader/pie-boot-loader-aarch64/        # Loader层MMU和初始化
├── somehal/somehal/src/common/entry.rs            # 虚拟入口点，HAL核心
├── axplat-aarch64-dyn/                            # 平台抽象层初始化
└── axvisor/kernel/src/main.rs                     # Hypervisor主函数
```

## 详细启动流程

### 5.1 U-Boot到Loader阶段

#### 5.1.1 入口点：引导加载器传递控制权

**调用链关系：** 
```
U-Boot加载器 → somehal/somehal/src/arch/aarch64/mod.rs::_start
```

**文件位置：** `somehal/somehal/src/arch/aarch64/mod.rs`

```rust
#[unsafe(naked)]
#[unsafe(no_mangle)]
#[unsafe(link_section = ".head.text")]
pub unsafe extern "C" fn _start() -> ! {
    naked_asm!(
        // ARM64标准内核头部 - U-Boot通过这些信息识别内核类型
        "nop",                           // 代码0：保留
        "bl {entry}",                     // 代码1：跳转到实际入口点
        ".quad 0",                       // text_offset: 内核相对于加载地址的偏移
        ".quad __kernel_load_end - _start", // image_size: 内核镜像大小
        ".quad {flags}",                  // flags: 内核属性标志
        // ... 其他标准头部字段
    )
}
```

**关键原理和基础知识：**

1. **ARM64内核规范**：遵循Linux内核镜像标准，U-Boot通过解析头部信息了解内核属性
2. **链接脚本控制**：`.head.text`段确保代码位于镜像开头，便于U-Boot识别
3. **标志位含义**：
   - `FLAG_LE (0b0)`：小端序模式
   - `FLAG_PAGE_SIZE_4K (0b10)`：4KB页面大小支持
   - `FLAG_ANY_MEM (0b1000)`：支持任意内存地址加载

**宏和汇编机制：**
- `naked_asm!`宏：Rust内联汇编，不生成函数序言/结语
- `naked`属性：禁用Rust函数生成栈帧等标准代码
- `link_section`：将代码放置到特定段

#### 5.1.2 启动参数保存

**调用链关系：**
```
_start → primary_entry → preserve_boot_args → switch_sp
```

**文件位置：** `somehal/somehal/src/arch/aarch64/mod.rs` - `preserve_boot_args`

```rust
#[start_code(naked)]
fn preserve_boot_args() {
    naked_asm!(
    adr_l!(x8, "{boot_args}"), // 获取BOOT_ARGS结构体地址
    "
	stp	x0,  x1, [x8]			// 保存U-Boot传递的启动参数
	stp	x2,  x3, [x8, #16]

    LDR  x0,  ={virt_entry}        // 设置虚拟入口点地址
    str  x0,  [x8, {args_of_entry_vma}]",

    adr_l!(x0, "_start"),          // 获取_start符号的物理地址
    "str x0,  [x8, {args_of_kimage_addr_lma}]", // 保存LMA

    LDR  x0,  =_start              // 获取_start符号的虚拟地址
    str x0,  [x8, {args_of_kimage_addr_vma}]",  // 保存VMA

    adr_l!(x0, "__cpu0_stack_top"), // 获取栈顶物理地址
    "str x0,  [x8, {args_of_stack_top_lma}]",

    LDR x0,  =__cpu0_stack_top    // 获取栈顶虚拟地址
    str x0,  [x8, {args_of_stack_top_vma}]",
    
    adr_l!(x0, "__kernel_code_end"), // 获取内核代码结束地址
    "str x0,  [x8, {args_of_kcode_end}]"

    mov x0, {el_value}              // 设置目标异常级别
    str x0,  [x8, {args_of_el}]

    LDR x0, ={kliner_offset}        // 设置线性映射偏移量
    str x0,  [x8, {args_of_kliner_offset}]
    ",)
}
```

**函数作用和参数获取机制：**

该函数的核心作用是**获取并保存系统启动所需的关键参数到BOOT_ARGS结构体中**，为后续的MMU建立和地址空间切换提供必要信息。

**参数来源和获取方式：**

1. **U-Boot传递的参数（寄存器来源）**：
   - `x0`：设备树指针（FDT地址）- U-Boot加载器传递给内核的硬件配置信息
   - `x1-x3`：保留参数 - ARM64启动协议规定但当前未使用
   - **获取方式**：U-Boot通过ARM64标准启动协议，在跳转到内核入口时将参数放入寄存器

2. **地址和偏移信息（符号解析）**：
   - `kimage_addr_lma`：通过`adr_l!(x0, "_start")`获取_start符号的**物理地址**
   - `kimage_addr_vma`：通过`LDR x0, =_start`获取_start符号的**虚拟地址**
   - **获取原理**：`adr_l!`宏计算当前位置相关地址（物理），`LDR`加载链接时确定的虚拟地址

3. **栈地址信息（内存布局）**：
   - `stack_top_lma/vma`：通过符号`__cpu0_stack_top`获取栈顶的物理和虚拟地址
   - **获取方式**：汇编时确定，链接脚本中定义的栈区域符号

4. **运行时配置（编译时确定）**：
   - `el`：异常级别 - 通过编译时特性`hv`控制（EL1或EL2）
   - `kliner_offset`：线性映射偏移 - 来自`kdef_pgtable`库的`KLINER_OFFSET`
   - **获取方式**：编译时常量，直接嵌入到代码中

**BOOT_ARGS结构体：**
```rust
// 在somehal/somehal/src/lib.rs中定义
#[unsafe(link_section = ".data")]
static mut BOOT_ARGS: EarlyBootArgs = EarlyBootArgs::new();
```

**地址转换关键公式：**
```
虚拟地址 = 物理地址 + kliner_offset
kcode_offset = VMA - LMA
```

### 5.2 Loader阶段初始化

**调用链关系：**
```
preserve_boot_args → primary_entry → somehal/loader/pie-boot-loader-aarch64/src/main.rs::switch_to_target_el
```

#### 5.2.1 异常级别切换

**文件位置：** `somehal/loader/pie-boot-loader-aarch64/src/main.rs` - `switch_to_target_el`

```rust
fn switch_to_target_el(bootargs: &EarlyBootArgs) {
    let target_el = bootargs.el;
    let bootargs_ptr = bootargs as *const _ as usize;

    match target_el {
        1 => el1::switch_to_elx(bootargs_ptr),  // 切换到EL1 (OS级别)
        2 => el2::switch_to_elx(bootargs_ptr),  // 切换到EL2 (Hypervisor级别)
        _ => panic!("Unsupported exception level: {}", target_el),
    }
}
```

#### 5.1.3 启动流程跳转控制

**调用链关系：**
```
_start → primary_entry → preserve_boot_args → 跳转到LOADER_BIN
```

**文件位置：** `somehal/somehal/src/arch/aarch64/mod.rs` - `primary_entry`

```rust
#[start_code(naked)]
fn primary_entry() -> ! {
    naked_asm!(
    "
    bl  {preserve_boot_args}",     // 调用参数保存函数
    adr_l!(x0, "{boot_args}"),      // 加载BOOT_ARGS结构体地址
    adr_l!(x8, "{loader}"),         // 加载LOADER_BIN入口地址
    "
    br   x8",                       // 跳转到Loader代码
        preserve_boot_args = sym preserve_boot_args,
        boot_args = sym crate::BOOT_ARGS,
        loader = sym crate::loader::LOADER_BIN,
    )
}
```

**函数作用和控制流程：**

该函数实现从**汇编入口点**到**Loader二进制代码**的关键跳转，是启动流程的重要控制节点。

**LOADER_BIN的获取机制：**

对于axvisor来说，`loader.bin`是一个**位置无关的引导加载器（PIE Boot Loader）**，它负责在启动早期建立MMU并完成从物理地址到虚拟地址的跳转。

1. **LOADER_BIN定义**（`somehal/somehal/src/loader.rs`）：
```rust
macro_rules! loader_bin_slice {
    () => {
        include_bytes!(concat!(env!("OUT_DIR"), "/loader.bin"))  // 编译时嵌入
    };
}

const LOADER_BIN_LEN: usize = loader_bin_slice!().len();

#[unsafe(link_section = ".boot_loader")]
pub static LOADER_BIN: [u8; LOADER_BIN_LEN] = loader_bin();  // 静态二进制数据
```

2. **LOADER_BIN的双重获取机制**：

   **方式一：优先从Release下载**
   - **检测版本**：构建脚本读取`somehal/Cargo.toml`中的`pie-boot-loader-aarch64`依赖版本
   - **远程下载**：从GitHub/Gitee仓库的对应Release下载预编译的`pie-boot-loader-aarch64.bin`
   - **下载目标**：`{OUT_DIR}/loader.bin`
   - **备用机制**：下载失败时自动回退到本地构建

   **方式二：本地构建回退**
   - **依赖声明**：`pie-boot-loader-aarch64 = {path = "../loader/pie-boot-loader-aarch64", version = "0.3" }`
   - **构建命令**：
     ```bash
     export RUSTFLAGS="-C relocation-model=pic -Clink-args=-pie"
     cargo build -p pie-boot-loader-aarch64 --target aarch64-unknown-none-softfloat --release
     rust-objcopy --strip-all -O binary target/.../pie-boot-loader-aarch64 target/.../loader.bin
     ```
   - **PIE特性**：位置无关可执行文件，可在任意地址加载运行

3. **构建脚本执行流程**（`somehal/somehal/build.rs`）：
```rust
fn aarch64_set_loader() {
    // 1. 尝试从Release下载预编译版本
    let download_success = download_latest_release();
    
    if !download_success {
        // 2. 下载失败时进行本地构建
        build_loader_locally();
    }
}
```

4. **LOADER_BIN的身份和入口**：
   - **身份**：`pie-boot-loader-aarch64`是一个独立的Rust crate，专门用于ARM64的早期启动
   - **入口点**：`_start`函数（文件：`somehal/loader/pie-boot-loader-aarch64/src/main.rs`）
   - **第一条指令**：`mov x19, x0` - 保存bootargs参数指针到x19寄存器
   - **功能**：负责异常级别切换、MMU建立、页表映射、地址空间转换等底层操作
   - **位置**：位于`somehal/loader/pie-boot-loader-aarch64/`目录，是axvisor项目的一部分但独立构建
   - **输出**：编译后生成纯二进制文件（无ELF头部），可直接在裸机上运行

5. **地址解析和入口执行**：
   - `adr_l!(x8, "{loader}")`：获取LOADER_BIN符号的**运行时地址**
   - `loader = sym crate::loader::LOADER_BIN`：编译时符号解析，将符号名映射到实际地址
   - **跳转执行**：`br x8`指令直接跳转到LOADER_BIN的入口点
   - **Loader第一条指令**：`mov x19, x0` - 立即保存传入的bootargs参数指针

**Loader入口详细实现**（`pie-boot-loader-aarch64/src/main.rs`）：
```rust
#[unsafe(naked)]
#[unsafe(no_mangle)]
#[unsafe(link_section = ".text.init")]
unsafe extern "C" fn _start(_args: &EarlyBootArgs) -> ! {
    naked_asm!(
        "
        mov   x19, x0              // ← 第一条指令！保存bootargs指针
        ldr   x8, [x0, {args_of_stack_top_lma}]  // 设置栈指针
        mov   sp, x8

        mov   x0, x19              // 恢复bootargs指针
        BL    {switch_to_target_el}", // 异常级别切换

        "mov   x0, x19",           // 再次恢复bootargs指针
        "BL     {entry}",           // 进入主要初始化逻辑
        // ... 后续MMU建立和虚拟地址跳转
    )
}
```

**关键技术特点**：
- **解耦设计**：Loader作为独立模块，便于维护和更新
- **双重保障**：Release下载优先，本地构建备用
- **位置无关**：PIE特性确保可在任意内存地址运行
- **自动获取**：构建脚本自动处理依赖，无需手动干预

**跳转流程详解：**

```
_start (somehal/somehal)
  ↓ (ARM64内核头部跳转)
primary_entry
  ↓ bl {preserve_boot_args}     // 保存启动参数
  ↓ adr_l!(x0, "{boot_args}")  // x0 = BOOT_ARGS地址
  ↓ adr_l!(x8, "{loader}")     // x8 = LOADER_BIN入口
  ↓ br x8                       // 间接跳转到Loader

_start (pie-boot-loader-aarch64)  ← Loader的第一条指令
  ↓ mov x19, x0                 // 保存bootargs指针到x19寄存器
  ↓ 设置栈指针                   // 从bootargs.stack_top_lma加载栈地址
  ↓ BL {switch_to_target_el}     // 切换到目标异常级别
  ↓ BL {entry}                   // 进入主要初始化流程
  → MMU建立 → 虚拟地址空间
```

**关键技术特点：**

1. **静态嵌入**：Loader代码编译时嵌入，无外部依赖
2. **间接跳转**：通过寄存器间接跳转，支持地址无关代码
3. **参数传递**：通过x0寄存器传递BOOT_ARGS结构体指针
4. **段管理**：专用链接段确保LOADER_BIN的正确定位和访问

**异常级别切换原理：**

ARM64异常级别切换的核心机制是修改**系统控制寄存器**和**异常返回寄存器**，然后执行`eret`指令：

```
当前EL → 设置SPSR_ELx → 设置ELR_ELx → 执行eret → 目标EL
```

- **SPSR_ELx (Saved Program Status Register)**：保存目标状态，包括：
  - **M字段**：指定目标异常级别
  - **D/A/I/F位**：中断屏蔽状态
  - **其他控制位**：endianness、数据访问权限等

- **ELR_ELx (Exception Link Register)**：保存返回地址（入口点）

- **eret指令**：从异常返回，恢复到SPSR和ELR指定的状态

#### 5.2.2 MMU启用前的准备工作

**调用链关系：**
```
switch_to_target_el → somehal/loader/pie-boot-loader-aarch64/src/main.rs::entry → MMU建立
```

**文件位置：** `somehal/loader/pie-boot-loader-aarch64/src/main.rs` - `entry`

```rust
fn entry(bootargs: &EarlyBootArgs) -> *mut () {
    enable_fp();  // 启用浮点运算单元
    
    unsafe {
        clean_bss();                    // 清零BSS段，准备干净的全局变量
        relocate::apply();              // 应用重定位，处理位置无关代码
        cache::dcache_all(cache::DcacheOp::CleanAndInvalidate);  // 数据缓存维护
        
        // 初始化关键内存管理参数
        OFFSET = bootargs.kimage_addr_vma as usize - bootargs.kimage_addr_lma as usize;
        set_page_size(bootargs.page_size);      // 设置页面大小(4KB)
        ram::init(bootargs.kcode_end as _);     // 初始化内存分配器
        
        // 设置异常向量表 - 处理同步/异步异常
        trap::setup();
        
        // 禁用定时器中断 - 避免启动过程中的干扰
        asm!("msr daifset, #2");       // DAIF.I = 1 (屏蔽IRQ)
        CNTP_CTL_EL0.modify(CNTP_CTL_EL0::IMASK::SET);  // 屏蔽物理定时器
        
        // 处理设备树(FDT) - 获取硬件配置信息
        fdt = save_fdt(fdt as _);
    }
    
    // 建立并启用MMU - 这是启动过程的关键转折点
    enable_mmu(bootargs, fdt);
    
    // MMU启用后，返回虚拟地址空间中的入口点
    let jump = bootargs.virt_entry;
    jump
}
```

**准备工作的原理和重要性：**

1. **BSS段清理**：
   - **BSS段**：包含未初始化的全局变量和静态变量
   - **必要性**：确保变量初始值为0，避免随机数据影响程序逻辑

2. **代码重定位**：
   - **位置无关代码(PIC)**：代码可以在任意地址运行
   - **重定位过程**：将符号引用修正到实际加载地址

3. **缓存管理**：
   - **Clean**：将脏数据写回内存
   - **Invalidate**：使缓存失效
   - **必要性**：确保MMU启用前后内存一致性

### 5.3 SomeHAL层初始化

#### 5.3.1 虚拟入口点

**调用链关系：**
```
Loader阶段entry → enable_mmu → 跳转到虚拟地址 → somehal/somehal/src/common/entry.rs::virt_entry
```

此时系统已经进入虚拟地址空间运行，这是MMU启用后的第一个Rust函数。

**文件位置：** `somehal/somehal/src/common/entry.rs` - `virt_entry`

```rust
pub fn virt_entry(args: &BootInfo) {
    // 此时已在虚拟地址空间运行，但需要再次清理BSS
    common::mem::clean_bss();          // 确保虚拟地址空间的BSS已清理
    
    // 全局初始化启动信息，供后续模块使用
    BOOT_INFO.init(args.clone());
    
    // 初始化调试输出 - 此时可以使用虚拟地址访问串口
    common::fdt::init_debugcon(boot_info().fdt);
    println!("SomeHAL booting...");
    
    // 设置异常向量表 - 虚拟地址空间的异常处理
    setup_exception_vectors();
    
    // 电源管理初始化 - 基于设备树的硬件配置
    power::init_by_fdt(boot_info().fdt);
    
    // 平台特定信息设置
    common::fdt::setup_plat_info();
    
    // 完善内存区域管理 - 基于设备树发现的内存信息
    common::mem::init_regions(&args.memory_regions);

    unsafe {
        // 为每个CPU核心准备栈空间
        BOOT_INFO.edit(|info| info.free_memory_start = common::mem::init_percpu_stack());

        // 更新内存区域信息到全局数据结构
        let (region_ptr, region_len) = common::mem::with_regions(|regions| 
            (regions.as_mut_ptr(), regions.len()));
        let region_slice = core::slice::from_raw_parts_mut(region_ptr, region_len);
        BOOT_INFO.edit(|info| info.memory_regions = region_slice.into());

        // 调用平台初始化主函数 - 跳转到axplat层
        unsafe extern "Rust" {
            fn __pie_boot_main(args: &BootInfo);
        }
        println!("Goto main...");
        __pie_boot_main(&BOOT_INFO);

        // 正常情况下不会执行到这里
        power::shutdown();
    }
}
```

**虚拟入口点的意义：**

1. **地址空间切换**：从物理地址跳转到虚拟地址空间
2. **HAL完善**：在Loader建立的MMU基础上，建立完整的硬件抽象层
3. **平台抽象**：为上层提供统一的硬件接口

### 5.4 Axvisor主体启动

#### 5.4.1 主函数入口

**调用链关系：**
```
virt_entry → __pie_boot_main → axplat-aarch64-dyn启动 → axvisor/kernel/src/main.rs::main
```

**文件位置：** `axvisor/kernel/src/main.rs`

**axplat-aarch64-dyn的角色和作用：**

`axplat-aarch64-dyn`是**ArceOS在AArch64架构下的动态平台实现**，在启动流程中扮演着**硬件抽象层平台实现**的关键角色。它位于Loader层和Hypervisor主函数之间，负责完成平台的底层初始化和硬件抽象。

**1. 平台依赖和集成方式：**

**依赖配置**（`axvisor/modules/axruntime/Cargo.toml`）：
```toml
[target.'cfg(target_arch = "aarch64")'.dependencies]
axplat-aarch64-dyn = {git = "https://github.com/arceos-hypervisor/axplat-aarch64-dyn", 
                       tag = "v0.3.3", features = ["irq", "smp", "hv"]}
somehal = "0.4"
```

**外部声明**（`axvisor/modules/axruntime/src/lib.rs`）：
```rust
#[cfg(target_arch = "aarch64")]
extern crate axplat_aarch64_dyn;
```

**2. 启动流程中的代码路径：**

**Step 1: SomeHAL跳转到axplat入口**

**符号声明和绑定机制：**

```rust
// somehal/somehal/src/common/entry.rs::virt_entry
unsafe extern "Rust" {
    fn __pie_boot_main(args: &BootInfo);  // 声明外部符号
}
__pie_boot_main(&BOOT_INFO);           // 调用该符号
```

**符号绑定原理：**

这个跳转是通过**编译时符号绑定**和**链接时符号解析**实现的：

1. **宏定义符号名**（`somehal/macros/pie-boot-macros/src/lib.rs`）：
```rust
#[proc_macro_attribute]
pub fn entry(args: TokenStream, input: TokenStream) -> TokenStream {
    entry::entry(args, input, "__pie_boot_main")  // 强制使用__pie_boot_main符号名
}
```

2. **宏展开生成函数**（`pie-boot-macros/src/entry.rs`）：
```rust
pub fn entry(args: TokenStream, input: TokenStream, name: &str) -> TokenStream {
    // name 参数被硬编码为 "__pie_boot_main"
    quote!(
        #[allow(non_snake_case)]
        #[unsafe(no_mangle)]
        pub #unsafety extern "C" fn #name(#args) {  // 生成 __pie_boot_main 函数
            #(#stmts)*
        }
    )
}
```

3. **axplat-aarch64-dyn中的实际实现**：
```rust
// axplat-aarch64-dyn/src/boot.rs
#[somehal::entry]  // ← 宏展开后变成 __pie_boot_main
fn main(args: &BootInfo) -> ! {
    unsafe {
        switch_sp(args);
    }
}
```

**链接时符号解析过程：**

```
编译时：
┌─────────────────────────────────┐
│ somehal/src/common/entry.rs   │ → extern "Rust" fn __pie_boot_main(args: &BootInfo)  [符号声明]
│                              │
│ axplat-aarch64-dyn/src/boot.rs│ → #[somehal::entry] fn main(args: &BootInfo)     [符号定义]
└─────────────────────────────────┘
                ↓ 宏展开
┌─────────────────────────────────┐
│ 生成: #[no_mangle] extern "C" fn __pie_boot_main(args: &BootInfo) { ... }
└─────────────────────────────────┘

链接时：
┌─────────────────────────────────┐
│ Rust链接器解析符号表            │
│ __pie_boot_main符号 → 实际函数地址  │ ← 符号绑定
└─────────────────────────────────┘
```

**关键技术特点：**

1. **宏控制符号名**：`#[entry]`宏强制所有入口函数都叫`__pie_boot_main`
2. **no_mangle属性**：确保符号名不被Rust编译器修改
3. **extern "C"调用约定**：使用C调用约定，确保跨crate兼容
4. **链接时解析**：Rust链接器在链接时将符号引用绑定到实际函数

**实际执行流程：**

```
SomeHAL virt_entry:
  ↓ __pie_boot_main(&BOOT_INFO)     // 调用__pie_boot_main符号
    ↓
axplat-aarch64-dyn main函数        // 符号解析到这个函数的实际地址
  ↓ switch_sp(args)               // 设置栈空间
    ↓ axplat::call_main()          // 进入平台初始化流程
```

**Step 2: axplat-aarch64-dyn的入口函数**
```rust
// axplat-aarch64-dyn/src/boot.rs
#[somehal::entry]                     // 接收SomeHAL传递的控制权
fn main(args: &BootInfo) -> ! {
    unsafe {
        switch_sp(args);               // 设置新的栈空间
    }
}

fn sp_reset(args: &BootInfo) -> ! {
    axplat::call_main(0, args.fdt.map(|p| p.as_ptr() as usize).unwrap_or_default());
}
```

**Step 3: axplat平台初始化流程**
```rust
// axplat库的call_main函数内部流程
axplat::call_main(cpu_id: usize, fdt_addr: usize) → 
    ↓ init_early()                    // 早期初始化
    ↓ axhal::init()                  // HAL初始化
    ↓ init_later()                   // 后期初始化
    ↓ 跳转到axvisor main()
```

**3. 使用的核心代码模块：**

**平台初始化模块**（`axplat-aarch64-dyn/src/init.rs`）：
```rust
impl InitIf for InitIfImpl {
    // 早期初始化：设置控制台、陷阱处理、内存
    fn init_early(_cpu_id: usize, _arg: usize) {
        console::setup_early();         // 控制台初始化
        axcpu::init::init_trap();       // 陷阱处理设置
        crate::mem::setup();           // 内存管理设置
    }

    // 后期初始化：中断、定时器、驱动
    fn init_later(_cpu_id: usize, _arg: usize) {
        somehal::mem::flush_tlb(None); // TLB刷新
        #[cfg(feature = "smp")]
        crate::smp::init();            // 多核初始化
        
        crate::time::enable();          // 时间管理启用
        driver::setup();               // 设备驱动初始化
        
        #[cfg(feature = "irq")]
        {
            crate::irq::init();        // 中断控制器初始化
            crate::irq::init_current_cpu();
            crate::time::enable_irqs(); // 定时器中断启用
        }
    }
}
```

**其他关键模块：**
- **内存管理**（`src/mem.rs`）：物理/虚拟内存管理、MMIO区域映射
- **中断处理**（`src/irq/`）：GICv2/GICv3中断控制器支持
- **多核支持**（`src/smp.rs`）：SMP多核启动和管理
- **时间管理**（`src/time.rs`）：系统定时器和中断
- **控制台**（`src/console.rs`）：串口和调试输出
- **设备树**（`src/fdt.rs`）：硬件配置解析

**4. 特性配置：**

axplat-aarch64-dyn启用了以下关键特性：
- `irq`：中断控制器支持（GICv2/GICv3）
- `smp`：多核处理器支持
- `hv`：EL2虚拟化支持（Hypervisor环境）

**5. 在启动流程中的关键作用：**

1. **硬件抽象桥梁**：将SomeHAL的底层抽象与axvisor的高层需求连接
2. **平台特定初始化**：完成ARM64架构特定的硬件初始化
3. **多核环境建立**：设置SMP多核运行环境
4. **中断系统启用**：建立完整的中断处理机制
5. **设备驱动准备**：初始化必要的平台设备驱动
6. **虚拟化支持**：启用EL2级别的虚拟化扩展

通过axplat-aarch64-dyn，axvisor实现了**跨架构兼容性**，同一套Hypervisor代码可以在不同ARM64硬件平台上运行。

```rust
#![no_std]
#![no_main]

#[macro_use]
extern crate log;
extern crate axstd as std;
extern crate axruntime;
extern crate driver;

mod hal;
mod logo;
mod shell;
mod task;
mod vmm;

#[unsafe(no_mangle)]
fn main() {
    logo::print_logo();

    info!("Starting virtualization...");
    info!("Hardware support: {:?}", axvm::has_hardware_support());
    
    // 启用硬件虚拟化支持
    hal::enable_virtualization();

    // 初始化虚拟机管理器
    vmm::init();
    vmm::start();

    info!("[OK] Default guest initialized");

    // 初始化控制台
    shell::console_init();
}
```

#### 5.4.2 硬件虚拟化启用

**调用链关系：**
```
main → hal::enable_virtualization → 各架构的hardware_enable
```

**文件位置：** `axvisor/kernel/src/hal/mod.rs` - `enable_virtualization`

```rust
pub(crate) fn enable_virtualization() {
    use core::sync::atomic::{AtomicUsize, Ordering};
    use std::thread;
    use arceos::api::task::{AxCpuMask, ax_set_current_affinity};

    static CORES: AtomicUsize = AtomicUsize::new(0);

    info!("Enabling hardware virtualization support on all cores...");

    // 硬件兼容性检查 - 架构特定实现
    hardware_check();

    let cpu_count = axruntime::cpu_count();

    // 在每个CPU核心上启用虚拟化
    for cpu_id in 0..cpu_count {
        thread::spawn(move || {
            info!("Core {cpu_id} is initializing hardware virtualization support...");
            
            // 设置CPU亲和性
            assert!(
                ax_set_current_affinity(AxCpuMask::one_shot(cpu_id)).is_ok(),
                "Initialize CPU affinity failed!"
            );

            info!("Enabling hardware virtualization support on core {cpu_id}");

            // 初始化定时器
            vmm::init_timer_percpu();

            // 初始化每CPU状态
            let percpu = unsafe { AXVM_PER_CPU.current_ref_mut_raw() };
            percpu
                .init(this_cpu_id())
                .expect("Failed to initialize percpu state");
            percpu
                .hardware_enable()
                .expect("Failed to enable virtualization");

            info!("Hardware virtualization support enabled on core {cpu_id}");

            let _ = CORES.fetch_add(1, Ordering::Release);
        });
    }

    // 等待所有核心完成初始化
    info!("Waiting for all cores to enable hardware virtualization...");
    while CORES.load(Ordering::Acquire) != cpu_count {
        thread::yield_now();
    }

    info!("All cores have enabled hardware virtualization support.");
}
```

#### 5.4.3 硬件兼容性检查

**文件位置：** `axvisor/kernel/src/hal/arch/aarch64/mod.rs` - `hardware_check`

```rust
pub fn hardware_check() {
    // 检查物理地址宽度支持
    let pa_bits = match ID_AA64MMFR0_EL1.read_as_enum(ID_AA64MMFR0_EL1::PARange) {
        Some(ID_AA64MMFR0_EL1::PARange::Value::Bits_32) => 32,
        Some(ID_AA64MMFR0_EL1::PARange::Value::Bits_36) => 36,
        Some(ID_AA64MMFR0_EL1::PARange::Value::Bits_40) => 40,
        Some(ID_AA64MMFR0_EL1::PARange::Value::Bits_42) => 42,
        Some(ID_AA64MMFR0_EL1::PARange::Value::Bits_44) => 44,
        Some(ID_AA64MMFR0_EL1::PARange::Value::Bits_48) => 48,
        Some(ID_AA64MMFR0_EL1::PARange::Value::Bits_52) => 52,
        _ => 32,
    };

    // 根据物理地址宽度确定页表级数
    let level = match pa_bits {
        44.. => 4,
        _ => 3,
    };

    // 检查页表级数配置
    #[cfg(feature = "ept-level-4")]
    {
        if level < 4 {
            panic!(
                "4-level EPT feature is enabled, but the hardware only supports {}-level page tables. 
                 Please disable the 4-level EPT feature or use hardware that supports 4-level page tables.",
                level
            );
        }
    }
    #[cfg(not(feature = "ept-level-4"))]
    {
        if level > 3 {
            panic!(
                "The hardware supports {}-level page tables, but the 4-level EPT feature is not enabled. 
                 Please enable the 4-level EPT feature to utilize the hardware's full capabilities.",
                level
            );
        }
    }
}
```

## 架构特定的启动差异

### x86_64架构启动差异

1. **引导方式**：
   - ARM64：U-Boot + 设备树
   - x86_64：BIOS/UEFI + GRUB + ACPI

2. **虚拟化扩展**：
   - ARM64：EL2异常级别 + Stage-2翻译
   - x86_64：Intel VT-x + EPT

3. **页表结构**：
   - ARM64：4级页表，支持大页
   - x86_64：4级页表，支持2MB/1GB大页

4. **异常处理**：
   - ARM64：向量表 + 异常级别
   - x86_64：IDT + 特权级别

**x86_64实现示例：**
```rust
// axvisor/kernel/src/hal/arch/x86_64/mod.rs
pub fn hardware_check() {
    // 检查CPU虚拟化支持
    // TODO: 实现x86_64的硬件检查
}

pub fn inject_interrupt(_vector: u8) {
    // TODO: 实现x86_64的中断注入
}
```

### RISC-V架构启动差异（计划支持）

1. **虚拟化扩展**：H扩展
2. **页表结构**：Sv48/Sv57
3. **异常处理**：异常向量表
4. **设备描述**：设备树

## 总结

### axvisor启动流程的关键特点

1. **跨架构支持**：
   - 统一的上层接口，架构特定的底层实现
   - 条件编译支持多架构代码共存
   - 抽象层隐藏架构差异

2. **分层启动架构**：
   - **Loader阶段**：建立基础MMU，实现物理地址到虚拟地址的跳转
   - **HAL阶段**：完善内存映射，建立完整的硬件抽象层
   - **Hypervisor阶段**：启用虚拟化扩展，建立虚拟机管理框架

3. **硬件虚拟化支持**：
   - **ARM64**：EL2运行级别，双重地址翻译
   - **x86_64**：VT-x扩展，EPT页表
   - **未来架构**：相应的虚拟化扩展支持

4. **灵活的配置**：
   - 基于编译时特性选择功能
   - 支持不同的页表级数配置
   - 动态硬件发现和初始化

### 技术优势

1. **高性能**：充分利用硬件虚拟化扩展
2. **跨平台**：统一接口支持多种架构
3. **模块化**：清晰的分层架构便于维护
4. **可扩展**：易于添加新的架构支持

### 应用场景

axvisor适用于需要跨架构虚拟化的场景：
- **云原生环境**：多架构容器和虚拟机
- **边缘计算**：不同硬件平台的统一虚拟化
- **开发测试**：跨平台的开发和测试环境
- **嵌入式系统**：多架构设备的虚拟化需求

---

*本文档基于axvisor源码分析，涵盖了从系统启动到虚拟化环境建立的完整技术细节。关于MMU和页表的详细内容，请参考《axvisor MMU和页表管理详解》文档。*