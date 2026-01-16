//! Simple buddy allocator test
//! Test if buddy allocator can successfully allocate memory

#[path = "allocator/src/buddy/mod.rs"]
mod buddy;

#[path = "allocator/src/lib.rs"]
mod allocator;

use allocator::{PageAllocator, buddy::BuddyPageAllocator};

// Simulated physical memory buffer
const MEMORY_SIZE: usize = 0x100000; // 1 MB
const MEMORY_START: usize = 0x8000_0000; // 2 GB

static mut MEMORY_BUFFER: [u8; MEMORY_SIZE] = [0u8; MEMORY_SIZE];

fn main() {
    println!("=== Buddy Allocator Allocation Test ===\n");

    let mut allocator = BuddyPageAllocator::<4096>::new();

    // Test 1: Initialize allocator
    println!("Test 1: Initialize allocator");
    let start = unsafe { (&raw mut MEMORY_BUFFER) as usize };
    let size = MEMORY_SIZE;
    println!("  Initializing with [{:#x}, {:#x}) ({} bytes)", start, start + size, size);
    allocator.init(start, size);
    println!("  Initialized successfully\n");

    // Test 2: Check initial state
    println!("Test 2: Initial state");
    println!("  Total pages: {}", allocator.total_pages());
    println!("  Free pages: {}", allocator.available_pages());
    println!("  Used pages: {}", allocator.used_pages());
    println!();

    // Test 3: Allocate single page
    println!("Test 3: Allocate single page");
    match allocator.alloc_pages(1, 0x1000) {
        Ok(addr) => {
            println!("  ✅ Allocated 1 page at {:#x}", addr);
            println!("  Free pages: {}", allocator.available_pages());
            println!();
        }
        Err(e) => {
            println!("  ❌ Failed to allocate: {:?}", e);
            return;
        }
    }

    // Test 4: Allocate multiple pages (power of 2)
    println!("Test 4: Allocate 2 pages");
    match allocator.alloc_pages(2, 0x1000) {
        Ok(addr) => {
            println!("  ✅ Allocated 2 pages at {:#x}", addr);
            println!("  Free pages: {}", allocator.available_pages());
            println!();
        }
        Err(e) => {
            println!("  ❌ Failed to allocate: {:?}", e);
            return;
        }
    }

    // Test 5: Allocate with alignment
    println!("Test 5: Allocate 4 pages with 16KB alignment");
    match allocator.alloc_pages(4, 0x4000) {
        Ok(addr) => {
            println!("  ✅ Allocated 4 pages at {:#x}", addr);
            println!("  Alignment check: {:#x} % 0x4000 = {}", addr, addr % 0x4000);
            println!("  Free pages: {}", allocator.available_pages());
            println!();
        }
        Err(e) => {
            println!("  ❌ Failed to allocate: {:?}", e);
            return;
        }
    }

    // Test 6: Allocate large block (8 pages)
    println!("Test 6: Allocate 8 pages");
    match allocator.alloc_pages(8, 0x1000) {
        Ok(addr) => {
            println!("  ✅ Allocated 8 pages at {:#x}", addr);
            println!("  Free pages: {}", allocator.available_pages());
            println!();
        }
        Err(e) => {
            println!("  ❌ Failed to allocate: {:?}", e);
            return;
        }
    }

    // Test 7: Check memory usage
    println!("Test 7: Memory usage summary");
    println!("  Total pages: {}", allocator.total_pages());
    println!("  Used pages: {}", allocator.used_pages());
    println!("  Free pages: {}", allocator.available_pages());
    println!();

    // Test 8: Deallocate and reallocate
    println!("Test 8: Deallocate and reallocate");
    let addr1 = allocator.alloc_pages(1, 0x1000).unwrap();
    println!("  Allocated page at {:#x}", addr1);
    allocator.dealloc_pages(addr1, 1);
    println!("  Deallocated page");
    match allocator.alloc_pages(1, 0x1000) {
        Ok(addr2) => {
            println!("  ✅ Reallocated page at {:#x}", addr2);
            println!();
        }
        Err(e) => {
            println!("  ❌ Failed to reallocate: {:?}", e);
            return;
        }
    }

    println!("=== All tests completed successfully ===");
}
