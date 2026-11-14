//! Embeds the app icon and version information in airbridge.exe on Windows.

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
        let env = |key: &str| std::env::var(key).unwrap_or_default();
        // Forward slashes: rc treats backslashes in strings as escapes.
        let icon = format!("{}/packaging/windows/airbridge.ico", env("CARGO_MANIFEST_DIR")).replace('\\', "/");
        let rc = std::fs::read_to_string("airbridge.rc")
            .expect("reading airbridge.rc")
            .replace("@ICON@", &icon)
            .replace("@MAJOR@", &env("CARGO_PKG_VERSION_MAJOR"))
            .replace("@MINOR@", &env("CARGO_PKG_VERSION_MINOR"))
            .replace("@PATCH@", &env("CARGO_PKG_VERSION_PATCH"))
            .replace("@VERSION@", &env("CARGO_PKG_VERSION"));
        assert!(!rc.contains('@'), "airbridge.rc has an unfilled @...@ value");
        let generated = std::path::Path::new(&env("OUT_DIR")).join("airbridge.rc");
        std::fs::write(&generated, rc).expect("writing the generated airbridge.rc");
        embed_resource::compile(&generated, embed_resource::NONE)
            .manifest_optional()
            .expect("embedding the Windows icon");
    }
}
