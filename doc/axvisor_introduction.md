# AxVisor 综合介绍文档

## 概述

AxVisor 是一个基于 ArceOS 内核实现的统一组件化 Type I 类型的虚拟机管理程序（Hypervisor）。该项目旨在利用 ArceOS 提供的基础操作系统功能，实现一个轻量级、高性能、跨架构的虚拟化解决方案。

### 核心特性

**统一架构**：
- 使用同一套代码同时支持三种架构：x86_64、Arm (aarch64) 和 RISC-V
- 最大化复用架构无关代码，简化开发和维护成本
- 支持从虚拟化环境到实际物理设备的广泛部署

**组件化设计**：
- Hypervisor 功能被分解为多个可独立使用的组件
- 组件间通过标准接口通信，实现功能解耦和复用
- 五层软件架构，每层都是独立组件，便于扩展和维护

## 技术架构

### 软件架构层次

```
┌─────────────────────────────────────────────┐
│             应用层 (Applications)            │
├─────────────────────────────────────────────┤
│           客户机操作系统 (Guest OS)          │
│  ArceOS │ Linux │ Starry-OS │ NimbOS        │
├─────────────────────────────────────────────┤
│          AxVisor Hypervisor 层              │
│  VM管理 │ VCPU调度 │ 内存管理 │ 设备虚拟化  │
├─────────────────────────────────────────────┤
│            ArceOS 内核层                    │
│  任务调度 │ 内存分配 │ 驱动框架 │ 中断处理    │
├─────────────────────────────────────────────┤
│          硬件抽象层 (HAL)                   │
│   x86_64   │   aarch64   │   riscv64       │
└─────────────────────────────────────────────┘
```

### 技术栈

- **编程语言**：Rust (内存安全、高性能)
- **基础内核**：ArceOS (组件化操作系统)
- **构建工具**：Cargo + xtask 工具链
- **配置格式**：TOML (分层配置系统)
- **许可证**：Apache-2.0、MulanPubL-2.0、MulanPSL2、GPL-3.0

## 支持平台

### 硬件平台

| 平台名称 | 架构支持 | 主要特点 | 适用场景 |
|---------|---------|---------|---------|
| **QEMU** | ARM64, x86_64 | 虚拟化平台，支持多架构 | 开发、测试、CI/CD |
| **Orange Pi 5 Plus** | ARM64 | Rockchip RK3588，高性能 | 嵌入式开发、原型验证 |
| **飞腾派** | ARM64 | 飞腾 E2000Q，国产平台 | 国产化替代、工业应用 |
| **ROC-RK3568-PC** | ARM64 | Rockchip RK3568，工业级 | 工业控制、边缘计算 |
| **EVM3588** | ARM64 | Rockchip RK3588，企业级 | 企业应用、服务器虚拟化 |

### 客户机系统

| 客户机系统 | 系统类型 | 架构支持 | 特点描述 | 适用场景 |
|-----------|---------|---------|---------|---------|
| **ArceOS** | Unikernel | ARM64, x86_64, RISC-V | Rust组件化OS，轻量级高性能 | 云原生、微服务 |
| **Starry-OS** | 宏内核OS | ARM64, x86_64 | 嵌入式实时操作系统 | 实时控制、物联网 |
| **NimbOS** | RTOS系统 | ARM64, x86_64, RISC-V | 类Unix系统，支持POSIX | 系统开发、教学 |
| **Linux** | 宏内核OS | ARM64, x86_64, RISC-V | 成熟稳定，生态丰富 | 通用计算、企业应用 |

## 核心组件

### 1. 虚拟机管理器 (VMM)

负责虚拟机的生命周期管理，包括创建、启动、暂停、恢复、停止和删除。

**主要功能**：
- 虚拟机状态管理（Loading → Loaded → Running → Suspended → Stopping → Stopped）
- VCPU 任务调度和管理
- 内存区域分配和映射
- 设备虚拟化和直通
- 中断虚拟化

**状态转换**：
```
Loading → Loaded → Running
    ↑         ↓         ↓
    └─────────┴─── Suspended
                  ↓
              Stopping → Stopped
```

### 2. 内存管理器

基于 ArceOS 的内存管理机制，提供分层内存分配和管理。

**内存分配器类型**：
- **Buddy 分配器**：处理大块内存分配
- **Slab 分配器**：处理小对象高效分配
- **TLSF 分配器**：实时系统友好的两级分离适配算法
- **Bitmap 分配器**：位图管理的简单分配器

**内存映射模式**：
- `MAP_Alloc`：由 host 负责随机分配内存
- `MAP_Identical`：1:1 映射，起始地址随机
- `MAP_Reserved`：完全 1:1 映射保留内存

### 3. 设备管理

支持设备虚拟化，提供灵活的设备访问控制。

**配置方式**：
- 基于设备树 (FDT) 的自动设备发现
- TOML 配置文件的灵活设备配置
- 动态设备树生成和更新

### 4. Shell 交互界面

提供功能丰富的命令行界面，支持交互式虚拟机管理。

**核心功能**：
- 交互式 Shell 界面
- 命令历史记录和导航
- 虚拟机生命周期管理命令
- 文件系统操作命令（可选）
- 系统信息查询

**主要命令**：
```bash
# 虚拟机管理
vm list                    # 列出所有虚拟机
vm create config.toml      # 创建虚拟机
vm start [vm_id]           # 启动虚拟机
vm stop [vm_id]            # 停止虚拟机
vm show [vm_id]            # 显示虚拟机详情

# 系统操作
help                       # 显示帮助
clear                      # 清屏
uname -a                   # 系统信息
```

## 项目结构

```
axvisor/
├── kernel/                 # 内核核心代码
│   ├── main.rs            # 主入口
│   ├── vmm/               # 虚拟机管理器
│   │   ├── config.rs      # 配置管理
│   │   ├── vcpus.rs       # VCPU 管理
│   │   ├── vm_list.rs     # VM 列表管理
│   │   └── fdt/           # 设备树处理
│   ├── shell/             # Shell 交互界面
│   └── hal/               # 硬件抽象层
├── modules/               # 功能模块
│   ├── axalloc/           # 内存分配器
│   ├── axconfig/          # 配置管理
│   ├── axruntime/         # 运行时环境
│   └── driver/            # 驱动程序
├── platform/              # 平台相关代码
│   └── x86-qemu-q35/      # x86 平台支持
├── configs/               # 配置文件
│   ├── board/             # 硬件平台配置
│   └── vms/               # 客户机配置
├── crates/                # 核心功能 crate
├── scripts/               # 构建脚本
├── doc/                   # 文档
└── xtask/                 # 构建工具
```

## 配置系统

### 分层配置架构

AxVisor 采用分层配置系统，包含硬件平台配置和客户机配置：

```
.axvisor/
├── configs/
│   ├── board/                    # 硬件平台配置
│   │   ├── qemu-aarch64.toml     # QEMU aarch64 平台
│   │   ├── qemu-x86_64.toml      # QEMU x86_64 平台
│   │   └── orangepi-5-plus.toml  # Orange Pi 5 Plus
│   └── vms/                      # 客户机配置
│       ├── linux-aarch64-*.toml  # Linux 客户机配置
│       ├── arceos-aarch64-*.toml # ArceOS 客户机配置
│       └── nimbos-aarch64-*.toml # NimbOS 客户机配置
```

### 配置文件格式

**硬件平台配置示例**：
```toml
# configs/board/qemu-aarch64.toml
[arch]
name = "aarch64"
smp = 4

[memory]
size = "8G"

[features]
fs = true
log = "info"
```

**客户机配置示例**：
```toml
# configs/vms/linux-aarch64-qemu-smp1.toml
[base]
id = 1
name = "linux-vm"
vm_type = 1
cpu_num = 1
phys_cpu_ids = [0]

[kernel]
entry_point = 0x8020_0000
image_location = "memory"
kernel_path = "tmp/Image"
kernel_load_addr = 0x8020_0000
dtb_load_addr = 0x8000_0000

memory_regions = [
  [0x8000_0000, 0x1000_0000, 0x7, 1], # System RAM 1G MAP_IDENTICAL
]

[devices]
passthrough_devices = [
  ["/intc"],
  ["/timer"],
]
```

## 构建和部署

### 构建环境要求

**基础工具**：
```bash
# Ubuntu/Debian
sudo apt-get install libssl-dev gcc libudev-dev pkg-config

# CentOS/RHEL
sudo yum install openssl-devel gcc libudev-devel pkgconfig
```

**Rust 环境**：
```bash
# 安装 Rust
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh

# 安装 cargo-binutils
cargo install cargo-binutils
```

### 构建流程

1. **生成配置**：
```bash
cargo xtask defconfig <board_name>
```

2. **修改配置**（可选）：
```bash
cargo xtask menuconfig
```

3. **执行构建**：
```bash
cargo xtask build
```

### 运行方式

**方式一：自动启动 VM**
```bash
./axvisor.sh run \
  --plat aarch64-generic \
  --vmconfigs configs/vms/nimbos-aarch64-qemu-smp1.toml
```

**方式二：空 Shell 模式**
```bash
./axvisor.sh run \
  --plat aarch64-generic \
  --features fs,ept-level-4
```

**方式三：完整功能模式**
```bash
./axvisor.sh run \
  --plat aarch64-generic \
  --vmconfigs configs/vms/nimbos-aarch64-qemu-smp1.toml \
  --features fs,ept-level-4 \
  --arceos-args "BUS=mmio,BLK=y,DISK_IMG=disk.img,MEM=8g"
```

## 开发指南

### 添加新平台支持

1. **创建平台目录**：
```bash
mkdir platform/<platform-name>
```

2. **实现平台特定代码**：
- `src/lib.rs`：平台初始化
- `src/boot.rs`：启动代码
- `src/mem.rs`：内存管理
- `linker.lds.S`：链接脚本

3. **添加配置文件**：
```toml
# configs/board/<platform-name>.toml
[arch]
name = "aarch64"
smp = 4
```

### 添加新客户机支持

1. **创建客户机配置**：
```toml
# configs/vms/<os>-<arch>-<board>-smp<x>.toml
[base]
id = 1
name = "<os>-vm"
vm_type = 1
cpu_num = 1
phys_cpu_ids = [0]
```

2. **实现客户机加载器**（如需要）：
```rust
// kernel/src/vmm/images/<os>.rs
pub fn load_os(config: &VmConfig) -> Result<(), Error> {
    // 实现 OS 特定的加载逻辑
}
```

### 添加新 Shell 命令

1. **实现命令处理函数**：
```rust
// kernel/src/shell/command/mycmd.rs
pub fn my_command_handler(cmd: &ParsedCommand) {
    // 命令处理逻辑
}
```

2. **注册命令**：
```rust
// 在 build_command_tree() 中添加
tree.insert(
    "mycmd".to_string(),
    CommandNode::new("My custom command")
        .with_handler(my_command_handler)
        .with_usage("mycmd [OPTIONS] <ARGS>")
);
```

## 性能优化

### 内存优化

- **内存池管理**：预分配内存池，减少运行时分配开销
- **NUMA 感知**：针对 NUMA 架构优化内存分配策略
- **大页支持**：支持 2MB/1GB 大页，减少 TLB Miss

### 调度优化

- **CPU 亲和性**：VCPU 绑定到特定物理 CPU
- **实时调度**：支持实时调度策略
- **负载均衡**：动态负载均衡算法

### I/O 优化

- **零拷贝**：支持零拷贝 I/O 操作
- **异步 I/O**：异步 I/O 处理框架
- **设备直通**：高性能设备直通支持

## 故障排除

### 常见问题

**1. VM 启动失败**
```bash
# 检查配置文件
cargo xtask menuconfig

# 查看日志
axvisor:/$ log debug

# 检查设备树
axvisor:/$ vm show -c <vm_id>
```

**2. 内存不足**
```bash
# 检查内存分配
axvisor:/$ vm show -s <vm_id>

# 调整内存配置
# 编辑 configs/vms/*.toml 中的 memory_regions
```

**3. 设备问题**
```bash
# 检查设备直通配置
axvisor:/$ vm show -c <vm_id>

# 查看设备树
cat /proc/device-tree/
```

### 调试技巧

**1. 启用详细日志**：
```bash
axvisor:/$ log trace
```

**2. 使用 GDB 调试**：
```bash
# 启动时添加调试参数
./axvisor.sh run --gdb
```

**3. 内存分析**：
```bash
# 检查内存使用情况
axvisor:/$ vm show -s <vm_id>

# 查看内存分配器状态
cat /proc/axalloc/stats
```

## 社区和贡献

### 贡献指南

1. **Fork 仓库**：在 GitHub 上 fork 项目
2. **创建分支**：为你的功能创建新分支
3. **提交代码**：遵循项目的代码规范
4. **提交 PR**：创建 Pull Request 并描述改动

### 代码规范

- **Rust 代码**：使用 `rustfmt` 格式化代码
- **提交信息**：使用清晰的提交信息格式
- **文档**：为新功能添加相应文档

### 社区资源

- **GitHub 仓库**：https://github.com/arceos-hypervisor/axvisor
- **文档网站**：https://arceos-hypervisor.github.io/axvisorbook
- **问题反馈**：GitHub Issues
- **讨论交流**：GitHub Discussions

## 路线图

### 短期目标（3-6 个月）

- [ ] 完善 RISC-V 架构支持
- [ ] 增强设备虚拟化功能
- [ ] 优化内存管理性能
- [ ] 完善文档和测试用例

### 中期目标（6-12 个月）

- [ ] 支持更多硬件平台
- [ ] 实现热迁移功能
- [ ] 增强安全性特性
- [ ] 性能基准测试和优化

### 长期目标（1-2 年）

- [ ] 支持容器虚拟化
- [ ] 实现分布式虚拟化管理
- [ ] 云原生集成
- [ ] 企业级功能完善

## 总结

AxVisor 作为一个现代化的 Type I 虚拟化管理程序，通过其统一架构和组件化设计，为用户提供了一个轻量级、高性能、跨架构的虚拟化解决方案。无论是用于嵌入式系统、边缘计算还是云原生应用，AxVisor 都能够提供稳定可靠的虚拟化服务。

随着项目的不断发展和社区的积极参与，AxVisor 将在虚拟化技术领域发挥越来越重要的作用，为开源虚拟化生态贡献力量。
