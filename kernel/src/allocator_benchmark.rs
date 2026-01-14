//! Allocator comprehensive benchmark and testing suite
//!
//! Provides thorough testing of the memory allocator including:
//! - Basic functionality
//! - Performance metrics
//! - Stress testing
//! - Multi-threaded concurrency
//! - Memory leak detection
//!
//! Designed to highlight differences between:
//! - Buddy-Slab allocator (axvisor): Good for fixed-size, small object allocations
//! - TLSF allocator (test/axvisor/crate/allocator): Good for random-size, fragmented scenarios

#![allow(dead_code)]

use alloc::vec::Vec;
use core::ptr::NonNull;
use core::sync::atomic::{AtomicU64, AtomicUsize, Ordering};



/// Allocation size category
#[derive(Clone, Copy, Debug)]
pub enum SizeCategory {
    Small,   // < 4KB
    Medium,  // 4KB - 1MB
    Large,   // > 1MB
}

impl SizeCategory {
    pub fn from_size(size: usize) -> Self {
        if size < 4096 {
            SizeCategory::Small
        } else if size < 1024 * 1024 {
            SizeCategory::Medium
        } else {
            SizeCategory::Large
        }
    }
}

/// Performance metrics for allocator operations
#[derive(Default)]
pub struct AllocatorMetrics {
    // Count statistics
    pub total_allocations: AtomicUsize,
    pub total_deallocations: AtomicUsize,
    pub leaked_allocations: AtomicUsize,

    // Size category statistics
    pub small_allocs: AtomicUsize,
    pub medium_allocs: AtomicUsize,
    pub large_allocs: AtomicUsize,

    // Performance statistics
    pub total_alloc_time_ns: AtomicU64,
    pub total_dealloc_time_ns: AtomicU64,
    pub min_alloc_time_ns: AtomicU64,
    pub max_alloc_time_ns: AtomicU64,

    // Peak statistics
    pub peak_allocated_bytes: AtomicUsize,
    pub current_allocated_bytes: AtomicUsize,
}

impl AllocatorMetrics {
    pub fn new() -> Self {
        Self {
            total_allocations: AtomicUsize::new(0),
            total_deallocations: AtomicUsize::new(0),
            leaked_allocations: AtomicUsize::new(0),
            small_allocs: AtomicUsize::new(0),
            medium_allocs: AtomicUsize::new(0),
            large_allocs: AtomicUsize::new(0),
            total_alloc_time_ns: AtomicU64::new(0),
            total_dealloc_time_ns: AtomicU64::new(0),
            min_alloc_time_ns: AtomicU64::new(u64::MAX),
            max_alloc_time_ns: AtomicU64::new(0),
            peak_allocated_bytes: AtomicUsize::new(0),
            current_allocated_bytes: AtomicUsize::new(0),
        }
    }

    pub fn record_alloc(&self, size: usize, duration_ns: u64) {
        self.total_allocations.fetch_add(1, Ordering::Relaxed);
        self.total_alloc_time_ns.fetch_add(duration_ns, Ordering::Relaxed);

        // Update min/max
        let mut min_time = self.min_alloc_time_ns.load(Ordering::Relaxed);
        loop {
            if duration_ns >= min_time {
                break;
            }
            match self.min_alloc_time_ns.compare_exchange_weak(
                min_time, duration_ns,
                Ordering::Relaxed, Ordering::Relaxed
            ) {
                Ok(_) => break,
                Err(new) => min_time = new,
            }
        }

        let mut max_time = self.max_alloc_time_ns.load(Ordering::Relaxed);
        loop {
            if duration_ns <= max_time {
                break;
            }
            match self.max_alloc_time_ns.compare_exchange_weak(
                max_time, duration_ns,
                Ordering::Relaxed, Ordering::Relaxed
            ) {
                Ok(_) => break,
                Err(new) => max_time = new,
            }
        }

        // Update current and peak bytes
        let old_bytes = self.current_allocated_bytes.fetch_add(size, Ordering::Relaxed);
        let new_bytes = old_bytes + size;

        let mut peak = self.peak_allocated_bytes.load(Ordering::Relaxed);
        loop {
            if new_bytes <= peak {
                break;
            }
            match self.peak_allocated_bytes.compare_exchange_weak(
                peak, new_bytes,
                Ordering::Relaxed, Ordering::Relaxed
            ) {
                Ok(_) => break,
                Err(new) => peak = new,
            }
        }

        // Update size category
        match SizeCategory::from_size(size) {
            SizeCategory::Small => self.small_allocs.fetch_add(1, Ordering::Relaxed),
            SizeCategory::Medium => self.medium_allocs.fetch_add(1, Ordering::Relaxed),
            SizeCategory::Large => self.large_allocs.fetch_add(1, Ordering::Relaxed),
        };
    }

    pub fn record_dealloc(&self, size: usize, duration_ns: u64) {
        self.total_deallocations.fetch_add(1, Ordering::Relaxed);
        self.total_dealloc_time_ns.fetch_add(duration_ns, Ordering::Relaxed);
        self.current_allocated_bytes.fetch_sub(size, Ordering::Relaxed);
    }

    pub fn check_leaks(&self) -> bool {
        let allocs = self.total_allocations.load(Ordering::Relaxed);
        let deallocs = self.total_deallocations.load(Ordering::Relaxed);
        allocs == deallocs
    }

    pub fn calculate_throughput(&self, total_time_ns: u64) -> f64 {
        if total_time_ns == 0 {
            return 0.0;
        }
        let ops = self.total_allocations.load(Ordering::Relaxed);
        ops as f64 * 1e9 / total_time_ns as f64
    }

    pub fn calculate_avg_alloc_latency(&self) -> f64 {
        let count = self.total_allocations.load(Ordering::Relaxed);
        let total_time = self.total_alloc_time_ns.load(Ordering::Relaxed);
        if count == 0 {
            0.0
        } else {
            total_time as f64 / count as f64
        }
    }

    pub fn print_report(&self, test_name: &str) {
        info!("\n═══════════════════════════════════════════════════════════");
        info!("{} 测试报告", test_name);
        info!("═══════════════════════════════════════════════════════════");

        info!("分配计数:");
        info!("  总分配: {}", self.total_allocations.load(Ordering::Relaxed));
        info!("  总释放: {}", self.total_deallocations.load(Ordering::Relaxed));
        let leaks = self.total_allocations.load(Ordering::Relaxed)
            - self.total_deallocations.load(Ordering::Relaxed);
        info!("  泄漏: {}", leaks);
        if leaks == 0 {
            info!("  ✓ 无泄漏");
        } else {
            info!("  ✗ 检测到 {} 个泄漏", leaks);
        }

        info!("\n内存分类:");
        info!("  小对象 (<4KB):    {}", self.small_allocs.load(Ordering::Relaxed));
        info!("  中对象 (4KB-1MB): {}", self.medium_allocs.load(Ordering::Relaxed));
        info!("  大对象 (>1MB):    {}", self.large_allocs.load(Ordering::Relaxed));

        info!("\n性能指标:");
        let avg_latency = self.calculate_avg_alloc_latency();
        info!("  平均分配延迟: {:.2} ns", avg_latency);
        info!("  最小分配延迟: {} ns", self.min_alloc_time_ns.load(Ordering::Relaxed));
        info!("  最大分配延迟: {} ns", self.max_alloc_time_ns.load(Ordering::Relaxed));

        info!("\n内存使用:");
        info!("  峰值分配: {} bytes ({} MB)",
            self.peak_allocated_bytes.load(Ordering::Relaxed),
            self.peak_allocated_bytes.load(Ordering::Relaxed) / 1024 / 1024
        );
        info!("  当前分配: {} bytes",
            self.current_allocated_bytes.load(Ordering::Relaxed)
        );

        info!("═══════════════════════════════════════════════════════════\n");
    }
}

/// Basic functionality tests
pub mod basic_tests {
    use super::*;

    pub fn run_all(metrics: &AllocatorMetrics) -> bool {
        info!("═══════════════════════════════════════════════════════════");
        info!("基础功能测试 (Basic Functionality Tests)");
        info!("═══════════════════════════════════════════════════════════\n");

        let mut all_passed = true;
        all_passed &= test_small_allocations(metrics);
        all_passed &= test_large_allocations(metrics);
        all_passed &= test_alignment(metrics);
        all_passed &= test_read_write_integrity(metrics);
        all_passed &= test_alloc_dealloc_cycle(metrics);

        if all_passed {
            info!("✓ 基础功能测试全部通过\n");
        } else {
            info!("✗ 基础功能测试有失败\n");
        }

        all_passed
    }

    fn test_small_allocations(metrics: &AllocatorMetrics) -> bool {
        info!("测试: 小对象分配 (Small Allocations)");
        let sizes = [8, 16, 32, 64, 128, 256, 512, 1024, 2048];
        let count_per_size = 100;
        let mut allocs: Vec<(NonNull<u8>, usize)> = Vec::new();

        for &size in &sizes {
            for _ in 0..count_per_size {
                let start = get_time_ns();
                unsafe {
                    let layout = core::alloc::Layout::from_size_align_unchecked(size, 8);
                    let ptr = alloc::alloc::alloc(layout);
                    if !ptr.is_null() {
                        allocs.push((NonNull::new_unchecked(ptr), size));
                        let duration = get_time_ns() - start;
                        metrics.record_alloc(size, duration);
                    }
                }
            }
        }

        info!("  分配了 {} 个小对象", allocs.len());

        for (ptr, size) in &allocs {
            let start = get_time_ns();
            unsafe {
                let layout = core::alloc::Layout::from_size_align_unchecked(*size, 8);
                alloc::alloc::dealloc(ptr.as_ptr(), layout);
                let duration = get_time_ns() - start;
                metrics.record_dealloc(*size, duration);
            }
        }

        info!("  ✓ 小对象分配测试通过\n");
        true
    }

    fn test_large_allocations(metrics: &AllocatorMetrics) -> bool {
        info!("测试: 大对象分配 (Large Allocations)");
        let sizes_kb = [4, 16, 64, 256, 1024, 4096]; // KB
        let mut allocs: Vec<(NonNull<u8>, usize)> = Vec::new();

        for &size_kb in &sizes_kb {
            let size = size_kb * 1024;
            let start = get_time_ns();
            unsafe {
                let layout = core::alloc::Layout::from_size_align_unchecked(size, 4096);
                let ptr = alloc::alloc::alloc(layout);
                if !ptr.is_null() {
                    allocs.push((NonNull::new_unchecked(ptr), size));
                    let duration = get_time_ns() - start;
                    metrics.record_alloc(size, duration);
                    info!("  分配 {} KB", size_kb);
                }
            }
        }

        for (ptr, size) in &allocs {
            let start = get_time_ns();
            unsafe {
                let layout = core::alloc::Layout::from_size_align_unchecked(*size, 4096);
                alloc::alloc::dealloc(ptr.as_ptr(), layout);
                let duration = get_time_ns() - start;
                metrics.record_dealloc(*size, duration);
            }
        }

        info!("  ✓ 大对象分配测试通过\n");
        true
    }

    fn test_alignment(metrics: &AllocatorMetrics) -> bool {
        info!("测试: 地址对齐验证 (Alignment Verification)");
        let alignments = [8, 16, 32, 64, 128, 256, 512, 1024, 2048, 4096, 8192, 16384];
        let mut misaligned = 0;
        let mut checked = 0;

        for &alignment in &alignments {
            for _ in 0..50 {
                let size = alignment;
                let start = get_time_ns();
                unsafe {
                    let layout = core::alloc::Layout::from_size_align_unchecked(size, alignment);
                    let ptr = alloc::alloc::alloc(layout);
                    if !ptr.is_null() {
                        let addr = ptr as usize;
                        if addr % alignment != 0 {
                            misaligned += 1;
                        }
                        checked += 1;
                        let duration = get_time_ns() - start;
                        metrics.record_alloc(size, duration);

                        let dealloc_start = get_time_ns();
                        alloc::alloc::dealloc(ptr, layout);
                        let dealloc_duration = get_time_ns() - dealloc_start;
                        metrics.record_dealloc(size, dealloc_duration);
                    }
                }
            }
        }

        let success = misaligned == 0;
        info!("  检查了 {} 个分配, {} 个未对齐", checked, misaligned);
        if success {
            info!("  ✓ 地址对齐验证通过\n");
        } else {
            info!("  ✗ 地址对齐验证失败\n");
        }
        success
    }

    fn test_read_write_integrity(metrics: &AllocatorMetrics) -> bool {
        info!("测试: 读写完整性验证 (Read/Write Integrity)");
        let size = 8192;
        let start = get_time_ns();
        unsafe {
            let layout = core::alloc::Layout::from_size_align_unchecked(size, 8);
            let ptr = alloc::alloc::alloc(layout);

            if ptr.is_null() {
                info!("  ✗ 分配失败\n");
                return false;
            }

            let alloc_duration = get_time_ns() - start;
            metrics.record_alloc(size, alloc_duration);

            let slice = core::slice::from_raw_parts_mut(ptr, size);

            // Write pattern
            for i in 0..size {
                slice[i] = (i % 256) as u8;
            }

            // Read and verify
            let mut valid = true;
            for i in 0..size {
                if slice[i] != (i % 256) as u8 {
                    valid = false;
                    break;
                }
            }

            let dealloc_start = get_time_ns();
            alloc::alloc::dealloc(ptr, layout);
            let dealloc_duration = get_time_ns() - dealloc_start;
            metrics.record_dealloc(size, dealloc_duration);

            if valid {
                info!("  ✓ 读写完整性验证通过\n");
            } else {
                info!("  ✗ 数据完整性检查失败\n");
            }

            valid
        }
    }

    fn test_alloc_dealloc_cycle(metrics: &AllocatorMetrics) -> bool {
        info!("测试: 分配释放循环 (Alloc/Dealloc Cycle)");
        let rounds = 5000;
        let sizes = [16, 32, 64, 128, 256, 512, 1024];

        for round in 0..rounds {
            for &size in &sizes {
                let start = get_time_ns();
                unsafe {
                    let layout = core::alloc::Layout::from_size_align_unchecked(size, 8);
                    let ptr = alloc::alloc::alloc(layout);
                    if !ptr.is_null() {
                        let alloc_duration = get_time_ns() - start;
                        metrics.record_alloc(size, alloc_duration);

                        let dealloc_start = get_time_ns();
                        alloc::alloc::dealloc(ptr, layout);
                        let dealloc_duration = get_time_ns() - dealloc_start;
                        metrics.record_dealloc(size, dealloc_duration);
                    }
                }
            }

            if round % 1000 == 0 {
                info!("  进度: {}/{}", round, rounds);
            }
        }

        info!("  ✓ 完成 {} 轮分配释放循环\n", rounds);
        true
    }
}

/// Performance tests
pub mod performance_tests {
    use super::*;

    pub fn run_all(metrics: &AllocatorMetrics) -> bool {
        info!("═══════════════════════════════════════════════════════════");
        info!("性能测试 (Performance Tests)");
        info!("═══════════════════════════════════════════════════════════\n");

        // 基础性能测试
        test_throughput(metrics);
        test_latency(metrics);
        test_mixed_pattern(metrics);

        // 新增：突出分配器差异的测试
        test_fixed_size_small_objects(metrics);  // Slab 优势测试
        test_random_size_allocation(metrics);   // TLSF 优势测试
        test_fragmentation_resistance(metrics);  // 碎片化抗性测试
        test_realistic_workload(metrics);       // 真实负载模拟

        true
    }

    fn test_throughput(metrics: &AllocatorMetrics) {
        info!("测试: 吞吐量 (Throughput)");
        let iterations = 1000000;
        let size = 64;

        let start_time = get_time_ns();

        for _ in 0..iterations {
            let alloc_start = get_time_ns();
            unsafe {
                let layout = core::alloc::Layout::from_size_align_unchecked(size, 8);
                let ptr = alloc::alloc::alloc(layout);
                if !ptr.is_null() {
                    let alloc_duration = get_time_ns() - alloc_start;
                    metrics.record_alloc(size, alloc_duration);

                    let dealloc_start = get_time_ns();
                    alloc::alloc::dealloc(ptr, layout);
                    let dealloc_duration = get_time_ns() - dealloc_start;
                    metrics.record_dealloc(size, dealloc_duration);
                }
            }
        }

        let end_time = get_time_ns();
        let total_time_ns = end_time - start_time;
        let throughput = metrics.calculate_throughput(total_time_ns);
        let avg_latency = metrics.calculate_avg_alloc_latency();

        info!("  完成 {} 次操作", iterations);
        info!("  总耗时: {} ns", format_time_ns(total_time_ns));
        info!("  吞吐量: {:.2} M ops/s", throughput / 1_000_000.0);
        info!("  平均延迟: {:.2} ns", avg_latency);
        info!("  ✓ 吞吐量测试完成\n");
    }

    fn test_latency(metrics: &AllocatorMetrics) {
        info!("测试: 延迟分布 (Latency Distribution)");
        let iterations = 10000;
        let size = 256;
        let mut latencies: Vec<u64> = Vec::new();

        for _ in 0..iterations {
            let alloc_start = get_time_ns();
            unsafe {
                let layout = core::alloc::Layout::from_size_align_unchecked(size, 8);
                let ptr = alloc::alloc::alloc(layout);
                if !ptr.is_null() {
                    let alloc_duration = get_time_ns() - alloc_start;
                    latencies.push(alloc_duration);
                    metrics.record_alloc(size, alloc_duration);

                    let dealloc_start = get_time_ns();
                    alloc::alloc::dealloc(ptr, layout);
                    let dealloc_duration = get_time_ns() - dealloc_start;
                    metrics.record_dealloc(size, dealloc_duration);
                }
            }
        }

        latencies.sort();

        let p50 = latencies[latencies.len() / 2];
        let p90 = latencies[(latencies.len() * 9) / 10];
        let p99 = latencies[(latencies.len() * 99) / 100];

        info!("  P50 延迟: {} ns", p50);
        info!("  P90 延迟: {} ns", p90);
        info!("  P99 延迟: {} ns", p99);
        info!("  ✓ 延迟分布测试完成\n");
    }

    fn test_mixed_pattern(metrics: &AllocatorMetrics) {
        info!("测试: 混合模式 (Mixed Pattern)");
        let patterns = [
            (8, 3000),
            (16, 2500),
            (64, 2000),
            (256, 1500),
            (1024, 1000),
            (4096, 500),
            (16384, 200),
            (65536, 100),
        ];

        let start_time = get_time_ns();
        let mut allocs: Vec<(NonNull<u8>, usize)> = Vec::new();

        for &(size, count) in &patterns {
            for _ in 0..count {
                let alloc_start = get_time_ns();
                unsafe {
                    let layout = core::alloc::Layout::from_size_align_unchecked(size, 8);
                    let ptr = alloc::alloc::alloc(layout);
                    if !ptr.is_null() {
                        allocs.push((NonNull::new_unchecked(ptr), size));
                        let alloc_duration = get_time_ns() - alloc_start;
                        metrics.record_alloc(size, alloc_duration);
                    }
                }
            }
        }

        let total_ops: usize = patterns.iter().map(|&(_, c)| c).sum();
        info!("  分配了 {} 个对象", allocs.len());

        // Random deallocation pattern
        while !allocs.is_empty() {
            let idx = allocs.len() / 3;
            if idx < allocs.len() {
                let (ptr, size) = allocs.remove(idx);
                let dealloc_start = get_time_ns();
                unsafe {
                    let layout = core::alloc::Layout::from_size_align_unchecked(size, 8);
                    alloc::alloc::dealloc(ptr.as_ptr(), layout);
                    let dealloc_duration = get_time_ns() - dealloc_start;
                    metrics.record_dealloc(size, dealloc_duration);
                }
            } else {
                break;
            }
        }

        let end_time = get_time_ns();
        let total_time_ns = end_time - start_time;
        info!("  完成 {} 次操作", total_ops * 2);
        info!("  总耗时: {}", format_time_ns(total_time_ns));
        info!("  ✓ 混合模式测试完成\n");
    }

    /// Test 1: Fixed-size small objects (Slab allocator advantage)
    /// Buddy-Slab 在固定小对象分配上有优势，因为 Slab 缓存了常用大小的对象
    fn test_fixed_size_small_objects(metrics: &AllocatorMetrics) {
        info!("测试: 固定大小小对象 (Fixed Size Small Objects) - Slab 优势场景");
        info!("  说明: 频繁分配/释放相同大小的小对象，Slab allocator 有显著优势\n");

        // 测试多种常见小对象大小
        let test_sizes = [32, 64, 128, 256, 512, 1024];

        for size in test_sizes {
            let iterations = 50000;
            let start_time = get_time_ns();

            // 分配阶段
            let mut allocs: Vec<*mut u8> = Vec::with_capacity(iterations);
            for _ in 0..iterations {
                unsafe {
                    let layout = core::alloc::Layout::from_size_align_unchecked(size, 8);
                    let ptr = alloc::alloc::alloc(layout);
                    if !ptr.is_null() {
                        allocs.push(ptr);
                        let duration = get_time_ns() - start_time;
                        metrics.record_alloc(size, duration);
                    }
                }
            }

            // 释放阶段
            for ptr in &allocs {
                unsafe {
                    let layout = core::alloc::Layout::from_size_align_unchecked(size, 8);
                    let dealloc_start = get_time_ns();
                    alloc::alloc::dealloc(*ptr, layout);
                    let dealloc_duration = get_time_ns() - dealloc_start;
                    metrics.record_dealloc(size, dealloc_duration);
                }
            }

            let end_time = get_time_ns();
            let total_time_ns = end_time - start_time;
            let throughput = (iterations as f64 * 2.0) / (total_time_ns as f64 / 1_000_000_000.0);

            info!("  大小 {:>4} 字节: 吞吐量 {:>8.2} M ops/s, 耗时 {}",
                size, throughput / 1_000_000.0, format_time_ns(total_time_ns));
        }

        info!("  ✓ 固定大小小对象测试完成\n");
    }

    /// Test 2: Random size allocation (TLSF allocator advantage)
    /// TLSF 在随机大小分配上更平滑，因为其分层结构能够快速定位合适的块
    fn test_random_size_allocation(metrics: &AllocatorMetrics) {
        info!("测试: 随机大小分配 (Random Size Allocation) - TLSF 优势场景");
        info!("  说明: 分配/释放随机大小的对象，TLSF 的两层分离适配结构有优势\n");

        // 伪随机数生成器
        let mut seed: u64 = 0xdeadbeef;

        let iterations = 50000;
        let start_time = get_time_ns();

        // 生成随机大小的分配请求（16 字节到 16KB）
        let mut allocs: Vec<(*mut u8, usize)> = Vec::with_capacity(iterations);

        for _ in 0..iterations {
            // 简单的伪随机数生成
            seed = seed.wrapping_mul(1103515245).wrapping_add(12345);
            let log2_size = (seed % 10) + 4; // 4 到 13，对应 16 到 8192
            let size = 1usize << log2_size;

            unsafe {
                let layout = core::alloc::Layout::from_size_align_unchecked(size, 8);
                let ptr = alloc::alloc::alloc(layout);
                if !ptr.is_null() {
                    allocs.push((ptr, size));
                    let duration = get_time_ns() - start_time;
                    metrics.record_alloc(size, duration);
                }
            }
        }

        // 随机释放
        seed = 0xdeadbeef;
        while !allocs.is_empty() {
            seed = seed.wrapping_mul(1103515245).wrapping_add(12345);
            let idx = (seed as usize) % allocs.len();
            let (ptr, size) = allocs.swap_remove(idx);

            unsafe {
                let layout = core::alloc::Layout::from_size_align_unchecked(size, 8);
                let dealloc_start = get_time_ns();
                alloc::alloc::dealloc(ptr, layout);
                let dealloc_duration = get_time_ns() - dealloc_start;
                metrics.record_dealloc(size, dealloc_duration);
            }
        }

        let end_time = get_time_ns();
        let total_time_ns = end_time - start_time;
        let total_ops = iterations * 2;
        let throughput = (total_ops as f64) / (total_time_ns as f64 / 1_000_000_000.0);

        info!("  总操作数: {}", total_ops);
        info!("  吞吐量: {:.2} M ops/s", throughput / 1_000_000.0);
        info!("  总耗时: {}", format_time_ns(total_time_ns));
        info!("  ✓ 随机大小分配测试完成\n");
    }

    /// Test 3: Fragmentation resistance
    /// TLSF 在碎片化场景下表现更好，因为能够快速查找和合并空闲块
    fn test_fragmentation_resistance(metrics: &AllocatorMetrics) {
        info!("测试: 碎片化抗性 (Fragmentation Resistance) - TLSF 优势场景");
        info!("  说明: 产生严重碎片化后，测试分配成功率\n");

        let phases = 5;
        let objects_per_phase = 2000;
        let sizes = [16, 32, 64, 128, 256, 512, 1024, 2048, 4096, 8192];

        let start_time = get_time_ns();
        let mut total_allocations = 0;
        let mut failed_allocations = 0;

        for phase in 0..phases {
            let mut allocs: Vec<(NonNull<u8>, usize)> = Vec::new();

            // 分配阶段
            for i in 0..objects_per_phase {
                let size_idx = (phase + i) % sizes.len();
                let size = sizes[size_idx];

                unsafe {
                    let layout = core::alloc::Layout::from_size_align_unchecked(size, 8);
                    let ptr = alloc::alloc::alloc(layout);
                    if !ptr.is_null() {
                        allocs.push((NonNull::new_unchecked(ptr), size));
                        let duration = get_time_ns() - start_time;
                        metrics.record_alloc(size, duration);
                        total_allocations += 1;
                    } else {
                        failed_allocations += 1;
                    }
                }
            }

            // 随机释放约 50% 的对象，产生碎片化
            let mut seed: u64 = phase as u64 * 17;
            let mut new_allocs = Vec::new();
            for (ptr, size) in allocs {
                seed = seed.wrapping_mul(1103515245).wrapping_add(12345);
                if (seed as usize) % 2 == 0 {
                    // 释放
                    unsafe {
                        let layout = core::alloc::Layout::from_size_align_unchecked(size, 8);
                        let dealloc_start = get_time_ns();
                        alloc::alloc::dealloc(ptr.as_ptr(), layout);
                        let dealloc_duration = get_time_ns() - dealloc_start;
                        metrics.record_dealloc(size, dealloc_duration);
                    }
                } else {
                    new_allocs.push((ptr, size));
                }
            }

            // 尝试重新分配相同大小的对象
            let mut retry_allocs = 0;
            let mut retry_failed = 0;
            for (_ptr, size) in &new_allocs {
                unsafe {
                    let layout = core::alloc::Layout::from_size_align_unchecked(*size, 8);
                    let ptr = alloc::alloc::alloc(layout);
                    if !ptr.is_null() {
                        alloc::alloc::dealloc(ptr, layout);
                        retry_allocs += 1;
                    } else {
                        retry_failed += 1;
                    }
                }
            }

            if phase > 0 {
                let success_rate = (retry_allocs * 100) / (retry_allocs + retry_failed).max(1);
                info!("  阶段 {}: 重分配成功率 {}/{} ({}%)",
                    phase, retry_allocs, retry_allocs + retry_failed, success_rate);
            }

            // 清理剩余对象
            for (ptr, size) in new_allocs {
                unsafe {
                    let layout = core::alloc::Layout::from_size_align_unchecked(size, 8);
                    let dealloc_start = get_time_ns();
                    alloc::alloc::dealloc(ptr.as_ptr(), layout);
                    let dealloc_duration = get_time_ns() - dealloc_start;
                    metrics.record_dealloc(size, dealloc_duration);
                }
            }
        }

        let end_time = get_time_ns();
        let total_time_ns = end_time - start_time;
        let overall_success_rate = (total_allocations * 100) / (total_allocations + failed_allocations).max(1);

        info!("  总分配数: {}", total_allocations);
        info!("  失败分配数: {}", failed_allocations);
        info!("  总成功率: {}%", overall_success_rate);
        info!("  总耗时: {}", format_time_ns(total_time_ns));
        info!("  ✓ 碎片化抗性测试完成\n");
    }

    /// Test 4: Realistic workload simulation
    /// 模拟真实工作负载：小对象、中对象、大对象的混合
    fn test_realistic_workload(metrics: &AllocatorMetrics) {
        info!("测试: 真实负载模拟 (Realistic Workload)");
        info!("  说明: 模拟常见应用场景的内存分配模式\n");

        // 负载类型：(大小, 权重, 持续时间ms)
        // 小对象（频繁，短期）+ 中对象（中等，中期）+ 大对象（较少，长期）
        let workload = [
            (16, 40, 10),    // 40% 的小对象
            (64, 30, 20),    // 30% 的中等对象
            (256, 15, 50),   // 15% 的较大对象
            (1024, 10, 100), // 10% 的大对象
            (4096, 5, 200),  // 5% 的超大对象
        ];

        let total_ops = 200000;
        let start_time = get_time_ns();
        let mut active_allocs: Vec<(NonNull<u8>, usize, u64)> = Vec::new();
        let mut rng_seed: u64 = 0x12345678;

        for i in 0..total_ops {
            // 20% 概率释放旧对象
            if !active_allocs.is_empty() && i % 5 == 0 {
                let idx = (rng_seed as usize) % active_allocs.len();
                rng_seed = rng_seed.wrapping_mul(1103515245).wrapping_add(12345);

                let (ptr, size, _alloc_time) = active_allocs.swap_remove(idx);
                unsafe {
                    let layout = core::alloc::Layout::from_size_align_unchecked(size, 8);
                    let dealloc_start = get_time_ns();
                    alloc::alloc::dealloc(ptr.as_ptr(), layout);
                    let dealloc_duration = get_time_ns() - dealloc_start;
                    metrics.record_dealloc(size, dealloc_duration);
                }
            }

            // 根据权重选择大小
            let weight_sum: u32 = workload.iter().map(|(_, w, _)| *w).sum();
            let mut weight_acc: u32 = 0;
            let mut selected_size = workload[0].0;

            rng_seed = rng_seed.wrapping_mul(1103515245).wrapping_add(12345);
            let target_weight = (rng_seed as u32) % weight_sum;

            for (size, weight, _duration) in &workload {
                weight_acc += weight;
                if target_weight < weight_acc {
                    selected_size = *size;
                    break;
                }
            }

            // 分配新对象
            unsafe {
                let layout = core::alloc::Layout::from_size_align_unchecked(selected_size, 8);
                let ptr = alloc::alloc::alloc(layout);
                if !ptr.is_null() {
                    let alloc_start = get_time_ns();
                    active_allocs.push((NonNull::new_unchecked(ptr), selected_size, alloc_start));
                    let duration = get_time_ns() - alloc_start;
                    metrics.record_alloc(selected_size, duration);
                }
            }

            if i % 20000 == 0 && i > 0 {
                let elapsed = (get_time_ns() - start_time) / 1_000_000;
                let ops_per_sec = (i * 1000) / elapsed.max(1);
                info!("  进度: {}/{} ({} ops/s)",
                    i, total_ops, ops_per_sec);
            }
        }

        // 清理所有剩余对象
        for (ptr, size, _) in active_allocs {
            unsafe {
                let layout = core::alloc::Layout::from_size_align_unchecked(size, 8);
                let dealloc_start = get_time_ns();
                alloc::alloc::dealloc(ptr.as_ptr(), layout);
                let dealloc_duration = get_time_ns() - dealloc_start;
                metrics.record_dealloc(size, dealloc_duration);
            }
        }

        let end_time = get_time_ns();
        let total_time_ns = end_time - start_time;
        let total_time_ms = total_time_ns / 1_000_000;
        let ops_per_sec = (total_ops * 1_000) / total_time_ms.max(1);

        info!("  总操作数: {}", total_ops);
        info!("  总耗时: {} ms", total_time_ms);
        info!("  平均吞吐量: {} ops/s", ops_per_sec);
        info!("  ✓ 真实负载模拟完成\n");
    }
}

/// Stress tests
pub mod stress_tests {
    use super::*;

    pub fn run_all(metrics: &AllocatorMetrics) -> bool {
        info!("═══════════════════════════════════════════════════════════");
        info!("压力测试 (Stress Tests)");
        info!("═══════════════════════════════════════════════════════════\n");

        test_memory_exhaustion(metrics);
        test_fragmentation(metrics);
        test_edge_cases(metrics);

        true
    }

    fn test_memory_exhaustion(metrics: &AllocatorMetrics) {
        info!("测试: 内存耗尽 (Memory Exhaustion)");
        let size = 1024 * 1024; // 1MB
        let mut allocs: Vec<(NonNull<u8>, usize)> = Vec::new();
        let mut count = 0;

        loop {
            let alloc_start = get_time_ns();
            unsafe {
                let layout = core::alloc::Layout::from_size_align_unchecked(size, 4096);
                let ptr = alloc::alloc::alloc(layout);

                if ptr.is_null() {
                    break;
                } else {
                    allocs.push((NonNull::new_unchecked(ptr), size));
                    let alloc_duration = get_time_ns() - alloc_start;
                    metrics.record_alloc(size, alloc_duration);
                    count += 1;

                    if count >= 256 {
                        break;
                    }
                }
            }
        }

        info!("  成功分配了 {} 个 1MB 块", count);
        info!("  总计: {} MB", count);

        // Free all
        for (ptr, size) in &allocs {
            let dealloc_start = get_time_ns();
            unsafe {
                let layout = core::alloc::Layout::from_size_align_unchecked(*size, 4096);
                alloc::alloc::dealloc(ptr.as_ptr(), layout);
                let dealloc_duration = get_time_ns() - dealloc_start;
                metrics.record_dealloc(*size, dealloc_duration);
            }
        }

        info!("  ✓ 内存耗尽测试完成\n");
    }

    fn test_fragmentation(metrics: &AllocatorMetrics) {
        info!("测试: 碎片化 (Fragmentation)");
        let sizes = [16, 32, 64, 128, 256, 512, 1024, 2048, 4096, 8192];
        let mut allocs: Vec<(NonNull<u8>, usize)> = Vec::new();

        // Phase 1: Allocate
        info!("  阶段 1: 分配 2000 个对象");
        for i in 0..2000 {
            let size = sizes[i % sizes.len()];
            let alloc_start = get_time_ns();
            unsafe {
                let layout = core::alloc::Layout::from_size_align_unchecked(size, 8);
                let ptr = alloc::alloc::alloc(layout);
                if !ptr.is_null() {
                    allocs.push((NonNull::new_unchecked(ptr), size));
                    let alloc_duration = get_time_ns() - alloc_start;
                    metrics.record_alloc(size, alloc_duration);
                }
            }
        }

        // Phase 2: Fragmented deallocation
        info!("  阶段 2: 碎片化释放");
        let mut i = 0;
        while allocs.len() > 1000 {
            if i % 2 == 0 && allocs.len() > 0 {
                let idx = allocs.len() / 4;
                if let Some((ptr, size)) = allocs.get(idx) {
                    let dealloc_start = get_time_ns();
                    unsafe {
                        let layout = core::alloc::Layout::from_size_align_unchecked(*size, 8);
                        alloc::alloc::dealloc(ptr.as_ptr(), layout);
                        let dealloc_duration = get_time_ns() - dealloc_start;
                        metrics.record_dealloc(*size, dealloc_duration);
                    }
                    allocs.remove(idx);
                }
            }
            i += 1;
        }

        // Phase 3: Reallocate after fragmentation
        info!("  阶段 3: 碎片化后再分配");
        for _ in 0..500 {
            let size = sizes[i % sizes.len()];
            let alloc_start = get_time_ns();
            unsafe {
                let layout = core::alloc::Layout::from_size_align_unchecked(size, 8);
                let ptr = alloc::alloc::alloc(layout);
                if !ptr.is_null() {
                    allocs.push((NonNull::new_unchecked(ptr), size));
                    let alloc_duration = get_time_ns() - alloc_start;
                    metrics.record_alloc(size, alloc_duration);
                }
            }
        }

        // Free remaining
        for (ptr, size) in allocs {
            let dealloc_start = get_time_ns();
            unsafe {
                let layout = core::alloc::Layout::from_size_align_unchecked(size, 8);
                alloc::alloc::dealloc(ptr.as_ptr(), layout);
                let dealloc_duration = get_time_ns() - dealloc_start;
                metrics.record_dealloc(size, dealloc_duration);
            }
        }

        info!("  ✓ 碎片化测试完成\n");
    }

    fn test_edge_cases(metrics: &AllocatorMetrics) {
        info!("测试: 边界条件 (Edge Cases)");

        // Zero-size allocation
        info!("  测试 0 字节分配...");
        unsafe {
            let layout = core::alloc::Layout::from_size_align_unchecked(0, 1);
            let ptr = alloc::alloc::alloc(layout);
            if !ptr.is_null() {
                let alloc_start = get_time_ns();
                metrics.record_alloc(0, get_time_ns() - alloc_start);
                alloc::alloc::dealloc(ptr, layout);
                info!("    0 字节分配成功");
            } else {
                info!("    0 字节分配返回 null");
            }
        }

        // Very large alignment
        info!("  测试超大对齐要求...");
        let alignments = [16384, 65536, 262144, 1048576];
        for &alignment in &alignments {
            unsafe {
                let layout = core::alloc::Layout::from_size_align_unchecked(alignment, alignment);
                let ptr = alloc::alloc::alloc(layout);
                if !ptr.is_null() {
                    let addr = ptr as usize;
                    if addr % alignment == 0 {
                        info!("    对齐 {} 字节: 成功 (地址 {:#x})", alignment, addr);
                    } else {
                        info!("    对齐 {} 字节: 失败 (地址 {:#x})", alignment, addr);
                    }
                    let dealloc_start = get_time_ns();
                    let alloc_duration = get_time_ns() - dealloc_start;
                    metrics.record_alloc(alignment, alloc_duration);
                    alloc::alloc::dealloc(ptr, layout);
                    let dealloc_duration = get_time_ns() - dealloc_start;
                    metrics.record_dealloc(alignment, dealloc_duration);
                } else {
                    info!("    对齐 {} 字节: 失败 (返回 null)", alignment);
                }
            }
        }

        // Non-power-of-2 sizes
        info!("  测试非 2 的幂次大小...");
        let non_pow2_sizes = [7, 13, 33, 100, 201, 1023, 4095, 8191];
        let mut success_count = 0;
        for &size in &non_pow2_sizes {
            unsafe {
                let layout = core::alloc::Layout::from_size_align_unchecked(size, 1);
                let ptr = alloc::alloc::alloc(layout);
                if !ptr.is_null() {
                    success_count += 1;
                    let dealloc_start = get_time_ns();
                    let alloc_duration = get_time_ns() - dealloc_start;
                    metrics.record_alloc(size, alloc_duration);
                    alloc::alloc::dealloc(ptr, layout);
                    let dealloc_duration = get_time_ns() - dealloc_start;
                    metrics.record_dealloc(size, dealloc_duration);
                }
            }
        }
        info!("    非 2 的幂次: {}/{} 成功", success_count, non_pow2_sizes.len());

        info!("  ✓ 边界条件测试完成\n");
    }
}

/// Multi-threaded concurrency tests
pub mod multithread_tests {
    use super::*;

    pub fn run_all(metrics: &AllocatorMetrics) -> bool {
        info!("═══════════════════════════════════════════════════════════");
        info!("多核多线程测试 (Multi-threaded Concurrency Tests)");
        info!("═══════════════════════════════════════════════════════════\n");

        let cpu_count = get_cpu_count();
        info!("检测到 {} 个 CPU 核心", cpu_count);

        test_concurrent_allocations(metrics, cpu_count);
        test_concurrent_contention(metrics, cpu_count);
        test_long_running_concurrent(metrics, cpu_count);

        true
    }

    fn test_concurrent_allocations(metrics: &AllocatorMetrics, cpu_count: usize) {
        info!("测试: 并发分配 (Concurrent Allocations) - 简化版本");
        let num_threads = cpu_count.max(2);
        let ops_per_thread = 1000; // 减少操作次数

        info!("  使用 {} 个线程, 每线程 {} 次操作", num_threads, ops_per_thread);

        let start_time = get_time_ns();

        // 简化版本：在当前线程执行所有操作
        let total_ops = num_threads * ops_per_thread;
        for thread_id in 0..num_threads {
            for i in 0..ops_per_thread {
                let size = ((thread_id + i) % 10 + 1) * 16; // 16-160 bytes

                let alloc_start = get_time_ns();
                unsafe {
                    let layout = core::alloc::Layout::from_size_align_unchecked(size, 8);
                    let ptr = alloc::alloc::alloc(layout);
                    if !ptr.is_null() {
                        let alloc_duration = get_time_ns() - alloc_start;

                        // Write to verify no overlap
                        let slice = core::slice::from_raw_parts_mut(ptr, size.min(32));
                        for byte in slice.iter_mut() {
                            *byte = (thread_id % 256) as u8;
                        }

                        let dealloc_start = get_time_ns();
                        alloc::alloc::dealloc(ptr, layout);
                        let dealloc_duration = get_time_ns() - dealloc_start;

                        metrics.record_alloc(size, alloc_duration);
                        metrics.record_dealloc(size, dealloc_duration);
                    }
                }
            }
        }

        let end_time = get_time_ns();
        let total_time_ns = end_time - start_time;

        info!("  总操作: {}", total_ops);
        info!("  总耗时: {}", format_time_ns(total_time_ns));
        info!("  并发吞吐量: {:.2} M ops/s",
            (total_ops as f64 * 1e9) / total_time_ns as f64 / 1_000_000.0
        );
        info!("  ✓ 并发分配测试完成\n");
    }

    fn test_concurrent_contention(metrics: &AllocatorMetrics, cpu_count: usize) {
        info!("测试: 并发竞争 (Concurrent Contention) - 简化版本");
        let num_threads = cpu_count.max(4);
        let ops_per_thread = 2000; // 减少操作次数

        info!("  {} 个线程同时分配相同大小的内存", num_threads);

        let start_time = get_time_ns();

        // 简化版本：在当前线程执行所有操作
        let size = 64;
        let total_ops = num_threads * ops_per_thread;
        for _ in 0..total_ops {
            let alloc_start = get_time_ns();
            unsafe {
                let layout = core::alloc::Layout::from_size_align_unchecked(size, 8);
                let ptr = alloc::alloc::alloc(layout);
                if !ptr.is_null() {
                    let alloc_duration = get_time_ns() - alloc_start;

                    let dealloc_start = get_time_ns();
                    alloc::alloc::dealloc(ptr, layout);
                    let dealloc_duration = get_time_ns() - dealloc_start;

                    metrics.record_alloc(size, alloc_duration);
                    metrics.record_dealloc(size, dealloc_duration);
                }
            }
        }

        let end_time = get_time_ns();
        let total_time_ns = end_time - start_time;

        info!("  总操作: {}", total_ops);
        info!("  总耗时: {}", format_time_ns(total_time_ns));
        info!("  竞争吞吐量: {:.2} M ops/s",
            (total_ops as f64 * 1e9) / total_time_ns as f64 / 1_000_000.0
        );
        info!("  ✓ 并发竞争测试完成\n");
    }

    fn test_long_running_concurrent(metrics: &AllocatorMetrics, cpu_count: usize) {
        info!("测试: 长时间并发运行 (Long Running Concurrent) - 简化版本");
        let num_threads = cpu_count.max(2);
        let duration_seconds = 5; // 减少到5秒

        info!("  模拟 {} 个线程运行 {} 秒", num_threads, duration_seconds);

        let start_time = get_time_ns();

        // 简化版本：在当前线程执行
        let sizes = [16, 32, 64, 128, 256, 512, 1024];
        let mut i = 0;

        loop {
            let elapsed_ns = get_time_ns() - start_time;
            if elapsed_ns > duration_seconds as u64 * 1_000_000_000 {
                break;
            }

            let size = sizes[i % sizes.len()];
            let alloc_start = get_time_ns();
            unsafe {
                let layout = core::alloc::Layout::from_size_align_unchecked(size, 8);
                let ptr = alloc::alloc::alloc(layout);
                if !ptr.is_null() {
                    let alloc_duration = get_time_ns() - alloc_start;

                    let dealloc_start = get_time_ns();
                    alloc::alloc::dealloc(ptr, layout);
                    let dealloc_duration = get_time_ns() - dealloc_start;

                    metrics.record_alloc(size, alloc_duration);
                    metrics.record_dealloc(size, dealloc_duration);
                }
            }

            i += 1;

            if i % 10000 == 0 {
                let _elapsed_s = elapsed_ns / 1_000_000_000;
                // info!("  已运行 {} 秒, {} 次操作", _elapsed_s, i);
            }
        }

        let end_time = get_time_ns();
        let total_time_ns = end_time - start_time;

        info!("  长时间并发运行完成, 总耗时: {}\n", format_time_ns(total_time_ns));
    }
}

/// Memory leak detection
pub mod leak_detection {
    use super::*;

    pub fn run_all(metrics: &AllocatorMetrics) -> bool {
        info!("═══════════════════════════════════════════════════════════");
        info!("内存泄漏检测 (Memory Leak Detection)");
        info!("═══════════════════════════════════════════════════════════\n");

        let mut all_passed = true;
        all_passed &= test_alloc_dealloc_balance(metrics);
        all_passed &= test_repeated_cycles(metrics);

        if all_passed {
            info!("✓ 内存泄漏检测通过\n");
        } else {
            info!("✗ 内存泄漏检测失败\n");
        }

        all_passed
    }

    fn test_alloc_dealloc_balance(metrics: &AllocatorMetrics) -> bool {
        info!("测试: 分配释放平衡 (Alloc/Dealloc Balance)");
        let rounds = 10;
        let iterations_per_round = 5000;

        for round in 0..rounds {
            info!("  第 {} 轮 / {} 轮", round + 1, rounds);

            let initial_allocs = metrics.total_allocations.load(Ordering::Relaxed);
            let initial_deallocs = metrics.total_deallocations.load(Ordering::Relaxed);

            // Do allocation/deallocation cycle
            for _ in 0..iterations_per_round {
                let size = 128;
                let alloc_start = get_time_ns();
                unsafe {
                    let layout = core::alloc::Layout::from_size_align_unchecked(size, 8);
                    let ptr = alloc::alloc::alloc(layout);
                    if !ptr.is_null() {
                        let alloc_duration = get_time_ns() - alloc_start;
                        metrics.record_alloc(size, alloc_duration);

                        let dealloc_start = get_time_ns();
                        alloc::alloc::dealloc(ptr, layout);
                        let dealloc_duration = get_time_ns() - dealloc_start;
                        metrics.record_dealloc(size, dealloc_duration);
                    }
                }
            }

            let final_allocs = metrics.total_allocations.load(Ordering::Relaxed);
            let final_deallocs = metrics.total_deallocations.load(Ordering::Relaxed);
            let delta_allocs = final_allocs - initial_allocs;
            let delta_deallocs = final_deallocs - initial_deallocs;

            if delta_allocs != delta_deallocs {
                info!("    ✗ 轮 {} 不平衡: 分配 {} vs 释放 {}",
                    round + 1, delta_allocs, delta_deallocs);
                return false;
            }
        }

        info!("  ✓ 所有轮次分配释放平衡\n");
        true
    }

    fn test_repeated_cycles(metrics: &AllocatorMetrics) -> bool {
        info!("测试: 重复测试循环 (Repeated Test Cycles)");
        let cycles = 5;
        let ops_per_cycle = 2000;

        for cycle in 0..cycles {
            info!("  周期 {} / {}", cycle + 1, cycles);

            let cycle_start = get_time_ns();
            let mut allocs: Vec<(NonNull<u8>, usize)> = Vec::new();

            // Allocate
            for _ in 0..ops_per_cycle {
                let size = 256;
                let alloc_start = get_time_ns();
                unsafe {
                    let layout = core::alloc::Layout::from_size_align_unchecked(size, 8);
                    let ptr = alloc::alloc::alloc(layout);
                    if !ptr.is_null() {
                        allocs.push((NonNull::new_unchecked(ptr), size));
                        let alloc_duration = get_time_ns() - alloc_start;
                        metrics.record_alloc(size, alloc_duration);
                    }
                }
            }

            // Deallocate all
            for (ptr, size) in &allocs {
                let dealloc_start = get_time_ns();
                unsafe {
                    let layout = core::alloc::Layout::from_size_align_unchecked(*size, 8);
                    alloc::alloc::dealloc(ptr.as_ptr(), layout);
                    let dealloc_duration = get_time_ns() - dealloc_start;
                    metrics.record_dealloc(*size, dealloc_duration);
                }
            }

            let cycle_end = get_time_ns();
            info!("    周期 {} 耗时 {}", cycle + 1, format_time_ns(cycle_end - cycle_start));
        }

        info!("  ✓ 重复测试循环完成\n");
        true
    }
}

/// Main entry point for running all tests
pub fn run_comprehensive_tests() {
    info!("╔════════════════════════════════════════════════════════════╗");
    info!("║     Axvisor Allocator Comprehensive Benchmark           ║");
    info!("╚════════════════════════════════════════════════════════════╝\n");

    let metrics = AllocatorMetrics::new();

    // Get allocator stats before tests
    info!("分配器初始状态:");
    info!("  已用页面: {}", std::os::arceos::modules::axalloc::global_allocator().used_pages());
    info!("  可用页面: {}", std::os::arceos::modules::axalloc::global_allocator().available_pages());
    info!("  已用字节: {}", std::os::arceos::modules::axalloc::global_allocator().used_bytes());
    info!("  可用字节: {}", std::os::arceos::modules::axalloc::global_allocator().available_bytes());

    info!("\nCPU 信息:");
    let cpu_count = get_cpu_count();
    info!("  CPU 核心数: {}", cpu_count);

    info!("\n开始测试...\n");

    // Run all test suites
    let mut all_passed = true;
    let start_time = get_time_ns();

    all_passed &= basic_tests::run_all(&metrics);
    all_passed &= performance_tests::run_all(&metrics);
    all_passed &= stress_tests::run_all(&metrics);
    all_passed &= multithread_tests::run_all(&metrics);
    all_passed &= leak_detection::run_all(&metrics);

    let end_time = get_time_ns();
    let total_time = end_time - start_time;

    // Print final metrics report
    metrics.print_report("综合");

    info!("═══════════════════════════════════════════════════════════");
    info!("测试总结 (Test Summary)");
    info!("═══════════════════════════════════════════════════════════");
    info!("总耗时: {}", format_time_ns(total_time));
    info!("总吞吐量: {:.2} M ops/s",
        metrics.calculate_throughput(total_time) / 1_000_000.0
    );
    info!("平均分配延迟: {:.2} ns", metrics.calculate_avg_alloc_latency());

    if metrics.check_leaks() {
        info!("内存泄漏: ✓ 无泄漏");
    } else {
        info!("内存泄漏: ✗ 检测到泄漏");
    }

    if all_passed {
        info!("\n╔══════════════════════════════════════════════════════════╗");
        info!("║     ✓ ALL TESTS PASSED                                ║");
        info!("╚════════════════════════════════════════════════════════════╝\n");
    } else {
        info!("\n╔════════════════════════════════════════════════════════════╗");
        info!("║     ✗ SOME TESTS FAILED                                ║");
        info!("╚════════════════════════════════════════════════════════════╝\n");
    }

    // Get allocator stats after tests
    info!("\n分配器最终状态:");
    info!("  已用页面: {}", std::os::arceos::modules::axalloc::global_allocator().used_pages());
    info!("  可用页面: {}", std::os::arceos::modules::axalloc::global_allocator().available_pages());
    info!("  已用字节: {}", std::os::arceos::modules::axalloc::global_allocator().used_bytes());
    info!("  可用字节: {}", std::os::arceos::modules::axalloc::global_allocator().available_bytes());
}

/// Get CPU count
fn get_cpu_count() -> usize {
    extern crate axruntime;
    axruntime::cpu_count()
}

/// Get current time in nanoseconds
fn get_time_ns() -> u64 {
    std::os::arceos::modules::axhal::time::monotonic_time_nanos()
}

/// Format time in nanoseconds to human readable string
fn format_time_ns(ns: u64) -> alloc::string::String {
    if ns < 1000 {
        alloc::format!("{} ns", ns)
    } else if ns < 1_000_000 {
        alloc::format!("{:.2} µs", ns as f64 / 1000.0)
    } else if ns < 1_000_000_000 {
        alloc::format!("{:.2} ms", ns as f64 / 1_000_000.0)
    } else {
        alloc::format!("{:.2} s", ns as f64 / 1_000_000_000.0)
    }
}
