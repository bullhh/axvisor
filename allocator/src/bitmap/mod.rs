//! Bitmap allocator module
//!
//! This module provides a bitmap-based page allocator with multi-zone support.
//! Each zone manages its memory with a bitmap stored at the start of the region,
//! eliminating fixed metadata waste.

pub mod bitmap_allocator;

pub use bitmap_allocator::{BitmapAllocator, MAX_ZONES, BitmapZone};
