# OpenHE — Design Spec

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
