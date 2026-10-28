<p align="center">
  <picture>
    <source media="(prefers-color-scheme: dark)" srcset="assets/logo/final/lockup-horizontal-color-on-dark.svg">
    <img src="assets/logo/final/lockup-horizontal-color-on-light.svg" alt="OpenHE" width="420">
  </picture>
</p>

<p align="center">
  Indonesia &middot; <a href="#english">English</a>
</p>

# OpenHE (Bahasa Indonesia)

OpenHE adalah aplikasi desktop native, ringan, dan open source untuk mengatur keyboard Hall-effect
**AULA WIN 68 HE MAX** dan **AULA WIN 60 HE MAX**. Ditulis dengan Rust, berjalan di Windows, Linux, dan macOS,
bicara langsung ke keyboard lewat USB. Tanpa browser, tanpa akun, tanpa internet.

OpenHE tidak berafiliasi dengan AULA.

## Keyboard yang didukung

| Model | VID | PID |
|---|---|---|
| AULA WIN 68 HE MAX | `0x1CA2` | `0x1901` |
| AULA WIN 60 HE MAX | `0x1CA2` | `0x1902` |

## Fitur

Semua fitur AULA HUB, kecuali update firmware:

- Performa: titik aktuasi per tombol, rapid trigger (sensitivitas tekan/lepas), deadzone atas dan bawah, tampilan kedalaman tombol langsung
- Tombol lanjutan: DKS, MT, TGL, MPT, SOCD, END
- Remap tombol di 4 layer (Base, Fn1, Fn2, Fn3), Fn dilindungi
- Macro: rekam, edit langkah, mode putar, pasang ke tombol
- Lampu: efek, warna per tombol, kecerahan, kecepatan, arah, waktu tidur
- Polling rate (125 Hz sampai 8 kHz), 4 profil di keyboard, mode Win/Mac
- Kalibrasi, reset pabrik (dengan konfirmasi ketik)
- Ekspor/impor profil ke JSON, pergantian profil otomatis per aplikasi, jalan saat login

Antarmuka berbahasa Indonesia.

## Instalasi

Unduh versi terbaru dari halaman Releases.

**macOS** (11 atau lebih baru): unduh `OpenHE-<versi>-macos-arm64.zip`, buka zip, seret `OpenHE.app` ke
Applications. Aplikasi hanya ditandatangani ad-hoc. Saat pertama dibuka, klik kanan lalu pilih Buka. Jika
keyboard tidak terdeteksi, izinkan OpenHE di System Settings, Privacy & Security, Input Monitoring.

**Windows**: unduh `OpenHE-<versi>-windows-x86_64.exe` dan jalankan. Tidak perlu driver atau installer.
Karena belum ditandatangani, SmartScreen bisa muncul: pilih More info, lalu Run anyway.

**Linux**: unduh `OpenHE-<versi>-linux-x86_64.AppImage`, beri izin eksekusi (`chmod +x`), lalu jalankan.
Pasang aturan udev satu kali agar pengguna biasa boleh membuka keyboard:

```sh
sudo cp packaging/linux/70-openhe.rules /etc/udev/rules.d/70-openhe.rules
sudo udevadm control --reload-rules
sudo udevadm trigger
```

Cabut dan colok lagi keyboard. Detail ada di [`packaging/linux/README.md`](packaging/linux/README.md).

## Build dari source

Butuh Rust stable (1.85 atau lebih baru).

```sh
git clone <repository-url> openhe
cd openhe
cargo build --release -p openhe-app      # aplikasi GUI: target/release/openhe-app
cargo build --release -p openhe-cli      # alat CLI: target/release/openhe
cargo test --workspace                   # semua test memakai transport tiruan, tanpa keyboard
```

Di Linux pasang dulu pustaka sistem:

```sh
sudo apt install libudev-dev pkg-config libxkbcommon-dev libgl1-mesa-dev \
  libwayland-dev libx11-dev libxcursor-dev libxrandr-dev libxi-dev
```

Paket per platform:

- macOS: `packaging/macos/build-app.sh` membuat `dist/macos/OpenHE.app` (opsi `--zip`)
- Linux: `packaging/linux/build-appimage.sh` membuat AppImage dengan linuxdeploy
- Windows: ikon di-embed lewat `build.rs`, lihat [`packaging/windows/README.md`](packaging/windows/README.md)

## Alat CLI `openhe`

```sh
openhe list                      # daftar perangkat HID yang terhubung
openhe info                      # info keyboard (model, firmware, mode)
openhe dump                      # info, matriks, dan pengaturan per tombol
openhe dump --json               # sama, dalam JSON
openhe set-actuation A 1.50      # titik aktuasi tombol A = 1,50 mm (menulis ke keyboard, lalu membaca ulang)
openhe --trace dump              # tampilkan setiap laporan HID (hex) ke stderr
```

Tombol bisa berupa huruf A-Z, angka 0-9, atau kode HID heksadesimal seperti `0x2C`. `set-actuation` adalah
satu-satunya perintah yang menulis ke keyboard.

## Catatan keamanan

- **Update firmware di luar cakupan.** OpenHE tidak pernah mengirim perintah bootloader atau flash.
  Gunakan alat resmi AULA untuk itu.
- OpenHE menulis pengaturan ke keyboard. Setiap penulisan dibaca ulang untuk verifikasi. Jika gagal, aplikasi
  membaca ulang keadaan keyboard dan tidak memercayai cache-nya.
- Ekspor profil Anda ke JSON sebelum mencoba pengaturan besar. Reset pabrik meminta Anda mengetik `RESET`.
- Protokol diteliti dari analisis statis aplikasi AULA HUB dan diuji pada keyboard sungguhan, tetapi OpenHE
  proyek komunitas tanpa jaminan. Pakai dengan risiko sendiri.

## Lisensi

[MIT](LICENSE). Logo dan ikon: lihat [`assets/logo/README.md`](assets/logo/README.md).

---

<a id="english"></a>

# OpenHE (English)

OpenHE is a lightweight, native, open-source desktop app for configuring the **AULA WIN 68 HE MAX** and
**AULA WIN 60 HE MAX** Hall-effect keyboards. It is written in Rust, runs on Windows, Linux and macOS, and
talks to the keyboard directly over USB. No browser, no account, no internet connection.

OpenHE is not affiliated with AULA.

## Supported keyboards

| Model | VID | PID |
|---|---|---|
| AULA WIN 68 HE MAX | `0x1CA2` | `0x1901` |
| AULA WIN 60 HE MAX | `0x1CA2` | `0x1902` |

## Features

Everything in AULA HUB except firmware update:

- Performance: per-key actuation point, rapid trigger (press/release sensitivity), top and bottom deadzones, live key-depth view
- Advanced keys: DKS, MT, TGL, MPT, SOCD, END
- Remapping on 4 layers (Base, Fn1, Fn2, Fn3), with the Fn keys protected
- Macros: record, step editor, play modes, bind to a key
- Lighting: effects, per-key colour, brightness, speed, direction, sleep timeout
- Polling rate (125 Hz to 8 kHz), 4 onboard profiles, Win/Mac mode
- Calibration and factory reset (with a typed confirmation)
- Profile import/export as JSON, automatic per-app profile switching, launch at login

The interface is in Indonesian.

## Install

Download the latest build from the Releases page.

**macOS** (11 or newer): download `OpenHE-<version>-macos-arm64.zip`, unzip it and drag `OpenHE.app` to
Applications. The app is only ad-hoc signed, so on first launch right-click it and choose Open. If the
keyboard is not detected, allow OpenHE in System Settings, Privacy & Security, Input Monitoring.

**Windows**: download `OpenHE-<version>-windows-x86_64.exe` and run it. No driver or installer needed. It is
not code-signed yet, so SmartScreen may warn: choose More info, then Run anyway.

**Linux**: download `OpenHE-<version>-linux-x86_64.AppImage`, make it executable (`chmod +x`) and run it.
Install the udev rule once so that your normal user may open the keyboard:

```sh
sudo cp packaging/linux/70-openhe.rules /etc/udev/rules.d/70-openhe.rules
sudo udevadm control --reload-rules
sudo udevadm trigger
```

Then unplug and replug the keyboard. More in [`packaging/linux/README.md`](packaging/linux/README.md).

## Build from source

You need stable Rust (1.85 or newer).

```sh
git clone <repository-url> openhe
cd openhe
cargo build --release -p openhe-app      # GUI app: target/release/openhe-app
cargo build --release -p openhe-cli      # CLI tool: target/release/openhe
cargo test --workspace                   # all tests use a mock transport, no keyboard needed
```

On Linux, install the system libraries first:

```sh
sudo apt install libudev-dev pkg-config libxkbcommon-dev libgl1-mesa-dev \
  libwayland-dev libx11-dev libxcursor-dev libxrandr-dev libxi-dev
```

Per-platform packages:

- macOS: `packaging/macos/build-app.sh` builds `dist/macos/OpenHE.app` (`--zip` also writes a zip)
- Linux: `packaging/linux/build-appimage.sh` builds an AppImage with linuxdeploy
- Windows: the icon is embedded by a `build.rs`, see [`packaging/windows/README.md`](packaging/windows/README.md)

## The `openhe` CLI

```sh
openhe list                      # list connected HID devices
openhe info                      # keyboard info (model, firmware, mode)
openhe dump                      # info, matrix and per-key settings
openhe dump --json               # same, as JSON
openhe set-actuation A 1.50      # set key A's actuation point to 1.50 mm (writes to the keyboard, then reads it back)
openhe --trace dump              # print every HID report (hex) to stderr
```

A key is a letter A-Z, a digit 0-9 or a hex HID usage code such as `0x2C`. `set-actuation` is the only
command that writes to the keyboard.

## Safety notes

- **Firmware updates are out of scope.** OpenHE never sends bootloader or flash commands. Use AULA's
  official tool for that.
- OpenHE writes settings to your keyboard. Every write is read back and verified. After a failed write the
  app re-reads the keyboard and does not trust its cache.
- Export your profile to JSON before trying large changes. Factory reset asks you to type `RESET`.
- The protocol was worked out from a static read of the AULA HUB app and checked on real hardware, but
  OpenHE is a community project with no warranty. Use it at your own risk.

## Docs

- [`docs/protocol/PROTOCOL.md`](docs/protocol/PROTOCOL.md): HID protocol notes
- [`assets/logo/`](assets/logo/): logo kit and usage

## License

[MIT](LICENSE). OpenHE is not affiliated with AULA.
