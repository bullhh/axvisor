fn main() {
    let addr = 0x8249000;
    let align_pow2 = 16;
    
    println!("addr: {:#x}", addr);
    println!("align_pow2: {}", align_pow2);
    println!("addr % align_pow2 = {:#x}", addr % align_pow2);
    println!("addr % align_pow2 == 0: {}", addr % align_pow2 == 0);
    
    // 验证二进制表示
    println!("addr in binary: {:b}", addr);
    println!("align_pow2 in binary: {:b}", align_pow2);
    println!("addr & (align_pow2 - 1) = {:#x}", addr & (align_pow2 - 1));
}
