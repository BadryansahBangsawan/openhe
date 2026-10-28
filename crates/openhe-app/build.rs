//! Embeds the Windows icon and version info into openhe-app.exe. On other
//! targets this does nothing (see packaging/windows/README.md).

#![forbid(unsafe_code)]

fn main() {
    println!("cargo:rerun-if-changed=../../assets/icons/openhe.ico");
    println!("cargo:rerun-if-changed=build.rs");

    // CARGO_CFG_TARGET_OS is the target, not the host, so cross builds work too.
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        let mut res = winresource::WindowsResource::new();
        res.set_icon("../../assets/icons/openhe.ico");
        res.set("ProductName", "OpenHE");
        res.set("FileDescription", "OpenHE keyboard configurator");
        res.set("LegalCopyright", "Copyright (c) 2026 OpenHE contributors");
        res.compile().expect("failed to embed the Windows icon");
    }
}
