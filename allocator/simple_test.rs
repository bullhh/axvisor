use crate::{BuddyPageAllocator, PageAllocator, AllocError};

fn main() {
    let mut allocator = BuddyPageAllocator::new();
    
    let base_addr = 0x80000000;
    let size = 0x10000; // 64KB = 16 pages
    allocator.init(base_addr, size);
    
    println!("Initialized allocator with {} pages", size / 0x1000);
    
    // Try to allocate a single page
    match PageAllocator::alloc_pages(&mut allocator, 1, 0x1000) {
        Ok(addr) => {
            println!("Successfully allocated page at: {:#x}", addr);
            
            // Try to allocate another page
            match PageAllocator::alloc_pages(&mut allocator, 1, 0x1000) {
                Ok(addr2) => {
                    println!("Successfully allocated second page at: {:#x}", addr2);
                }
                Err(e) => {
                    println!("Failed to allocate second page: {:?}", e);
                }
            }
        }
        Err(e) => {
            println!("Failed to allocate first page: {:?}", e);
        }
    }
}
