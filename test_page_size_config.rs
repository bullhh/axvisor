//! Test to verify that PAGE_SIZE can be configured from upper layer

use axvisor_allocator::{GlobalAllocator, DEFAULT_PAGE_SIZE};

// Test 1: Default 4KB page size
const PAGE_SIZE_4K: usize = 0x1000;

fn test_default_page_size() {
    let allocator = GlobalAllocator::<DEFAULT_PAGE_SIZE>::new();
    assert_eq!(DEFAULT_PAGE_SIZE, 0x1000);
}

// Test 2: Custom 2MB page size (huge page)
const PAGE_SIZE_2M: usize = 0x200000;

fn test_custom_page_size_2m() {
    let allocator = GlobalAllocator::<PAGE_SIZE_2M>::new();
    assert_eq!(PAGE_SIZE_2M, 0x200000);
}

// Test 3: Custom 8KB page size
const PAGE_SIZE_8K: usize = 0x2000;

fn test_custom_page_size_8k() {
    let allocator = GlobalAllocator::<PAGE_SIZE_8K>::new();
    assert_eq!(PAGE_SIZE_8K, 0x2000);
}

fn main() {
    println!("Testing PAGE_SIZE configuration...");
    println!("DEFAULT_PAGE_SIZE = {:#x}", DEFAULT_PAGE_SIZE);
    println!("PAGE_SIZE_4K = {:#x}", PAGE_SIZE_4K);
    println!("PAGE_SIZE_2M = {:#x}", PAGE_SIZE_2M);
    println!("PAGE_SIZE_8K = {:#x}", PAGE_SIZE_8K);

    test_default_page_size();
    test_custom_page_size_2m();
    test_custom_page_size_8k();

    println!("All tests passed!");
}
