fn main() {
    let addr = 0x9400000usize;

    println!("Testing address access:");
    println!("addr = {:#x}", addr);

    unsafe {
        let ptr = addr as *mut u64;
        println!("ptr = {:?}", ptr);

        // 尝试读取
        let val = *ptr;
        println!("Read successful: value = {:#x}", val);
    }
}
