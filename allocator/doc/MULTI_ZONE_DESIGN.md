# Multi-Zone Buddy Allocator Design

## Overview

This document describes the multi-zone buddy allocator implementation for Axvisor, inspired by Linux kernel's zone-based memory management.

## Design Goals

1. **Support for multiple non-contiguous memory regions**: Similar to how Linux uses zones (ZONE_DMA, ZONE_NORMAL, ZONE_HIGHMEM)
2. **Isolation between memory regions**: Each zone manages its own memory independently
3. **Fallback allocation strategy**: Try zones in order, falling back to next zone if current fails
4. **Robust validation**: Ensure addresses belong to valid zones before operations

## Architecture

### Key Components

#### ZoneInfo
```rust
pub struct ZoneInfo {
    pub start_addr: usize,
    pub end_addr: usize,
    pub total_pages: usize,
    pub zone_id: usize,
}
```

Represents metadata about a memory zone.

#### BuddySet (Single Zone)
Each `BuddySet` represents one memory zone:
- Has its own free lists for each order
- Manages a contiguous memory region
- Independent buddy merging within the zone

#### BuddyPageAllocator (Multi-Zone Manager)
```rust
pub struct BuddyPageAllocator {
    zones: [BuddySet; MAX_ZONES],
    num_zones: usize,
    stats: BuddyStats,
}
```

Manages up to `MAX_ZONES` (8) independent memory zones.

## Key Differences from Linux

### Similarities

1. **Zone-based organization**: Like Linux, we divide memory into zones
2. **Independent buddy systems**: Each zone has its own buddy allocator
3. **Fallback allocation**: Try zones in order (Linux: DMA -> NORMAL -> HIGHMEM)

### Simplifications

1. **Fixed maximum zones**: 8 zones (vs Linux's dynamic allocation)
2. **No zone types**: All zones are equal (vs Linux's DMA/NORMAL/HIGHMEM types)
3. **No NUMA support**: Single node architecture
4. **No migratetype**: No fragmentation mitigation through migration types

## Memory Layout Example

```
Zone 0: 0x80000000 - 0x81000000  (16MB)
Zone 1: 0x90000000 - 0x91000000  (16MB)
Zone 2: 0xA0000000 - 0xA0800000  (8MB)
```

## API Usage

### Initialization

```rust
let mut allocator = BuddyPageAllocator::new();

// Initialize first zone
allocator.init(0x80000000, 0x01000000);

// Add additional zones for non-contiguous memory
allocator.add_memory(0x90000000, 0x01000000)?;
allocator.add_memory(0xA0000000, 0x00800000)?;
```

### Allocation

Allocation tries zones in order (0, 1, 2, ...):

```rust
// Allocates from first available zone
let addr = allocator.alloc_pages(num_pages, alignment)?;
```

### Deallocation

Automatically finds the correct zone:

```rust
// Finds the zone containing this address
allocator.dealloc_pages(addr, num_pages);
```

## Implementation Details

### Zone Management

1. **Zone Addition**: `add_memory()` creates a new zone
   - Validates no overlap with existing zones
   - Initializes the zone's buddy system
   - Updates global statistics

2. **Zone Lookup**: `find_zone_for_addr()` locates the zone for an address
   - Linear search through active zones
   - Returns `None` if address is in no zone

3. **Zone Isolation**: Each zone maintains:
   - Independent free lists
   - Address range validation
   - Separate buddy merging

### Allocation Strategy

```rust
for i in 0..self.num_zones {
    match self.zones[i].alloc_pages(num_pages, align_pow2) {
        Ok(addr) => return Ok(addr),  // Success from zone i
        Err(_) => continue,             // Try next zone
    }
}
Err(AllocError::NoMemory)  // All zones exhausted
```

### Deallocation Strategy

```rust
if let Some(zone_idx) = self.find_zone_for_addr(addr) {
    self.zones[zone_idx].dealloc_pages(addr, num_pages);
} else {
    warn!("Address not in any zone");
}
```

### Buddy Merging

Buddy merging is confined to each zone:

1. Check if buddy PFN is within zone bounds
2. Only merge if buddy is in the same zone
3. Prevents merging across zone boundaries

## Validation

### Zone Isolation

- Addresses validated against zone bounds before operations
- Overlap detection on zone addition
- Invalid zone references rejected

### Address Validation

```rust
fn addr_in_zone(&self, addr: usize) -> bool {
    addr >= self.base_addr && addr < self.end_addr
}
```

## Statistics

Aggregated statistics from all zones:

```rust
pub struct BuddyStats {
    pub total_pages: usize,
    pub free_pages: usize,
    pub used_pages: usize,
    pub free_pages_by_order: [usize; DEFAULT_MAX_ORDER + 1],
}
```

Each zone maintains its own statistics, aggregated on updates.

## Debugging

Use `get_free_lists_info()` to inspect:

```rust
info!("{}", allocator.get_free_lists_info());
```

Output format:
```
=== Multi-Zone Buddy Allocator Info ===
Total Zones: 3

Zone 0:
  Range: [0x80000000, 0x81000000)
  Total Pages: 4096
  Order 0: 16 blocks...
  ...
Zone 1:
  ...

Overall Summary:
  Total pages: 10240
  Free pages: 8192
  Used pages: 2048
```

## Advantages

1. **Memory isolation**: Zones are independent, preventing cross-contamination
2. **Flexible layout**: Supports any non-contiguous memory configuration
3. **Simplified debugging**: Per-zone statistics easier to debug
4. **Extensible**: Easy to add zone-specific policies

## Limitations

1. **Fixed zone count**: Maximum 8 zones
2. **No hotplug**: Cannot add/remove zones dynamically after init
3. **No zone preferences**: Allocation doesn't prefer specific zones
4. **Linear zone search**: O(n) zone lookup during allocation

## Future Enhancements

1. **Zone types**: Add DMA/NORMAL/HIGHMEM semantics
2. **NUMA support**: Multi-node architectures
3. **Hotplug support**: Dynamic zone addition/removal
4. **Zone allocation hints**: Prefer specific zones based on allocation type
5. **Zone priority**: Prioritize certain zones over others

## Testing

See `test_multi_zone.rs` for examples of:
- Multi-zone initialization
- Cross-zone allocation
- Zone exhaustion fallback
- Statistics verification

## References

- Linux Kernel Documentation: `Documentation/vm/`
- Linux Kernel Source: `mm/page_alloc.c`
- Understanding the Linux Kernel, Chapter 8: Memory Management
