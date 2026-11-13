//! Embeds the app icon in airbridge.exe on Windows.

fn main() {
    // `native_window`: this build has its own app window (WebView2 or WebKitGTK).
    println!("cargo::rustc-check-cfg=cfg(native_window)");
    let os = std::env::var("CARGO_CFG_TARGET_OS").unwrap_or_default();
    if os == "windows" || (os == "linux" && std::env::var_os("CARGO_FEATURE_WEBVIEW").is_some()) {
        println!("cargo::rustc-cfg=native_window");
    }
    println!("cargo:rerun-if-changed=airbridge.rc");
    println!("cargo:rerun-if-changed=packaging/windows/airbridge.ico");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        embed_resource::compile("airbridge.rc", embed_resource::NONE)
            .manifest_optional()
            .expect("embedding the Windows icon");
    }
}
