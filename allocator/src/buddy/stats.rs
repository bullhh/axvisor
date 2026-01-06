//! Statistics and debugging for buddy allocator
//!
//! Provides detailed statistics tracking and failure reporting.

use super::buddy_block::ZoneInfo;

/// Maximum order supported
pub const DEFAULT_MAX_ORDER: usize = 28;

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

    /// Add statistics from another BuddyStats
    pub fn add(&mut self, other: &BuddyStats) {
        self.total_pages += other.total_pages;
        self.free_pages += other.free_pages;
        self.used_pages += other.used_pages;
        for (i, &count) in other.free_pages_by_order.iter().enumerate() {
            self.free_pages_by_order[i] += count;
        }
    }
}

/// Detailed memory statistics reporter
pub struct MemoryStatsReporter;

impl MemoryStatsReporter {
    /// Print detailed allocation failure statistics
    /// This is a standalone function to keep allocation logic clean
    pub fn print_alloc_failure_stats(
        num_zones: usize,
        total_stats: &BuddyStats,
        zone_infos: &[ZoneInfo],
        zone_stats: &[BuddyStats],
        request_pages: usize,
        request_align: usize,
    ) {
        use log::error;
        const PAGE_SIZE: usize = 0x1000;

        error!("========================================");
        error!("ALLOCATION FAILURE STATISTICS");
        error!("========================================");
        error!(
            "Request: {} pages ({} KB, alignment:{})",
            request_pages,
            (request_pages * PAGE_SIZE) / (1024),
            request_align
        );
        error!("========================================");

        error!("Overall Memory State:");
        error!("  Total zones: {}", num_zones);
        error!(
            "  Total pages: {} ({} MB)",
            total_stats.total_pages,
            (total_stats.total_pages * PAGE_SIZE) / (1024 * 1024)
        );
        error!(
            "  Free pages: {} ({} MB)",
            total_stats.free_pages,
            (total_stats.free_pages * PAGE_SIZE) / (1024 * 1024)
        );
        error!(
            "  Used pages: {} ({} MB)",
            total_stats.used_pages,
            (total_stats.used_pages * PAGE_SIZE) / (1024 * 1024)
        );
        error!("========================================");

        for i in 0..num_zones {
            error!("Zone {}:", i);
            error!(
                "  Range: [{:#x}, {:#x})",
                zone_infos[i].start_addr, zone_infos[i].end_addr
            );
            error!("  Total pages: {}", zone_infos[i].total_pages);
            error!(
                "  Free pages: {} / {}",
                zone_stats[i].free_pages, zone_infos[i].total_pages
            );
            error!("  Free blocks by order:");

            let mut has_free = false;
            for order in (0..=DEFAULT_MAX_ORDER).rev() {
                let count = zone_stats[i].free_pages_by_order[order];
                if count > 0 {
                    has_free = true;
                    let block_size = (1 << order) * PAGE_SIZE;
                    let total_mb = (count * block_size) / (1024 * 1024);
                    error!(
                        "    Order {}: {} blocks ({} MB each, {} MB total)",
                        order,
                        block_size / (1024 * 1024),
                        count,
                        total_mb
                    );
                }
            }

            if !has_free {
                error!("    No free blocks available");
            }

            // Print max allocatable block
            let mut max_order = None;
            for order in (0..=DEFAULT_MAX_ORDER).rev() {
                if zone_stats[i].free_pages_by_order[order] > 0 {
                    max_order = Some(order);
                    break;
                }
            }

            match max_order {
                Some(order) => {
                    error!(
                        "  Max allocatable: Order {} ({} MB)",
                        order,
                        ((1 << order) * PAGE_SIZE) / (1024 * 1024)
                    );
                }
                None => {
                    error!("  Max allocatable: No memory available");
                }
            }

            error!("----------------------------------------");
        }

        error!("========================================");
    }
}
