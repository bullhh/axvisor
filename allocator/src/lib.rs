//! Axvisor Memory Allocator
//! 
//! This module implements a high-performance memory allocator for Axvisor hypervisor,
//! featuring:
//! - Buddy page allocator for page-level allocation
//! - Slab allocator for small object allocation
//! - Global allocator coordination
//! - Per-CPU caching support (future)

#![no_std]
#![cfg_attr(feature = "allocator_api", feature(allocator_api))]
#![feature(generic_const_exprs)]

extern crate alloc;

#[macro_use]
extern crate axlog;

use core::alloc::Layout;
use core::ptr::NonNull;

#[cfg(feature = "axerrno")]
use axerrno::AxError;

/// The error type used for allocation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AllocError {
    /// Invalid `size` or `align_pow2`. (e.g. unaligned)
    InvalidParam,
    /// Memory added by `add_memory` overlapped with existed memory.
    MemoryOverlap,
    /// No enough memory to allocate.
    NoMemory,
    /// Deallocate an unallocated memory region.
    NotAllocated,
}

#[cfg(feature = "axerrno")]
impl From<AllocError> for AxError {
    fn from(value: AllocError) -> Self {
        match value {
            AllocError::NoMemory => AxError::NoMemory,
            _ => AxError::InvalidInput,
        }
    }
}

/// A [`Result`] type with [`AllocError`] as the error type.
pub type AllocResult<T = ()> = Result<T, AllocError>;

/// The base allocator inherited by other allocators.
pub trait BaseAllocator {
    /// Initialize the allocator with a free memory region.
    fn init(&mut self, start: usize, size: usize);

    /// Add a free memory region to the allocator.
    fn add_memory(&mut self, start: usize, size: usize) -> AllocResult;
}

/// Byte-granularity allocator.
pub trait ByteAllocator: BaseAllocator {
    /// Allocate memory with the given size (in bytes) and alignment.
    fn alloc(&mut self, layout: Layout) -> AllocResult<NonNull<u8>>;

    /// Deallocate memory at the given position, size, and alignment.
    fn dealloc(&mut self, pos: NonNull<u8>, layout: Layout);

    /// Returns total memory size in bytes.
    fn total_bytes(&self) -> usize;

    /// Returns allocated memory size in bytes.
    fn used_bytes(&self) -> usize;

    /// Returns available memory size in bytes.
    fn available_bytes(&self) -> usize;
}

/// Page-granularity allocator.
pub trait PageAllocator: BaseAllocator {
    /// The size of a memory page.
    const PAGE_SIZE: usize;

    /// Allocate contiguous memory pages with given count and alignment.
    fn alloc_pages(&mut self, num_pages: usize, align_pow2: usize) -> AllocResult<usize> {
        self.alloc_pages_with_usage(num_pages, align_pow2, global_allocator::UsageKind::Other)
    }
    
    /// Deallocate contiguous memory pages with given position and count.
    fn dealloc_pages(&mut self, pos: usize, num_pages: usize) {
        self.dealloc_pages_with_usage(pos, num_pages, global_allocator::UsageKind::Other)
    }

    /// Allocate contiguous memory pages with given count, alignment, and usage kind.
    fn alloc_pages_with_usage(&mut self, num_pages: usize, align_pow2: usize, usage: global_allocator::UsageKind) -> AllocResult<usize>;

    /// Deallocate contiguous memory pages with given position, count, and usage kind.
    fn dealloc_pages_with_usage(&mut self, pos: usize, num_pages: usize, usage: global_allocator::UsageKind);

    /// Allocate contiguous memory pages with given base address, count and alignment.
    fn alloc_pages_at(
        &mut self,
        base: usize,
        num_pages: usize,
        align_pow2: usize,
    ) -> AllocResult<usize>;

    /// Returns the total number of memory pages.
    fn total_pages(&self) -> usize;

    /// Returns the number of allocated memory pages.
    fn used_pages(&self) -> usize;

    /// Returns the number of available memory pages.
    fn available_pages(&self) -> usize;
}

/// Used to allocate unique IDs (e.g., thread ID).
pub trait IdAllocator: BaseAllocator {
    /// Allocate contiguous IDs with given count and alignment.
    fn alloc_id(&mut self, count: usize, align_pow2: usize) -> AllocResult<usize>;

    /// Deallocate contiguous IDs with given position and count.
    fn dealloc_id(&mut self, start_id: usize, count: usize);

    /// Whether the given `id` was allocated.
    fn is_allocated(&self, id: usize) -> bool;

    /// Mark the given `id` has been allocated and cannot be reallocated.
    fn alloc_fixed_id(&mut self, id: usize) -> AllocResult;

    /// Returns the maximum number of supported IDs.
    fn size(&self) -> usize;

    /// Returns the number of allocated IDs.
    fn used(&self) -> usize;

    /// Returns the number of available IDs.
    fn available(&self) -> usize;
}

#[inline]
#[allow(dead_code)]
const fn align_down(pos: usize, align: usize) -> usize {
    pos & !(align - 1)
}

#[inline]
#[allow(dead_code)]
const fn align_up(pos: usize, align: usize) -> usize {
    (pos + align - 1) & !(align - 1)
}

/// Checks whether the address has the demanded alignment.
///
/// Equivalent to `addr % align == 0`, but the alignment must be a power of two.
#[inline]
#[allow(dead_code)]
const fn is_aligned(base_addr: usize, align: usize) -> bool {
    base_addr & (align - 1) == 0
}

// Export our allocator implementations
pub mod buddy_page_allocator;
pub use buddy_page_allocator::{BuddyPageAllocator, BuddyStats, MemoryRegion};

pub mod slab_byte_allocator;
pub use slab_byte_allocator::{SlabByteAllocator, SizeClass, SlabMeta, PageAllocatorForSlab};

pub mod global_allocator;
pub use global_allocator::{GlobalAllocator, UsageStats};

#[cfg(feature = "allocator_api")]
mod allocator_api {
    use super::ByteAllocator;
    use alloc::rc::Rc;
    use core::alloc::{AllocError, Allocator, Layout};
    use core::cell::RefCell;
    use core::ptr::NonNull;

    /// A byte-allocator wrapped in [`Rc<RefCell>`] that implements [`core::alloc::Allocator`].
    pub struct AllocatorRc<A: ByteAllocator>(Rc<RefCell<A>>);

    impl<A: ByteAllocator> AllocatorRc<A> {
        /// Creates a new allocator with the given memory pool.
        pub fn new(mut inner: A, pool: &mut [u8]) -> Self {
            inner.init(pool.as_mut_ptr() as usize, pool.len());
            Self(Rc::new(RefCell::new(inner)))
        }
    }

    unsafe impl<A: ByteAllocator> Allocator for AllocatorRc<A> {
        fn allocate(&self, layout: Layout) -> Result<NonNull<[u8]>, AllocError> {
            match layout.size() {
                0 => Ok(NonNull::slice_from_raw_parts(NonNull::dangling(), 0)),
                size => {
                    let raw_addr = self.0.borrow_mut().alloc(layout).map_err(|_| AllocError)?;
                    Ok(NonNull::slice_from_raw_parts(raw_addr, size))
                }
            }
        }

        unsafe fn deallocate(&self, ptr: NonNull<u8>, layout: Layout) {
            self.0.borrow_mut().dealloc(ptr, layout)
        }
    }

    impl<A: ByteAllocator> Clone for AllocatorRc<A> {
        fn clone(&self) -> Self {
            Self(self.0.clone())
        }
    }
}

#[cfg(feature = "allocator_api")]
pub use allocator_api::AllocatorRc;
