fn main() {
    let addr = 0x8249000;
    let align_pow2 = 16;
    
    // 正确的对齐检查：addr & (align_pow2 - 1) == 0
    let is_aligned = addr & (align_pow2 - 1);
    println!("addr: {:#x}", addr);
    println!("align_pow2: {}", align_pow2);
    println!("addr & (align_pow2 - 1) = {:#x}", is_aligned);
    println!("是否对齐: {}", is_aligned == 0);
    
    // 再测试一个明显不对齐的地址
    let addr2 = 0x8249001;
    let is_aligned2 = addr2 & (align_pow2 - 1);
    println!("\naddr2: {:#x}", addr2);
    println!("addr2 & (align_pow2 - 1) = {:#x}", is_aligned2);
    println!("是否对齐: {}", is_aligned2 == 0);
}
