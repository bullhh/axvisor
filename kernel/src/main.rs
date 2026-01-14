#![no_std]
#![no_main]

#[macro_use]
extern crate log;

#[macro_use]
extern crate alloc;

extern crate axstd as std;

extern crate axruntime;
extern crate driver;

mod allocator_benchmark;
mod hal;
mod logo;
mod shell;
mod task;
mod vmm;

#[unsafe(no_mangle)]
fn main() {
    logo::print_logo();

    // Run allocator comprehensive tests
    info!("\n");
    info!("Starting allocator comprehensive benchmarks...");
    allocator_benchmark::run_comprehensive_tests();

    info!("Starting virtualization...");
    info!("Hardware support: {:?}", axvm::has_hardware_support());
    hal::enable_virtualization();

    vmm::init();
    vmm::start();

    info!("[OK] Default guest initialized");



    shell::console_init();
}
