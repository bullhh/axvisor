//! Slab allocator implementation.
//!
//! This module implements an improved slab allocator for small object allocation
//! with pooled linked lists, inspired by asterinas design.

pub mod slab_byte_allocator;
pub mod slab_cache;
pub mod slab_node;
pub mod slab_node_pool;
pub mod slab_pooled_list;

// Re-export public types
pub use slab_byte_allocator::{PageAllocatorForSlab, SizeClass, SlabByteAllocator};
