//! Buddy allocator allocation test
//! This test checks if buddy allocator can successfully allocate memory

#![no_std]
#![no_main]

extern crate alloc;

use alloc::vec::Vec;

// Mock logging for testing
#[macro_use]
mod log {
    macro_rules! info {
        ($($arg:tt)*) => {
            println!("[INFO] {}", format_args!($($arg)*));
        };
    }
    macro_rules! error {
        ($($arg:tt)*) => {
            println!("[ERROR] {}", format_args!($($arg)*));
        };
    }
    macro_rules! warn {
        ($($arg:tt)*) => {
            println!("[WARN] {}", format_args!($($arg)*));
        };
    }
    macro_rules! debug {
        ($($arg:tt)*) => {
            println!("[DEBUG] {}", format_args!($($arg)*));
        };
    }
}

use axallocator::{
    buddy::BuddyPageAllocator,
    PageAllocator,
};

// Simulated physical memory buffer
const MEMORY_SIZE: usize = 0x100000; // 1 MB
const MEMORY_START: usize = 0x8000_0000; // 2 GB

static mut MEMORY_BUFFER: [u8; MEMORY_SIZE] = [0u8; MEMORY_SIZE];

fn get_memory_start() -> usize {
    unsafe { (&raw mut MEMORY_BUFFER) as usize }
}

#[no_mangle]
extern "C" fn main() -> i32 {
    println!("=== Buddy Allocator Allocation Test ===\n");

    let mut allocator = BuddyPageAllocator::<4096>::new();

    // Test 1: Initialize allocator
    println!("Test 1: Initialize allocator");
    let start = get_memory_start();
    let size = MEMORY_SIZE;
    println!("  Initializing with [{:#x}, {:#x}) ({} bytes)", start, start + size, size);
    allocator.init(start, size);
    println!("  Initialized successfully\n");

    // Test 2: Allocate single page
    println!("Test 2: Allocate single page");
    match allocator.alloc_pages(1, 0x1000) {
        Ok(addr) => {
            println!("  ✅ Allocated 1 page at {:#x}", addr);
            println!("  Free pages: {}", allocator.available_pages());
            println!();
        }
        Err(e) => {
            println!("  ❌ Failed to allocate: {:?}", e);
            return -1;
        }
    }

    // Test 3: Allocate multiple pages (power of 2)
    println!("Test 3: Allocate 2 pages (power of 2)");
    match allocator.alloc_pages(2, 0x1000) {
        Ok(addr) => {
            println!("  ✅ Allocated 2 pages at {:#x}", addr);
            println!("  Free pages: {}", allocator.available_pages());
            println!();
        }
        Err(e) => {
            println!("  ❌ Failed to allocate: {:?}", e);
            return -1;
        }
    }

    // Test 4: Allocate with alignment
    println!("Test 4: Allocate 4 pages with 16KB alignment");
    match allocator.alloc_pages(4, 0x4000) {
        Ok(addr) => {
            println!("  ✅ Allocated 4 pages at {:#x}", addr);
            println!("  Alignment check: {:#x} % 0x4000 = {}", addr, addr % 0x4000);
            println!("  Free pages: {}", allocator.available_pages());
            println!();
        }
        Err(e) => {
            println!("  ❌ Failed to allocate: {:?}", e);
            return -1;
        }
    }

    // Test 5: Allocate large block (8 pages)
    println!("Test 5: Allocate 8 pages");
    match allocator.alloc_pages(8, 0x1000) {
        Ok(addr) => {
            println!("  ✅ Allocated 8 pages at {:#x}", addr);
            println!("  Free pages: {}", allocator.available_pages());
            println!();
        }
        Err(e) => {
            println!("  ❌ Failed to allocate: {:?}", e);
            return -1;
        }
    }

    // Test 6: Check total memory usage
    println!("Test 6: Memory usage summary");
    println!("  Total pages: {}", allocator.total_pages());
    println!("  Used pages: {}", allocator.used_pages());
    println!("  Free pages: {}", allocator.available_pages());
    println!();

    // Test 7: Deallocate and reallocate
    println!("Test 7: Deallocate and reallocate");
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
            return -1;
        }
    }

    // Test 8: Allocate until failure
    println!("Test 8: Allocate until memory exhausted");
    let mut allocated = Vec::new();
    let mut count = 0;
    loop {
        match allocator.alloc_pages(1, 0x1000) {
            Ok(addr) => {
                allocated.push(addr);
                count += 1;
                if count % 10 == 0 {
                    println!("  Allocated {} pages, {} free", count, allocator.available_pages());
                }
            }
            Err(e) => {
                println!("  ❌ Allocation failed after {} pages: {:?}", count, e);
                println!("  Final state: {} used, {} free", allocator.used_pages(), allocator.available_pages());
                break;
            }
        }
    }
    println!();

    println!("=== All tests completed ===");
    0
}

// Minimal panic handler
#[panic_handler]
fn panic(_info: &core::panic::PanicInfo) -> ! {
    loop {}
}
