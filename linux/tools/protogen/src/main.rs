//! Regenerates the Quick Share protobuf code. Run from `linux/`:
//!
//!     cargo run --manifest-path tools/protogen/Cargo.toml
//!
//! The output is committed so building AirBridge needs no protobuf compiler.

use std::path::Path;

fn main() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let proto_dir = root.join("proto/quickshare");
    let out_dir = root.join("src/quickshare/proto");
    std::fs::create_dir_all(&out_dir).expect("create output dir");
    let files = [
        "securemessage.proto",
        "securegcm.proto",
        "ukey.proto",
        "device_to_device_messages.proto",
        "offline_wire_formats.proto",
        "sharing_enums.proto",
        "wire_format.proto",
    ];
    let descriptors =
        protox::compile(files.map(|f| proto_dir.join(f)), [&proto_dir]).expect("compile protos");
    prost_build::Config::new()
        .out_dir(&out_dir)
        .compile_fds(descriptors)
        .expect("generate Rust code");
    println!("Wrote {}", out_dir.display());
}
