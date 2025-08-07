use smmuv3::*;
use std::os::arceos::modules::{axalloc, axhal};
use memory_addr::{PhysAddr, VirtAddr, PAGE_SIZE_4K};
use core::{alloc::Layout, ptr::NonNull};
use std::os::arceos::modules::axdma::{alloc_coherent, dealloc_coherent, BusAddr, DMAInfo};
use crate::vmm::VMRef;


#[cfg(target_arch = "aarch64")]
use crate::utils::cache::cache_clean_invalidate_d;
 
pub struct Smmuv3PagingHandler;

impl PagingHandler for Smmuv3PagingHandler {

    const SID_BITS_SET:u32 = 18;
    const CMDQ_EVENTQ_BITS_SET:u32 = 16;

    fn alloc_pages(num_pages: usize) -> Option<PhysAddr> {
        let alloc_size = num_pages * PAGE_SIZE_4K;
        let align_bits = u32::min(Self::SID_BITS_SET + 6, Self::CMDQ_EVENTQ_BITS_SET + 4);
        let align = 1 << align_bits;
        let layout = Layout::from_size_align(alloc_size, align).unwrap();
        info!("align: 0x{:x}, num_pages: {}", align, num_pages);
        match unsafe { alloc_coherent(layout) } {
            Ok(dma_info) => Some(PhysAddr::from(
                dma_info.bus_addr.as_u64() as usize,
            )),
            Err(_) => None,
        }
    }

    // fn alloc_pages(num_pages: usize) -> Option<PhysAddr> {
    //     // Allocate contiguous 4K pages using the SMMUv3 allocator.
    //     info!("Allocating {} pages", num_pages);

    //     axalloc::global_allocator()
    //         .alloc_pages(num_pages, 0x80_0000).ok().map(PhysAddr::from)
    // }

    fn dealloc_pages(paddr: PhysAddr, num_pages: usize) {
        // Deallocate the allocated physical pages.
        axalloc::global_allocator()
            .dealloc_pages(paddr.into(), num_pages)
    }

    fn phys_to_virt(paddr: PhysAddr) -> VirtAddr {
        // Convert physical address to virtual address.
        axhal::mem::phys_to_virt(paddr)
    }

    fn flush(start: usize, len: usize) {
        unsafe {
            cache_clean_invalidate_d(start, len);
        }
    }

    fn wait_until(duration: core::time::Duration) -> Result<(), &'static str> {
        axhal::time::busy_wait_until(duration);
        Ok(())
    }
}

pub fn init_smmuv3(vm: VMRef) -> SMMUv3<Smmuv3PagingHandler> {
    // let mut smmuv3 = SMMUv3::<Smmuv3PagingHandler>::new(0x09050000 as *mut u8);
    // info!("Initializing SMMUv3 at address: 0x{:x?}", 0x09050000);
    let mut smmuv3 = SMMUv3::<Smmuv3PagingHandler>::new(0x30000000 as *mut u8);
    // info!("Initializing SMMUv3 at address: 0x{:x?}", 0x30000000);
    smmuv3.init();

    smmuv3.add_all_devices(vm.id(), vm.ept_root());

    info!("smmuv3 version: {:?}", smmuv3.version());

    smmuv3
}
