//! Static shared list pool for buddy allocator
//!
//! Provides a pool of static linked lists that can be dynamically allocated
//! and returned by different orders, allowing the buddy system to handle
//! more blocks than the fixed capacity of individual lists.

use super::{buddy_block::BuddyBlock, linked_list::StaticLinkedList, DEFAULT_MAX_ORDER};

/// Static shared list pool - all orders share lists from a common pool
///
/// Configuration:
/// - TOTAL_LISTS: Total number of lists in the pool (64 lists)
/// - LIST_CAPACITY: Capacity of each list (64 blocks)
/// - MAX_ORDERS: Number of orders (0-28, so 29)
///
/// This allows up to TOTAL_LISTS * LIST_CAPACITY = 64 * 64 = 4096 blocks total
/// shared across all orders, compared to the original 64 * 29 = 1856 blocks.
pub struct StaticSharedPool<const TOTAL_LISTS: usize, const LIST_CAPACITY: usize, const MAX_ORDERS: usize> {
    /// All lists in the pool
    lists: [StaticLinkedList<BuddyBlock, LIST_CAPACITY>; TOTAL_LISTS],
    /// Usage tracking: 0 = free, order+1 = in use by that order
    usage: [usize; TOTAL_LISTS],
    /// Count of lists allocated to each order
    order_list_count: [usize; MAX_ORDERS],
}

impl<const TOTAL_LISTS: usize, const LIST_CAPACITY: usize, const MAX_ORDERS: usize>
    StaticSharedPool<TOTAL_LISTS, LIST_CAPACITY, MAX_ORDERS>
{
    /// Create a new static shared pool (uninitialized, must call init())
    pub const fn new() -> Self {
        Self {
            lists: unsafe { core::mem::zeroed() },
            usage: [const { 0 }; TOTAL_LISTS],
            order_list_count: [const { 0 }; MAX_ORDERS],
        }
    }

    /// Initialize all lists in the pool
    pub fn init(&mut self) {
        for list in &mut self.lists {
            list.init();
        }
        // All lists are initially free (usage[i] = 0)
    }

    /// Allocate a list for the given order
    ///
    /// Returns the index of the allocated list, or None if no free lists available
    pub fn alloc_list(&mut self, order: usize) -> Option<usize> {
        // Find a free list
        for (i, &usage) in self.usage.iter().enumerate() {
            if usage == 0 {
                // Found a free list, allocate it
                self.usage[i] = order + 1;
                self.order_list_count[order] += 1;
                return Some(i);
            }
        }
        None
    }

    /// Free a list back to the pool
    ///
    /// Panics if the list index is invalid or not allocated to the given order
    pub fn free_list(&mut self, list_idx: usize, order: usize) {
        if list_idx >= TOTAL_LISTS {
            panic!("Invalid list index: {}", list_idx);
        }

        if self.usage[list_idx] != order + 1 {
            panic!(
                "List {} is not allocated to order {} (usage={})",
                list_idx, order, self.usage[list_idx]
            );
        }

        // Mark the list as free
        self.usage[list_idx] = 0;
        self.order_list_count[order] -= 1;

        // The list should be empty when returned to the pool
        if !self.lists[list_idx].is_empty() {
            panic!(
                "List {} being freed for order {} is not empty",
                list_idx, order
            );
        }
    }

    /// Get a list reference by index
    pub fn get_list(&self, list_idx: usize) -> &StaticLinkedList<BuddyBlock, LIST_CAPACITY> {
        if list_idx >= TOTAL_LISTS {
            panic!("Invalid list index: {}", list_idx);
        }
        &self.lists[list_idx]
    }

    /// Get a mutable list reference by index
    pub fn get_list_mut(
        &mut self,
        list_idx: usize,
    ) -> &mut StaticLinkedList<BuddyBlock, LIST_CAPACITY> {
        if list_idx >= TOTAL_LISTS {
            panic!("Invalid list index: {}", list_idx);
        }
        &mut self.lists[list_idx]
    }

    /// Get all lists belonging to a specific order
    ///
    /// Returns an iterator over the list indices for the given order
    pub fn get_order_lists(&self, order: usize) -> impl Iterator<Item = usize> + '_ {
        self.usage
            .iter()
            .enumerate()
            .filter(move |(_, &usage)| usage == order + 1)
            .map(|(i, _)| i)
    }

    /// Find the first list for an order that has available space
    ///
    /// Returns the index of the first list with len < LIST_CAPACITY, or None if
    /// either no lists are allocated or all are full
    pub fn find_available_list_for_order(&self, order: usize) -> Option<usize> {
        for (i, &usage) in self.usage.iter().enumerate() {
            if usage == order + 1 {
                // This list belongs to the order, check if it has space
                if self.lists[i].len() < LIST_CAPACITY {
                    return Some(i);
                }
            }
        }
        None
    }

    /// Get the number of lists allocated to a specific order
    pub fn order_list_count(&self, order: usize) -> usize {
        if order >= MAX_ORDERS {
            return 0;
        }
        self.order_list_count[order]
    }

    /// Get the number of free lists in the pool
    pub fn free_list_count(&self) -> usize {
        self.usage.iter().filter(|&&u| u == 0).count()
    }

    /// Get total blocks in all lists of a specific order
    pub fn order_total_blocks(&self, order: usize) -> usize {
        self.get_order_lists(order)
            .map(|i| self.lists[i].len())
            .sum()
    }

    /// Get pool statistics
    pub fn get_stats(&self) -> PoolStats {
        let mut stats = PoolStats::default();
        stats.total_lists = TOTAL_LISTS;
        stats.free_lists = self.free_list_count();
        stats.used_lists = TOTAL_LISTS - stats.free_lists;

        for order in 0..MAX_ORDERS {
            stats.lists_by_order[order] = self.order_list_count(order);
            stats.blocks_by_order[order] = self.order_total_blocks(order);
        }

        stats
    }
}

impl<const TOTAL_LISTS: usize, const LIST_CAPACITY: usize, const MAX_ORDERS: usize> Default
    for StaticSharedPool<TOTAL_LISTS, LIST_CAPACITY, MAX_ORDERS>
{
    fn default() -> Self {
        Self::new()
    }
}

/// Pool statistics
#[derive(Debug, Default)]
pub struct PoolStats {
    pub total_lists: usize,
    pub free_lists: usize,
    pub used_lists: usize,
    pub lists_by_order: [usize; DEFAULT_MAX_ORDER + 1],
    pub blocks_by_order: [usize; DEFAULT_MAX_ORDER + 1],
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_pool_allocation() {
        let mut pool: StaticSharedPool<8, 4, 4> = StaticSharedPool::new();
        pool.init();

        // All lists should be free initially
        assert_eq!(pool.free_list_count(), 8);

        // Allocate lists for order 0
        let idx1 = pool.alloc_list(0).unwrap();
        assert_eq!(pool.free_list_count(), 7);
        assert_eq!(pool.order_list_count(0), 1);

        let _idx2 = pool.alloc_list(0).unwrap();
        assert_eq!(pool.free_list_count(), 6);
        assert_eq!(pool.order_list_count(0), 2);

        // Free a list
        pool.free_list(idx1, 0);
        assert_eq!(pool.free_list_count(), 7);
        assert_eq!(pool.order_list_count(0), 1);

        // Allocate for different order
        let _idx3 = pool.alloc_list(2).unwrap();
        assert_eq!(pool.free_list_count(), 6);
        assert_eq!(pool.order_list_count(2), 1);
    }

    #[test]
    fn test_pool_exhaustion() {
        let mut pool: StaticSharedPool<4, 4, 4> = StaticSharedPool::new();
        pool.init();

        // Allocate all lists
        let mut indices = alloc::vec::Vec::new();
        for _ in 0..4 {
            indices.push(pool.alloc_list(0).unwrap());
        }

        assert_eq!(pool.free_list_count(), 0);

        // Should fail to allocate more
        assert!(pool.alloc_list(0).is_none());

        // Free one and allocate again
        pool.free_list(indices[0], 0);
        assert_eq!(pool.free_list_count(), 1);
        assert!(pool.alloc_list(0).is_some());
    }

    #[test]
    fn test_get_order_lists() {
        let mut pool: StaticSharedPool<8, 4, 4> = StaticSharedPool::new();
        pool.init();

        let idx0_1 = pool.alloc_list(0).unwrap();
        let idx0_2 = pool.alloc_list(0).unwrap();
        let idx1_1 = pool.alloc_list(1).unwrap();
        let _idx2_1 = pool.alloc_list(2).unwrap();

        let order0_lists: alloc::vec::Vec<_> = pool.get_order_lists(0).collect();
        assert_eq!(order0_lists.len(), 2);
        assert!(order0_lists.contains(&idx0_1));
        assert!(order0_lists.contains(&idx0_2));

        let order1_lists: alloc::vec::Vec<_> = pool.get_order_lists(1).collect();
        assert_eq!(order1_lists.len(), 1);
        assert_eq!(order1_lists[0], idx1_1);
    }

    #[test]
    fn test_order_total_blocks() {
        let mut pool: StaticSharedPool<4, 4, 4> = StaticSharedPool::new();
        pool.init();

        let idx = pool.alloc_list(0).unwrap();

        // Add blocks to the list
        let list = pool.get_list_mut(idx);
        list.insert_sorted(BuddyBlock { order: 0, addr: 0x1000 });
        list.insert_sorted(BuddyBlock { order: 0, addr: 0x2000 });

        assert_eq!(pool.order_total_blocks(0), 2);
    }
}
