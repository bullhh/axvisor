//! Memory allocation tracking for Axvisor.
//!
//! This module provides comprehensive memory allocation tracking capabilities,
//! inspired by asterinas, with allocation backtraces, usage statistics,
//! and memory leak detection.

use core::alloc::Layout;
use core::ptr::NonNull;
use core::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};

use alloc::collections::BTreeMap;
use kspin::SpinNoIrq;

/// Global tracking state
static TRACKING_ENABLED: AtomicBool = AtomicBool::new(false);
static ALLOCATION_GENERATION: AtomicU64 = AtomicU64::new(0);

/// Allocation metadata with backtrace support
#[derive(Debug, Clone)]
pub struct AllocationInfo {
    /// Layout of the allocation
    pub layout: Layout,
    /// Backtrace at the time of allocation (simplified)
    pub backtrace: [usize; 8], // Simplified backtrace
    /// Generation when this allocation was made
    pub generation: u64,
    /// Timestamp of allocation (simplified)
    pub timestamp: u64,
    /// Allocation source tag
    pub tag: AllocationTag,
}

/// Allocation source tags for categorization
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AllocationTag {
    Unknown,
    Kernel,
    Driver,
    Network,
    FileSystem,
    Process,
    VMMemory,
    PageTable,
    DMA,
    Slab,
    Buddy,
}

impl AllocationTag {
    pub fn from_string(s: &str) -> Self {
        match s {
            "kernel" => AllocationTag::Kernel,
            "driver" => AllocationTag::Driver,
            "network" => AllocationTag::Network,
            "fs" | "filesystem" => AllocationTag::FileSystem,
            "process" => AllocationTag::Process,
            "vm" | "vmmemory" => AllocationTag::VMMemory,
            "pagetable" | "page_table" => AllocationTag::PageTable,
            "dma" => AllocationTag::DMA,
            "slab" => AllocationTag::Slab,
            "buddy" => AllocationTag::Buddy,
            _ => AllocationTag::Unknown,
        }
    }
}

/// Global allocation tracking state
pub struct TrackingState {
    /// Map of allocation address to info
    allocations: BTreeMap<usize, AllocationInfo>,
    /// Current generation counter
    generation: u64,
    /// Statistics by tag
    tag_stats: [AllocationStats; 11], // Fixed size for all AllocationTag variants
    /// Total allocations count
    total_allocations: AtomicUsize,
    /// Total deallocations count
    total_deallocations: AtomicUsize,
    /// Peak memory usage
    peak_memory_usage: AtomicUsize,
    /// Current memory usage
    current_memory_usage: AtomicUsize,
}

impl TrackingState {
    pub const fn new() -> Self {
        Self {
            allocations: BTreeMap::new(),
            generation: 0,
            tag_stats: [AllocationStats::new(); 11],
            total_allocations: AtomicUsize::new(0),
            total_deallocations: AtomicUsize::new(0),
            peak_memory_usage: AtomicUsize::new(0),
            current_memory_usage: AtomicUsize::new(0),
        }
    }

    pub fn track_allocation(&mut self, addr: usize, layout: Layout, tag: AllocationTag) {
        let generation = ALLOCATION_GENERATION.fetch_add(1, Ordering::SeqCst);

        // Create simplified backtrace (in real implementation, this would capture actual frames)
        let backtrace = [addr; 8]; // Placeholder

        let info = AllocationInfo {
            layout,
            backtrace,
            generation,
            timestamp: generation, // Use generation as timestamp
            tag,
        };

        self.allocations.insert(addr, info);

        // Update statistics
        self.tag_stats[tag as usize].record_allocation(layout.size());

        // Update counters
        let old_usage = self
            .current_memory_usage
            .fetch_add(layout.size(), Ordering::SeqCst);
        let new_usage = old_usage + layout.size();

        // Update peak if needed
        let mut current_peak = self.peak_memory_usage.load(Ordering::SeqCst);
        while new_usage > current_peak {
            match self.peak_memory_usage.compare_exchange_weak(
                current_peak,
                new_usage,
                Ordering::SeqCst,
                Ordering::Relaxed,
            ) {
                Ok(_) => break,
                Err(actual) => current_peak = actual,
            }
        }

        self.total_allocations.fetch_add(1, Ordering::SeqCst);
    }

    pub fn track_deallocation(&mut self, addr: usize) -> Option<AllocationInfo> {
        let info = self.allocations.remove(&addr)?;

        // Update statistics
        self.tag_stats[info.tag as usize].record_deallocation(info.layout.size());

        // Update counters
        self.current_memory_usage
            .fetch_sub(info.layout.size(), Ordering::SeqCst);
        self.total_deallocations.fetch_add(1, Ordering::SeqCst);

        Some(info)
    }

    pub fn get_leaks(&self) -> impl Iterator<Item = (&usize, &AllocationInfo)> {
        self.allocations.iter()
    }

    pub fn get_stats_by_tag(&self, tag: AllocationTag) -> &AllocationStats {
        &self.tag_stats[tag as usize]
    }

    pub fn get_overall_stats(&self) -> OverallStats {
        OverallStats {
            total_allocations: self.total_allocations.load(Ordering::SeqCst),
            total_deallocations: self.total_deallocations.load(Ordering::SeqCst),
            current_allocations: self.allocations.len(),
            current_memory_usage: self.current_memory_usage.load(Ordering::SeqCst),
            peak_memory_usage: self.peak_memory_usage.load(Ordering::SeqCst),
            generation: self.generation,
        }
    }
}

/// Per-tag allocation statistics
#[derive(Debug, Clone, Copy)]
pub struct AllocationStats {
    pub total_bytes_allocated: u64,
    pub total_bytes_deallocated: u64,
    pub current_bytes_in_use: u64,
    pub allocation_count: u64,
    pub deallocation_count: u64,
}

impl AllocationStats {
    pub const fn new() -> Self {
        Self {
            total_bytes_allocated: 0,
            total_bytes_deallocated: 0,
            current_bytes_in_use: 0,
            allocation_count: 0,
            deallocation_count: 0,
        }
    }

    pub fn record_allocation(&mut self, size: usize) {
        self.total_bytes_allocated += size as u64;
        self.current_bytes_in_use += size as u64;
        self.allocation_count += 1;
    }

    pub fn record_deallocation(&mut self, size: usize) {
        self.total_bytes_deallocated += size as u64;
        self.current_bytes_in_use = self.current_bytes_in_use.saturating_sub(size as u64);
        self.deallocation_count += 1;
    }
}

/// Overall memory statistics
#[derive(Debug, Clone)]
pub struct OverallStats {
    pub total_allocations: usize,
    pub total_deallocations: usize,
    pub current_allocations: usize,
    pub current_memory_usage: usize,
    pub peak_memory_usage: usize,
    pub generation: u64,
}

/// Global tracking state instance
static GLOBAL_TRACKING_STATE: SpinNoIrq<TrackingState> = SpinNoIrq::new(TrackingState::new());

/// Enable memory allocation tracking
pub fn enable_tracking() {
    TRACKING_ENABLED.store(true, Ordering::SeqCst);
    log::info!("Memory allocation tracking enabled");
}

/// Disable memory allocation tracking
pub fn disable_tracking() {
    TRACKING_ENABLED.store(false, Ordering::SeqCst);
    log::info!("Memory allocation tracking disabled");
}

/// Check if tracking is enabled
pub fn is_tracking_enabled() -> bool {
    TRACKING_ENABLED.load(Ordering::SeqCst)
}

/// Track an allocation
pub fn track_allocation(addr: NonNull<u8>, layout: Layout, tag: AllocationTag) {
    if !is_tracking_enabled() {
        return;
    }

    let mut state = GLOBAL_TRACKING_STATE.lock();
    state.track_allocation(addr.as_ptr() as usize, layout, tag);
}

/// Track a deallocation
pub fn track_deallocation(addr: NonNull<u8>) -> Option<AllocationInfo> {
    if !is_tracking_enabled() {
        return None;
    }

    let mut state = GLOBAL_TRACKING_STATE.lock();
    state.track_deallocation(addr.as_ptr() as usize)
}

/// Get current overall statistics
pub fn get_overall_stats() -> OverallStats {
    let state = GLOBAL_TRACKING_STATE.lock();
    state.get_overall_stats()
}

/// Get statistics for a specific tag
pub fn get_stats_by_tag(tag: AllocationTag) -> AllocationStats {
    let state = GLOBAL_TRACKING_STATE.lock();
    *state.get_stats_by_tag(tag)
}

/// Get memory leaks (current allocations)
pub fn get_memory_leaks() -> alloc::vec::Vec<(usize, AllocationInfo)> {
    let state = GLOBAL_TRACKING_STATE.lock();
    state
        .get_leaks()
        .map(|(addr, info)| (*addr, info.clone()))
        .collect()
}

/// Print memory usage report
pub fn print_memory_report() {
    let overall = get_overall_stats();

    log::info!("=== Memory Allocation Report ===");
    log::info!("Total allocations: {}", overall.total_allocations);
    log::info!("Total deallocations: {}", overall.total_deallocations);
    log::info!("Current allocations: {}", overall.current_allocations);
    log::info!(
        "Current memory usage: {} bytes",
        overall.current_memory_usage
    );
    log::info!("Peak memory usage: {} bytes", overall.peak_memory_usage);
    log::info!("Allocation generation: {}", overall.generation);

    // Print per-tag statistics
    for tag in [
        AllocationTag::Kernel,
        AllocationTag::Driver,
        AllocationTag::Network,
        AllocationTag::FileSystem,
        AllocationTag::Process,
        AllocationTag::VMMemory,
        AllocationTag::PageTable,
        AllocationTag::DMA,
        AllocationTag::Slab,
        AllocationTag::Buddy,
        AllocationTag::Unknown,
    ] {
        let stats = get_stats_by_tag(tag);
        if stats.allocation_count > 0 {
            log::info!(
                "{:?}: {} allocations, {} deallocations, {} bytes in use",
                tag,
                stats.allocation_count,
                stats.deallocation_count,
                stats.current_bytes_in_use
            );
        }
    }

    // Check for leaks
    let leaks = get_memory_leaks();
    if !leaks.is_empty() {
        log::warn!("Memory leaks detected: {} allocations", leaks.len());
        for (addr, info) in leaks.iter().take(10) {
            // Show first 10 leaks
            log::warn!(
                "Leak: addr={:#x}, size={} bytes, tag={:?}, generation={}",
                addr,
                info.layout.size(),
                info.tag,
                info.generation
            );
        }
        if leaks.len() > 10 {
            log::warn!("... and {} more leaks", leaks.len() - 10);
        }
    }

    log::info!("=== End Memory Report ===");
}

/// Macro for easy tracking with automatic tag detection
#[macro_export]
macro_rules! track_alloc {
    ($ptr:expr, $layout:expr) => {
        $crate::tracking::track_allocation($ptr, $layout, $crate::tracking::AllocationTag::Unknown)
    };
    ($ptr:expr, $layout:expr, $tag:expr) => {
        $crate::tracking::track_allocation($ptr, $layout, $tag)
    };
}

/// Macro for easy deallocation tracking
#[macro_export]
macro_rules! track_dealloc {
    ($ptr:expr) => {
        $crate::tracking::track_deallocation($ptr)
    };
}

/// Reset all tracking statistics and clear allocation records.
pub fn reset_tracking() {
    let mut state = GLOBAL_TRACKING_STATE.lock();
    *state = TrackingState::new();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_allocation_tag() {
        assert_eq!(AllocationTag::from_string("kernel"), AllocationTag::Kernel);
        assert_eq!(
            AllocationTag::from_string("unknown"),
            AllocationTag::Unknown
        );
    }

    #[test]
    fn test_allocation_stats() {
        let mut stats = AllocationStats::new();
        assert_eq!(stats.allocation_count, 0);

        stats.record_allocation(1024);
        assert_eq!(stats.allocation_count, 1);
        assert_eq!(stats.current_bytes_in_use, 1024);

        stats.record_deallocation(1024);
        assert_eq!(stats.deallocation_count, 1);
        assert_eq!(stats.current_bytes_in_use, 0);
    }

    #[test]
    fn test_tracking_state() {
        let mut state = TrackingState::new();
        let layout = Layout::from_size_align(1024, 8).unwrap();

        state.track_allocation(0x1000, layout, AllocationTag::Kernel);
        assert_eq!(state.allocations.len(), 1);

        let info = state.track_deallocation(0x1000).unwrap();
        assert_eq!(info.layout.size(), 1024);
        assert_eq!(info.tag, AllocationTag::Kernel);
        assert_eq!(state.allocations.len(), 0);
    }
}
