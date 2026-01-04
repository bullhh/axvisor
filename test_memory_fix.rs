//! Test to verify the memory allocator fix

use allocator::{BuddyPageAllocator, PageAllocator};

fn main() {
    println!("Testing multi-memory region support...");
    
    let mut allocator = BuddyPageAllocator::new();
    
    // Initialize with first region
    println!("Initializing with first region [0x80000000, 0x80100000) (1MB)");
    allocator.init(0x80000000, 0x100000);
    
    let stats1 = allocator.get_stats();
    println!("After first region: {} total pages", stats1.total_pages);
    
    // Add second region
    println!("Adding second region [0x81000000, 0x81100000) (1MB)");
    allocator.add_memory(0x81000000, 0x100000).unwrap();
    
    let stats2 = allocator.get_stats();
    println!("After second region: {} total pages", stats2.total_pages);
    
    // Add third region
    println!("Adding third region [0x82000000, 0x82100000) (1MB)");
    allocator.add_memory(0x82000000, 0x100000).unwrap();
    
    let stats3 = allocator.get_stats();
    println!("After third region: {} total pages", stats3.total_pages);
    
    // Test allocation
    println!("Testing allocation...");
    match allocator.alloc_pages(1, 4096) {
        Ok(addr) => {
            println!("Successfully allocated page at {:#x}", addr);
            allocator.dealloc_pages(addr, 1);
            println!("Successfully deallocated page");
        }
        Err(e) => {
            println!("Allocation failed: {:?}", e);
        }
    }
    
    let final_stats = allocator.get_stats();
    println!("Final stats: {} total pages, {} free pages, {} used pages", 
              final_stats.total_pages, final_stats.free_pages, final_stats.used_pages);
    
    // Verify that all regions are accounted for
    let expected_total = (0x100000 + 0x100000 + 0x100000) / 4096; // 3MB / 4KB = 768 pages
    if final_stats.total_pages == expected_total {
        println!("✓ SUCCESS: All memory regions are properly accounted for!");
        println!("  Expected: {} pages, Got: {} pages", expected_total, final_stats.total_pages);
    } else {
        println!("✗ FAILED: Memory regions not properly accounted for");
        println!("  Expected: {} pages, Got: {} pages", expected_total, final_stats.total_pages);
    }
}
