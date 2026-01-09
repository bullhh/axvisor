//! Test static shared list pool
//!
//! Tests that fully exercise list allocation and release scenarios

#![no_std]
extern crate alloc;

use alloc::vec::Vec;
use axvisor_allocator::buddy::BuddySet;

/// Test basic list allocation when order 0 exceeds 64 blocks
#[test]
fn test_basic_list_allocation() {
    let mut buddy = BuddySet::new(0x1000_0000, 1024 * 4096, 0); // 4MB
    buddy.init(0x1000_0000, 1024 * 4096);

    // Allocate 65 order-0 blocks (more than MAX_BLOCKS_PER_LIST)
    let mut allocations = Vec::new();

    for _ in 0..65 {
        if let Ok(addr) = buddy.alloc_pages(1, 4096) {
            allocations.push(addr);
        }
    }

    assert_eq!(allocations.len(), 65, "Should allocate 65 blocks");

    // Now free them all - this should require allocating multiple lists
    for addr in allocations {
        buddy.dealloc_pages(addr, 1);
    }

    // Check that we have some blocks (may be merged to higher orders)
    // The key is that blocks were successfully freed without being discarded
    let total_free = buddy.get_stats().free_pages;
    assert_eq!(total_free, 1024, "Should have all 1024 pages freed");
}

/// Test multiple orders sharing lists
#[test]
fn test_multiple_orders_share_pool() {
    let mut buddy = BuddySet::new(0x2000_0000, 4096 * 1024, 0); // 4MB
    buddy.init(0x2000_0000, 4096 * 1024);

    // Allocate blocks from different orders
    let mut allocs = Vec::new();

    // Order 0: 100 blocks
    for _ in 0..100 {
        if let Ok(addr) = buddy.alloc_pages(1, 4096) {
            allocs.push((addr, 1));
        }
    }

    // Order 1: 50 blocks (2 pages each)
    for _ in 0..50 {
        if let Ok(addr) = buddy.alloc_pages(2, 4096) {
            allocs.push((addr, 2));
        }
    }

    // Order 2: 30 blocks (4 pages each)
    for _ in 0..30 {
        if let Ok(addr) = buddy.alloc_pages(4, 4096) {
            allocs.push((addr, 4));
        }
    }

    // Free everything
    for (addr, size) in allocs {
        buddy.dealloc_pages(addr, size);
    }

    // Check that all pages are freed
    let total_free = buddy.get_stats().free_pages;
    assert_eq!(total_free, 1024, "Should have all 1024 pages freed");
}

/// Test list release when empty
#[test]
fn test_list_release_on_empty() {
    let mut buddy = BuddySet::new(0x3000_0000, 4096 * 1024, 0); // 4MB
    buddy.init(0x3000_0000, 4096 * 1024);

    // Allocate and free in a way that causes multiple lists to be allocated
    let mut allocs = Vec::new();

    // First, allocate 65 blocks
    for _ in 0..65 {
        if let Ok(addr) = buddy.alloc_pages(1, 4096) {
            allocs.push(addr);
        }
    }

    // Free 63 of them (leaving 2)
    for i in 0..63 {
        buddy.dealloc_pages(allocs[i], 1);
    }

    // Allocate and free 65 more blocks
    let mut allocs2 = Vec::new();
    for _ in 0..65 {
        if let Ok(addr) = buddy.alloc_pages(1, 4096) {
            allocs2.push(addr);
        }
    }

    for addr in allocs2 {
        buddy.dealloc_pages(addr, 1);
    }

    // Free the remaining 2
    for i in 63..65 {
        buddy.dealloc_pages(allocs[i], 1);
    }

    // Check stats
    let stats = buddy.get_pool_stats();
    assert!(stats.used_lists <= 64, "Should not exceed total lists");
    assert_eq!(
        buddy.get_stats().free_pages,
        1024,
        "All pages should be freed"
    );
}

/// Test stress: allocate and free many small blocks
#[test]
fn test_stress_small_blocks() {
    let mut buddy = BuddySet::new(0x4000_0000, 4096 * 2048, 0); // 8MB
    buddy.init(0x4000_0000, 4096 * 2048);

    let mut allocs = Vec::new();

    // Allocate 200 order-0 blocks
    for _ in 0..200 {
        if let Ok(addr) = buddy.alloc_pages(1, 4096) {
            allocs.push(addr);
        }
    }

    assert_eq!(allocs.len(), 200, "Should allocate 200 blocks");

    // Free in random order
    let mut indices: Vec<usize> = (0..200).collect();

    // Swap pairs to shuffle
    for i in (0..199).step_by(2) {
        indices.swap(i, i + 1);
    }

    for idx in indices {
        buddy.dealloc_pages(allocs[idx], 1);
    }

    // All should be freed
    assert_eq!(
        buddy.get_stats().free_pages,
        2048,
        "All 2048 pages should be freed"
    );
}

/// Test merging with multiple lists
#[test]
fn test_merging_with_multiple_lists() {
    let mut buddy = BuddySet::new(0x5000_0000, 4096 * 256, 0); // 1MB
    buddy.init(0x5000_0000, 4096 * 256);

    let mut allocs = Vec::new();

    // Allocate 100 order-0 blocks
    for _ in 0..100 {
        if let Ok(addr) = buddy.alloc_pages(1, 4096) {
            allocs.push(addr);
        }
    }

    // Free them - this will distribute across multiple lists
    for addr in allocs {
        buddy.dealloc_pages(addr, 1);
    }

    // Now check that merging still works - allocate a single block and free it
    let addr = buddy.alloc_pages(1, 4096).unwrap();
    buddy.dealloc_pages(addr, 1);

    // After merge, we should still have all pages
    assert_eq!(
        buddy.get_stats().free_pages,
        256,
        "Should still have all 256 pages after merge"
    );
}

/// Test pool exhaustion
#[test]
fn test_pool_exhaustion() {
    let mut buddy = BuddySet::new(0x6000_0000, 4096 * 4096, 0); // 16MB
    buddy.init(0x6000_0000, 4096 * 4096);

    // Try to allocate more than the pool can handle
    // Pool has 64 lists * 64 blocks = 4096 blocks
    let mut allocs = Vec::new();

    // Try to allocate 4100 blocks (more than pool capacity)
    for _ in 0..4100 {
        match buddy.alloc_pages(1, 4096) {
            Ok(addr) => allocs.push(addr),
            Err(_) => break,
        }
    }

    // Should succeed in allocating all 4096 blocks
    assert!(allocs.len() >= 4000, "Should allocate most blocks");

    // Free everything
    for addr in allocs {
        buddy.dealloc_pages(addr, 1);
    }

    // All pages should be back
    assert_eq!(
        buddy.get_stats().free_pages,
        4096,
        "All 4096 pages should be freed"
    );
}

/// Test list reuse after being freed
#[test]
fn test_list_reuse() {
    let mut buddy = BuddySet::new(0x7000_0000, 4096 * 512, 0); // 2MB
    buddy.init(0x7000_0000, 4096 * 512);

    // Round 1: Allocate and free 100 blocks
    let mut allocs = Vec::new();
    for _ in 0..100 {
        if let Ok(addr) = buddy.alloc_pages(1, 4096) {
            allocs.push(addr);
        }
    }
    for addr in allocs {
        buddy.dealloc_pages(addr, 1);
    }

    let stats1 = buddy.get_stats();

    // Round 2: Allocate and free another 100 blocks
    let mut allocs = Vec::new();
    for _ in 0..100 {
        if let Ok(addr) = buddy.alloc_pages(1, 4096) {
            allocs.push(addr);
        }
    }
    for addr in allocs {
        buddy.dealloc_pages(addr, 1);
    }

    let stats2 = buddy.get_stats();

    // Stats should be similar (all pages should be freed)
    assert_eq!(
        stats1.free_pages, stats2.free_pages,
        "Free pages should be same"
    );
    assert_eq!(stats1.free_pages, 512, "All pages should be freed");
}

/// Test fragmentation with multiple orders
#[test]
fn test_fragmentation_scenarios() {
    let mut buddy = BuddySet::new(0x8000_0000, 4096 * 1024, 0); // 4MB
    buddy.init(0x8000_0000, 4096 * 1024);

    let mut allocs = Vec::new();

    // Allocate alternating orders to cause fragmentation
    for i in 0..64 {
        if i % 2 == 0 {
            // Order 0
            if let Ok(addr) = buddy.alloc_pages(1, 4096) {
                allocs.push((addr, 1));
            }
        } else {
            // Order 1
            if let Ok(addr) = buddy.alloc_pages(2, 4096) {
                allocs.push((addr, 2));
            }
        }
    }

    // Free everything
    for (addr, size) in allocs {
        buddy.dealloc_pages(addr, size);
    }

    // Check that all pages are freed
    assert_eq!(
        buddy.get_stats().free_pages,
        1024,
        "All pages should be freed"
    );
}

/// Test order transitions
#[test]
fn test_order_transitions() {
    let mut buddy = BuddySet::new(0x9000_0000, 4096 * 512, 0); // 2MB
    buddy.init(0x9000_0000, 4096 * 512);

    // Allocate order-2 blocks and free them
    let mut allocs = Vec::new();
    for _ in 0..40 {
        if let Ok(addr) = buddy.alloc_pages(4, 4096) {
            allocs.push((addr, 4));
        }
    }

    for (addr, size) in &allocs {
        buddy.dealloc_pages(*addr, *size);
    }

    // Now allocate and free order-0 blocks
    let mut allocs2 = Vec::new();
    for _ in 0..160 {
        if let Ok(addr) = buddy.alloc_pages(1, 4096) {
            allocs2.push(addr);
        }
    }

    for addr in allocs2 {
        buddy.dealloc_pages(addr, 1);
    }

    // All pages should be freed
    assert_eq!(
        buddy.get_stats().free_pages,
        512,
        "All pages should be freed"
    );
}

/// Test max order scenarios
#[test]
fn test_max_order_scenarios() {
    let mut buddy = BuddySet::new(0xA000_0000, 4096 * 4096, 0); // 16MB
    buddy.init(0xA000_0000, 4096 * 4096);

    // Allocate a large block (order 8 = 1024 pages = 4MB)
    if let Ok(addr) = buddy.alloc_pages(1024, 4096) {
        buddy.dealloc_pages(addr, 1024);
    }

    // Allocate many small blocks
    let mut allocs = Vec::new();
    for _ in 0..1000 {
        if let Ok(addr) = buddy.alloc_pages(1, 4096) {
            allocs.push(addr);
        }
    }

    for addr in allocs {
        buddy.dealloc_pages(addr, 1);
    }

    // All pages should be freed
    assert_eq!(
        buddy.get_stats().free_pages,
        4096,
        "All pages should be freed"
    );
}

/// Test that list pool doesn't leak blocks
#[test]
fn test_no_memory_leak() {
    let mut buddy = BuddySet::new(0xB000_0000, 4096 * 2048, 0); // 8MB
    buddy.init(0xB000_0000, 4096 * 2048);

    // Run multiple rounds of allocate and free
    for round in 0..10 {
        let num_allocs = 100 + round * 50;
        let mut allocs = Vec::new();

        for _ in 0..num_allocs {
            if let Ok(addr) = buddy.alloc_pages(1, 4096) {
                allocs.push(addr);
            }
        }

        for addr in allocs {
            buddy.dealloc_pages(addr, 1);
        }

        // After each round, all pages should be freed
        assert_eq!(
            buddy.get_stats().free_pages,
            2048,
            "Round {}: All pages should be freed",
            round
        );
    }
}

/// Test that list pool handles extreme fragmentation
#[test]
fn test_extreme_fragmentation() {
    let mut buddy = BuddySet::new(0xC000_0000, 4096 * 2048, 0); // 8MB
    buddy.init(0xC000_0000, 4096 * 2048);

    let mut allocs = Vec::new();

    // Allocate many small blocks at different addresses
    for _ in 0..500 {
        if let Ok(addr) = buddy.alloc_pages(1, 4096) {
            allocs.push(addr);
        }
    }

    // Free them in random order
    let mut indices: Vec<usize> = (0..500).collect();

    // Shuffle by swapping adjacent elements
    for i in 0..250 {
        let j = i * 2 + (i % 2);
        if j + 1 < 500 {
            indices.swap(j, j + 1);
        }
    }

    for idx in indices {
        buddy.dealloc_pages(allocs[idx], 1);
    }

    // All pages should be freed
    assert_eq!(
        buddy.get_stats().free_pages,
        2048,
        "All pages should be freed"
    );

    // Verify we can still allocate
    let mut allocs2 = Vec::new();
    for _ in 0..200 {
        if let Ok(addr) = buddy.alloc_pages(1, 4096) {
            allocs2.push(addr);
        }
    }
    assert_eq!(
        allocs2.len(),
        200,
        "Should be able to allocate after freeing"
    );
}
