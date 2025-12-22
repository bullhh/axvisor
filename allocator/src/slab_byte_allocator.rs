//! Slab byte allocator implementation for Axvisor.
//! 
//! This module implements a simplified slab allocator for small object allocation
//! with size classes and page-level backing.

extern crate alloc;

use core::alloc::Layout;
use core::ptr::NonNull;
use crate::{AllocError, AllocResult, BaseAllocator, ByteAllocator};

const PAGE_SIZE: usize = 0x1000;
const MAX_OBJ_SIZE: usize = 2048;

/// Size classes for slab allocation
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(usize)]
pub enum SizeClass {
    Bytes8 = 8,
    Bytes16 = 16,
    Bytes32 = 32,
    Bytes64 = 64,
    Bytes128 = 128,
    Bytes256 = 256,
    Bytes512 = 512,
    Bytes1024 = 1024,
    Bytes2048 = 2048,
}

impl SizeClass {
    pub const COUNT: usize = 9;

    pub fn from_layout(layout: Layout) -> Option<Self> {
        if layout.size() > MAX_OBJ_SIZE {
            return None;
        }

        let size_class = match layout.size() {
            0..=8 => SizeClass::Bytes8,
            9..=16 => SizeClass::Bytes16,
            17..=32 => SizeClass::Bytes32,
            33..=64 => SizeClass::Bytes64,
            65..=128 => SizeClass::Bytes128,
            129..=256 => SizeClass::Bytes256,
            257..=512 => SizeClass::Bytes512,
            513..=1024 => SizeClass::Bytes1024,
            1025..=2048 => SizeClass::Bytes2048,
            _ => return None,
        };

        Some(size_class)
    }

    pub fn size(&self) -> usize {
        *self as usize
    }

    pub fn objects_per_page(&self) -> usize {
        PAGE_SIZE / self.size()
    }
}

/// Slab metadata
#[derive(Debug)]
pub struct SlabMeta {
    pub size_class: SizeClass,
    pub in_use: u32,
    pub total: u32,
}

impl SlabMeta {
    pub fn new(size_class: SizeClass, total: u32) -> Self {
        Self {
            size_class,
            in_use: 0,
            total,
        }
    }
}

/// Page allocator trait for slab allocator
pub trait PageAllocatorForSlab {
    fn alloc_pages(&mut self, num_pages: usize, align_pow2: usize) -> AllocResult<usize>;
    fn dealloc_pages(&mut self, pos: usize, num_pages: usize);
}

/// Simple slab cache for each size class
#[derive(Debug)]
struct SimpleSlabCache {
    size_class: SizeClass,
    free_objects: alloc::collections::LinkedList<NonNull<u8>>,
}

// SAFETY: SimpleSlabCache is only accessed through SlabByteAllocator which is protected by locks
unsafe impl Send for SimpleSlabCache {}
unsafe impl Sync for SimpleSlabCache {}

impl SimpleSlabCache {
    pub const fn new(size_class: SizeClass) -> Self {
        Self {
            size_class,
            free_objects: alloc::collections::LinkedList::new(),
        }
    }

    fn alloc(&mut self) -> AllocResult<NonNull<u8>> {
        if let Some(obj) = self.free_objects.pop_front() {
            Ok(obj)
        } else {
            Err(AllocError::NoMemory)
        }
    }

    fn dealloc(&mut self, ptr: NonNull<u8>) {
        self.free_objects.push_back(ptr);
    }
}

/// Simplified slab byte allocator
pub struct SlabByteAllocator {
    global_caches: [SimpleSlabCache; SizeClass::COUNT],
    page_allocator: Option<*mut dyn PageAllocatorForSlab>,
    total_bytes: usize,
    used_bytes: usize,
}

// SAFETY: SlabByteAllocator is used behind SpinNoIrq locks which provide synchronization
unsafe impl Send for SlabByteAllocator {}
unsafe impl Sync for SlabByteAllocator {}

impl SlabByteAllocator {
    pub const fn new() -> Self {
        Self {
            global_caches: [
                SimpleSlabCache::new(SizeClass::Bytes8),
                SimpleSlabCache::new(SizeClass::Bytes16),
                SimpleSlabCache::new(SizeClass::Bytes32),
                SimpleSlabCache::new(SizeClass::Bytes64),
                SimpleSlabCache::new(SizeClass::Bytes128),
                SimpleSlabCache::new(SizeClass::Bytes256),
                SimpleSlabCache::new(SizeClass::Bytes512),
                SimpleSlabCache::new(SizeClass::Bytes1024),
                SimpleSlabCache::new(SizeClass::Bytes2048),
            ],
            page_allocator: None,
            total_bytes: 0,
            used_bytes: 0,
        }
    }

    pub fn set_page_allocator(&mut self, page_allocator: *mut dyn PageAllocatorForSlab) {
        self.page_allocator = Some(page_allocator);
    }

    fn create_slab(&mut self, size_class: SizeClass) -> AllocResult<()> {
        let Some(page_allocator_ptr) = self.page_allocator else {
            return Err(AllocError::NoMemory);
        };

        let page_allocator = unsafe { &mut *page_allocator_ptr };
        let page_addr = page_allocator.alloc_pages(1, PAGE_SIZE)?;
        
        // Initialize free objects in the slab
        let obj_size = size_class.size();
        let objects_per_page = size_class.objects_per_page();
        
        for i in 0..objects_per_page {
            let obj_addr = page_addr + i * obj_size;
            let obj_ptr = unsafe { NonNull::new_unchecked(obj_addr as *mut u8) };
            
            let idx = size_class as usize / 8 - 1; // Convert to index
            if idx < SizeClass::COUNT {
                self.global_caches[idx].dealloc(obj_ptr);
            }
        }

        self.total_bytes += PAGE_SIZE;
        Ok(())
    }

    fn alloc_from_global(&mut self, size_class: SizeClass) -> AllocResult<NonNull<u8>> {
        let idx = match size_class {
            SizeClass::Bytes8 => 0,
            SizeClass::Bytes16 => 1,
            SizeClass::Bytes32 => 2,
            SizeClass::Bytes64 => 3,
            SizeClass::Bytes128 => 4,
            SizeClass::Bytes256 => 5,
            SizeClass::Bytes512 => 6,
            SizeClass::Bytes1024 => 7,
            SizeClass::Bytes2048 => 8,
        };

        // Try to allocate from existing cache
        if let Ok(obj) = self.global_caches[idx].alloc() {
            return Ok(obj);
        }

        // Create a new slab if needed
        if self.create_slab(size_class).is_ok() {
            // Try again after creating slab
            self.global_caches[idx].alloc()
        } else {
            Err(AllocError::NoMemory)
        }
    }
}

impl Default for SlabByteAllocator {
    fn default() -> Self {
        Self::new()
    }
}

impl BaseAllocator for SlabByteAllocator {
    fn init(&mut self, start: usize, size: usize) {
        self.total_bytes = size;
        self.used_bytes = 0;
    }

    fn add_memory(&mut self, _start: usize, size: usize) -> AllocResult {
        self.total_bytes += size;
        Ok(())
    }
}

impl ByteAllocator for SlabByteAllocator {
    fn alloc(&mut self, layout: Layout) -> AllocResult<NonNull<u8>> {
        let size_class = SizeClass::from_layout(layout).ok_or(AllocError::InvalidParam)?;
        self.alloc_from_global(size_class)
    }

    fn dealloc(&mut self, ptr: NonNull<u8>, layout: Layout) {
        let size_class = SizeClass::from_layout(layout).unwrap_or(SizeClass::Bytes8);
        let idx = match size_class {
            SizeClass::Bytes8 => 0,
            SizeClass::Bytes16 => 1,
            SizeClass::Bytes32 => 2,
            SizeClass::Bytes64 => 3,
            SizeClass::Bytes128 => 4,
            SizeClass::Bytes256 => 5,
            SizeClass::Bytes512 => 6,
            SizeClass::Bytes1024 => 7,
            SizeClass::Bytes2048 => 8,
        };

        if idx < SizeClass::COUNT {
            self.global_caches[idx].dealloc(ptr);
            self.used_bytes = self.used_bytes.saturating_sub(layout.size());
        }
    }

    fn total_bytes(&self) -> usize {
        self.total_bytes
    }

    fn used_bytes(&self) -> usize {
        self.used_bytes
    }

    fn available_bytes(&self) -> usize {
        self.total_bytes - self.used_bytes
    }
}
