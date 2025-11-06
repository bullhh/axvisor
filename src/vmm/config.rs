use axaddrspace::GuestPhysAddr;
use axerrno::AxResult;
use axvm::{
    VMMemoryRegion,
    config::{AxVMConfig, AxVMCrateConfig, VmMemMappingType},
};
use core::alloc::Layout;

use crate::vmm::{VM, images::ImageLoader, vm_list::push_vm};

#[cfg(target_arch = "aarch64")]
use crate::vmm::fdt::*;

use alloc::sync::Arc;

#[allow(clippy::module_inception, dead_code)]
pub mod config {
    use alloc::string::String;
    use alloc::vec::Vec;

    /// Default static VM configs. Used when no VM config is provided.
    pub fn default_static_vm_configs() -> Vec<&'static str> {
        vec![]
    }

    /// Read VM configs from filesystem
    #[cfg(feature = "fs")]
    pub fn filesystem_vm_configs() -> Vec<String> {
        use axstd::fs;

        let config_dir = "/guest";
        let mut configs = Vec::new();

        debug!("Read VM config files from filesystem.");

        // Read directories vm1, vm2, ... from /guest
        if let Ok(entries) = fs::read_dir(config_dir) {
            info!("Read VM config files from {}", config_dir);
            
            // Iterate through each entry in /guest
            for entry_result in entries {
                if let Ok(entry) = entry_result {
                    if is_vm_directory(&entry) {
                        if let Some(valid_config) = read_first_valid_config_from_vm_dir(&entry) {
                            configs.push(valid_config);
                        }
                    }
                }
            }
        }
        info!("Found {} VM config files", configs.len());
        configs
    }

    /// Check if a directory entry is a VM directory (vm1, vm2, etc.)
    #[cfg(feature = "fs")]
    fn is_vm_directory(entry: &axstd::fs::DirEntry) -> bool {
        let file_name = entry.file_name().clone();
        let path_str = format!("/guest/{}", file_name);
        
        // Check if the entry is a directory and matches the pattern vmX
        let file_type = entry.file_type();
        if !file_type.is_dir() {
            info!("Entry {} is not a directory", path_str);
            return false;
        }
        true
        // info!("Considering directory: {}", path_str);
        // // Check if directory name starts with "vm" followed by digits
        // if !file_name.starts_with("vm") || file_name.len() <= 2 {
        //     info!("Entry {} does not match vmX pattern", path_str);
        //     return false;
        // }
        // // Check if the rest of the name consists of digits
        // let suffix = &file_name[2..];
        // suffix.chars().all(|c| c.is_ascii_digit())
    }

    /// Read the first valid TOML config file from a VM directory
    #[cfg(feature = "fs")]
    fn read_first_valid_config_from_vm_dir(entry: &axstd::fs::DirEntry) -> Option<String> {
        use axstd::fs;
        
        let file_name = entry.file_name();
        let vm_config_dir = format!("/guest/{}", file_name);
        
        if let Ok(vm_entries) = fs::read_dir(&vm_config_dir) {
            info!("Reading config files from {}", vm_config_dir);
            
            // Look for the first valid TOML file in this directory
            for vm_entry_result in vm_entries {
                if let Ok(vm_entry) = vm_entry_result {
                    let vm_file_name = vm_entry.file_name();
                    info!("Considering file: {}", vm_file_name);
                    // Check if the file has a .toml extension
                    if vm_file_name.ends_with(".toml") {
                        let vm_path_str = format!("{}/{}", vm_config_dir, vm_file_name);
                        if let Some(content) = read_and_validate_config_file(&vm_path_str) {
                            info!("TOML config: {} is valid", vm_path_str);
                            return Some(content);
                        }
                    }
                }
            }
        }
        None
    }

    /// Read and validate a config file
    #[cfg(feature = "fs")]
    fn read_and_validate_config_file(file_path: &str) -> Option<String> {
        use axstd::fs::File;
        use axstd::io::{BufReader, Read};
        
        let toml_file = match File::open(file_path) {
            Ok(file) => file,
            Err(e) => {
                error!("Failed to open file {}: {:?}", file_path, e);
                return None;
            }
        };
        
        let file_size = toml_file.metadata()
            .expect("Failed to get file metadata")
            .len() as usize;

        info!("File {} size: {}", file_path, file_size);

        if file_size == 0 {
            warn!("File {} is empty", file_path);
            return None;
        }

        let mut file = BufReader::new(toml_file);
        let mut buffer = vec![0u8; file_size];
        file.read_exact(&mut buffer).expect("Failed to read file");
        
        info!(
            "Successfully read config file {} as bytes, size: {}",
            file_path,
            buffer.len()
        );
        
        // Convert to string
        let content = alloc::string::String::from_utf8(buffer)
            .expect("Failed to convert bytes to UTF-8 string");

        if content.contains("[base]")
            && content.contains("[kernel]")
            && content.contains("[devices]")
        {
            Some(content)
        } else {
            warn!(
                "File {} does not appear to contain valid VM config structure",
                file_path
            );
            None
        }
    }

    /// Fallback function for when "fs" feature is not enabled
    #[cfg(not(feature = "fs"))]
    pub fn filesystem_vm_configs() -> Vec<String> {
        Vec::new()
    }

    include!(concat!(env!("OUT_DIR"), "/vm_configs.rs"));
}

pub fn get_vm_dtb_arc(_vm_cfg: &AxVMConfig) -> Option<Arc<[u8]>> {
    #[cfg(target_arch = "aarch64")]
    {
        let cache_lock = dtb_cache().lock();
        if let Some(dtb) = cache_lock.get(&_vm_cfg.id()) {
            return Some(Arc::from(dtb.as_slice()));
        }
    }
    None
}

pub fn init_guest_vms() {
    // Initialize the DTB cache in the fdt module
    #[cfg(target_arch = "aarch64")]
    {
        init_dtb_cache();
    }

    // First try to get configs from filesystem if fs feature is enabled
    let mut gvm_raw_configs = config::filesystem_vm_configs();

    // If no filesystem configs found, fallback to static configs
    if gvm_raw_configs.is_empty() {
        let static_configs = config::static_vm_configs();
        // Convert static configs to String type
        gvm_raw_configs.extend(static_configs.into_iter().map(|s| s.into()));
    }

    for raw_cfg_str in gvm_raw_configs {
        info!("Initializing guest VM with config: {:#?}", raw_cfg_str);
        if let Err(e) = init_guest_vm(&raw_cfg_str) {
            error!("Failed to initialize guest VM: {:?}", e);
        }
    }
}

pub fn init_guest_vm(raw_cfg: &str) -> AxResult {
    let vm_create_config =
        AxVMCrateConfig::from_toml(raw_cfg).expect("Failed to resolve VM config");

    if let Some(linux) = super::images::get_image_header(&vm_create_config) {
        debug!(
            "VM[{}] Linux header: {:#x?}",
            vm_create_config.base.id, linux
        );
    }

    #[cfg(target_arch = "aarch64")]
    let mut vm_config = AxVMConfig::from(vm_create_config.clone());

    #[cfg(not(target_arch = "aarch64"))]
    let vm_config = AxVMConfig::from(vm_create_config.clone());

    // Handle FDT-related operations for aarch64
    #[cfg(target_arch = "aarch64")]
    handle_fdt_operations(&mut vm_config, &vm_create_config);

    // info!("after parse_vm_interrupt, crate VM[{}] with config: {:#?}", vm_config.id(), vm_config);
    info!("Creating VM[{}] {:?}", vm_config.id(), vm_config.name());

    // Create VM.
    let vm = VM::new(vm_config).expect("Failed to create VM");
    push_vm(vm.clone());

    vm_alloc_memorys(&vm_create_config, &vm);

    let main_mem = vm
        .memory_regions()
        .first()
        .cloned()
        .expect("VM must have at least one memory region");

    config_guest_address(&vm, &main_mem);

    // Load corresponding images for VM.
    info!("VM[{}] created success, loading images...", vm.id());

    let mut loader = ImageLoader::new(main_mem, vm_create_config, vm.clone());
    loader.load().expect("Failed to load VM images");

    if let Err(e) = vm.init() {
        panic!("VM[{}] setup failed: {:?}", vm.id(), e);
    }

    Ok(())
}

fn config_guest_address(vm: &VM, main_memory: &VMMemoryRegion) {
    const MB: usize = 1024 * 1024;
    vm.with_config(|config| {
        if main_memory.is_identical() {
            debug!(
                "Adjusting kernel load address from {:#x} to {:#x}",
                config.image_config.kernel_load_gpa, main_memory.gpa
            );
            let mut kernel_addr = main_memory.gpa;
            if config.image_config.bios_load_gpa.is_some() {
                kernel_addr += MB * 2; // leave 2MB for BIOS
            }

            config.image_config.kernel_load_gpa = kernel_addr;
            config.cpu_config.bsp_entry = kernel_addr;
            config.cpu_config.ap_entry = kernel_addr;
        }
    });
}

fn vm_alloc_memorys(vm_create_config: &AxVMCrateConfig, vm: &VM) {
    const MB: usize = 1024 * 1024;
    const ALIGN: usize = 2 * MB;

    for memory in &vm_create_config.kernel.memory_regions {
        match memory.map_type {
            VmMemMappingType::MapAlloc => {
                vm.alloc_memory_region(
                    Layout::from_size_align(memory.size, ALIGN).unwrap(),
                    Some(GuestPhysAddr::from(memory.gpa)),
                )
                .expect("Failed to allocate memory region for VM");
            }
            VmMemMappingType::MapIdentical => {
                vm.alloc_memory_region(Layout::from_size_align(memory.size, ALIGN).unwrap(), None)
                    .expect("Failed to allocate memory region for VM");
            }
            VmMemMappingType::MapReserved => {
                info!("VM[{}] map same region: {:#x?}", vm.id(), memory);
                let layout = Layout::from_size_align(memory.size, ALIGN).unwrap();
                vm.map_reserved_memory_region(layout, Some(GuestPhysAddr::from(memory.gpa)))
                    .expect("Failed to map memory region for VM");
            }
        }
    }
}
