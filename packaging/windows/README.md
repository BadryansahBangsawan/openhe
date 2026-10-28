# OpenHE on Windows

Windows needs no driver and no installer: `openhe-app.exe` is a single portable executable. The keyboard
is a standard HID device, so the app opens it directly.

The file icon and the taskbar icon come from `assets/icons/openhe.ico`, embedded into the `.exe` as a
resource. This is done by a `build.rs` in the `openhe-app` crate using the
[`winresource`](https://crates.io/crates/winresource) crate. These files are not wired in yet: the
integration step adds them. Copy the snippets below as they are.

## 1. `crates/openhe-app/Cargo.toml`

```toml
[build-dependencies]
winresource = "0.1"
```

`winresource` is compiled on every host but only called when the target is Windows, so Linux and macOS
builds are unaffected (the `build.rs` below does nothing on other targets).

## 2. `crates/openhe-app/build.rs`

```rust
// Embeds the Windows icon and version info into openhe-app.exe.
// On other targets this does nothing.
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
```

Notes:
- `#![forbid(unsafe_code)]` is not affected; `build.rs` contains no `unsafe`.
- The path is relative to the crate directory (`crates/openhe-app`).
- Building on Windows with the MSVC toolchain works out of the box (`rc.exe` from the Windows SDK). For
  `x86_64-pc-windows-gnu` or cross compiling from Linux, `winresource` needs `windres`
  (`x86_64-w64-mingw32-windres`).
- To hide the console window of the GUI, the app's `main.rs` needs
  `#![cfg_attr(all(not(debug_assertions), target_os = "windows"), windows_subsystem = "windows")]`.
  This belongs to the app crate and is left to the integration step.

## 3. The icon file

`assets/icons/openhe.ico` has seven PNG-compressed sizes (16, 24, 32, 48, 64, 128 and 256 px), so it is
sharp from the taskbar to the large Explorer view. Sizes up to 32 px use the small-size logo cut.

## 4. Release

CI (`.github/workflows/release.yml`) builds `openhe-app.exe` on `windows-latest` and attaches it as
`OpenHE-<tag>-windows-x86_64.exe`. The executable is not code-signed, so Windows SmartScreen may show
"Windows protected your PC" on first launch: choose "More info", then "Run anyway".
