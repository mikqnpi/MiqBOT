use std::fs;
use std::path::PathBuf;

fn main() {
    let protoc = protoc_bin_vendored::protoc_bin_path().expect("protoc not found");
    std::env::set_var("PROTOC", protoc);

    let mut cfg = prost_build::Config::new();
    cfg.include_file("pb_mod.rs");
    cfg.compile_protos(
        &["../proto/MiqBOT_bridge_v1.proto"],
        &["../proto"],
    )
    .expect("prost_build failed");

    let out_dir = PathBuf::from(std::env::var("OUT_DIR").expect("OUT_DIR missing"));
    let mut generated_files = fs::read_dir(&out_dir)
        .expect("read OUT_DIR failed")
        .filter_map(|entry| entry.ok())
        .map(|entry| entry.file_name().to_string_lossy().to_string())
        .filter(|name| name.ends_with(".rs"))
        .filter(|name| name != "pb_mod.rs" && name != "pb_wrapper.rs")
        .collect::<Vec<_>>();
    generated_files.sort();

    assert!(
        generated_files.len() == 1,
        "expected exactly one generated proto file, got {:?}",
        generated_files
    );

    let wrapper = format!(
        "pub mod bridge_v1 {{ include!(\\\"{}\\\"); }}\\n",
        generated_files[0]
    );
    fs::write(out_dir.join("pb_wrapper.rs"), wrapper).expect("write pb_wrapper.rs failed");

    println!("cargo:rerun-if-changed=../proto/MiqBOT_bridge_v1.proto");
}
