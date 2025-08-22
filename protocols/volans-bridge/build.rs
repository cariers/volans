use std::env;
use std::io::Result;
use std::path::PathBuf;

fn main() -> Result<()> {
    let out_dir = PathBuf::from(env::var("OUT_DIR").unwrap());

    println!("cargo:rerun-if-changed=./proto");
    println!("cargo:rerun-if-changed=./proto/bridge.proto");

    // 构建 prost 配置
    let mut config = prost_build::Config::new();
    config.out_dir(&out_dir);
    config.compile_protos(&["./proto/bridge.proto"], &["./proto"])?;
    Ok(())
}
