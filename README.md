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

OpenHE adalah proyek aplikasi desktop native, ringan, dan open source untuk mengatur keyboard Hall-effect
**AULA WIN 68 HE MAX** dan **AULA WIN 60 HE MAX**. Ditulis dengan Rust, ditargetkan untuk Windows, Linux, dan
macOS, bicara langsung ke keyboard lewat USB. Tanpa browser, tanpa akun. Internet hanya dipakai untuk memeriksa
dan mengunduh pembaruan OpenHE; tidak ada telemetri.

OpenHE tidak berafiliasi dengan AULA.

> **Status: dalam pengembangan.**
> - Alat CLI `openhe` sudah berfungsi. Perintah baca-saja (`list`, `info`, `dump`) sudah diverifikasi pada
>   satu AULA WIN 68 HE MAX sungguhan, dan satu penulisan (`set-actuation`) sudah diverifikasi dan dipulihkan.
> - Aplikasi GUI (`openhe-app`) sudah punya semua halaman. Di macOS sudah dipakai dengan WIN 68 HE MAX
>   sungguhan; di Windows dan Linux baru lulus tes otomatis (dengan simulator keyboard).
> - Rilis tersedia di halaman [Releases](https://github.com/BadryansahBangsawan/openhe/releases).

## Keyboard yang didukung

| Model | VID | PID |
|---|---|---|
| AULA WIN 68 HE MAX | `0x1CA2` | `0x1901` |
| AULA WIN 60 HE MAX | `0x1CA2` | `0x1902` |

## Fitur (rencana, belum semuanya tersedia)

Targetnya semua fitur AULA HUB kecuali update firmware. Hari ini hanya CLI (lihat di bawah) yang berfungsi;
sisanya sedang dikerjakan di GUI:

- Performa: titik aktuasi per tombol, rapid trigger (sensitivitas tekan/lepas), deadzone atas dan bawah, tampilan kedalaman tombol langsung
- Tombol lanjutan: DKS, MT, TGL, MPT, SOCD, END
- Remap tombol di 4 layer (Base, Fn1, Fn2, Fn3), Fn dilindungi
- Macro: rekam, edit langkah, mode putar, pasang ke tombol
- Lampu: efek, warna per tombol, kecerahan, kecepatan, arah, waktu tidur
- Polling rate (125 Hz sampai 8 kHz), 4 profil di keyboard, mode Win/Mac
- Kalibrasi, reset pabrik (dengan konfirmasi ketik)
- Ekspor/impor profil ke JSON, pergantian profil otomatis per aplikasi, jalan saat login

Antarmuka GUI direncanakan berbahasa Indonesia.

## Instalasi

Unduh file untuk sistem Anda dari halaman [Releases](https://github.com/BadryansahBangsawan/openhe/releases),
atau bangun dari source (bagian berikutnya).

**macOS** (11 atau lebih baru): unduh `OpenHE-<versi>-macos-arm64.zip`, buka zip, seret `OpenHE.app` ke
Applications. Aplikasi hanya ditandatangani ad-hoc, jadi Gatekeeper akan memblokir saat pertama dibuka:
- macOS 14 atau lebih lama: klik kanan `OpenHE.app`, pilih Buka, lalu Buka lagi.
- macOS 15 atau lebih baru: buka aplikasi sekali (akan ditolak), lalu buka System Settings, Privacy & Security,
  gulir ke bawah, dan klik "Open Anyway" untuk OpenHE.

Jika keyboard tidak terdeteksi, sebagai langkah pemecahan masalah yang belum terkonfirmasi, coba izinkan
OpenHE di System Settings, Privacy & Security, Input Monitoring. Catatan protokol menduga izin ini
kemungkinan besar tidak diperlukan.

**Windows**: unduh `OpenHE-<versi>-windows-x86_64.exe` dan jalankan. Tidak perlu driver atau installer.
Karena belum ditandatangani, SmartScreen bisa muncul: pilih More info, lalu Run anyway.

**Linux**: unduh `OpenHE-<versi>-linux-x86_64.AppImage`, beri izin eksekusi (`chmod +x`), lalu jalankan.
Pasang aturan udev satu kali agar pengguna biasa boleh membuka keyboard:

```sh
sudo cp packaging/linux/70-openhe.rules /etc/udev/rules.d/70-openhe.rules
sudo udevadm control --reload-rules
sudo udevadm trigger
```

Aturan udev ini juga diperlukan untuk menjalankan CLI hasil build dari source di Linux. Cabut dan colok lagi keyboard. Detail ada di [`packaging/linux/README.md`](packaging/linux/README.md).

## Pembaruan

OpenHE bisa memperbarui dirinya sendiri dari Pengaturan, kartu **Pembaruan**:

- **Periksa sekarang** meminta daftar rilis ke GitHub (`api.github.com`, repo ini). Ini satu-satunya
  pemakaian internet oleh OpenHE: tidak ada telemetri, dan tidak ada data lain yang dikirim.
- **Periksa pembaruan otomatis** (bawaan: nyala) memeriksa paling sering sekali sehari saat OpenHE dibuka.
  Bila ada versi baru, muncul pemberitahuan kecil di sidebar. Matikan sakelar ini bila tidak mau ada
  pemeriksaan sama sekali.
- **Ikut versi uji (beta)** (bawaan: mati) juga menawarkan pra-rilis.
- **Unduh & pasang** mengunduh file untuk sistem Anda. Setiap rilis membawa manifest (`openhe-update.json`)
  yang ditandatangani Ed25519; OpenHE hanya menerima manifest yang cocok dengan kunci publik di dalam
  aplikasi, lalu memeriksa ukuran dan SHA-256 file sebelum mengganti apa pun. Setelah terpasang, tekan
  **Mulai ulang sekarang** (tidak bisa ditekan selama perubahan masih dikirim ke keyboard).

Cara penggantiannya per sistem:

- macOS: `OpenHE.app` diganti di tempatnya. Folder tempat aplikasi itu berada harus bisa ditulis (misalnya
  Applications). Jika macOS menjalankan aplikasi dari lokasi sementara (dibuka langsung dari Downloads),
  pindahkan dulu `OpenHE.app` ke Applications.
- Windows: file `.exe` yang sedang berjalan diganti; versi lama disimpan sebagai `.exe.old` lalu dihapus
  saat OpenHE dibuka berikutnya.
- Linux: hanya versi AppImage yang bisa memperbarui diri.

Jika pembaruan otomatis tidak bisa (build sendiri, folder tidak bisa ditulis, dan sebagainya), unduh versi
baru secara manual dari halaman [Releases](https://github.com/BadryansahBangsawan/openhe/releases); kartu
Pembaruan juga menautkannya.

## Build dari source

Butuh Rust stable (1.88 atau lebih baru).

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
  libwayland-dev libx11-dev libxcursor-dev libxrandr-dev libxi-dev \
  libdbus-1-dev libxcb1-dev
```

Paket per platform:

- macOS: `packaging/macos/build-app.sh` membuat `dist/macos/OpenHE.app` (opsi `--zip`)
- Linux: `packaging/linux/build-appimage.sh` membuat AppImage dengan linuxdeploy
- Windows: ikon dan info versi di-embed ke `openhe-app.exe` oleh `build.rs` (crate `winresource`), lihat
  [`packaging/windows/README.md`](packaging/windows/README.md)

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
- OpenHE menulis pengaturan ke keyboard. Desainnya: setiap penulisan dibaca ulang untuk verifikasi, dan jika
  gagal, aplikasi membaca ulang keadaan keyboard dan tidak memercayai cache-nya.
- Ekspor profil Anda ke JSON sebelum mencoba pengaturan besar. Reset pabrik meminta Anda mengetik `RESET`.
- Protokol diteliti dari analisis statis aplikasi AULA HUB. Sejauh ini baru perintah baca-saja dan satu
  penulisan aktuasi yang diverifikasi dan dipulihkan pada satu WIN 68 HE MAX; sisanya belum diuji di
  perangkat keras. OpenHE proyek komunitas tanpa jaminan. Pakai dengan risiko sendiri.

## Lisensi

[MIT](LICENSE). Logo dan ikon: lihat [`assets/logo/README.md`](assets/logo/README.md).

---

<a id="english"></a>

# OpenHE (English)

OpenHE is a project to build a lightweight, native, open-source desktop app for configuring the
**AULA WIN 68 HE MAX** and **AULA WIN 60 HE MAX** Hall-effect keyboards. It is written in Rust, targets
Windows, Linux and macOS, and talks to the keyboard directly over USB. No browser, no account. The internet
is only used to check for and download OpenHE updates; there is no telemetry.

OpenHE is not affiliated with AULA.

> **Status: in development.**
> - The `openhe` CLI works today. Its read-only commands (`list`, `info`, `dump`) have been verified on one
>   real WIN 68 HE MAX, and one write (`set-actuation`) has been verified and restored.
> - The GUI app (`openhe-app`) has all its pages. On macOS it has been used with a real WIN 68 HE MAX; on
>   Windows and Linux it has only passed the automated tests (against a keyboard simulator).
> - Releases are on the [Releases](https://github.com/BadryansahBangsawan/openhe/releases) page.

## Supported keyboards

| Model | VID | PID |
|---|---|---|
| AULA WIN 68 HE MAX | `0x1CA2` | `0x1901` |
| AULA WIN 60 HE MAX | `0x1CA2` | `0x1902` |

## Features (planned, not all available yet)

The goal is everything in AULA HUB except firmware update. Today only the CLI (see below) works; the rest is
being built in the GUI:

- Performance: per-key actuation point, rapid trigger (press/release sensitivity), top and bottom deadzones, live key-depth view
- Advanced keys: DKS, MT, TGL, MPT, SOCD, END
- Remapping on 4 layers (Base, Fn1, Fn2, Fn3), with the Fn keys protected
- Macros: record, step editor, play modes, bind to a key
- Lighting: effects, per-key colour, brightness, speed, direction, sleep timeout
- Polling rate (125 Hz to 8 kHz), 4 onboard profiles, Win/Mac mode
- Calibration and factory reset (with a typed confirmation)
- Profile import/export as JSON, automatic per-app profile switching, launch at login

The GUI is planned to be in Indonesian.

## Install

Download the file for your system from the [Releases](https://github.com/BadryansahBangsawan/openhe/releases)
page, or build from source (next section).

**macOS** (11 or newer): download `OpenHE-<version>-macos-arm64.zip`, unzip it and drag `OpenHE.app` to
Applications. The app is only ad-hoc signed, so Gatekeeper blocks it on first launch:
- macOS 14 or older: right-click `OpenHE.app`, choose Open, then Open again.
- macOS 15 or newer: try to open the app once (it is refused), then go to System Settings, Privacy & Security,
  scroll down and click "Open Anyway" for OpenHE.

If the keyboard is not detected, an unconfirmed troubleshooting step is to allow OpenHE in System Settings,
Privacy & Security, Input Monitoring. The protocol notes suggest this permission is probably not needed.

**Windows**: download `OpenHE-<version>-windows-x86_64.exe` and run it. No driver or installer needed. It is
not code-signed yet, so SmartScreen may warn: choose More info, then Run anyway.

**Linux**: download `OpenHE-<version>-linux-x86_64.AppImage`, make it executable (`chmod +x`) and run it.
Install the udev rule once so that your normal user may open the keyboard:

```sh
sudo cp packaging/linux/70-openhe.rules /etc/udev/rules.d/70-openhe.rules
sudo udevadm control --reload-rules
sudo udevadm trigger
```

The rule is also needed to run a CLI built from source on Linux. Then unplug and replug the keyboard. More in [`packaging/linux/README.md`](packaging/linux/README.md).

## Update

OpenHE can update itself from Pengaturan (Settings), card **Pembaruan**:

- **Periksa sekarang** (check now) asks GitHub (`api.github.com`, this repository) for the list of releases.
  This is the only network use of OpenHE: no telemetry, nothing else is sent.
- **Periksa pembaruan otomatis** (check automatically, on by default) checks at most once a day when OpenHE
  starts. When a newer version exists, a small notice shows in the sidebar. Turn it off for no checks at all.
- **Ikut versi uji (beta)** (off by default) also offers pre-releases.
- **Unduh & pasang** (download and install) downloads the file for your system. Every release carries an
  Ed25519-signed manifest (`openhe-update.json`); OpenHE only accepts a manifest that verifies against the
  public key built into the app, then checks the file's size and SHA-256 before replacing anything. Once it
  is installed, press **Mulai ulang sekarang** (restart now); it waits while changes are still being sent to
  the keyboard.

How the app is replaced:

- macOS: `OpenHE.app` is replaced where it is. The folder that holds it must be writable (e.g. Applications).
  If macOS runs the app from a temporary location (opened straight from Downloads), move `OpenHE.app` to
  Applications first.
- Windows: the running `.exe` is replaced; the old one is kept as `.exe.old` and deleted on the next start.
- Linux: only the AppImage can update itself.

When self-update is not possible (a build from source, a folder you cannot write to, ...), download the new
version by hand from the [Releases](https://github.com/BadryansahBangsawan/openhe/releases) page; the
Pembaruan card links to it.

## Build from source

You need stable Rust (1.88 or newer).

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
  libwayland-dev libx11-dev libxcursor-dev libxrandr-dev libxi-dev \
  libdbus-1-dev libxcb1-dev
```

Per-platform packages:

- macOS: `packaging/macos/build-app.sh` builds `dist/macos/OpenHE.app` (`--zip` also writes a zip)
- Linux: `packaging/linux/build-appimage.sh` builds an AppImage with linuxdeploy
- Windows: the icon and version info are embedded into `openhe-app.exe` by `build.rs` (the `winresource`
  crate), see [`packaging/windows/README.md`](packaging/windows/README.md)

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
- OpenHE writes settings to your keyboard. By design every write is read back and verified, and after a failed
  write the app re-reads the keyboard and does not trust its cache.
- Export your profile to JSON before trying large changes. Factory reset asks you to type `RESET`.
- The protocol was worked out from a static read of the AULA HUB app. So far only the read-only commands and
  one actuation write (verified and restored) have been tested on a single WIN 68 HE MAX; the other write
  features have not been tested on hardware. OpenHE is a community project with no warranty. Use it at your
  own risk.

## Docs

- [`docs/protocol/PROTOCOL.md`](docs/protocol/PROTOCOL.md): HID protocol notes
- [`assets/logo/`](assets/logo/): logo kit and usage

## License

[MIT](LICENSE). OpenHE is not affiliated with AULA.
