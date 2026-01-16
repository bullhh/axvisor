//! Multi-zone bitmap allocator implementation.
//!
//! This allocator uses a bitmap stored in managed memory region to track
//! page allocation state. Each zone has its own bitmap at start of the region.
//! The bitmap size is dynamically calculated based on zone size.

use crate::{AllocError, AllocResult, PageAllocator};

/// Maximum number of memory zones supported
pub const MAX_ZONES: usize = 16;

/// A single memory zone managed by a bitmap
#[derive(Clone, Copy)]
pub struct BitmapZone<const PAGE_SIZE: usize> {
    /// Base address of zone (aligned)
    pub base_addr: usize,
    /// End address of zone (exclusive, aligned)
    pub end_addr: usize,
    /// Start address of allocatable pages (after bitmap)
    pub alloc_start: usize,
    /// Number of allocatable pages in this zone
    pub alloc_pages: usize,
    /// Base address of bitmap (in the zone)
    pub bitmap_base: usize,
    /// Size of bitmap in bytes
    pub bitmap_size: usize,
    /// Total number of pages in the zone (including bitmap pages)
    pub total_pages: usize,
}

impl<const PAGE_SIZE: usize> BitmapZone<PAGE_SIZE> {
    /// Create an empty zone
    pub const fn empty() -> Self {
        Self {
            base_addr: 0,
            end_addr: 0,
            alloc_start: 0,
            alloc_pages: 0,
            bitmap_base: 0,
            bitmap_size: 0,
            total_pages: 0,
        }
    }

    /// Calculate the metadata size (bitmap) needed for a given number of pages
    pub const fn calculate_metadata_size(total_pages: usize) -> usize {
        const BITS_PER_WORD: usize = core::mem::size_of::<usize>() * 8;
        let words_needed = (total_pages + BITS_PER_WORD - 1) / BITS_PER_WORD;
        let bytes_needed = words_needed * core::mem::size_of::<usize>();
        // Align up to page boundary
        ((bytes_needed + PAGE_SIZE - 1) / PAGE_SIZE) * PAGE_SIZE
    }

    /// Initialize the zone with a memory region
    ///
    /// # Safety
    /// Caller must ensure the memory region is valid and accessible.
    pub unsafe fn init(&mut self, start: usize, size: usize) {
        // Align start and size to page boundaries
        let aligned_start = start & !(PAGE_SIZE - 1);
        let aligned_end = (start + size + PAGE_SIZE - 1) & !(PAGE_SIZE - 1);
        let aligned_size = aligned_end - aligned_start;

        if aligned_size < PAGE_SIZE * 2 {
            // Need at least 2 pages: one for bitmap, one for allocation
            self.base_addr = 0;
            self.end_addr = 0;
            return;
        }

        let total_pages = aligned_size / PAGE_SIZE;
        let metadata_size = Self::calculate_metadata_size(total_pages);
        let metadata_pages = metadata_size / PAGE_SIZE;

        self.base_addr = aligned_start;
        self.end_addr = aligned_end;
        self.bitmap_base = aligned_start;
        self.bitmap_size = metadata_size;
        self.alloc_start = aligned_start + metadata_size;
        self.alloc_pages = total_pages - metadata_pages;
        self.total_pages = total_pages;

        // Initialize bitmap: mark all allocatable pages as free (bit = 1)
        let bitmap_ptr = self.bitmap_base as *mut usize;
        let words_count = self.bitmap_size / core::mem::size_of::<usize>();
        for i in 0..words_count {
            // Initialize all bits to 1 (free)
            bitmap_ptr.add(i).write_volatile(!0usize);
        }
    }

    /// Allocate contiguous pages from this zone
    ///
    /// Returns the physical address of the allocated pages.
    pub unsafe fn alloc_pages(&self, num_pages: usize, alignment: usize) -> AllocResult<usize> {
        if num_pages == 0 || num_pages > self.alloc_pages {
            log::info!("[BitmapZone] alloc_pages failed: num_pages={}, alloc_pages={}", num_pages, self.alloc_pages);
            return Err(AllocError::InvalidParam);
        }

        // Validate alignment is power of 2
        if alignment == 0 || (alignment & (alignment - 1)) != 0 {
            log::info!("[BitmapZone] alloc_pages failed: invalid alignment={}", alignment);
            return Err(AllocError::InvalidParam);
        }

        // Calculate alignment in pages (ensure power of 2)
        let align_pages = if alignment > PAGE_SIZE {
            alignment / PAGE_SIZE
        } else {
            1
        };

        // Find a contiguous free block
        if let Some(start_page) = self.find_contiguous_free_pages(num_pages, align_pages) {
            // Mark pages as allocated (clear bits)
            for i in 0..num_pages {
                self.clear_bit(start_page + i);
            }

            let addr = self.alloc_start + start_page * PAGE_SIZE;
            
            // Verify the returned address meets alignment requirement
            if addr & (alignment - 1) != 0 {
                log::info!("[BitmapZone] alloc_pages address misaligned: addr={:#x}, alignment={:#x}", addr, alignment);
                // Rollback allocation
                for i in 0..num_pages {
                    self.set_bit(start_page + i);
                }
                return Err(AllocError::NoMemory);
            }
            
            // log::info!("[BitmapZone] alloc_pages success: addr={:#x}, num_pages={}, alignment={:#x}", addr, num_pages, alignment);
            Ok(addr)
        } else {
            // log::info!("[BitmapZone] alloc_pages failed: no contiguous free pages, num_pages={}, align_pages={}", num_pages, align_pages);
            Err(AllocError::NoMemory)
        }
    }

    /// Deallocate contiguous pages
    pub unsafe fn dealloc_pages(&self, addr: usize, num_pages: usize) {
        if num_pages == 0 {
            log::info!("[BitmapZone] dealloc_pages: num_pages is 0, ignored");
            return;
        }

        // Validate address is within zone range
        if addr < self.alloc_start || addr >= self.end_addr {
            log::info!("[BitmapZone] dealloc_pages failed: addr={:#x} out of range [{:#x}, {:#x})", 
                addr, self.alloc_start, self.end_addr);
            return;
        }

        // Validate address is page-aligned
        if addr & (PAGE_SIZE - 1) != 0 {
            log::info!("[BitmapZone] dealloc_pages failed: addr={:#x} not page-aligned (PAGE_SIZE={:#x})", 
                addr, PAGE_SIZE);
            return;
        }

        let page_offset = (addr - self.alloc_start) / PAGE_SIZE;
        
        // Validate page range
        if page_offset + num_pages > self.alloc_pages {
            log::info!("[BitmapZone] dealloc_pages failed: page range out of bounds, offset={}, num_pages={}, alloc_pages={}", 
                page_offset, num_pages, self.alloc_pages);
            return;
        }

        // Mark pages as free (set bits)
        for i in 0..num_pages {
            self.set_bit(page_offset + i);
        }
        
        // log::info!("[BitmapZone] dealloc_pages success: addr={:#x}, num_pages={}", addr, num_pages);
    }

    /// Find a contiguous block of free pages with proper alignment
    unsafe fn find_contiguous_free_pages(&self, num_pages: usize, align_pages: usize) -> Option<usize> {
        // When alignment is required, we need to check if alloc_start itself is properly aligned
        let base_offset = if align_pages > 1 {
            let align_bytes = align_pages * PAGE_SIZE;
            let misalign = self.alloc_start & (align_bytes - 1);
            if misalign != 0 {
                // Calculate offset to first aligned position
                let adjust_bytes = align_bytes - misalign;
                adjust_bytes / PAGE_SIZE
            } else {
                0
            }
        } else {
            0
        };

        let mut start = base_offset;
        while start < self.alloc_pages {
            if start + num_pages > self.alloc_pages {
                break;
            }

            // Check if all pages are free
            let mut all_free = true;
            for i in 0..num_pages {
                if !self.is_bit_set(start + i) {
                    all_free = false;
                    break;
                }
            }

            if all_free {
                return Some(start);
            }
            
            // Move to next aligned position
            start += align_pages;
        }

        None
    }

    /// Set a bit (mark page as free)
    unsafe fn set_bit(&self, page: usize) {
        const BITS_PER_WORD: usize = core::mem::size_of::<usize>() * 8;
        let word_idx = page / BITS_PER_WORD;
        let bit_idx = page % BITS_PER_WORD;

        let bitmap_ptr = self.bitmap_base as *mut usize;
        let word = bitmap_ptr.add(word_idx).read_volatile();
        bitmap_ptr.add(word_idx).write_volatile(word | (1usize << bit_idx));
    }

    /// Clear a bit (mark page as allocated)
    unsafe fn clear_bit(&self, page: usize) {
        const BITS_PER_WORD: usize = core::mem::size_of::<usize>() * 8;
        let word_idx = page / BITS_PER_WORD;
        let bit_idx = page % BITS_PER_WORD;

        let bitmap_ptr = self.bitmap_base as *mut usize;
        let word = bitmap_ptr.add(word_idx).read_volatile();
        bitmap_ptr.add(word_idx).write_volatile(word & !(1usize << bit_idx));
    }

    /// Check if a bit is set (page is free)
    unsafe fn is_bit_set(&self, page: usize) -> bool {
        const BITS_PER_WORD: usize = core::mem::size_of::<usize>() * 8;
        let word_idx = page / BITS_PER_WORD;
        let bit_idx = page % BITS_PER_WORD;

        let bitmap_ptr = self.bitmap_base as *const usize;
        let word = bitmap_ptr.add(word_idx).read_volatile();
        (word & (1usize << bit_idx)) != 0
    }

    /// Count the number of free pages in this zone
    pub unsafe fn free_pages(&self) -> usize {
        const BITS_PER_WORD: usize = core::mem::size_of::<usize>() * 8;
        let bitmap_ptr = self.bitmap_base as *const usize;
        let _words_count = self.bitmap_size / core::mem::size_of::<usize>();

        let mut free_count = 0;
        for i in 0..self.alloc_pages {
            let word_idx = i / BITS_PER_WORD;
            let bit_idx = i % BITS_PER_WORD;
            let word = bitmap_ptr.add(word_idx).read_volatile();
            if (word & (1usize << bit_idx)) != 0 {
                free_count += 1;
            }
        }

        free_count
    }

    /// Count the number of used pages in this zone
    pub unsafe fn used_pages(&self) -> usize {
        self.alloc_pages - self.free_pages()
    }
}

/// Multi-zone bitmap allocator
pub struct BitmapAllocator<const PAGE_SIZE: usize = { crate::DEFAULT_PAGE_SIZE }> {
    zones: [Option<BitmapZone<PAGE_SIZE>>; MAX_ZONES],
    num_zones: usize,
}

impl<const PAGE_SIZE: usize> BitmapAllocator<PAGE_SIZE> {
    /// Create a new bitmap allocator
    pub const fn new() -> Self {
        Self {
            zones: [None; MAX_ZONES],
            num_zones: 0,
        }
    }

    /// Initialize with a single memory region
    pub unsafe fn init_internal(&mut self, start: usize, size: usize) {
        self.add_memory_region_internal(start, size).ok();
    }

    /// Add a memory region to the allocator
    pub unsafe fn add_memory_region_internal(&mut self, start: usize, size: usize) -> AllocResult<()> {
        if self.num_zones >= MAX_ZONES {
            return Err(AllocError::NoMemory);
        }

        // Check for overlap with existing zones
        for zone in self.zones.iter().filter_map(|z| z.as_ref()) {
            if !(start + size <= zone.base_addr || start >= zone.end_addr) {
                return Err(AllocError::MemoryOverlap);
            }
        }

        // Initialize the zone
        let zone_id = self.num_zones;
        let mut zone = BitmapZone::empty();
        zone.init(start, size);

        if zone.base_addr == 0 {
            // Zone initialization failed (too small)
            return Err(AllocError::InvalidParam);
        }

        self.zones[zone_id] = Some(zone);
        self.num_zones += 1;

        Ok(())
    }

    /// Find which zone contains the given address
    fn find_zone_for_addr(&self, addr: usize) -> Option<usize> {
        for (i, zone) in self.zones.iter().enumerate() {
            if let Some(z) = zone {
                if addr >= z.alloc_start && addr < z.end_addr {
                    return Some(i);
                }
            }
        }
        None
    }

    /// Allocate pages from any zone
    unsafe fn alloc_pages_internal(&mut self, num_pages: usize, alignment: usize) -> AllocResult<usize> {
        for i in 0..self.num_zones {
            if let Some(zone) = &self.zones[i] {
                if let Ok(addr) = zone.alloc_pages(num_pages, alignment) {
                    return Ok(addr);
                }
            }
        }
        Err(AllocError::NoMemory)
    }

    /// Deallocate pages to the appropriate zone
    unsafe fn dealloc_pages_internal(&mut self, addr: usize, num_pages: usize) {
        if let Some(zone_idx) = self.find_zone_for_addr(addr) {
            if let Some(zone) = &self.zones[zone_idx] {
                zone.dealloc_pages(addr, num_pages);
            }
        }
    }

    /// Get total number of zones
    pub fn zone_count(&self) -> usize {
        self.num_zones
    }

    /// Get zone by index
    pub fn zone(&self, index: usize) -> Option<&BitmapZone<PAGE_SIZE>> {
        self.zones.get(index).and_then(|z| z.as_ref())
    }
}

impl<const PAGE_SIZE: usize> crate::BaseAllocator for BitmapAllocator<PAGE_SIZE> {
    fn init(&mut self, start: usize, size: usize) {
        // SAFETY: Caller must ensure memory is valid
        unsafe { self.init_internal(start, size) };
    }

    fn add_memory(&mut self, start: usize, size: usize) -> AllocResult<()> {
        // SAFETY: Caller must ensure memory is valid
        unsafe { self.add_memory_region_internal(start, size) }
    }
}

impl<const PAGE_SIZE: usize> Default for BitmapAllocator<PAGE_SIZE> {
    fn default() -> Self {
        Self::new()
    }
}

impl<const PAGE_SIZE: usize> PageAllocator for BitmapAllocator<PAGE_SIZE> {
    const PAGE_SIZE: usize = PAGE_SIZE;

    fn alloc_pages(&mut self, num_pages: usize, alignment: usize) -> AllocResult<usize> {
        // SAFETY: Memory access is protected by bitmap
        unsafe { self.alloc_pages_internal(num_pages, alignment) }
    }

    fn dealloc_pages(&mut self, pos: usize, num_pages: usize) {
        // SAFETY: Memory access is protected by bitmap
        unsafe { self.dealloc_pages_internal(pos, num_pages) }
    }

    fn alloc_pages_at(
        &mut self,
        base: usize,
        num_pages: usize,
        _alignment: usize,
    ) -> AllocResult<usize> {
        // Find the zone containing the base address
        let zone_idx = self.find_zone_for_addr(base).ok_or(AllocError::InvalidParam)?;
        let zone = self.zones[zone_idx].as_ref().ok_or(AllocError::InvalidParam)?;

        // Check if the requested pages are free and contiguous
        let page_offset = (base - zone.alloc_start) / PAGE_SIZE;
        for i in 0..num_pages {
            // SAFETY: Memory access is protected by the bitmap
            if unsafe { !zone.is_bit_set(page_offset + i) } {
                return Err(AllocError::NoMemory);
            }
        }

        // Mark pages as allocated
        // SAFETY: Memory access is protected by the bitmap
        unsafe {
            for i in 0..num_pages {
                zone.clear_bit(page_offset + i);
            }
        }

        Ok(base)
    }

    fn total_pages(&self) -> usize {
        self.zones
            .iter()
            .filter_map(|z| z.as_ref())
            .map(|z| z.total_pages)
            .sum()
    }

    fn used_pages(&self) -> usize {
        self.zones
            .iter()
            .filter_map(|z| z.as_ref())
            .map(|z| unsafe { z.used_pages() })
            .sum()
    }

    fn available_pages(&self) -> usize {
        self.zones
            .iter()
            .filter_map(|z| z.as_ref())
            .map(|z| unsafe { z.free_pages() })
            .sum()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_metadata_size() {
        // 4096 pages (16MB) with 4KB pages
        let metadata_size = BitmapZone::<4096>::calculate_metadata_size(4096);
        // Need 4096 bits = 512 bytes (64 64-bit words)
        // Align to page boundary = 4096 bytes = 1 page
        assert_eq!(metadata_size, 4096);

        // 8192 pages (32MB)
        let metadata_size = BitmapZone::<4096>::calculate_metadata_size(8192);
        // Need 8192 bits = 1024 bytes = 1 page
        assert_eq!(metadata_size, 4096);

        // 32768 pages (128MB)
        let metadata_size = BitmapZone::<4096>::calculate_metadata_size(32768);
        // Need 32768 bits = 4096 bytes = 1 page
        assert_eq!(metadata_size, 4096);

        // 65536 pages (256MB)
        let metadata_size = BitmapZone::<4096>::calculate_metadata_size(65536);
        // Need 65536 bits = 8192 bytes = 2 pages
        assert_eq!(metadata_size, 8192);
    }
}
