# PAGE_SIZE 配置说明

现在 `PAGE_SIZE` 可以从 `modules/axalloc/src/lib.rs` 中配置并传递到底层 allocator。

## 使用方法

### 1. 修改 `modules/axalloc/src/lib.rs`

在文件顶部找到这一行：

```rust
const PAGE_SIZE: usize = 0x1000;
```

修改为你需要的页大小：

```rust
// 标准页大小 (4KB)
const PAGE_SIZE: usize = 0x1000;

// 大页 (2MB)
const PAGE_SIZE: usize = 0x200000;

// 中等页 (8KB)
const PAGE_SIZE: usize = 0x2000;

// 巨页 (1GB)
const PAGE_SIZE: usize = 0x40000000;
```

### 2. 自动传递

`GlobalAllocator` 会自动使用这个 `PAGE_SIZE` 初始化内部的 allocator：

```rust
pub struct GlobalAllocator {
    inner: axvisor_allocator::GlobalAllocator<PAGE_SIZE>,
    usages: SpinNoIrq<Usages>,
}

impl GlobalAllocator {
    pub const fn new() -> Self {
        Self {
            inner: axvisor_allocator::GlobalAllocator::<PAGE_SIZE>::new(),
            usages: SpinNoIrq::new(Usages::new()),
        }
    }
    // ...
}
```

## 向后兼容

所有底层 allocator 的泛型参数都有默认值 `DEFAULT_PAGE_SIZE` (4KB)，所以：

1. **现有代码无需修改**：如果未指定 `PAGE_SIZE`，会使用默认的 4KB
2. **可以自定义**：通过泛型参数指定不同的页大小
3. **类型安全**：不同的 `PAGE_SIZE` 生成不同的类型，编译时检查

## 实现的泛型类型

以下类型都支持 `PAGE_SIZE` 泛型参数：

1. **`BuddyPageAllocator<const PAGE_SIZE: usize>`**
   - 核心的 buddy 分配器
   - 默认: `DEFAULT_PAGE_SIZE` (4KB)

2. **`BuddySet<const PAGE_SIZE: usize>`**
   - 单个内存区域的 buddy 集合
   - 默认: `DEFAULT_PAGE_SIZE` (4KB)

3. **`CompositePageAllocator<const PAGE_SIZE: usize>`**
   - 组合页分配器，支持连续块组合
   - 默认: `DEFAULT_PAGE_SIZE` (4KB)

4. **`SlabByteAllocator<const PAGE_SIZE: usize>`**
   - Slab 字节分配器
   - 默认: `DEFAULT_PAGE_SIZE` (4KB)

5. **`GlobalAllocator<const PAGE_SIZE: usize>`**
   - 全局分配器，协调页分配和 slab 分配
   - 默认: `DEFAULT_PAGE_SIZE` (4KB)

## 使用示例

### 使用默认页大小 (4KB)

```rust
// 在 modules/axalloc/src/lib.rs
const PAGE_SIZE: usize = 0x1000;  // 4KB

// 自动使用默认配置
let allocator = GlobalAllocator::new();
```

### 使用自定义页大小

```rust
// 在 modules/axalloc/src/lib.rs
const PAGE_SIZE: usize = 0x200000;  // 2MB

// 自动使用自定义配置
let allocator = GlobalAllocator::new();
```

### 直接使用底层 allocator（不推荐）

```rust
use axvisor_allocator::{BuddyPageAllocator, DEFAULT_PAGE_SIZE};

// 使用默认页大小
let allocator = BuddyPageAllocator::new();

// 使用自定义页大小
const MY_PAGE_SIZE: usize = 0x200000;  // 2MB
let allocator = BuddyPageAllocator::<MY_PAGE_SIZE>::new();
```

## 技术细节

### 泛型默认值语法

```rust
pub struct MyStruct<const PAGE_SIZE: usize = { crate::DEFAULT_PAGE_SIZE }> {
    // ...
}
```

注意：默认值必须用花括号包围，这是 Rust 泛型默认值的语法要求。

### Trait 实现

所有相关的 trait 都有泛型实现：

```rust
impl<const PAGE_SIZE: usize> BaseAllocator for BuddyPageAllocator<PAGE_SIZE> { }
impl<const PAGE_SIZE: usize> PageAllocator for BuddyPageAllocator<PAGE_SIZE> {
    const PAGE_SIZE: usize = PAGE_SIZE;
    // ...
}
```

### 编译时检查

不同的 `PAGE_SIZE` 是不同的类型：

```rust
let alloc_4k = GlobalAllocator::<0x1000>::new();      // GlobalAllocator<4096>
let alloc_2m = GlobalAllocator::<0x200000>::new();     // GlobalAllocator<2097152>
// alloc_4k 和 alloc_2m 是完全不同的类型，编译时检查
```

## 注意事项

1. **性能影响**：页大小会影响内存分配的粒度和效率
   - 小页 (4KB)：更灵活，但管理开销大
   - 大页 (2MB)：管理开销小，但可能浪费内存

2. **对齐要求**：分配的内存必须按照 `PAGE_SIZE` 对齐

3. **向后兼容**：所有现有代码无需修改即可继续使用默认的 4KB

4. **编译时确定**：`PAGE_SIZE` 在编译时确定，运行时无法更改

## 测试

要验证配置是否正确，可以检查 `GlobalAllocator` 的 `PAGE_SIZE` 关联常量：

```rust
use axvisor_allocator::GlobalAllocator;

const CUSTOM_PAGE_SIZE: usize = 0x2000;  // 8KB

let allocator = GlobalAllocator::<CUSTOM_PAGE_SIZE>::new();
assert_eq!(<GlobalAllocator<CUSTOM_PAGE_SIZE> as PageAllocator>::PAGE_SIZE, CUSTOM_PAGE_SIZE);
```
