//! Buddy page allocator implementation for Axvisor.
//! 
//! This module implements a complete buddy system for page-level allocation
//! with support for memory regions, statistics, and proper buddy merging.

use crate::{AllocError, AllocResult, BaseAllocator, PageAllocator};
use super::slab_byte_allocator::PageAllocatorForSlab;
use log::{error, info, trace, warn};
use alloc::vec::Vec;

/// Simple static linked list node
#[derive(Debug, Clone, Copy)]
struct ListNode<T> {
    data: T,
    next: Option<usize>,
}

/// Static linked list implementation that doesn't require dynamic allocation
#[derive(Debug)]
pub struct StaticLinkedList<T, const N: usize> {
    nodes: [Option<ListNode<T>>; N],
    head: Option<usize>,
    tail: Option<usize>,
    free_head: Option<usize>,
    len: usize,
}

impl<T, const N: usize> StaticLinkedList<T, N> {
    /// Create a new empty static linked list
    pub const fn new() -> Self {
        Self {
            nodes: [const { None }; N],
            head: None,
            tail: None,
            free_head: Some(0),
            len: 0,
        }
    }

    /// Initialize the free list
    pub fn init(&mut self) {
        // Initialize all nodes as free
        for i in 0..N {
            // Ensure i is within bounds (should always be true with 0..N)
            debug_assert!(i < N, "Index out of bounds in StaticLinkedList::init");
            
            self.nodes[i] = Some(ListNode {
                data: unsafe { core::mem::zeroed() },
                next: if i < N-1 { Some(i + 1) } else { None },
            });
        }
        self.free_head = Some(0);
        self.head = None;
        self.tail = None;
        self.len = 0;
    }

    /// Push an element to the back of the list
    pub fn push_back(&mut self, data: T) -> bool {
        if self.free_head.is_none() {
            return false; // No free nodes available
        }

        // Get a free node index and ensure it's within bounds
        let new_node_idx = self.free_head.unwrap();
        if new_node_idx >= N {
            return false; // Invalid free node index
        }
        
        // Get the next free node before modifying the current one
        let next_free = if let Some(node) = &self.nodes[new_node_idx] {
            node.next
        } else {
            // Node is invalid
            panic!("Free node {} is corrupted, cannot append new node", new_node_idx);
        };
        
        // Update the node with new data
        self.nodes[new_node_idx] = Some(ListNode {
            data,
            next: None,
        });
        
        // Update free head
        self.free_head = next_free;
        
        // Add to list
        if self.tail.is_none() {
            // Empty list
            self.head = Some(new_node_idx);
            self.tail = Some(new_node_idx);
        } else {
            // Append to tail
            let tail_idx = self.tail.unwrap();
            if tail_idx >= N {
                return false; // Invalid tail index
            }
            
            if let Some(tail_node) = self.nodes[tail_idx].as_mut() {
                tail_node.next = Some(new_node_idx);
            } else {
                // Tail node is invalid, reinitialize it with dummy data
                panic!("Tail node {} is corrupted, cannot append new node", tail_idx);
            }
            self.tail = Some(new_node_idx);
        }
        
        self.len += 1;
        true
    }

    /// Pop an element from the front of the list
    pub fn pop_front(&mut self) -> Option<T> {
        if self.head.is_none() {
            return None;
        }

        // Get head node index and ensure it's within bounds
        let head_idx = self.head.unwrap();
        if head_idx >= N {
            // Invalid head index
            panic!("Head node {} is corrupted, cannot pop front", head_idx);
        }
        
        // Get head node
        let Some(head_node) = self.nodes[head_idx].take() else {
            // Invalid head node
            panic!("Head node {} is corrupted, cannot pop front", head_idx);
        };
        
        // Update head
        self.head = head_node.next;
        if self.head.is_none() {
            self.tail = None;
        }
        
        // Return node to free list
        // Create a new dummy node to return to free list
        let node = ListNode { data: unsafe { core::mem::zeroed() }, next: self.free_head };
        self.nodes[head_idx] = Some(node);
        self.free_head = Some(head_idx);
        
        self.len -= 1;
        Some(head_node.data)
    }

    /// Check if the list is empty
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// Get the length of the list
    pub fn len(&self) -> usize {
        self.len
    }
}

const PAGE_SIZE: usize = 0x1000;
pub const DEFAULT_MAX_ORDER: usize = 28; // Support up to 256GB allocations (2^28 * 4KB)

/// Buddy system statistics
#[derive(Debug, Clone, Copy)]
pub struct BuddyStats {
    pub total_pages: usize,
    pub free_pages: usize,
    pub used_pages: usize,
    pub free_pages_by_order: [usize; DEFAULT_MAX_ORDER + 1],
}

impl Default for BuddyStats {
    fn default() -> Self {
        Self {
            total_pages: 0,
            free_pages: 0,
            used_pages: 0,
            free_pages_by_order: [0; DEFAULT_MAX_ORDER + 1],
        }
    }
}

impl BuddyStats {
    pub const fn new() -> Self {
        Self {
            total_pages: 0,
            free_pages: 0,
            used_pages: 0,
            free_pages_by_order: [0; DEFAULT_MAX_ORDER + 1],
        }
    }
}

/// A memory region descriptor
#[derive(Debug, Clone)]
pub struct MemoryRegion {
    pub start_addr: usize,
    pub size: usize,
}

/// Buddy block metadata
#[derive(Debug, Clone, Copy)]
struct BuddyBlock {
    order: usize,
    addr: usize,
}

impl BuddyBlock {
    /// Calculate the buddy address for this block
    /// The buddy is the other half of the parent block at the next higher order
    /// For a block at order k with address A, its buddy is at A ^ (2^k * PAGE_SIZE)
    #[allow(dead_code)]
    fn buddy_addr(&self) -> usize {
        self.addr ^ ((1 << self.order) * PAGE_SIZE)
    }
}

/// Maximum number of blocks in each free list
const MAX_BLOCKS_PER_LIST: usize = 64;

/// Maximum number of memory zones supported
const MAX_ZONES: usize = 8;

/// A memory zone descriptor (similar to Linux kernel's zone)
#[derive(Debug, Clone, Copy)]
pub struct ZoneInfo {
    pub start_addr: usize,
    pub end_addr: usize,
    pub total_pages: usize,
    pub zone_id: usize,
}

/// Buddy set implementation - represents a single zone
#[derive(Debug)]
pub struct BuddySet {
    base_addr: usize,
    end_addr: usize,
    total_pages: usize,
    zone_id: usize,
    free_lists: [StaticLinkedList<BuddyBlock, MAX_BLOCKS_PER_LIST>; DEFAULT_MAX_ORDER + 1],
}

impl BuddySet {
    /// Create a new buddy set for a zone
    pub const fn new(base_addr: usize, size: usize, zone_id: usize) -> Self {
        Self {
            base_addr,
            end_addr: base_addr + size,
            total_pages: size / PAGE_SIZE,
            zone_id,
            free_lists: [const { StaticLinkedList::new() }; DEFAULT_MAX_ORDER + 1],
        }
    }

    /// Create an empty buddy set
    pub const fn empty() -> Self {
        Self::new(0, 0, 0)
    }

    pub const fn max_order(&self) -> usize {
        DEFAULT_MAX_ORDER
    }

    /// Check if an address belongs to this zone
    pub fn addr_in_zone(&self, addr: usize) -> bool {
        addr >= self.base_addr && addr < self.end_addr
    }

    /// Get zone information
    pub fn zone_info(&self) -> ZoneInfo {
        ZoneInfo {
            start_addr: self.base_addr,
            end_addr: self.end_addr,
            total_pages: self.total_pages,
            zone_id: self.zone_id,
        }
    }

    pub fn init(&mut self, base_addr: usize, size: usize) {
        info!("zone {}: Initialize with region [{:#x}, {:#x})", 
              self.zone_id, base_addr, base_addr + size);

        // 向下对齐基地址到页边界
        let aligned_base = base_addr & !(PAGE_SIZE - 1);
        // 向上对齐结束地址到页边界
        let end = base_addr + size;
        let aligned_end = (end + PAGE_SIZE - 1) & !(PAGE_SIZE - 1);

        // 计算对齐后的大小
        let aligned_size = aligned_end - aligned_base;

        // 如果对齐后没有有效内存，直接返回
        if aligned_size == 0 || aligned_size < PAGE_SIZE {
            panic!("Aligned size is too small: {:#x}", aligned_size);
        }

        info!("zone {}: Adjusted region [{:#x}, {:#x}) (original [{:#x}, {:#x}))",
              self.zone_id, aligned_base, aligned_end, base_addr, end);

        self.base_addr = aligned_base;
        self.end_addr = aligned_end;
        self.total_pages = aligned_size / PAGE_SIZE;

        // Initialize all free lists
        for list in &mut self.free_lists {
            list.init();
        }

        // Linux 风格的初始化：逐页释放，让 buddy 系统自动合并
        // 这样可以自然地处理任意大小的内存区域
        for pfn in 0..self.total_pages {
            let page_addr = self.base_addr + pfn * PAGE_SIZE;
            self.dealloc_pages(page_addr, 1);
        }

        info!("zone {}: {} pages initialized in buddy system", 
              self.zone_id, self.total_pages);
    }

    pub fn alloc_pages(&mut self, num_pages: usize, _align_pow2: usize) -> AllocResult<usize> {
        if num_pages == 0 {
            return Err(AllocError::InvalidParam);
        }

        // Find the required order (round up to next power of 2)
        let required_order = if num_pages.is_power_of_two() {
            num_pages.trailing_zeros() as usize
        } else {
            num_pages.next_power_of_two().trailing_zeros() as usize
        }.min(DEFAULT_MAX_ORDER);

        // Try to find a block of the required order or higher
        for order in required_order..=DEFAULT_MAX_ORDER {
            if !self.free_lists[order].is_empty() {
                let mut block = self.free_lists[order].pop_front().unwrap();

                // Split down to required order
                // When splitting a block of order k into two blocks of order k-1:
                // - First block stays at original address
                // - Second block is at original address + 2^(k-1) * PAGE_SIZE
                while block.order > required_order {
                    block.order -= 1;
                    let split_size = (1 << block.order) * PAGE_SIZE;
                    let buddy_addr = block.addr + split_size;

                    // Push the second half back to free list
                    let success = self.free_lists[block.order].push_back(BuddyBlock {
                        order: block.order,
                        addr: buddy_addr,
                    });
                    if !success {
                        warn!("Failed to push buddy block to free list during split");
                        // Put the original block back
                        self.free_lists[block.order + 1].push_back(block);
                        return Err(AllocError::NoMemory);
                    }
                }

                return Ok(block.addr);
            }
        }

        Err(AllocError::NoMemory)
    }

    /// Print detailed debug information when allocation fails
    #[allow(dead_code)]
    fn print_alloc_failure_debug(&self, num_pages: usize, _align_pow2: usize) {
        let zone_stats = self.get_stats();
        error!("zone {}: Allocation of {} pages ({} bytes) failed",
               self.zone_id, num_pages, num_pages * PAGE_SIZE);
        error!("zone {}: Current state - Free: {} pages, Used: {} pages, Total: {} pages",
               self.zone_id, zone_stats.free_pages, zone_stats.used_pages, zone_stats.total_pages);

        // Find maximum available block size
        let mut max_order = None;
        for order in (0..=DEFAULT_MAX_ORDER).rev() {
            if zone_stats.free_pages_by_order[order] > 0 {
                max_order = Some(order);
                break;
            }
        }
        match max_order {
            Some(order) => {
                error!("zone {}: Maximum available block size: Order {} ({} MB, {} pages)",
                       self.zone_id, order,
                       ((1 << order) * PAGE_SIZE) / (1024 * 1024),
                       1 << order);
            }
            None => {
                error!("zone {}: No free blocks available at any order", self.zone_id);
            }
        }
    }

    /// Find a block with the given address in the free list of the given order
    fn find_block_in_free_list(&self, order: usize, addr: usize) -> Option<usize> {
        let list = &self.free_lists[order];
        let mut current_idx = list.head;
        let mut visited = 0; // 防止无限循环
        
        while let Some(idx) = current_idx {
            if visited > list.len {
                // 检测到可能的循环，退出
                warn!("Potential cycle detected in free list during search");
                return None;
            }
            
            if let Some(node) = &list.nodes[idx] {
                if node.data.addr == addr {
                    return Some(idx);
                }
                current_idx = node.next;
            } else {
                break;
            }
            visited += 1;
        }
        
        None
    }
    
    /// Remove a block from the free list at the given position
    fn remove_block_from_free_list(&mut self, order: usize, node_idx: usize) {
        // 首先验证节点确实在链表中（在获取可变借用之前）
        if !self.node_exists_in_list(&self.free_lists[order], node_idx) {
            return; // 节点不在链表中，直接返回
        }
        
        let list = &mut self.free_lists[order];
        
        // 验证node_idx是否有效
        if node_idx >= MAX_BLOCKS_PER_LIST || list.nodes[node_idx].is_none() {
            return; // 无效索引，直接返回
        }
        
        // 找到前一个节点
        let mut prev_idx = None;
        let mut current_idx = list.head;
        let mut visited = 0; // 防止无限循环
        
        while let Some(idx) = current_idx {
            if visited > list.len {
                // 检测到可能的循环，退出
                warn!("Potential cycle detected in free list");
                return;
            }
            
            if idx == node_idx {
                break;
            }
            prev_idx = current_idx;
            if let Some(node) = &list.nodes[idx] {
                current_idx = node.next;
            } else {
                break;
            }
            visited += 1;
        }
        
        // 如果没有找到节点，直接返回
        if current_idx != Some(node_idx) {
            return;
        }
        
        // 执行删除操作
        if let Some(node) = list.nodes[node_idx].take() {
            // 更新链接
            if let Some(prev_idx) = prev_idx {
                if let Some(prev_node) = &mut list.nodes[prev_idx] {
                    prev_node.next = node.next;
                }
            } else {
                list.head = node.next;
            }
            
            // 更新尾指针（如果需要）
            if list.tail == Some(node_idx) {
                if node.next.is_none() {
                    // 如果删除的是尾节点，更新tail为prev
                    list.tail = prev_idx;
                }
            } else if list.head.is_none() {
                // 如果删除后链表为空，确保tail也被设置为None
                list.tail = None;
            }
            
            // 将节点返回到空闲列表
            let dummy_node = ListNode { 
                data: unsafe { core::mem::zeroed() }, 
                next: list.free_head 
            };
            list.nodes[node_idx] = Some(dummy_node);
            list.free_head = Some(node_idx);
            list.len -= 1;
        }
    }

    /// 辅助方法：检查节点是否在链表中
    fn node_exists_in_list(&self, list: &StaticLinkedList<BuddyBlock, MAX_BLOCKS_PER_LIST>, node_idx: usize) -> bool {
        let mut current_idx = list.head;
        let mut visited = 0;
        
        while let Some(idx) = current_idx {
            if visited > list.len {
                return false; // 检测到循环
            }
            if idx == node_idx {
                return true;
            }
            if let Some(node) = &list.nodes[idx] {
                current_idx = node.next;
            } else {
                break;
            }
            visited += 1;
        }
        
        false
    }

    pub fn dealloc_pages(&mut self, addr: usize, num_pages: usize) {
        if num_pages == 0 {
            return;
        }

        // Validate address belongs to this zone
        if !self.addr_in_zone(addr) {
            warn!("zone {}: Address {:#x} not in zone [{:#x}, {:#x})",
                  self.zone_id, addr, self.base_addr, self.end_addr);
            return;
        }

        // Convert address and pages to PFN (Page Frame Number)
        let pfn = addr / PAGE_SIZE;

        // Calculate the order for this deallocation
        let mut order = if num_pages.is_power_of_two() {
            num_pages.trailing_zeros() as usize
        } else {
            num_pages.next_power_of_two().trailing_zeros() as usize
        }.min(DEFAULT_MAX_ORDER);

        // Check alignment using PFN
        if pfn & ((1 << order) - 1) != 0 {
            warn!("zone {}: Page PFN {} is not properly aligned for order {} (needs alignment to {} pages)",
                   self.zone_id, pfn, order, 1 << order);
            return;
        }

        // Check page alignment
        if addr & (PAGE_SIZE - 1) != 0 {
            warn!("zone {}: Attempt to free page at non-page-aligned address {:#x}", 
                  self.zone_id, addr);
            return;
        }

        // Initialize block for merging
        let mut current_pfn = pfn;

        // Try to merge with buddy blocks (Linux-style)
        // Loop continues while we can find and merge with a buddy
        while order < DEFAULT_MAX_ORDER {
            // Calculate buddy PFN using XOR operation (same as Linux kernel)
            let buddy_pfn = current_pfn ^ (1 << order);

            // Verify buddy is within the zone
            let buddy_addr = buddy_pfn * PAGE_SIZE;
            if !self.addr_in_zone(buddy_addr) {
                // Buddy is outside this zone, cannot merge
                break;
            }

            // Try to find buddy in free list
            if let Some(buddy_pos) = self.find_block_in_free_list(order, buddy_addr) {
                // Verify buddy has correct order and address
                if let Some(buddy_node) = &self.free_lists[order].nodes[buddy_pos] {
                    if buddy_node.data.order != order || buddy_node.data.addr != buddy_addr {
                        warn!("zone {}: Inconsistent buddy block found at PFN {}", 
                              self.zone_id, buddy_pfn);
                        break;
                    }
                }

                // Remove buddy from free list
                self.remove_block_from_free_list(order, buddy_pos);

                // Merge: use the aligned address (lower address)
                // This is equivalent to Linux's: combined_pfn = buddy_pfn & page_pfn
                current_pfn = current_pfn & buddy_pfn;

                // Move to next order
                order += 1;

                trace!("zone {}: Merged blocks at PFN {} and {} to order {}",
                       self.zone_id, current_pfn, buddy_pfn, order);
            } else {
                // No buddy found, cannot merge further
                break;
            }
        }

        // Add the final merged block to the appropriate free list
        let final_addr = current_pfn * PAGE_SIZE;
        let block = BuddyBlock {
            order,
            addr: final_addr,
        };

        let success = self.free_lists[order].push_back(block);
        if !success {
            warn!("zone {}: Failed to push block to free list: addr={:#x}, order={}, PFN={}",
                   self.zone_id, final_addr, order, current_pfn);
        }
    }

    pub fn get_stats(&self) -> BuddyStats {
        let mut stats = BuddyStats::new();
        stats.total_pages = self.total_pages;
        
        for (order, list) in self.free_lists.iter().enumerate() {
            let pages_in_order = list.len() * (1 << order);
            stats.free_pages_by_order[order] = list.len();
            stats.free_pages += pages_in_order;
        }
        
        stats.used_pages = stats.total_pages.saturating_sub(stats.free_pages);
        stats
    }

    /// Get detailed free list information as a string
    pub fn get_free_lists_info(&self) -> alloc::string::String {
        let mut result = alloc::string::String::new();
        result.push_str(&alloc::format!("Zone {}:\n", self.zone_id));
        result.push_str(&alloc::format!("Base Address: {:#x}\n", self.base_addr));
        result.push_str(&alloc::format!("End Address: {:#x}\n", self.end_addr));
        result.push_str(&alloc::format!("Total Pages: {}\n", self.total_pages));

        for (order, list) in self.free_lists.iter().enumerate() {
            if !list.is_empty() {
                let block_size = (1 << order) * PAGE_SIZE;
                let total_pages_in_order = list.len() * (1 << order);
                let total_bytes_in_order = total_pages_in_order * PAGE_SIZE;

                result.push_str(&alloc::format!(
                    "Order {}: {} blocks, {} bytes each, {} total pages ({} KB, {} MB)\n",
                    order,
                    list.len(),
                    block_size,
                    total_pages_in_order,
                    total_bytes_in_order / 1024,
                    total_bytes_in_order / (1024 * 1024)
                ));

                // Print first few block addresses for debugging
                let mut current_idx = list.head;
                let mut count = 0;
                result.push_str("  Block addresses: ");
                while let Some(idx) = current_idx {
                    if let Some(node) = &list.nodes[idx] {
                        result.push_str(&alloc::format!("{:#x} ", node.data.addr));
                        current_idx = node.next;
                        count += 1;
                        if count >= 8 { // Show max 8 addresses to avoid spam
                            result.push_str("... ");
                            break;
                        }
                    } else {
                        break;
                    }
                }
                result.push_str("\n");
            }
        }

        // Print summary
        let stats = self.get_stats();
        result.push_str("\nZone Summary:\n");
        result.push_str(&alloc::format!("  Free pages: {} / {}\n", stats.free_pages, stats.total_pages));
        result.push_str(&alloc::format!("  Used pages: {}\n", stats.used_pages));
        result.push_str(&alloc::format!("  Free memory: {} KB / {} KB\n",
            (stats.free_pages * PAGE_SIZE) / 1024,
            (stats.total_pages * PAGE_SIZE) / 1024
        ));
        result.push_str("------------------------------\n");

        result
    }
}

impl Default for BuddySet {
    fn default() -> Self {
        Self::empty()
    }
}

/// Buddy page allocator with multi-zone support
/// Similar to Linux kernel's zone-based memory management
pub struct BuddyPageAllocator {
    zones: [BuddySet; MAX_ZONES],
    num_zones: usize,
    stats: BuddyStats,
}

impl BuddyPageAllocator {
    pub const fn new() -> Self {
        Self {
            zones: [const { BuddySet::empty() }; MAX_ZONES],
            num_zones: 0,
            stats: BuddyStats::new(),
        }
    }

    pub fn bootstrap(&mut self, base_addr: usize, size: usize) {
        info!("buddy allocator: Bootstrap with region [{:#x}, {:#x})", 
              base_addr, base_addr + size);

        // Initialize the first zone
        if self.num_zones >= MAX_ZONES {
            panic!("Cannot bootstrap: maximum zones reached");
        }

        self.zones[0] = BuddySet::new(base_addr, size, 0);
        self.zones[0].init(base_addr, size);
        self.num_zones = 1;

        self.update_stats();
    }

    pub fn get_stats(&self) -> BuddyStats {
        self.stats
    }

    /// Get detailed free list information as a string
    pub fn get_free_lists_info(&self) -> alloc::string::String {
        let mut result = alloc::string::String::new();
        result.push_str("=== Multi-Zone Buddy Allocator Info ===\n");
        result.push_str(&alloc::format!("Total Zones: {}\n", self.num_zones));
        result.push_str("\n");

        for i in 0..self.num_zones {
            let zone_info = self.zones[i].zone_info();
            result.push_str(&alloc::format!("Zone {}:\n", i));
            result.push_str(&alloc::format!("  Range: [{:#x}, {:#x})\n", 
                  zone_info.start_addr, zone_info.end_addr));
            result.push_str(&alloc::format!("  Total Pages: {}\n", zone_info.total_pages));
            result.push_str(&self.zones[i].get_free_lists_info());
            result.push_str("\n");
        }

        // Print overall summary
        let stats = self.get_stats();
        result.push_str("Overall Summary:\n");
        result.push_str(&alloc::format!("  Total pages: {}\n", stats.total_pages));
        result.push_str(&alloc::format!("  Free pages: {}\n", stats.free_pages));
        result.push_str(&alloc::format!("  Used pages: {}\n", stats.used_pages));
        result.push_str("====================================\n");

        result
    }

    /// Update aggregated statistics from all zones
    fn update_stats(&mut self) {
        let mut total_stats = BuddyStats::new();
        
        for i in 0..self.num_zones {
            let zone_stats = self.zones[i].get_stats();
            total_stats.total_pages += zone_stats.total_pages;
            total_stats.free_pages += zone_stats.free_pages;
            total_stats.used_pages += zone_stats.used_pages;
            for (order, &count) in zone_stats.free_pages_by_order.iter().enumerate() {
                total_stats.free_pages_by_order[order] += count;
            }
        }
        
        self.stats = total_stats;
    }

    /// Add a new memory region as a new zone
    /// Try to merge with adjacent zones if they are physically contiguous
    fn add_memory_region(&mut self, start: usize, size: usize) -> AllocResult {
        info!("buddy allocator: Adding region [{:#x}, {:#x})", start, start + size);

        if self.num_zones >= MAX_ZONES {
            error!("buddy allocator: Cannot add region: maximum zones ({}) reached", MAX_ZONES);
            return Err(AllocError::NoMemory);
        }

        // 向下对齐起始地址到页边界
        let aligned_start = start & !(PAGE_SIZE - 1);
        // 向上对齐结束地址到页边界
        let end = start + size;
        let aligned_end = (end + PAGE_SIZE - 1) & !(PAGE_SIZE - 1);

        // 计算对齐后的大小
        let aligned_size = aligned_end - aligned_start;

        // 如果对齐后没有有效内存，返回错误
        if aligned_size == 0 || aligned_size < PAGE_SIZE {
            warn!("buddy allocator: Aligned size is too small: {:#x}, skipping region", aligned_size);
            return Err(AllocError::InvalidParam);
        }

        // Check for overlap with existing zones
        for i in 0..self.num_zones {
            let zone = &self.zones[i];
            if !(aligned_end <= zone.base_addr || aligned_start >= zone.end_addr) {
                error!("buddy allocator: Region [{:#x}, {:#x}) overlaps with zone {} [{:#x}, {:#x})",
                       aligned_start, aligned_end, i, zone.base_addr, zone.end_addr);
                return Err(AllocError::MemoryOverlap);
            }
        }

        info!("buddy allocator: Adjusted region [{:#x}, {:#x}) (original [{:#x}, {:#x}))",
              aligned_start, aligned_end, start, end);

        // Try to merge with adjacent zones first
        if let Some(_merged) = self.try_merge_with_adjacent_zones(aligned_start, aligned_end) {
            info!("buddy allocator: Merged region with adjacent zones");
            self.update_stats();
            return Ok(());
        }

        // Cannot merge, create a new zone
        let zone_id = self.num_zones;
        self.zones[zone_id] = BuddySet::new(aligned_start, aligned_size, zone_id);
        self.zones[zone_id].init(aligned_start, aligned_size);
        self.num_zones += 1;

        info!("buddy allocator: Added new zone {} with {} pages, total zones now: {}",
              zone_id, aligned_size / PAGE_SIZE, self.num_zones);

        Ok(())
    }

    /// Try to merge the new region with adjacent zones
    /// Returns true if merge was successful
    fn try_merge_with_adjacent_zones(&mut self, start: usize, end: usize) -> Option<bool> {
        let mut adjacent_zones: Vec<usize> = Vec::new();

        // Find zones that are adjacent to the new region
        for i in 0..self.num_zones {
            let zone = &self.zones[i];
            if zone.end_addr == start || zone.base_addr == end {
                adjacent_zones.push(i);
            }
        }

        if adjacent_zones.is_empty() {
            return None;
        }

        info!("buddy allocator: Found {} adjacent zones, attempting merge", adjacent_zones.len());

        // For now, handle the simple case of merging with one adjacent zone
        // This could be extended to merge multiple zones
        if adjacent_zones.len() == 1 {
            let zone_idx = adjacent_zones[0];
            let zone = &self.zones[zone_idx];

            let (merged_start, merged_end) = if zone.end_addr == start {
                // New region is after existing zone: [zone_start, new_end)
                (zone.base_addr, end)
            } else {
                // New region is before existing zone: [new_start, zone_end)
                (start, zone.end_addr)
            };

            info!("buddy allocator: Merging zone {} [{:#x}, {:#x}) with new region [{:#x}, {:#x}) into [{:#x}, {:#x})",
                  zone_idx, zone.base_addr, zone.end_addr, start, end, merged_start, merged_end);

            // Reinitialize the zone with the merged range
            let merged_size = merged_end - merged_start;
            self.zones[zone_idx] = BuddySet::new(merged_start, merged_size, zone_idx);
            self.zones[zone_idx].init(merged_start, merged_size);

            return Some(true);
        }

        warn!("buddy allocator: Cannot merge multiple adjacent zones yet, creating new zone");
        None
    }

    /// Find the zone that contains the given address
    fn find_zone_for_addr(&self, addr: usize) -> Option<usize> {
        for i in 0..self.num_zones {
            if self.zones[i].addr_in_zone(addr) {
                return Some(i);
            }
        }
        None
    }

    /// Print detailed debug information when multi-zone allocation fails
    fn print_multi_zone_alloc_failure_debug(&self, num_pages: usize, align_pow2: usize) {
        error!("buddy allocator: Allocation of {} pages ({} bytes, alignment: {}) failed",
               num_pages, num_pages * PAGE_SIZE, align_pow2);
        error!("buddy allocator: Detailed allocation request:");
        error!("  Requested pages: {}", num_pages);
        error!("  Requested bytes: {} ({} MB)",
               num_pages * PAGE_SIZE,
               (num_pages * PAGE_SIZE) / (1024 * 1024));
        error!("  Alignment: {} (0x{:x})", align_pow2, align_pow2);

        // Print detailed memory state
        let stats = self.get_stats();
        error!("\nbuddy allocator: Current Memory State:");
        error!("  Total zones: {}", self.num_zones);
        error!("  Total pages: {} ({} MB)",
               stats.total_pages,
               (stats.total_pages * PAGE_SIZE) / (1024 * 1024));
        error!("  Free pages: {} ({} MB)",
               stats.free_pages,
               (stats.free_pages * PAGE_SIZE) / (1024 * 1024));
        error!("  Used pages: {} ({} MB)",
               stats.used_pages,
               (stats.used_pages * PAGE_SIZE) / (1024 * 1024));

        // Print zone details
        error!("\nbuddy allocator: Zone Details:");
        for i in 0..self.num_zones {
            let zone_info = self.zones[i].zone_info();
            let zone_stats = self.zones[i].get_stats();
            error!("  Zone {}:", i);
            error!("    Range: [{:#x}, {:#x})", zone_info.start_addr, zone_info.end_addr);
            error!("    Size: {} MB", (zone_info.total_pages * PAGE_SIZE) / (1024 * 1024));
            error!("    Free pages: {} / {}", zone_stats.free_pages, zone_info.total_pages);
            error!("    Used pages: {}", zone_stats.used_pages);

            // Print free blocks by order
            error!("    Free blocks by order:");
            let mut has_free = false;
            for order in 0..=DEFAULT_MAX_ORDER {
                let count = zone_stats.free_pages_by_order[order];
                if count > 0 {
                    has_free = true;
                    error!("      Order {}: {} blocks ({} MB each, {} MB total)",
                           order,
                           count,
                           ((1 << order) * PAGE_SIZE) / (1024 * 1024),
                           (count * (1 << order) * PAGE_SIZE) / (1024 * 1024));
                }
            }
            if !has_free {
                error!("      No free blocks available");
            }
        }

        // Print max allocatable block sizes
        error!("\nbuddy allocator: Maximum Allocatable Blocks:");
        for i in 0..self.num_zones {
            let zone_stats = self.zones[i].get_stats();
            let mut max_order = None;
            for order in (0..=DEFAULT_MAX_ORDER).rev() {
                if zone_stats.free_pages_by_order[order] > 0 {
                    max_order = Some(order);
                    break;
                }
            }
            match max_order {
                Some(order) => {
                    error!("  Zone {}: Max order {} ({} MB)",
                           i, order, ((1 << order) * PAGE_SIZE) / (1024 * 1024));
                }
                None => {
                    error!("  Zone {}: No memory available", i);
                }
            }
        }
    }
}

impl Default for BuddyPageAllocator {
    fn default() -> Self {
        Self::new()
    }
}

impl BaseAllocator for BuddyPageAllocator {
    fn init(&mut self, start: usize, size: usize) {
        self.bootstrap(start, size);
    }

    fn add_memory(&mut self, start: usize, size: usize) -> AllocResult {
        info!("buddy allocator: Adding memory region [{:#x}, {:#x})", start, start + size);

        // Add the new memory region as a new zone
        self.add_memory_region(start, size)?;
        self.update_stats();
        self.print_multi_zone_alloc_failure_debug(0, 0);
        Ok(())
    }
}

impl PageAllocator for BuddyPageAllocator {
    const PAGE_SIZE: usize = PAGE_SIZE;

    fn alloc_pages(&mut self, num_pages: usize, align_pow2: usize) -> AllocResult<usize> {
        // Try to allocate from each zone in order
        // Linux uses a fallback strategy: DMA -> NORMAL -> HIGHMEM
        for i in 0..self.num_zones {
            match self.zones[i].alloc_pages(num_pages, align_pow2) {
                Ok(addr) => {
                    self.update_stats();
                    if num_pages > 10 {
                        info!("buddy allocator: Allocated {} pages at {:#x} from zone {}", 
                              num_pages, addr, i);
                    }
                    return Ok(addr);
                }
                Err(_) => {
                    // Try next zone
                    continue;
                }
            }
        }

        // No zone could satisfy the allocation - print debug info
        self.print_multi_zone_alloc_failure_debug(num_pages, align_pow2);
        Err(AllocError::NoMemory)
    }

    fn dealloc_pages(&mut self, pos: usize, num_pages: usize) {
        // Find which zone this address belongs to
        if let Some(zone_idx) = self.find_zone_for_addr(pos) {
            self.zones[zone_idx].dealloc_pages(pos, num_pages);
            self.update_stats();
            // info!("buddy allocator: Deallocated {} pages at {:#x} in zone {}", 
            //       num_pages, pos, zone_idx);
        } else {
            warn!("buddy allocator: Dealloc pages at {:#x}: address not in any zone", pos);
        }
    }

    fn alloc_pages_at(
        &mut self,
        base: usize,
        num_pages: usize,
        align_pow2: usize,
    ) -> AllocResult<usize> {
        // Find the zone that should contain this allocation
        if let Some(zone_idx) = self.find_zone_for_addr(base) {
            // Try to allocate from that specific zone
            match self.zones[zone_idx].alloc_pages(num_pages, align_pow2) {
                Ok(addr) if addr == base => {
                    self.update_stats();
                    Ok(addr)
                }
                Ok(addr) => {
                    // Wrong address, deallocate and return error
                    self.zones[zone_idx].dealloc_pages(addr, num_pages);
                    Err(AllocError::InvalidParam)
                }
                Err(e) => Err(e),
            }
        } else {
            warn!("buddy allocator: alloc_pages_at: address {:#x} not in any zone", base);
            Err(AllocError::InvalidParam)
        }
    }

    fn total_pages(&self) -> usize {
        self.stats.total_pages
    }

    fn used_pages(&self) -> usize {
        self.stats.used_pages
    }

    fn available_pages(&self) -> usize {
        self.stats.free_pages
    }
}

impl PageAllocatorForSlab for BuddyPageAllocator {
    fn alloc_pages(&mut self, num_pages: usize, align_pow2: usize) -> AllocResult<usize> {
        // Use the same allocation logic as PageAllocator
        <Self as PageAllocator>::alloc_pages(self, num_pages, align_pow2)
    }

    fn dealloc_pages(&mut self, pos: usize, num_pages: usize) {
        // Use the same deallocation logic as PageAllocator
        <Self as PageAllocator>::dealloc_pages(self, pos, num_pages)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_buddy_allocator_basic() {
        let mut allocator = BuddyPageAllocator::new();
        
        // Test initialization
        let base_addr = 0x80000000;
        let size = 0x100000; // 1MB
        allocator.init(base_addr, size);

        // Test allocation
        match PageAllocator::alloc_pages(&mut allocator, 1, PAGE_SIZE) {
            Ok(page_addr) => {
                PageAllocator::dealloc_pages(&mut allocator, page_addr, 1);
            }
            Err(_) => panic!("Page allocation failed"),
        }

        let stats = allocator.get_stats();
        assert!(stats.total_pages > 0);
    }

    #[test]
    fn test_buddy_merge() {
        let mut allocator = BuddyPageAllocator::new();
        
        let base_addr = 0x80000000;
        let size = 0x10000; // 64KB = 16 pages
        allocator.init(base_addr, size);

        // Allocate two adjacent pages
        let addr1 = PageAllocator::alloc_pages(&mut allocator, 1, PAGE_SIZE).unwrap();
        let addr2 = PageAllocator::alloc_pages(&mut allocator, 1, PAGE_SIZE).unwrap();
        
        // Deallocate both - they should merge back
        PageAllocator::dealloc_pages(&mut allocator, addr1, 1);
        PageAllocator::dealloc_pages(&mut allocator, addr2, 1);
        
        let stats = allocator.get_stats();
        assert_eq!(stats.free_pages, 16); // Should be back to all free
    }
}
