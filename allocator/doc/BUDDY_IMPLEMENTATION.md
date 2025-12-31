# Buddy Page Allocator - Implementation Summary

## Overview
This document summarizes the complete buddy system implementation that follows Linux kernel patterns.

## Key Design Principles

### 1. PFN-Based Addressing
All internal operations use Page Frame Numbers (PFNs) instead of physical addresses:
- `pfn = addr / PAGE_SIZE`
- `addr = pfn * PAGE_SIZE`

This matches Linux kernel's approach and simplifies calculations.

### 2. Correct Buddy Address Calculation
```rust
fn buddy_addr(&self) -> usize {
    self.addr ^ ((1 << self.order) * PAGE_SIZE)
}
```

For a block at order k:
- Block size: `2^k * PAGE_SIZE`
- Buddy offset: `2^k * PAGE_SIZE`
- Buddy address: `addr ^ (2^k * PAGE_SIZE)`

### 3. Merging with Aligned Address
When merging two buddy blocks, we use the lower (aligned) address:
```rust
current_pfn = current_pfn & buddy_pfn;
```

This ensures the merged block is properly aligned for the next higher order.

## Initialization Process

### Linux-Style Initialization
Instead of pre-calculating which orders to create, we:
1. Align the memory region to page boundaries
2. Release each page individually (order 0)
3. Let the buddy system automatically merge them

**Example**: For region `[0x80000000, 0x80100000)` (1MB = 256 pages):
```rust
for pfn in 0..256 {
    dealloc_pages(base_addr + pfn * PAGE_SIZE, 1);
}
```

The automatic merging creates:
- 256 order-0 pages → merge to 128 order-1 blocks
- 128 order-1 blocks → merge to 64 order-2 blocks
- ... and so on until the maximum possible block size

### Alignment Rules
- **Base address**: Align down to page boundary
- **End address**: Align up to page boundary
- Discard the small edge portions (typically < PAGE_SIZE each)

## Allocation Process

### Algorithm
1. Calculate required order (round up to next power of 2)
2. Search from required order to MAX_ORDER for a free block
3. If found, split down to required order
4. Return the block's address

### Splitting Logic
When splitting a block from order k to order k-1:
```rust
while block.order > required_order {
    block.order -= 1;
    let split_size = (1 << block.order) * PAGE_SIZE;
    let buddy_addr = block.addr + split_size;
    push_back(buddy_block { order: block.order, addr: buddy_addr });
}
```

This ensures proper buddy relationships are maintained.

## Deallocation Process

### Algorithm
1. Validate address and alignment
2. Try to merge with buddy blocks (from current order to MAX_ORDER)
3. Add the final merged block to the appropriate free list

### Merging Logic
```rust
while order < DEFAULT_MAX_ORDER {
    let buddy_pfn = current_pfn ^ (1 << order);

    if buddy_exists_in_free_list(order, buddy_pfn) {
        remove_buddy(order, buddy_pfn);
        current_pfn = current_pfn & buddy_pfn;  // Key: use aligned address
        order += 1;
    } else {
        break;  // No buddy, stop merging
    }
}
```

### Key Features
- **PFN-based**: All calculations use PFNs for clarity
- **Validation**: Checks address bounds and alignment
- **Robust merging**: Prevents infinite loops with iteration limit
- **Automatic cleanup**: Returns partially split blocks on failure

## Comparison with Linux Kernel

| Aspect | Our Implementation | Linux Kernel |
|---------|-------------------|---------------|
| **Buddy calculation** | `pfn ^ (1 << order)` | `page_pfn ^ (1UL << order)` |
| **Merge address** | `current_pfn & buddy_pfn` | `combined_pfn = buddy_pfn & page_pfn` |
| **Loop limit** | `order < DEFAULT_MAX_ORDER` | `order < max_order - 1` |
| **Initialization** | Per-page free then merge | Per-page free then merge |
| **Error handling** | Warn and continue | `VM_BUG_ON()` |

## Alignment Requirements

### For Order k Blocks
A block of order k must be aligned to `2^k * PAGE_SIZE`:
```rust
if pfn & ((1 << order) - 1) != 0 {
    // Not aligned!
}
```

### Examples
- Order 0 (1 page): No alignment requirement
- Order 1 (2 pages): Must be 2-page aligned
- Order 2 (4 pages): Must be 4-page aligned
- Order 10 (1024 pages): Must be 1024-page aligned

## Edge Cases Handled

1. **Non-power-of-2 allocation requests**: Round up to next power of 2
2. **Non-aligned memory regions**: Align and use interior pages
3. **Partial merges**: Stop when no buddy is available
4. **Memory region boundaries**: Check buddy is within valid range
5. **Free list exhaustion**: Return error without corrupting state

## Testing

### Basic Test
```rust
let mut allocator = BuddyPageAllocator::new();
allocator.init(0x80000000, 0x100000);  // 1MB
let addr = alloc_pages(&mut allocator, 1, PAGE_SIZE).unwrap();
dealloc_pages(&mut allocator, addr, 1);
```

### Merge Test
```rust
let mut allocator = BuddyPageAllocator::new();
allocator.init(0x80000000, 0x10000);  // 64KB
let addr1 = alloc_pages(&mut allocator, 1, PAGE_SIZE).unwrap();
let addr2 = alloc_pages(&mut allocator, 1, PAGE_SIZE).unwrap();
dealloc_pages(&mut allocator, addr1, 1);
dealloc_pages(&mut allocator, addr2, 1);
// Pages should merge back into larger blocks
```

## Performance Considerations

1. **Free list capacity**: `MAX_BLOCKS_PER_LIST = 64`
   - Limits memory overhead
   - Prevents allocation failures from list exhaustion

2. **Search complexity**: O(n) for finding buddy in free list
   - Acceptable for typical workloads
   - Can be optimized with hash table if needed

3. **Merge efficiency**: Automatic merging reduces fragmentation
   - Linear-time merge per deallocation
   - Maximum O(MAX_ORDER) merges per deallocation

## Known Limitations

1. **Static allocation**: Free lists use fixed-size arrays
   - Maximum 64 blocks per order
   - Could overflow with highly fragmented memory

2. **O(n) search**: Linear search for buddy blocks
   - Could be optimized with O(1) lookup structures

3. **No NUMA support**: Single memory region
   - Linux has per-NUMA node management

## Future Improvements

1. Add per-NUMA node support
2. Optimize buddy lookup with hash table
3. Add memory hotplug support
4. Implement memory reservation tracking
5. Add debug/validation mode for testing

## References

- Linux Kernel: `mm/page_alloc.c` - buddy system implementation
- Understanding the Linux Kernel: Chapter on memory management
- "The Art of Computer Programming, Volume 1" by Donald Knuth (Buddy system discussion)
