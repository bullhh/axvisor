# Buddy Allocator Statistics Accuracy Tests

## Overview

This document describes the statistics accuracy verification tests implemented in `kernel/src/allocator_benchmark.rs` under the `stats_accuracy` module.

These tests verify the accuracy and consistency of buddy allocator statistics under various scenarios, including single-threaded operations, multi-threaded concurrency, fragmentation, and stress testing.

## Test Modules

### 1. Single-threaded Statistics Accuracy (`test_single_thread_stats`)

**Purpose**: Verify that statistics remain accurate during sequential allocation and deallocation.

**Test Steps**:
1. Record initial statistics (total, free, used pages)
2. Perform 100 allocations with varying sizes (1-16 pages)
3. Verify statistics after allocation
4. Deallocate all allocated memory
5. Verify statistics after deallocation

**Validation Criteria**:
- `total_pages` remains constant throughout
- `free_pages + used_pages = total_pages` (accounting for fragmentation)
- `free_pages` decreases proportionally to allocations
- `free_pages` recovers to near initial value after deallocation

**Expected Outcome**: All consistency checks pass, with small variance allowed due to internal fragmentation.

### 2. Multi-threaded Statistics Accuracy (`test_multithread_stats`)

**Purpose**: Verify statistics remain accurate under concurrent allocation/deallocation patterns.

**Test Steps**:
1. Record initial statistics
2. Simulate 4 concurrent threads, each performing 50 allocations
3. Total: 200 allocations with varying sizes (1-128 pages)
4. Verify statistics after concurrent allocations
5. Perform random-order deallocation to trigger buddy merging
6. Verify final statistics

**Validation Criteria**:
- Consistency checks after allocation phase
- Consistency checks after deallocation phase
- Free pages recover to near initial value (allowing for fragmentation)

**Expected Outcome**: Statistics remain accurate even with concurrent access patterns and random deallocation order.

### 3. Fragmentation Statistics Accuracy (`test_fragmentation_stats`)

**Purpose**: Test statistics accuracy under high fragmentation scenarios.

**Test Steps**:
1. Record initial statistics
2. Create fragmentation pattern using mixed-size allocations:
   - Pattern: [1, 2, 1, 4, 1, 2, 8, 1, 16, 1] pages
3. Verify statistics in fragmented state
4. Deallocate all allocations to trigger buddy merging
5. Verify statistics after merging

**Validation Criteria**:
- Statistics consistent in fragmented state
- Statistics consistent after merging
- Free pages should be very close to initial after full deallocation
- Buddy merging should be reflected in statistics

**Expected Outcome**: Statistics accurately track memory even under severe fragmentation, and correctly reflect merged blocks.

### 4. Stress Test for Statistics (`test_stress_stats`)

**Purpose**: Verify statistics accuracy under repeated allocation/deallocation cycles.

**Test Steps**:
1. Record initial statistics
2. Run 30 cycles of:
   - 40 allocations (varying sizes 1-8 pages)
   - Verify statistics after allocation
   - Deallocate all
   - Verify statistics after deallocation
3. Verify final statistics

**Validation Criteria**:
- Statistics consistent after every allocation phase
- Statistics consistent after every deallocation phase
- Final free pages close to initial value

**Expected Outcome**: Statistics remain accurate across many allocation/deallocation cycles.

## Statistics Verification Function

All tests use the `validate_stats_consistency` helper function:

```rust
fn validate_stats_consistency(
    total_pages: usize,
    free_pages: usize,
    used_pages: usize,
    test_name: &str,
) -> bool
```

**Validation Logic**:
1. Calculate expected used pages: `calculated_used = total_pages - free_pages`
2. Compare with reported used pages
3. Report detailed information if mismatch detected
4. Return true if consistent, false otherwise

**Note on Fragmentation**:
The buddy system's `used_pages` represents pages marked as in-use, not the exact memory allocated. Due to internal fragmentation (allocating 2 pages when only 1 is requested), `used_pages` may be higher than the actual application memory usage. This is expected behavior.

## Test Output Format

Each test provides detailed output:

```
测试 1: 单线程统计准确性 (Single-threaded Stats Accuracy)
  初始状态:
    总页面: 4096
    空闲页面: 4096
    已用页面: 0
  验证统计一致性:
    ✓ 初始状态 统计一致性通过
  分配 100 次...
  分配后状态:
    总页面: 4096
    空闲页面: 3996 (减少: 100)
    已用页面: 100 (增加: 100)
  验证统计一致性:
    ✓ 分配后 统计一致性通过
  ...
```

## Integration

These tests are integrated into the main benchmark suite in `run_comprehensive_tests()`:

```rust
pub fn run_comprehensive_tests() {
    // ...

    all_passed &= basic_tests::run_all(&metrics);
    all_passed &= performance_tests::run_all(&metrics);
    all_passed &= stress_tests::run_all(&metrics);
    all_passed &= multithread_tests::run_all(&metrics);
    all_passed &= leak_detection::run_all(&metrics);
    all_passed &= stats_accuracy::run_all(&metrics); // <- New tests
    // ...
}
```

## Performance Impact

These tests add minimal overhead:
- No additional allocations (they use the existing allocator)
- Statistics are already collected by the allocator (via `tracking` feature)
- Tests simply read and verify existing statistics
- Time complexity: O(n) where n is number of allocation/deallocation operations

## Running the Tests

The tests run automatically as part of `run_comprehensive_tests()`:

```bash
cargo xtask build
# or
./target/aarch64-unknown-none-softfloat/debug/axvisor
```

## Interpretation of Results

### All Tests Pass

If all tests pass with output like:
```
✓ 统计准确性测试全部通过
```

This indicates:
- Statistics tracking is accurate
- Free/used page counts are consistent
- Buddy merging is correctly reflected in statistics

### Test Failures

Possible failure scenarios:

1. **Inconsistency Detected**
   - Symptom: `✗ XXX 统计不一致`
   - Cause: Bug in stats update logic
   - Action: Review `update_stats()` implementation

2. **Free Pages Not Recovered**
   - Symptom: `⚠ 空闲页面未完全恢复: XXX (初始: XXX)`
   - Cause: Memory leak or deallocation bug
   - Action: Review deallocation logic, check for missed deallocations

3. **Fragmentation Impact**
   - Symptom: Free pages significantly different from initial
   - Cause: High internal fragmentation (expected in buddy systems)
   - Action: This is normal; allow small variance in tests

## Conclusion

These statistics accuracy tests provide comprehensive verification of the buddy allocator's tracking mechanisms under various realistic scenarios. They help ensure that:

1. Statistics are always internally consistent
2. Statistics accurately reflect memory state
3. Buddy merging is properly tracked
4. No memory leaks go undetected

Regular execution of these tests during development helps maintain allocator correctness and provides early detection of regression bugs.
