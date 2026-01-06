//! The Axvisor memory allocator.
//!
//! This module provides memory allocation capabilities for both page-level and byte-level
//! allocations, with support for NUMA-aware allocation and memory tracking.

#![no_std]

#[macro_use]
extern crate log;
extern crate alloc;

use core::{
    alloc::{GlobalAlloc, Layout},
    fmt,
    ptr::NonNull,
};

use axvisor_allocator::{AllocResult, PageAllocator};
use kspin::SpinNoIrq;
use strum::{IntoStaticStr, VariantArray};

const PAGE_SIZE: usize = 0x1000;
const MIN_HEAP_SIZE: usize = 0x8000; // 32 K

mod page;
pub use page::GlobalPage;

#[cfg(feature = "tracking")]
mod tracking;
#[cfg(feature = "tracking")]
pub use tracking::*;

/// Kinds of memory usage for tracking.
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, VariantArray, IntoStaticStr)]
pub enum UsageKind {
    /// Heap allocations made by kernel Rust code.
    RustHeap,
    /// Virtual memory, usually used for user space.
    VirtMem,
    /// Page cache for file systems.
    PageCache,
    /// Page tables.
    PageTable,
    /// DMA memory.
    Dma,
    /// Memory used by [`GlobalPage`].
    Global,
}

/// Statistics of memory usages.
#[derive(Clone, Copy)]
pub struct Usages([usize; UsageKind::VARIANTS.len()]);

impl Usages {
    const fn new() -> Self {
        Self([0; UsageKind::VARIANTS.len()])
    }

    fn alloc(&mut self, kind: UsageKind, size: usize) {
        self.0[kind as usize] += size;
    }

    fn dealloc(&mut self, kind: UsageKind, size: usize) {
        self.0[kind as usize] -= size;
    }
}

impl fmt::Debug for Usages {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut d = f.debug_struct("UsageStats");
        for &kind in UsageKind::VARIANTS {
            d.field(kind.into(), &self.0[kind as usize]);
        }
        d.finish()
    }
}

/// The global allocator used by ArceOS.
///
/// This is an adapter around the axvisor_allocator::GlobalAllocator that provides
/// compatibility with the original axalloc API.
pub struct GlobalAllocator {
    inner: axvisor_allocator::GlobalAllocator,
    usages: SpinNoIrq<Usages>,
}

impl Default for GlobalAllocator {
    fn default() -> Self {
        Self::new()
    }
}

impl GlobalAllocator {
    /// Creates an empty [`GlobalAllocator`].
    pub const fn new() -> Self {
        Self {
            inner: axvisor_allocator::GlobalAllocator::new(),
            usages: SpinNoIrq::new(Usages::new()),
        }
    }

    /// Returns the name of the allocator.
    pub const fn name(&self) -> &'static str {
        "axvisor allocator"
    }

    /// Initializes the allocator with the given region.
    pub fn init(&self, start_vaddr: usize, size: usize) {
        info!("axalloc: Initialize global memory allocator...");
        if let Err(e) = self.inner.init(start_vaddr, size) {
            panic!("Failed to initialize allocator: {:?}", e);
        }
    }

    /// Add the given region to the allocator.
    pub fn add_memory(&self, start_vaddr: usize, size: usize) -> AllocResult {
        self.inner.add_memory(start_vaddr, size)
    }

    /// Allocate arbitrary number of bytes. Returns the left bound of the
    /// allocated region.
    pub fn alloc(&self, layout: Layout) -> AllocResult<NonNull<u8>> {
        let result = self.inner.alloc(layout);
        if let Ok(_ptr) = result {
            self.usages.lock().alloc(UsageKind::RustHeap, layout.size());
        }
        result
    }

    /// Gives back the allocated region to the byte allocator.
    pub fn dealloc(&self, pos: NonNull<u8>, layout: Layout) {
        self.usages
            .lock()
            .dealloc(UsageKind::RustHeap, layout.size());
        self.inner.dealloc(pos, layout);
    }

    /// Allocates contiguous pages.
    pub fn alloc_pages(
        &self,
        num_pages: usize,
        align_pow2: usize,
        kind: UsageKind,
    ) -> AllocResult<usize> {
        let result = self.inner.alloc_pages(num_pages, align_pow2);
        if let Ok(_addr) = result {
            let size = num_pages * PAGE_SIZE;
            self.usages.lock().alloc(kind, size);
        }
        result
    }

    /// Allocates contiguous pages starting from the given address.
    pub fn alloc_pages_at(
        &mut self,
        start: usize,
        num_pages: usize,
        align_pow2: usize,
        kind: UsageKind,
    ) -> AllocResult<usize> {
        let result = self.inner.alloc_pages_at(start, num_pages, align_pow2);
        if let Ok(_addr) = result {
            let size = num_pages * PAGE_SIZE;
            self.usages.lock().alloc(kind, size);
        }
        result
    }

    /// Gives back the allocated pages starts from `pos` to the page allocator.
    pub fn dealloc_pages(&self, pos: usize, num_pages: usize, kind: UsageKind) {
        let size = num_pages * PAGE_SIZE;
        self.usages.lock().dealloc(kind, size);
        self.inner.dealloc_pages(pos, num_pages);
    }

    /// Returns the number of allocated bytes in the byte allocator.
    pub fn used_bytes(&self) -> usize {
        let stats = self.inner.get_stats();
        stats.heap_bytes + stats.slab_bytes
    }

    /// Returns the number of available bytes in the byte allocator.
    pub fn available_bytes(&self) -> usize {
        // The new allocator doesn't have this exact method, so we approximate
        let stats = self.inner.get_stats();
        stats.free_pages * PAGE_SIZE
    }

    /// Returns the number of allocated pages in the page allocator.
    pub fn used_pages(&self) -> usize {
        let stats = self.inner.get_stats();
        stats.used_pages
    }

    /// Returns the number of available pages in the page allocator.
    pub fn available_pages(&self) -> usize {
        let stats = self.inner.get_stats();
        stats.free_pages
    }

    /// Returns the usage statistics of the allocator.
    pub fn usages(&self) -> Usages {
        *self.usages.lock()
    }
}

unsafe impl GlobalAlloc for GlobalAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let inner = move || {
            if let Ok(ptr) = GlobalAllocator::alloc(self, layout) {
                ptr.as_ptr()
            } else {
                alloc::alloc::handle_alloc_error(layout)
            }
        };
        inner()
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        let ptr = NonNull::new(ptr).expect("dealloc null ptr");
        let inner = || GlobalAllocator::dealloc(self, ptr, layout);
        inner();
    }
}

#[cfg_attr(all(target_os = "none", not(test)), global_allocator)]
static GLOBAL_ALLOCATOR: GlobalAllocator = GlobalAllocator::new();

/// Returns the reference to the global allocator.
pub fn global_allocator() -> &'static GlobalAllocator {
    &GLOBAL_ALLOCATOR
}

/// Initializes the global allocator with the given memory region.
///
/// Note that the memory region bounds are just numbers, and the allocator
/// does not actually access the region. Users should ensure that the region
/// is valid and not being used by others, so that the allocated memory is also
/// valid.
///
/// This function should be called only once, and before any allocation.
pub fn global_init(start_vaddr: usize, size: usize) {
    info!(
        "initialize global allocator at: [{:#x}, {:#x})",
        start_vaddr,
        start_vaddr + size
    );
    GLOBAL_ALLOCATOR.init(start_vaddr, size);
    info!("global allocator initialized");
}

/// Add the given memory region to the global allocator.
///
/// Users should ensure that the region is valid and not being used by others,
/// so that the allocated memory is also valid.
///
/// It's similar to [`global_init`], but can be called multiple times.
pub fn global_add_memory(start_vaddr: usize, size: usize) -> AllocResult {
    debug!(
        "add a memory region to global allocator: [{:#x}, {:#x})",
        start_vaddr,
        start_vaddr + size
    );
    GLOBAL_ALLOCATOR.add_memory(start_vaddr, size)
}
