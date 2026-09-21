fn main() {
    // 交叉编译出来的二进制要能自报目标三元组，免得在主机上认错构建产物
    println!(
        "cargo:rustc-env=TARGET_TRIPLE={}",
        std::env::var("TARGET").unwrap()
    );
    println!("cargo:rerun-if-changed=build.rs");
}
