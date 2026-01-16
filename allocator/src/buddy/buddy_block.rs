//! Buddy system constants and types

/// Maximum number of memory zones supported
pub const MAX_ZONES: usize = 10;

/// Maximum order supported
pub const DEFAULT_MAX_ORDER: usize = 28; // Support up to 256GB allocations (2^28 * 4KB)

/// A memory zone descriptor
#[derive(Debug, Clone, Copy)]
pub struct ZoneInfo {
    pub start_addr: usize,
    pub end_addr: usize,
    pub total_pages: usize,
    pub zone_id: usize,
}
