//! Buddy page allocator module
//!
//! This module provides a complete buddy system implementation with:
//! - Bitmap-based representation for efficient memory management
//! - Self-contained metadata within each zone
//! - Multi-zone support
//! - Detailed statistics and debugging

pub mod buddy_allocator;
pub mod buddy_block;
pub mod buddy_set;
pub mod stats;

pub use buddy_allocator::BuddyPageAllocator;
pub use buddy_block::{ZoneInfo, MAX_ZONES};
pub use buddy_set::BuddySet;
#[cfg(feature = "tracking")]
pub use stats::{BuddyStats, DEFAULT_MAX_ORDER};
#[cfg(not(feature = "tracking"))]
pub use stats::DEFAULT_MAX_ORDER;
