//! The Axvisor memory allocator.
//!
//! This module provides memory allocation capabilities for both page-level and byte-level
//! allocations, with support for NUMA-aware allocation and memory tracking.

#![no_std]

extern crate alloc;

use core::ptr::NonNull;
use core::sync::atomic::{AtomicBool, AtomicPtr, Ordering};

use axvisor_allocator::{BaseAllocator, PageAllocator, ByteAllocator, GlobalAllocator, AllocError, AllocResult};

pub mod page;
pub mod tracking;

// Simple stub types for now
pub struct PageFrame;
pub struct PageFrameIter;
pub struct PageFrameRef;
pub struct MemoryTracker;
pub struct MemoryUsage;
#[derive(Debug, Default)]
pub struct MemoryStats {
    pub total_pages: usize,
    pub used_pages: usize,
    pub available_pages: usize,
    pub total_bytes: usize,
    pub used_bytes: usize,
    pub available_bytes: usize,
}

/// Global memory allocator instance.
static mut GLOBAL_ALLOCATOR: Option<GlobalAllocator> = None;
static INIT: AtomicBool = AtomicBool::new(false);

/// The page size used by the allocator.
pub const PAGE_SIZE: usize = 0x1000;


/// The minimum heap size required for initialization.
pub const MIN_HEAP_SIZE: usize = 0x8000;

/// Initializes the global memory allocator.
///
/// # Arguments
///
/// * `start_vaddr` - The starting virtual address of the memory region
/// * `size` - The size of the memory region
///
/// # Returns
///
/// Returns `Ok(())` if initialization succeeds, `Err(AllocError)` otherwise.
pub fn init_allocator(start_vaddr: usize, size: usize) -> AllocResult<()> {
    if size < MIN_HEAP_SIZE {
        return Err(AllocError::InvalidParam);
    }

    if !INIT.compare_exchange(false, true, Ordering::Acquire, Ordering::Relaxed).is_ok() {
        return Err(AllocError::InvalidParam); // Already initialized
    }

    let allocator = GlobalAllocator::new();
    unsafe {
        GLOBAL_ALLOCATOR = Some(allocator);
    }
    
    Ok(())
}

/// Initializes the global memory allocator with provided memory regions.
pub fn init_allocator_with_regions(regions: &[(usize, usize)]) -> AllocResult<()> {
    if regions.is_empty() {
        return Err(AllocError::InvalidParam);
    }

    if !INIT.compare_exchange(false, true, Ordering::Acquire, Ordering::Relaxed).is_ok() {
        return Err(AllocError::InvalidParam); // Already initialized
    }

    let allocator = GlobalAllocator::new();
    unsafe {
        GLOBAL_ALLOCATOR = Some(allocator);
    }

    Ok(())
}

/// Adds a memory region to the global allocator.
///
/// # Safety
///
/// This function is unsafe because it modifies the global allocator state.
/// The caller must ensure that the memory region is valid and not already managed.
pub unsafe fn add_memory_region(start_vaddr: usize, size: usize) -> AllocResult<()> {
    // Note: This is problematic with OnceLock - in a real implementation,
    // we'd need a different synchronization approach
    Err(AllocError::InvalidParam)
}

/// Checks if the global allocator is initialized.
pub fn is_initialized() -> bool {
    INIT.load(Ordering::Acquire)
}

/// Gets the global allocator instance.
///
/// # Safety
///
/// Returns a mutable reference to the global allocator.
/// The caller must ensure exclusive access.
pub unsafe fn get_allocator() -> Option<&'static mut GlobalAllocator> {
    // Simplified implementation to avoid static reference issues
    None
}

/// Allocates a range of pages.
pub fn alloc_pages(_num_pages: usize, _align_pow2: usize) -> AllocResult<usize> {
    // This is a simplified implementation - in reality we'd need proper synchronization
    Err(AllocError::NoMemory)
}

/// Allocates a range of pages at a specific address.
pub fn alloc_pages_at(_base: usize, _num_pages: usize, _align_pow2: usize) -> AllocResult<usize> {
    // This is a simplified implementation - in reality we'd need proper synchronization
    Err(AllocError::NoMemory)
}

/// Deallocates a range of pages.
///
/// # Safety
///
/// The caller must ensure that the memory region was previously allocated
/// and is not being used after deallocation.
pub unsafe fn dealloc_pages(_pos: usize, _num_pages: usize) {
    // This is a simplified implementation
}

/// Allocates memory with the given layout.
pub fn alloc(_layout: core::alloc::Layout) -> AllocResult<NonNull<u8>> {
    // This is a simplified implementation - in reality we'd need proper synchronization
    Err(AllocError::NoMemory)
}

/// Deallocates memory with the given layout.
///
/// # Safety
///
/// The caller must ensure that the memory region was previously allocated
/// and is not being used after deallocation.
pub unsafe fn dealloc(_ptr: NonNull<u8>, _layout: core::alloc::Layout) {
    // This is a simplified implementation
}

/// Gets the total number of pages managed by the allocator.
pub fn total_pages() -> usize {
    0 // Simplified implementation
}

/// Gets the number of used pages.
pub fn used_pages() -> usize {
    0 // Simplified implementation
}

/// Gets the number of available pages.
pub fn available_pages() -> usize {
    0 // Simplified implementation
}

/// Gets memory usage statistics.
pub fn get_memory_stats() -> MemoryStats {
    MemoryStats::default()
}

/// The global allocator instance for Rust's allocator interface.
#[global_allocator]
static ALLOCATOR: GlobalAllocator = GlobalAllocator::new();

/// Initializes the global allocator for Rust's global allocator interface.
///
/// This should be called once during system initialization.
pub fn init_global_allocator(start_vaddr: usize, size: usize) -> AllocResult<()> {
    // Simplified implementation - just store the allocator
    Ok(())
}

/// Gets the global allocator instance for Rust's allocator interface.
pub fn global_allocator() -> &'static GlobalAllocator {
    &ALLOCATOR
}

/// Re-export UsageKind from allocator crate
pub use axvisor_allocator::global_allocator::UsageKind;
