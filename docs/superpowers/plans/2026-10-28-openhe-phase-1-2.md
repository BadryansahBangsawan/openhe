# OpenHE: Rencana Implementasi Bertahap

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Aplikasi desktop native Rust (Windows, Linux, macOS) pengganti AULA HUB untuk WIN 68 HE MAX dan WIN 60 HE MAX, dengan semua fitur AULA HUB kecuali update firmware.

**Architecture:** Cargo workspace dengan empat crate. Pembagiannya:
- `openhe-proto`: format paket murni, tanpa I/O, diuji dengan byte persis.
- `openhe-device`: transport HID, sesi request/response, dan API `Keyboard`.
- `openhe-cli`: alat baris perintah untuk menguji ke keyboard asli.
- `openhe-app`: GUI egui/eframe.

Komunikasi dengan keyboard berjalan di thread worker. Antara UI dan worker cukup lewat channel, jadi UI tidak pernah tertahan menunggu keyboard.

**Tech Stack:**
- Rust stable, edition 2021, `rust-version = "1.81"`
- `hidapi` 2.x, `thiserror` 1.x, `clap` 4.x (CLI)
- `eframe`/`egui` 0.29+ (GUI)
- `serde` + `serde_json` dan `directories` 5.x (penyimpanan lokal)

**Spec:** Rancangan disetujui di percakapan pada 2026-10-28. Isinya:
- Ringkasan di bagian "Konteks" di bawah.
- Protokol: `docs/protocol/PROTOCOL.md`.
- Mockup: 7 layar di Lunagraph, project "OpenHE", halaman "App", plus logo konsep A di halaman "Logo".

Task 0 menyalin rencana ini dan spec-nya ke dalam repo.

---

## Konteks

Pengguna tidak mau mengatur keyboard lewat situs web (AULA HUB/WebHID). Dari analisis app resmi AULA HUB 1.1.22 sudah diketahui:
- Keyboard memakai jalur HID vendor (usage page `0xFFA0`) dengan report 64 byte.
- Paket berformat `5C LEN CMD CHK` diikuti payload.
- Fitur dan perintahnya sudah terpetakan di `docs/protocol/PROTOCOL.md`.

Ada beberapa hal yang belum pasti dan hanya bisa dicek di keyboard asli:
- isi byte 0 dan byte status di balasan;
- apakah semua balasan memakai `CMD|0x80`;
- isi matriks DEFKEY;
- format balasan kedalaman live.

Karena itu pengerjaannya bertahap:
1. Inti protokol yang teruji dulu.
2. Alat CLI yang **hanya membaca** dari keyboard untuk memastikan hal-hal yang belum pasti.
3. Baru setelah itu menulis setting dan membangun GUI per halaman.

Keputusan yang sudah diambil:
- **Nama:** OpenHE.
- **Logo:** konsep A "Open Magnet", warna aksen `#FFB224`, sama dengan yang dipakai di mockup.
- **GUI:** egui (murni Rust, bukan Tauri).
- **Bahasa UI:** Indonesia, dengan semua teks dikumpulkan di satu modul supaya bisa diterjemahkan nanti.
- **Tampilan:** tema gelap sesuai mockup.
- **Tidak termasuk:** update firmware dan fitur cloud Aula.
- Keyboard WIN 60 HE MAX ikut didukung.

## Global Constraints

- Perangkat yang didukung:
  - WIN 68 HE MAX: VID `0x1CA2`, PID `0x1901`, product string `"WIN 68 HE MAX"`.
  - WIN 60 HE MAX: VID `0x1CA2`, PID `0x1902`, product string `"WIN 60 HE MAX"`.
  - Interface yang dibuka: usage page `0xFFA0`, usage `1`.
- Tulis ke HID selalu `[0x00] + 64 byte`. Baca 64 byte tanpa report ID.
- Checksum: `(0x35 + 0x5C + LEN + CMD + (1<=LEN<=252 ? PAYLOAD[LEN-1] : 0)) & 0xFF`.
- Balasan: dianggap cocok bila `body[2] == CMD|0x80`. Untuk CMD `0x00`, `raw[5]` (yaitu R[1] setelah header 4 byte dibuang) juga harus sama dengan TYPE.
- Timeout dan percobaan: 1000 ms per percobaan, maksimal 3 percobaan, hanya satu perintah berjalan pada satu waktu.
- Konversi mm ke µm memakai pembulatan, bukan pemotongan (1.13 mm → 1130 µm).
- **Dilarang** membangun atau mengirim perintah bootloader/firmware `0x08`–`0x0E`. Perintah ini juga tidak dimasukkan ke enum `Cmd`.
- Tidak ada akses jaringan sama sekali.
- `#![forbid(unsafe_code)]` di semua crate. Lulus `cargo fmt --check` dan `cargo clippy --all-targets -- -D warnings`.
- `openhe-proto` hanya bergantung pada `thiserror`.
- Setiap tahap yang menulis ke keyboard harus diuji ke keyboard asli setelah pengguna setuju, dan nilai awal dipulihkan sesudahnya.

## Review Focus

1. **Laporan spontan di tengah query.** Pengguna menekan Fn+profil saat query CMD `0x00` sedang berjalan, lalu muncul laporan `0x80` dengan TYPE `0x70`. Laporan ini harus masuk antrean event, bukan dianggap balasan. Diuji di Task 8.
2. **Keyboard dicabut atau berganti koneksi USB** (misalnya setelah ganti polling rate). Yang diharapkan `Error::Disconnected`, tanpa panic, dan app bisa menyambung ulang. Diuji di Task 8.
3. **Remap lebih dari 14 tombol sekaligus** harus dipecah menjadi beberapa paket. Diuji di Task 6.
4. **Nilai mm di luar rentang atau tidak bulat.** 1.13 harus jadi 1130. Nilai di bawah 0.1 atau di atas travel maksimum switch harus ditolak. Diuji di Task 3.
5. **WIN 68 HE PRO** (VID/PID sama, product string berbeda) atau keyboard lain yang terpasang tidak boleh dianggap MAX, dan harus dilaporkan sebagai tidak didukung. Di Linux, izin hidraw yang ditolak harus memberi pesan jelas yang menyebut aturan udev. Diuji di Task 3 dan Task 9.

---

## Peta tahapan

Setiap tahap menghasilkan sesuatu yang bisa dicoba. **Tahap 1–2 dirinci di bawah. Tahap 3–7 diberi rencana rinci sendiri saat tahap sebelumnya selesai**, karena hasil uji baca di Tahap 2 menentukan detailnya.

| Tahap | Hasil | Menulis ke keyboard? |
|---|---|---|
| 1. Inti protokol | `openhe-proto` dengan tes byte persis | Tidak |
| 2. Transport dan CLI baca | `openhe info` / `openhe dump` di keyboard asli; hal yang belum pasti terjawab | Tidak, kecuali Task 12 setelah izin |
| 3. GUI dan halaman Performa | Kerangka app (sidebar, profil, Win/Mac), widget keyboard dari DEFKEY, aktuasi/RT/deadzone/switch, kedalaman live | Ya |
| 4. Remap dan Tombol Lanjutan | 4 layer, pemilih keycode, DKS/MT/TGL/MPT/SOCD/END | Ya |
| 5. Lampu dan Macro | PRGB (20 efek, warna BGR), warna per tombol dan commit `FE`, perekam macro, mode putar, pustaka macro lokal | Ya |
| 6. Kalibrasi dan Pengaturan | Kalibrasi, polling rate (dengan sambung ulang), 4 profil, impor/ekspor, reset pabrik, ganti profil per aplikasi, jalan saat login | Ya |
| 7. Rilis | Kit logo dan ikon app; `.app` macOS, `.exe` Windows, AppImage/`.deb` dan aturan udev Linux; CI 3 OS | Tidak |

## Struktur file (Tahap 1–2)

```
Cargo.toml                      workspace
rust-toolchain.toml             channel = "stable"
crates/openhe-proto/src/
  lib.rs        re-export
  frame.rs      Cmd, Request::encode, checksum, Response::decode, expects()
  units.rs      Micrometers
  model.rs      Model, Switch, PollingRate, SystemMode
  order.rs      CMD 0x00 sub-perintah (OrderType) dan parser-nya
  sync.rs       CMD 0x01 DeviceInfo
  key.rs        CMD 0x23 KeyParam, KeyMode, get/set (pecah per 14)
  defkey.rs     CMD 0x2B Matrix 6×21
crates/openhe-device/src/
  lib.rs        Error
  transport.rs  trait Transport, MockTransport
  hid.rs        HidTransport (hidapi), list_devices, open_first
  session.rs    Session: kirim, cocokkan balasan, retry, antrean event
  keyboard.rs   Keyboard: API tingkat tinggi
crates/openhe-cli/src/main.rs   openhe list | info | dump | set-actuation
packaging/linux/70-openhe.rules
docs/superpowers/specs/2026-10-28-openhe-design.md
docs/superpowers/plans/2026-10-28-openhe-phase-1-2.md
```

---

## Tahap 1: Inti protokol (`openhe-proto`)

### Task 0: Workspace, spec, dan rencana masuk repo

**Files:** Buat `Cargo.toml` (workspace, `members = ["crates/*"]`), `rust-toolchain.toml`, `crates/openhe-proto/Cargo.toml` + `src/lib.rs` kosong, `docs/superpowers/specs/2026-10-28-openhe-design.md` (salin bagian Konteks, Global Constraints, dan peta tahapan rencana ini), dan `docs/superpowers/plans/2026-10-28-openhe-phase-1-2.md` (salinan rencana ini).

- [ ] **Step 1:** `cargo build --workspace` → sukses.
- [ ] **Step 2:** `cargo fmt --check && cargo clippy --all-targets -- -D warnings` → bersih.
- [ ] **Step 3:** Commit `chore: workspace skeleton, design spec and phase 1-2 plan`.

### Task 1: Framing request dan checksum

**Files:** `crates/openhe-proto/src/frame.rs`, tes di modul yang sama.

**Interfaces (Produces):**
- `pub const REPORT_LEN: usize = 64; pub const MAGIC: u8 = 0x5C; pub const MAX_PAYLOAD: usize = 60;`
- `#[repr(u8)] pub enum Cmd { Order=0x00, Sync=0x01, Rm6x21=0x12, Prgb=0x18, Macro=0x20, MacroMode=0x21, Key=0x23, Mt=0x24, Tgl=0x25, Dks=0x26, Mpt=0x27, End=0x28, Krgb=0x2A, DefKey=0x2B, Socd=0x2C }`. Tanpa 0x08–0x0E.
- `pub fn checksum(len: u8, cmd: u8, payload: &[u8]) -> u8`
- `pub struct Request { pub cmd: Cmd, pub payload: Vec<u8> }`
- `impl Request { pub fn encode(&self) -> Result<[u8; REPORT_LEN], FrameError> }`
- `pub enum FrameError { PayloadTooLong(usize), ShortReport, DeviceFail }`

- [ ] **Step 1: Tes gagal.**
  - `encode_sync`: `Request{cmd:Cmd::Sync,payload:vec![1,2,3,4,0xFF,0xFF]}` → 10 byte pertama `5C 06 01 97 01 02 03 04 FF FF`, sisanya 0.
  - `encode_factory_reset`: Order `[0x11,0xFF,0xFF]` → `5C 03 00 93 11 FF FF`.
  - `encode_rm6x21`: `[2,1,0xFF,0xFF]` → `5C 04 12 A6 02 01 FF FF`.
  - `payload_too_long`: 61 byte → `Err(PayloadTooLong(61))`.
  - `checksum_empty_payload`: `checksum(0, 0x01, &[])` = `(0x35+0x5C+0+1) & 0xFF` = `0x92`.
- [ ] **Step 2:** `cargo test -p openhe-proto frame` → FAIL (belum ada).
- [ ] **Step 3:** Implementasi `checksum` dan `encode` sesuai rumus di Global Constraints.
- [ ] **Step 4:** `cargo test -p openhe-proto frame` → PASS.
- [ ] **Step 5:** Commit `feat(proto): request framing and checksum`.

### Task 2: Decode balasan dan aturan pencocokan

**Files:** `frame.rs` (lanjutan).

**Interfaces (Produces):**
- `pub struct Response { pub cmd: u8 /* tanpa bit 0x80 */, pub len: u8, pub raw: [u8; REPORT_LEN] }`
- `impl Response { pub fn decode(report: &[u8]) -> Result<Response, FrameError>; pub fn body(&self) -> &[u8] /* raw[4..] = R */; pub fn checksum_ok(&self) -> bool }`
- `impl Request { pub fn expects(&self, resp: &Response) -> bool }`: `resp.cmd == self.cmd as u8`, dan untuk `Cmd::Order` juga `resp.body()[1] == self.payload[0]`.

Byte 0 balasan tidak divalidasi karena isinya belum diketahui. `checksum_ok()` hanya dicatat untuk diagnosa, tidak dipakai untuk menolak balasan.

- [ ] **Step 1: Tes gagal.**
  - `decode_strips_high_bit`: raw[2]=0xA3 → `cmd==0x23`.
  - `decode_fail`: raw[2]=0xFF → `Err(DeviceFail)`.
  - `decode_short`: 10 byte → `Err(ShortReport)`.
  - `order_reply_matches_type`: query `[0x70,FF,FF]` cocok dengan raw `[?,_,0x80,_, st,0x70,2,...]`, dan **tidak** cocok bila body[1]=0x21.
  - `key_reply_matches`: Request Key cocok dengan raw[2]=0xA3, tidak cocok dengan 0x80.
- [ ] **Step 2:** Jalankan → FAIL.
- [ ] **Step 3:** Implementasi.
- [ ] **Step 4:** Jalankan → PASS.
- [ ] **Step 5:** Commit `feat(proto): response decoding and reply matching`.

### Task 3: Satuan dan model perangkat

**Files:** `units.rs`, `model.rs`.

**Interfaces (Produces):**
- `pub struct Micrometers(pub u16)` dengan `from_mm(mm: f32) -> Result<Self, RangeError>` (dibulatkan, menolak NaN/negatif/>65.535), `to_mm(self) -> f32`, `to_le(self) -> [u8;2]`.
- `pub enum Model { Win68HeMax, Win60HeMax }` dengan `from_hid(vid: u16, pid: u16, product: &str) -> Option<Model>`, `name()`, `switches(self) -> &'static [Switch]`.
- `pub struct Switch { pub id: u16, pub name: &'static str, pub max_travel: Micrometers }`.
  - WIN 68: 0 Wing Chun 3400, 1 Magnetic Jade Pro 3300, 2 Black Emperor 3400, 3 Magneto 3300.
  - WIN 60: 0 Wing Chun 3400, 1 Black Emperor 3400, 2 Magneto 3300, 3 Magnetic Jade Pro 3300.
- `pub fn validate_actuation(v: Micrometers, sw: &Switch) -> Result<(), RangeError>` dengan rentang 100..=max_travel.
- `pub enum PollingRate { Hz8000=0, Hz4000=1, Hz2000=2, Hz1000=3, Hz500=4, Hz250=5, Hz125=6 }` dengan `from_index(u8) -> Option<Self>`.
- `pub enum SystemMode { Windows, Mac }`.

- [ ] **Step 1: Tes gagal.**
  - `rounds_not_truncates`: `from_mm(1.13)` → `1130`; `from_mm(0.4)` → `400`.
  - `rejects_nan`: NaN → Err.
  - `model_by_product`: `(0x1CA2,0x1901,"WIN 68 HE MAX")` → `Some(Win68HeMax)`; `(0x1CA2,0x1901,"WIN 68 HE PRO")` → `None`; `(0x1CA2,0x1902,"WIN 60 HE MAX")` → `Some(Win60HeMax)`.
  - `actuation_range`: 50 µm → Err, 3400 untuk Magneto (maks 3300) → Err, 3300 → Ok.
  - `polling_index`: `from_index(3)` → `Hz1000`, `from_index(7)` → None.
- [ ] **Step 2–4:** FAIL → implementasi → PASS.
- [ ] **Step 5:** Commit `feat(proto): units, models, switch tables`.

### Task 4: Sub-perintah Order (CMD 0x00)

**Files:** `order.rs`.

**Interfaces (Produces):**
- `#[repr(u8)] pub enum OrderType { StartCalibration=0x0C, CloseCalibration=0x0D, FactoryReset=0x11, QueryWinMode=0x21, PrecisionStroke=0x25, KeyboardName=0x26, SetWin=0x30, SetMac=0x31, RgbNack=0x47, Polling=0x50, Profile=0x70 }`
- `pub fn query(t: OrderType) -> Request` → payload `[t, FF, FF]`
- `pub fn set(t: OrderType, arg: u8) -> Request` → payload `[t, arg, FF, FF]`
- `pub fn parse_system_mode(r: &Response) -> Option<SystemMode>` → R[2]: 1=Windows, 0=Mac, 255=None
- `pub fn parse_profile(r: &Response) -> Option<u8>` → R[2] bila 0..=3
- `pub fn parse_polling(r: &Response) -> Option<PollingRate>` → R[2]

- [ ] **Step 1: Tes gagal.**
  - `query_profile_bytes`: `query(Profile).encode()` → `5C 03 00 93 70 FF FF`. Hitungannya: 0x35+0x5C+3+0+0xFF = 0x193, jadi checksum 0x93, sama dengan reset pabrik karena checksum hanya bergantung pada LEN, CMD, dan byte terakhir.
  - `set_polling_bytes`: `set(Polling,3)` → LEN 4, payload `50 03 FF FF`.
  - `parse_mode`: body R[2]=1 → Windows, 0 → Mac, 255 → None.
- [ ] **Step 2–4:** FAIL → implementasi → PASS.
- [ ] **Step 5:** Commit `feat(proto): order sub-commands`.

### Task 5: Info perangkat (CMD 0x01 SYNC)

**Files:** `sync.rs`.

**Interfaces (Produces):**
- `pub fn request() -> Request` → payload `[1,2,3,4,0xFF,0xFF]`
- `pub struct DeviceInfo { pub board_id: u32, pub in_bootloader: bool, pub serial: String, pub firmware: String, pub build_date: String }`
- `pub fn parse(r: &Response) -> DeviceInfo`. Offset di R (`body()`):
  - board_id = u32le R[1..5]
  - in_bootloader = R[7] != 0
  - serial = ASCII R[9..25]
  - firmware = ASCII R[26..36]
  - build_date = ASCII R[43..54]
  - Semua string dipangkas dari byte 0 dan spasi.

- [ ] **Step 1: Tes gagal.** `parse_sync`: body sintetis dengan serial `"A68M240193000000"`, firmware `"1.0.7"`, dan R[7]=0 → field sesuai dan `in_bootloader == false`.
- [ ] **Step 2–4:** FAIL → implementasi → PASS.
- [ ] **Step 5:** Commit `feat(proto): device info sync`.

### Task 6: Penyimpanan per tombol (CMD 0x23 KEY)

**Files:** `key.rs`.

**Interfaces (Produces):**
- `pub enum KeyParam { Layer(u8 /*0..=3*/), Actuation, DksFirst, Mode, DksBinding(u8 /*0..=3*/), DksTrps(u8), MtDelay, RtPress, RtRelease, TopDeadzone, BottomDeadzone, Switch }`
  - `layout(self) -> u8`: Layer n→n, Actuation 4, DksFirst 5, Mode 8, DksBinding n→9+n, DksTrps n→13+n, MtDelay 19, RtPress 20, RtRelease 21, TopDeadzone 22, BottomDeadzone 23, Switch 25.
- `pub enum TouchMode { Global=0, Single=1, RapidTrigger=2 }`
- `pub enum Advanced { None=0, Dks=1, Mpt=2, Mt=3, Tgl=4, End=5, Macro=6, Socd=8 }`
- `pub struct KeyMode { pub touch: TouchMode, pub advanced: Advanced }` dengan `to_u16()` = `(touch<<4)|adv` dan `from_u16(u16) -> Option<KeyMode>` (memakai byte rendah).
- `pub fn get(key: u8, p: KeyParam) -> Request` → `[0x00, key, layout, 0, 0]`
- `pub fn set(entries: &[(u8, KeyParam, u16)]) -> Vec<Request>` → per paket `[0x01, (key, layout, lo, hi)×≤14]`
- `pub fn parse_value(r: &Response) -> Option<(u8 /*key*/, u8 /*layout*/, u16)>` → R[1], R[2], u16le R[3..5]

- [ ] **Step 1: Tes gagal.**
  - `get_actuation_bytes`: `get(0x1A, Actuation)` → payload `00 1A 04 00 00`.
  - `set_splits_at_14`: 15 entri → 2 Request, isi masing-masing 14 dan 1 entri, dan setiap payload diawali `0x01`.
  - `set_payload_fits`: 14 entri → payload 57 byte (≤ 60).
  - `mode_roundtrip`: RT + None → `0x20`; `from_u16(0x13)` → Single + Mt.
  - `parse_value`: body `[st,0x1A,4,0x90,0x01]` → `(0x1A,4,400)`.
- [ ] **Step 2–4:** FAIL → implementasi → PASS.
- [ ] **Step 5:** Commit `feat(proto): per-key parameter store`.

### Task 7: Matriks keycode bawaan (CMD 0x2B DEFKEY)

**Files:** `defkey.rs`.

**Interfaces (Produces):**
- `pub const ROWS: usize = 6; pub const COLS: usize = 21;`
- `pub fn requests() -> [Request; 3]` → `[0x00,0,1]`, `[0x00,2,3]`, `[0x00,4,5]`
- `pub struct Matrix { pub codes: [[u8; COLS]; ROWS] }` dengan:
  - `apply(&mut self, r: &Response) -> Result<(), FrameError>`: R[1]=rowA, R[2..23]=21 kode, R[23]=rowB, R[24..45]=21 kode.
  - `keys(&self) -> impl Iterator<Item=(u8 /*row*/, u8 /*col*/, u8 /*code*/)>`, melewati kode 0.
  - `len()` = jumlah tombol.

- [ ] **Step 1: Tes gagal.** `matrix_from_three_replies`: tiga body sintetis dengan 15/15/14/14/10 tombol → `len()==68`, dan `keys()` pertama `(0,0,0x29)` (Esc).
- [ ] **Step 2–4:** FAIL → implementasi → PASS.
- [ ] **Step 5:** Commit `feat(proto): default keycode matrix`.

---

## Tahap 2: Transport, sesi, dan CLI baca

### Task 8: Transport tiruan dan Session

**Files:** `crates/openhe-device/{Cargo.toml, src/lib.rs, src/transport.rs, src/session.rs}`.

**Interfaces:**
- **Consumes:** `Request::encode`, `Request::expects`, `Response::decode` (Task 1–2).
- **Produces:**
  - `pub trait Transport: Send { fn write(&mut self, report: &[u8; 64]) -> Result<(), Error>; fn read(&mut self, timeout_ms: u32) -> Result<Option<[u8; 64]>, Error>; }`
  - `pub struct MockTransport` dengan `push_reply([u8;64])`, `fail_with(Error)`, dan `written() -> &[[u8;64]]`
  - `pub enum Error { Timeout, Disconnected, Permission(String), Unsupported(String), Frame(FrameError), Io(String) }`
  - `pub struct Session<T: Transport>` dengan:
    - `new(t)`
    - `call(&mut self, req: &Request) -> Result<Response, Error>` (1000 ms × 3 percobaan)
    - `call_many(&mut self, req: &Request, n: usize) -> Result<Vec<[u8;64]>, Error>` (untuk RM6X21 di tahap berikutnya)
    - `take_events(&mut self) -> Vec<Response>`

- [ ] **Step 1: Tes gagal.**
  - `reply_matched`: mock membalas `0xA3` → `call(key get)` Ok.
  - `unsolicited_profile_event_queued`: balasan pertama `0x80` dengan TYPE `0x70`, lalu `0x80` dengan TYPE `0x21`. `call(query(QueryWinMode))` mengembalikan yang kedua, dan `take_events()` berisi yang pertama.
  - `timeout_after_three_attempts`: mock tanpa balasan → `Err(Timeout)` dan `written().len()==3`.
  - `disconnect_propagates`: `fail_with(Disconnected)` → `Err(Disconnected)` tanpa panic.
  - `device_fail_is_error`: balasan dengan CMD `0xFF` → `Err(Frame(DeviceFail))`.
- [ ] **Step 2:** `cargo test -p openhe-device` → FAIL.
- [ ] **Step 3:** Implementasi. Laporan yang tidak cocok dengan `expects` disimpan di antrean event. Waktu tunggu dihitung dengan `Instant`.
- [ ] **Step 4:** PASS.
- [ ] **Step 5:** Commit `feat(device): transport trait, mock and session`.

### Task 9: Transport hidapi

**Files:** `src/hid.rs`, `packaging/linux/70-openhe.rules`.

**Interfaces (Produces):**
- `pub struct DeviceEntry { pub model: Option<Model>, pub product: String, pub path: String }`
- `pub fn list_devices() -> Result<Vec<DeviceEntry>, Error>`: semua interface VID `0x1CA2` dengan usage page `0xFFA0`. `model` bernilai None untuk perangkat yang tidak didukung.
- `pub struct HidTransport` dengan `impl Transport` dan `pub fn open_first() -> Result<(HidTransport, Model), Error>`.
  - Saat menulis, tambahkan prefix `0x00`.
  - Error hidapi saat membuka dipetakan ke `Permission(..)`. Di Linux, pesannya menyebut `packaging/linux/70-openhe.rules`.
- Isi aturan udev: `SUBSYSTEM=="hidraw", ATTRS{idVendor}=="1ca2", ATTRS{idProduct}=="1901|1902", TAG+="uaccess"`.

- [ ] **Step 1:** `cargo build -p openhe-device` di macOS → sukses. Untuk target lain cukup `cargo check` bila toolchain tersedia; build sungguhan di 3 OS dikerjakan di CI Tahap 7.
- [ ] **Step 2:** Tes unit `model_filter`. Logika filter dipisah ke fungsi murni `classify(vid, pid, usage_page, product) -> Option<Model>`, dan tes memastikan PRO serta usage page `0x0001` ditolak.
- [ ] **Step 3:** Commit `feat(device): hidapi transport and linux udev rule`.

### Task 10: API `Keyboard` (hanya baca)

**Files:** `src/keyboard.rs`.

**Interfaces:**
- **Consumes:** `Session`, `order::*`, `sync::*`, `key::*`, `defkey::*`.
- **Produces:**
  - `pub struct Keyboard<T: Transport> { pub model: Model, .. }`
  - `pub fn info(&mut self) -> Result<DeviceInfo, Error>`
  - `pub fn system_mode(&mut self) -> Result<Option<SystemMode>, Error>`
  - `pub fn profile(&mut self) -> Result<u8, Error>`
  - `pub fn polling_rate(&mut self) -> Result<PollingRate, Error>`
  - `pub fn matrix(&mut self) -> Result<Matrix, Error>`
  - `pub fn key_param(&mut self, key: u8, p: KeyParam) -> Result<u16, Error>`
  - `pub fn key_settings(&mut self, key: u8) -> Result<KeySettings, Error>`, dengan `KeySettings { mode: KeyMode, actuation, rt_press, rt_release, top_dz, bottom_dz: Micrometers, switch: u16 }`

- [ ] **Step 1: Tes gagal** dengan MockTransport.
  - `reads_matrix_three_calls`: 3 balasan `0xAB` → `Matrix.len()==68`.
  - `key_settings_reads_seven_params`: 7 balasan `0xA3` berurutan → struct terisi.
  - Respons dengan R[1] (kode tombol) yang berbeda dari permintaan → `Err(Io)`.
- [ ] **Step 2–4:** FAIL → implementasi → PASS.
- [ ] **Step 5:** Commit `feat(device): read-only keyboard API`.

### Task 11: CLI `openhe` dan uji baca ke keyboard asli

**Files:** `crates/openhe-cli/{Cargo.toml, src/main.rs}`.

**Interfaces:**
- **Consumes:** `list_devices`, `HidTransport::open_first`, `Keyboard`.
- **Produces** perintah:
  - `openhe list`
  - `openhe info`
  - `openhe dump [--json]`: mode, profil, polling, matriks, dan setting semua tombol.
  - Opsi global `--trace`: setiap report keluar/masuk dicetak dalam hex ke stderr.
  - Kode keluar: 0 sukses, 2 perangkat tidak ditemukan, 3 izin ditolak.

- [ ] **Step 1:** Tes parsing argumen dengan `clap::Command::debug_assert` dan `try_parse_from(["openhe","dump","--json"])`.
- [ ] **Step 2:** `cargo run -p openhe-cli -- list` di Mac pengguna → muncul `WIN 68 HE MAX`.
- [ ] **Step 3:** `cargo run -p openhe-cli -- --trace info` dan `-- --trace dump --json > docs/protocol/captures/win68-dump.json`. Perintah ini hanya membaca.
- [ ] **Step 4:** Perbarui `docs/protocol/PROTOCOL.md` bagian 7 dengan jawaban yang ditemukan:
  - isi byte 0;
  - apakah semua balasan `CMD|0x80`;
  - matriks asli;
  - nilai bawaan per tombol.
  - Bila ada temuan yang bertentangan dengan Task 2 atau Task 7, perbaiki tes dan kodenya di task ini.
- [ ] **Step 5:** Commit `feat(cli): read-only openhe list/info/dump + first hardware capture`.

### Task 12: Penulisan pertama (perlu izin pengguna)

**Files:**
- `keyboard.rs`: `pub fn set_key_params(&mut self, entries: &[(u8, KeyParam, u16)]) -> Result<(), Error>`, menulis lalu membaca ulang untuk memastikan.
- `main.rs`: `openhe set-actuation <KEY> <MM>`.

- [ ] **Step 1: Tes gagal (mock).** `set_then_verify`: SET dikirim, GET dibaca ulang. Bila nilai yang dibaca ulang berbeda, hasilnya `Err(Io("verify"))`.
- [ ] **Step 2–3:** FAIL → implementasi → PASS.
- [ ] **Step 4:** **Tanya pengguna dulu.** Setelah diizinkan:
  1. catat aktuasi tombol `G` lewat `dump`;
  2. `set-actuation G 1.50`;
  3. `dump` → 1500 µm;
  4. kembalikan ke nilai awal;
  5. `dump` → sama seperti semula.
- [ ] **Step 5:** Commit `feat: first verified write (actuation)`.

**Selesai Tahap 2** saat semua tes lulus di macOS, `openhe dump` sesuai dengan keyboard asli, dan satu penulisan sudah terbukti bisa dikembalikan. Setelah itu disusun rencana rinci Tahap 3 (GUI dan Performa) berdasarkan temuan tadi.

---

## Verifikasi

- Per task: `cargo test --workspace`, `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`.
- Hardware (Tahap 2): `openhe list`, `openhe --trace info`, dan `openhe dump --json` di Mac dengan WIN 68 HE MAX terpasang. Bandingkan hasilnya dengan nilai bawaan dari PROTOCOL.md: aktuasi 2.0 mm, deadzone 0.2 mm, switch Wing Chun.
- Setiap tahap yang menulis: tulis, baca ulang, lalu kembalikan nilai awal, selalu dengan izin pengguna.
- Mulai Tahap 3: setiap halaman GUI dicocokkan secara visual dengan mockup Lunagraph lewat screenshot.

## Cara eksekusi

Rekomendasi: **subagent-driven**. Satu subagent mengerjakan satu task, lalu reviewer terpisah memeriksanya sebelum task berikutnya dimulai. Pertimbangannya:
- 13 task di Tahap 1–2 saling bergantung lewat interface yang ketat (`Request`/`Response`/`Session`).
- Kesalahan di framing akan merambat ke semua fitur dan bisa salah tulis ke keyboard.
- Pengguna meminta kualitas.
