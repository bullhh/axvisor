use smmuv3::*;
use std::os::arceos::modules::{axalloc, axhal};
use memory_addr::{align_up_4k, PhysAddr, VirtAddr, PAGE_SIZE_4K};

pub struct Smmuv3PagingHandler;

impl PagingHandler for Smmuv3PagingHandler {
    fn alloc_pages(num_pages: usize) -> Option<PhysAddr> {
        // Allocate contiguous 4K pages using the SMMUv3 allocator.
        axalloc::global_allocator()
            .alloc_pages(num_pages, PAGE_SIZE_4K).ok().map(PhysAddr::from)
    }

    fn dealloc_pages(paddr: PhysAddr, num_pages: usize) {
        // Deallocate the allocated physical pages.
        axalloc::global_allocator()
            .dealloc_pages(paddr.into(), num_pages)
    }

    fn phys_to_virt(paddr: PhysAddr) -> VirtAddr {
        // Convert physical address to virtual address.
        axhal::mem::phys_to_virt(paddr)
    }
}

pub fn init_smmuv3() {
    let mut smmuv3 = SMMUv3::<Smmuv3PagingHandler>::new(0x09050000 as *mut u8);
    info!("Initializing SMMUv3 at address: 0x{:x?}", 0x09050000);
    smmuv3.init();

    info!("smmuv3 version: {:?}", smmuv3.version());

}
