//! Embeds the app icon in airbridge.exe on Windows.

fn main() {
    println!("cargo:rerun-if-changed=airbridge.rc");
    println!("cargo:rerun-if-changed=packaging/windows/airbridge.ico");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        embed_resource::compile("airbridge.rc", embed_resource::NONE)
            .manifest_optional()
            .expect("embedding the Windows icon");
    }
}
