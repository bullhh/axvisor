// Simple test to verify the axalloc adapter works with the new axvisor_allocator
use axalloc::{global_allocator, init_heap, PAGE_SIZE, UsageKind};
use core::alloc::{GlobalAlloc, Layout};

fn main() {
    println!("Testing axalloc adapter...");

    // Initialize the allocator
    init_heap(0x1000_0000, 0x1000_0000).expect("Failed to initialize heap");
    println!("Heap initialized successfully");

    // Test page allocation
    let page_addr = global_allocator()
        .alloc_pages(1, PAGE_SIZE, UsageKind::Global)
        .expect("Failed to allocate page");
    println!("Allocated page at: {:#x}", page_addr);

    // Test byte allocation
    let layout = Layout::from_size_align(1024, 8).unwrap();
    let ptr = unsafe { global_allocator().alloc(layout) }.expect("Failed to allocate bytes");
    println!("Allocated {} bytes at: {:p}", layout.size(), ptr);

    // Check usage statistics
    let usages = global_allocator().usages();
    println!("Usage statistics: {:?}", usages);

    // Check page statistics
    let used_pages = global_allocator().used_pages();
    let available_pages = global_allocator().available_pages();
    println!("Used pages: {}, Available pages: {}", used_pages, available_pages);

    // Check byte statistics
    let used_bytes = global_allocator().used_bytes();
    let available_bytes = global_allocator().available_bytes();
    println!("Used bytes: {}, Available bytes: {}", used_bytes, available_bytes);

    // Clean up
    unsafe {
        global_allocator().dealloc(ptr, layout);
        global_allocator().dealloc_pages(page_addr, 1, UsageKind::Global);
    }
    println!("Memory deallocated successfully");

    // Check final statistics
    let final_usages = global_allocator().usages();
    println!("Final usage statistics: {:?}", final_usages);
}