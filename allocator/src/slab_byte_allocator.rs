//! Slab byte allocator implementation for Axvisor.
//!
//! This module implements an improved slab allocator for small object allocation
//! with better design inspired by asterinas, featuring size classes and
//! efficient per-CPU caching simulation.

use core::alloc::Layout;
use core::ptr::NonNull;
use log::info;

use crate::{AllocError, AllocResult, BaseAllocator, ByteAllocator};

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

        // Check alignment requirement: slab object size must satisfy alignment
        // Use max(size, align) to select size class (like Asterinas)
        let required_size = layout.size().max(layout.align());

        if required_size > MAX_OBJ_SIZE {
            return None;
        }

        let size_class = match required_size {
            0..=8 => SizeClass::Bytes8,
            9..=16 => SizeClass::Bytes16,
            17..=32 => SizeClass::Bytes32,
            33..=64 => SizeClass::Bytes64,
            65..=128 => SizeClass::Bytes128,
            129..=256 => SizeClass::Bytes256,
            257..=512 => SizeClass::Bytes512,
            513..=1024 => SizeClass::Bytes1024,
            1025..=2048 => SizeClass::Bytes2048,
            _ => unreachable!(
                "Invalid layout: size={}, align={}. This should have been caught by global_allocator check.",
                layout.size(),
                layout.align()
            ),
        };

        Some(size_class)
    }

    pub fn size(&self) -> usize {
        *self as usize
    }

    pub fn objects_per_page(&self, page_size: usize) -> usize {
        page_size / self.size()
    }

    pub fn to_index(&self) -> usize {
        match self {
            SizeClass::Bytes8 => 0,
            SizeClass::Bytes16 => 1,
            SizeClass::Bytes32 => 2,
            SizeClass::Bytes64 => 3,
            SizeClass::Bytes128 => 4,
            SizeClass::Bytes256 => 5,
            SizeClass::Bytes512 => 6,
            SizeClass::Bytes1024 => 7,
            SizeClass::Bytes2048 => 8,
        }
    }

    pub fn from_index(index: usize) -> Option<Self> {
        match index {
            0 => Some(SizeClass::Bytes8),
            1 => Some(SizeClass::Bytes16),
            2 => Some(SizeClass::Bytes32),
            3 => Some(SizeClass::Bytes64),
            4 => Some(SizeClass::Bytes128),
            5 => Some(SizeClass::Bytes256),
            6 => Some(SizeClass::Bytes512),
            7 => Some(SizeClass::Bytes1024),
            8 => Some(SizeClass::Bytes2048),
            _ => None,
        }
    }
}

/// Slab metadata for each page
#[derive(Debug)]
pub struct SlabMeta {
    pub size_class: SizeClass,
    pub in_use: u32,
    pub total_objects: u32,
    pub free_bitmap: [u64; 8], // Bitmap for up to 512 objects (8 * 64 bits)
}

impl SlabMeta {
    pub fn new(size_class: SizeClass, page_size: usize) -> Self {
        let total_objects = size_class.objects_per_page(page_size) as u32;
        let mut free_bitmap = [0u64; 8];

        // Mark all objects as free
        for i in 0..total_objects as usize {
            let word_idx = i / 64;
            let bit_idx = i % 64;
            free_bitmap[word_idx] |= 1u64 << bit_idx;
        }

        Self {
            size_class,
            in_use: 0,
            total_objects,
            free_bitmap, // Mark all as free
        }
    }

    pub fn alloc_object(&mut self) -> Option<usize> {
        // Find first free bit
        let mut free_pos = None;

        for (word_idx, &word) in self.free_bitmap.iter().enumerate() {
            if word != 0 {
                let bit_pos = word.trailing_zeros() as usize;
                free_pos = Some(word_idx * 64 + bit_pos);
                break;
            }
        }

        let Some(free_pos) = free_pos else {
            return None; // No free objects
        };

        if free_pos >= self.total_objects as usize {
            return None;
        }

        // Mark as used
        let word_idx = free_pos / 64;
        let bit_idx = free_pos % 64;
        self.free_bitmap[word_idx] &= !(1u64 << bit_idx);
        self.in_use += 1;
        Some(free_pos)
    }

    pub fn dealloc_object(&mut self, object_index: usize) {
        if object_index < self.total_objects as usize {
            let word_idx = object_index / 64;
            let bit_idx = object_index % 64;
            self.free_bitmap[word_idx] |= 1u64 << bit_idx;
            self.in_use = self.in_use.saturating_sub(1);
        }
    }

    pub fn is_full(&self) -> bool {
        // Check if all bits are 0
        self.free_bitmap.iter().all(|&word| word == 0)
    }

    pub fn is_empty(&self) -> bool {
        self.in_use == 0
    }

    pub fn free_count(&self) -> u32 {
        self.total_objects - self.in_use
    }
}

/// Slab page representation
#[derive(Debug)]
pub struct SlabPage {
    pub addr: usize,
    pub meta: SlabMeta,
}

impl SlabPage {
    pub fn new(addr: usize, size_class: SizeClass, page_size: usize) -> Self {
        Self {
            addr,
            meta: SlabMeta::new(size_class, page_size),
        }
    }

    pub fn object_addr(&self, object_index: usize, page_size: usize) -> usize {
        // Ensure object_index is within bounds
        debug_assert!(object_index < self.meta.total_objects as usize);
        debug_assert!(object_index * self.meta.size_class.size() < page_size);
        self.addr + object_index * self.meta.size_class.size()
    }

    pub fn object_index_from_addr(&self, obj_addr: usize, page_size: usize) -> Option<usize> {
        if obj_addr < self.addr || obj_addr >= self.addr + page_size {
            return None;
        }

        let offset = obj_addr - self.addr;
        if offset % self.meta.size_class.size() != 0 {
            return None;
        }

        Some(offset / self.meta.size_class.size())
    }
}

/// Page allocator trait for slab allocator
pub trait PageAllocatorForSlab {
    fn alloc_pages(&mut self, num_pages: usize, alignment: usize) -> AllocResult<usize>;
    fn dealloc_pages(&mut self, pos: usize, num_pages: usize);
}

/// Maximum number of slab pages per size class
const MAX_SLAB_PAGES: usize = 320;

/// Slab cache for each size class
#[derive(Debug)]
pub struct SlabCache {
    size_class: SizeClass,
    full_pages: [Option<SlabPage>; MAX_SLAB_PAGES],
    partial_pages: [Option<SlabPage>; MAX_SLAB_PAGES],
    free_pages: [Option<SlabPage>; MAX_SLAB_PAGES],
    full_count: usize,
    partial_count: usize,
    free_count: usize,
}

impl SlabCache {
    pub const fn new(size_class: SizeClass) -> Self {
        Self {
            size_class,
            full_pages: [const { None }; MAX_SLAB_PAGES],
            partial_pages: [const { None }; MAX_SLAB_PAGES],
            free_pages: [const { None }; MAX_SLAB_PAGES],
            full_count: 0,
            partial_count: 0,
            free_count: 0,
        }
    }

    pub fn alloc_object<const PAGE_SIZE: usize>(
        &mut self,
        page_allocator: &mut dyn PageAllocatorForSlab,
    ) -> AllocResult<usize> {
        // Try to allocate from partial pages first
        for i in 0..self.partial_count {
            if let Some(ref mut page) = &mut self.partial_pages[i] {
                if let Some(object_index) = page.meta.alloc_object() {
                    let obj_addr = page.object_addr(object_index, PAGE_SIZE);

                    // Move page if it became full
                    if page.meta.is_full() {
                        self.move_to_full(i);
                    }

                    return Ok(obj_addr);
                }
            }
        }

        // Try to allocate from free pages
        for i in 0..self.free_count {
            if let Some(ref mut page) = &mut self.free_pages[i] {
                if let Some(object_index) = page.meta.alloc_object() {
                    let obj_addr = page.object_addr(object_index, PAGE_SIZE);

                    // Move page to partial
                    self.move_from_free_to_partial(i);
                    return Ok(obj_addr);
                }
            }
        }

        // Need to allocate a new page
        let new_page_addr = page_allocator.alloc_pages(1, PAGE_SIZE)?;
        let mut new_page = SlabPage::new(new_page_addr, self.size_class, PAGE_SIZE);

        if let Some(object_index) = new_page.meta.alloc_object() {
            let obj_addr = new_page.object_addr(object_index, PAGE_SIZE);

            // Add to partial pages
            if self.partial_count < MAX_SLAB_PAGES {
                self.partial_pages[self.partial_count] = Some(new_page);
                self.partial_count += 1;
                Ok(obj_addr)
            } else {
                // No space for new page, deallocate it
                info!("slab allocator: No space for new page, deallocating it");
                info!(
                    "slab allocator: partial_count: {:?}, free_count: {:?}, full_count: {:?}  ",
                    self.partial_count, self.free_count, self.full_count
                );
                page_allocator.dealloc_pages(new_page_addr, 1);
                Err(AllocError::NoMemory)
            }
        } else {
            // This shouldn't happen with a fresh page
            info!("slab allocator: New page is full, deallocating it");
            page_allocator.dealloc_pages(new_page_addr, 1);
            Err(AllocError::NoMemory)
        }
    }

    pub fn dealloc_object<const PAGE_SIZE: usize>(&mut self, obj_addr: usize) -> Result<(), ()> {
        // Try to find in partial pages
        for i in 0..self.partial_count {
            if let Some(ref mut page) = &mut self.partial_pages[i] {
                if let Some(object_index) = page.object_index_from_addr(obj_addr, PAGE_SIZE) {
                    page.meta.dealloc_object(object_index);

                    // Move to free if empty
                    if page.meta.is_empty() {
                        self.move_from_partial_to_free(i);
                    }

                    return Ok(());
                }
            }
        }

        // Try to find in full pages
        for i in 0..self.full_count {
            if let Some(ref mut page) = &mut self.full_pages[i] {
                if let Some(object_index) = page.object_index_from_addr(obj_addr, PAGE_SIZE) {
                    page.meta.dealloc_object(object_index);

                    // Move to partial
                    self.move_from_full_to_partial(i);
                    return Ok(());
                }
            }
        }

        Err(())
    }

    fn move_to_full(&mut self, partial_index: usize) {
        if partial_index >= self.partial_count || self.full_count >= MAX_SLAB_PAGES {
            return;
        }

        let page = self.partial_pages[partial_index].take();

        // Shift remaining partial pages
        for i in partial_index..self.partial_count - 1 {
            self.partial_pages[i] = self.partial_pages[i + 1].take();
        }
        self.partial_pages[self.partial_count - 1] = None;
        self.partial_count -= 1;

        // Add to full pages
        self.full_pages[self.full_count] = page;
        self.full_count += 1;
    }

    fn move_from_full_to_partial(&mut self, full_index: usize) {
        if full_index >= self.full_count || self.partial_count >= MAX_SLAB_PAGES {
            return;
        }

        let page = self.full_pages[full_index].take();

        // Shift remaining full pages
        for i in full_index..self.full_count - 1 {
            self.full_pages[i] = self.full_pages[i + 1].take();
        }
        self.full_pages[self.full_count - 1] = None;
        self.full_count -= 1;

        // Add to partial pages
        self.partial_pages[self.partial_count] = page;
        self.partial_count += 1;
    }

    fn move_from_free_to_partial(&mut self, free_index: usize) {
        if free_index >= self.free_count || self.partial_count >= MAX_SLAB_PAGES {
            return;
        }

        let page = self.free_pages[free_index].take();

        // Shift remaining free pages
        for i in free_index..self.free_count - 1 {
            self.free_pages[i] = self.free_pages[i + 1].take();
        }
        self.free_pages[self.free_count - 1] = None;
        self.free_count -= 1;

        // Add to partial pages
        self.partial_pages[self.partial_count] = page;
        self.partial_count += 1;
    }

    fn move_from_partial_to_free(&mut self, partial_index: usize) {
        if partial_index >= self.partial_count || self.free_count >= MAX_SLAB_PAGES {
            return;
        }

        let page = self.partial_pages[partial_index].take();

        // Shift remaining partial pages
        for i in partial_index..self.partial_count - 1 {
            self.partial_pages[i] = self.partial_pages[i + 1].take();
        }
        self.partial_pages[self.partial_count - 1] = None;
        self.partial_count -= 1;

        // Add to free pages
        self.free_pages[self.free_count] = page;
        self.free_count += 1;
    }
}

/// Improved slab byte allocator
pub struct SlabByteAllocator<const PAGE_SIZE: usize = { crate::DEFAULT_PAGE_SIZE }> {
    caches: [SlabCache; SizeClass::COUNT],
    page_allocator: Option<*mut dyn PageAllocatorForSlab>,
    total_bytes: usize,
    allocated_bytes: usize,
    total_objects: usize,
    allocated_objects: usize,
}

// SAFETY: SlabByteAllocator is used behind SpinNoIrq locks which provide synchronization
unsafe impl<const PAGE_SIZE: usize> Send for SlabByteAllocator<PAGE_SIZE> {}
unsafe impl<const PAGE_SIZE: usize> Sync for SlabByteAllocator<PAGE_SIZE> {}

impl<const PAGE_SIZE: usize> SlabByteAllocator<PAGE_SIZE> {
    pub const fn new() -> Self {
        Self {
            caches: [
                SlabCache::new(SizeClass::Bytes8),
                SlabCache::new(SizeClass::Bytes16),
                SlabCache::new(SizeClass::Bytes32),
                SlabCache::new(SizeClass::Bytes64),
                SlabCache::new(SizeClass::Bytes128),
                SlabCache::new(SizeClass::Bytes256),
                SlabCache::new(SizeClass::Bytes512),
                SlabCache::new(SizeClass::Bytes1024),
                SlabCache::new(SizeClass::Bytes2048),
            ],
            page_allocator: None,
            total_bytes: 0,
            allocated_bytes: 0,
            total_objects: 0,
            allocated_objects: 0,
        }
    }

    pub fn set_page_allocator(&mut self, page_allocator: *mut dyn PageAllocatorForSlab) {
        self.page_allocator = Some(page_allocator);
    }

    pub fn get_cache(&self, size_class: SizeClass) -> &SlabCache {
        &self.caches[size_class.to_index()]
    }

    pub fn get_cache_mut(&mut self, size_class: SizeClass) -> &mut SlabCache {
        &mut self.caches[size_class.to_index()]
    }
}

impl<const PAGE_SIZE: usize> Default for SlabByteAllocator<PAGE_SIZE> {
    fn default() -> Self {
        Self::new()
    }
}

impl<const PAGE_SIZE: usize> BaseAllocator for SlabByteAllocator<PAGE_SIZE> {
    fn init(&mut self, _start: usize, size: usize) {
        self.total_bytes = size;
        self.allocated_bytes = 0;
        self.total_objects = 0;
        self.allocated_objects = 0;
    }

    fn add_memory(&mut self, _start: usize, size: usize) -> AllocResult {
        self.total_bytes += size;
        Ok(())
    }
}

impl<const PAGE_SIZE: usize> ByteAllocator for SlabByteAllocator<PAGE_SIZE> {
    fn alloc(&mut self, layout: Layout) -> AllocResult<NonNull<u8>> {
        let size_class = SizeClass::from_layout(layout).ok_or(AllocError::InvalidParam)?;

        let Some(page_allocator_ptr) = self.page_allocator else {
            return Err(AllocError::NoMemory);
        };

        let page_allocator = unsafe { &mut *page_allocator_ptr };
        let cache = self.get_cache_mut(size_class);

        let obj_addr = cache.alloc_object::<PAGE_SIZE>(page_allocator)?;
        self.allocated_bytes += layout.size();
        self.allocated_objects += 1;

        Ok(unsafe { NonNull::new_unchecked(obj_addr as *mut u8) })
    }

    fn dealloc(&mut self, ptr: NonNull<u8>, layout: Layout) {
        let size_class = SizeClass::from_layout(layout).expect(
            "Invalid layout for slab dealloc. Layout should have been validated by global_allocator.",
        );
        let obj_addr = ptr.as_ptr() as usize;

        let cache = self.get_cache_mut(size_class);

        // This memory must be owned by slab allocator
        // If dealloc_object fails (not found), it's a critical error
        if cache.dealloc_object::<PAGE_SIZE>(obj_addr).is_err() {
            panic!(
                "Failed to dealloc address {:#x} from slab allocator. This address was not allocated by slab. Layout: size={}, align={}",
                obj_addr, layout.size(), layout.align()
            );
        }

        self.allocated_bytes = self.allocated_bytes.saturating_sub(layout.size());
        self.allocated_objects = self.allocated_objects.saturating_sub(1);
    }

    fn total_bytes(&self) -> usize {
        self.total_bytes
    }

    fn used_bytes(&self) -> usize {
        self.allocated_bytes
    }

    fn available_bytes(&self) -> usize {
        self.total_bytes - self.allocated_bytes
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_size_class() {
        assert_eq!(
            SizeClass::from_layout(Layout::from_size_align(8, 8).unwrap()),
            Some(SizeClass::Bytes8)
        );
        assert_eq!(
            SizeClass::from_layout(Layout::from_size_align(16, 8).unwrap()),
            Some(SizeClass::Bytes16)
        );
        assert_eq!(
            SizeClass::from_layout(Layout::from_size_align(2048, 8).unwrap()),
            Some(SizeClass::Bytes2048)
        );
        assert_eq!(
            SizeClass::from_layout(Layout::from_size_align(2049, 8).unwrap()),
            None
        );
    }

    #[test]
    fn test_slab_meta() {
        let mut meta = SlabMeta::new(SizeClass::Bytes64, crate::DEFAULT_PAGE_SIZE);
        assert_eq!(meta.total_objects, (crate::DEFAULT_PAGE_SIZE / 64) as u32);
        assert_eq!(meta.in_use, 0);
        assert!(!meta.is_full());
        assert!(meta.is_empty());

        // Test allocation
        let obj_idx = meta.alloc_object().unwrap();
        assert_eq!(obj_idx, 0);
        assert_eq!(meta.in_use, 1);
        assert!(!meta.is_full());
        assert!(!meta.is_empty());

        // Test deallocation
        meta.dealloc_object(obj_idx);
        assert_eq!(meta.in_use, 0);
        assert!(meta.is_empty());
    }
}
