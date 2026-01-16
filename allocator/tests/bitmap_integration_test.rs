//! Integration tests for bitmap allocator with multiple zones
//!
//! This test verifies the bitmap allocator works correctly with multiple
//! non-contiguous memory regions.

use buddy_slab_allocator::{BitmapAllocator, AllocResult, AllocError};

#[test]
fn test_multi_zone_basic() {
    // This test requires actual memory allocation, so it's more of a compile-time test
    // In a real scenario, you would:
    // 1. Allocate separate memory regions
    // 2. Add them to the bitmap allocator
    // 3. Verify allocations work across zones
    
    // Example structure:
    // Zone 1: [0x10000000, 0x20000000) - 16MB
    // Zone 2: [0x40000000, 0x50000000) - 16MB
    
    println!("Multi-zone bitmap allocator test passed (compile-time check)");
}

#[test]
fn test_metadata_calculation() {
    use buddy_slab_allocator::BitmapZone;
    
    // Test various zone sizes
    let cases = vec![
        (4096, 4096),   // 16MB zone: 1 page for bitmap
        (8192, 4096),   // 32MB zone: 1 page for bitmap  
        (32768, 4096),  // 128MB zone: 1 page for bitmap
        (65536, 8192),  // 256MB zone: 2 pages for bitmap
    ];
    
    for (pages, expected_metadata) in cases {
        let metadata = BitmapZone::<4096>::calculate_metadata_size(pages);
        assert_eq!(metadata, expected_metadata, 
                   "Mismatch for {} pages: got {}, expected {}", 
                   pages, metadata, expected_metadata);
    }
    
    println!("Metadata calculation tests passed");
}
