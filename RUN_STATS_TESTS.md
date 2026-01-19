# Running Statistics Accuracy Tests

## Overview

This document explains how to run the statistics accuracy tests that have been added to the buddy allocator benchmark suite.

## Test Location

Tests are located in:
- **Code**: `kernel/src/allocator_benchmark.rs` - `stats_accuracy` module
- **Documentation**: `allocator/doc/STATS_ACCURACY_TEST.md`

## Building the Tests

### Prerequisites
Ensure your development environment is properly set up:
```bash
rustup default nightly-2025-05-20
```

### Build Command
```bash
cargo xtask build
```

Or specify a specific board configuration:
```bash
cargo xtask build --config=qemu-aarch64
```

## Running the Tests

The statistics accuracy tests run automatically as part of the comprehensive benchmark suite when Axvisor boots:

```bash
# Run in QEMU
cargo xtask run

# Or with specific configuration
cargo xtask run --config=qemu-aarch64
```

## Test Output

When Axvisor starts, you will see the following output sections:

### 1. Initial Allocator State
```
分配器初始状态:
  已用页面: 0
  可用页面: 6144
  已用字节: 0
  可用字节: 25165824
```

### 2. Statistics Accuracy Test Section
```
═══════════════════════════════════════════════════════════
统计准确性测试 (Statistics Accuracy Tests)
═══════════════════════════════════════════════════════════

测试 1: 单线程统计准确性 (Single-threaded Stats Accuracy)
  初始状态:
    总页面: 6144
    空闲页面: 6144
    已用页面: 0
  验证统计一致性:
    ✓ 初始状态 统计一致性通过
  ...
✓ 统计准确性测试全部通过
```

## Test Details

### Test 1: Single-threaded Statistics Accuracy
- Performs 100 sequential allocations (1-16 pages each)
- Verifies statistics after each allocation
- Deallocates all and verifies recovery
- **Expected**: Pass with small variance due to fragmentation

### Test 2: Multi-threaded Statistics Accuracy
- Simulates 4 concurrent threads
- Each thread performs 50 allocations
- Total: 200 allocations with varying sizes (1-128 pages)
- Random-order deallocation to test buddy merging
- **Expected**: Pass with variance allowed for fragmentation

### Test 3: Fragmentation Statistics Accuracy
- Creates fragmentation pattern: [1, 2, 1, 4, 1, 2, 8, 1, 16, 1] pages
- Verifies statistics in fragmented state
- Tests buddy merging by deallocating all
- **Expected**: Pass, with accurate merging reflected in stats

### Test 4: Statistics Stress Test
- Runs 30 cycles of 40 allocations each
- Verifies statistics after every allocation phase
- Verifies statistics after every deallocation phase
- **Expected**: Pass, consistent statistics throughout

## Expected Results

### All Tests Pass
```
✓ 统计准确性测试全部通过
```

This indicates:
- Statistics tracking is accurate
- Free/used page counts are consistent
- Buddy merging is correctly reflected
- No memory leaks

### Test Failures

#### Inconsistency Detected
```
✗ XXX 统计不一致:
   已用页面: XXX (预期: XXX)
   总页面: XXX, 空闲页面: XXX
```
**Cause**: Bug in `update_stats()` or statistics calculation
**Action**: Review allocator's stats update logic

#### Free Pages Not Recovered
```
⚠ 空闲页面未完全恢复: XXX (初始: XXX)
```
**Cause**: Memory leak or deallocation bug
**Action**: Review deallocation logic

## Performance Impact

The tests add minimal overhead:
- No additional allocations (use existing allocator)
- Statistics already collected by allocator (via `tracking` feature)
- Tests only read and verify existing statistics
- Time complexity: O(n) where n is number of operations

## Troubleshooting

### Build Failures
```
error[E0425]: cannot find value `metrics` in this scope
```
**Solution**: Ensure all test functions receive `metrics: &AllocatorMetrics` parameter

### Link Errors
```
error: linking with `cc` failed
```
**Solution**: Check toolchain and ensure target is installed:
```bash
rustup target add aarch64-unknown-none-softfloat
```

### Runtime Issues
If QEMU fails to start:
1. Check QEMU installation: `qemu-system-aarch64 --version`
2. Ensure memory is sufficient for tests
3. Try with smaller memory regions in configuration

## Continuous Integration

To run tests automatically in CI:

```yaml
# .github/workflows/test.yml
- name: Run Statistics Accuracy Tests
  run: |
    cargo xtask build
    cargo xtask run --config=qemu-aarch64 &
    # Wait for test completion
    timeout 60 bash -c 'until grep "ALL TESTS PASSED" serial.out; do sleep 1; done'
```

## Customization

### Changing Test Parameters

In `kernel/src/allocator_benchmark.rs`, modify these values:

```rust
// Test 1: Change number of allocations
let num_allocs = 100; // Increase to 1000 for more thorough test

// Test 2: Change concurrency
let num_threads = 4;    // Increase to 8 for more stress
let allocs_per_thread = 50; // Increase to 100

// Test 3: Change fragmentation pattern
let fragmentation_pattern = [1, 2, 1, 4, 1, 2, 8, 1, 16, 1];

// Test 4: Change stress cycles
let num_cycles = 30;    // Increase to 100 for longer test
let ops_per_cycle = 40;  // Increase to 100 for more operations
```

### Adding New Tests

To add a new test case:

```rust
fn test_your_scenario(metrics: &AllocatorMetrics) -> bool {
    info!("测试 X: Your Test Name");

    // Your test logic
    // ...
    // Use validate_stats_consistency() for validation
    validate_stats_consistency(total, free, used, "your stage")
}

// Add to run_all():
pub fn run_all(metrics: &AllocatorMetrics) -> bool {
    let mut all_passed = true;
    all_passed &= test_single_thread_stats(metrics);
    all_passed &= test_your_scenario(metrics);  // Add here
    // ...
}
```

## References

- [Allocator Design](allocator/doc/MULTI_ZONE_DESIGN.md)
- [Allocator Implementation](allocator/doc/MULTI_ZONE_IMPLEMENTATION.md)
- [Statistics Analysis](allocator/src/buddy/stats.rs)
