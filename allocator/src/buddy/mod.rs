//! Buddy page allocator module
//!
//! This module provides a complete buddy system implementation with:
//! - Sorted linked lists for efficient contiguity checking
//! - Multi-zone support
//! - Detailed statistics and debugging

pub mod buddy_allocator;
pub mod buddy_block;
pub mod buddy_set_pool;
pub mod global_node_pool;
pub mod linked_list;
pub mod pooled_list;
pub mod stats;

pub use buddy_allocator::BuddyPageAllocator;
pub use buddy_block::{BuddyBlock, ZoneInfo, MAX_BLOCKS_PER_LIST, MAX_ZONES};
pub use buddy_set_pool::BuddySetPool;
pub use global_node_pool::{GlobalNodePool, GLOBAL_TOTAL_NODES};
pub use linked_list::Iter;
pub use linked_list::StaticLinkedList;
pub use pooled_list::PooledLinkedList;
pub use stats::{BuddyStats, DEFAULT_MAX_ORDER};

pub const PAGE_SIZE: usize = 0x1000;
