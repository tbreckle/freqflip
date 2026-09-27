fn main() {
    // CI injects the version computed by scripts/version.sh. Cargo.toml stays at 0.0.0, so
    // local builds are marked as unofficial.
    println!("cargo:rerun-if-env-changed=FREQFLIP_VERSION");
    let version = std::env::var("FREQFLIP_VERSION")
        .ok()
        .filter(|v| !v.is_empty())
        .unwrap_or_else(|| env!("CARGO_PKG_VERSION").to_string());
    println!("cargo:rustc-env=FREQFLIP_VERSION={version}");

    // Windows: embed the application icon and version info into the .exe.
    println!("cargo:rerun-if-changed=images/freqflip.ico");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        let mut res = winresource::WindowsResource::new();
        res.set_icon("images/freqflip.ico")
            .set("ProductName", "FreqFlip")
            .set("FileDescription", "FreqFlip")
            .set("ProductVersion", &version)
            .set("LegalCopyright", "MIT License");
        if let Err(e) = res.compile() {
            // Cross-checks from other hosts have no resource compiler; Windows builds need it.
            if cfg!(windows) {
                panic!("could not embed the Windows resources: {e}");
            }
            println!("cargo:warning=Windows resources not embedded: {e}");
        }
    }
}
