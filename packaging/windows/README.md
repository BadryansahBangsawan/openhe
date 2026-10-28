# OpenHE on Windows

Windows needs no driver and no installer: `openhe-app.exe` is a single portable executable. The keyboard
is a standard HID device, so the app opens it directly.

The file icon and the taskbar icon come from `assets/icons/openhe.ico`, embedded into the `.exe` as a
resource by `crates/openhe-app/build.rs` with the [`winresource`](https://crates.io/crates/winresource)
crate. This is wired in: every Windows build of `openhe-app` carries the icon and the version info.

## 1. `crates/openhe-app/Cargo.toml`

```toml
[build-dependencies]
winresource = { version = "0.1.34", default-features = false }
```

`winresource` is compiled on every host but only called when the target is Windows, so Linux and macOS
builds are unaffected (`build.rs` does nothing on other targets). The default `toml` feature is off: no
Cargo.toml metadata is read, and the crate builds on Rust 1.88.

## 2. `crates/openhe-app/build.rs`

`build.rs` sets the icon (`../../assets/icons/openhe.ico`), `ProductName`, `FileDescription` and
`LegalCopyright`, and only runs `winresource` when `CARGO_CFG_TARGET_OS` (the target, not the host) is
`windows`, so cross builds work too.

Notes:
- `#![forbid(unsafe_code)]` holds in `build.rs` too.
- The icon path is relative to the crate directory (`crates/openhe-app`).
- Building on Windows with the MSVC toolchain works out of the box (`rc.exe` from the Windows SDK). For
  `x86_64-pc-windows-gnu` or cross compiling from Linux, `winresource` needs `windres`
  (`x86_64-w64-mingw32-windres`).
- Release builds hide the console window: `main.rs` has
  `#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]`.

## 3. The icon file

`assets/icons/openhe.ico` has seven PNG-compressed sizes (16, 24, 32, 48, 64, 128 and 256 px), so it is
sharp from the taskbar to the large Explorer view. Sizes up to 32 px use the small-size logo cut.

## 4. Release

CI (`.github/workflows/release.yml`) builds `openhe-app.exe` on `windows-latest` and attaches it as
`OpenHE-<tag>-windows-x86_64.exe`. The executable is not code-signed, so Windows SmartScreen may show
"Windows protected your PC" on first launch: choose "More info", then "Run anyway".
