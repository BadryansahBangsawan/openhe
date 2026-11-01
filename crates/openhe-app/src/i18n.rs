//! Indonesian UI strings, grouped by page. Every user-facing string lives
//! here; pages add their own group.

/// App-wide strings.
pub mod common {
    pub const APP_NAME: &str = "OpenHE";
    pub const ALREADY_RUNNING: &str = "OpenHE sudah berjalan";
    pub const ALREADY_RUNNING_BODY: &str = "Pakai jendela OpenHE yang sudah terbuka.";
    pub const RETRY: &str = "Coba lagi";
    pub const CLOSE: &str = "Tutup";
    pub const MM: &str = "mm";
    pub const UNTESTED_LAYOUT: &str = "Tata letak WIN 60 belum diuji";
    pub const WAITING_DEVICE: &str = "Menunggu keyboard…";
    pub const WAITING_DEVICE_BODY: &str = "Colokkan WIN 68 HE MAX atau WIN 60 HE MAX lewat USB.";
    pub const LOADING_DEVICE: &str = "Membaca pengaturan keyboard…";
    pub const DISCONNECTED: &str = "Keyboard terputus.";
    pub const DISMISS: &str = "Tutup";
    pub const RELOAD: &str = "Muat ulang";
    pub const TECH_DETAIL: &str = "Detail teknis";
    pub const CLOSING: &str = "Menunggu perubahan selesai, lalu OpenHE ditutup…";
}

/// Error texts. Every user-facing error goes through [`device_error`], so no
/// English protocol text reaches the UI (the raw text is only offered as a
/// "Detail teknis" tooltip via [`technical`]).
pub mod error {
    pub const TIMEOUT: &str = "Keyboard tidak menjawab. Coba lagi.";
    pub const PERMISSION: &str = "Akses ke keyboard ditolak oleh sistem.";
    pub const PERMISSION_LINUX: &str =
        "Akses ke keyboard ditolak. Pasang aturan udev packaging/linux/70-openhe.rules lalu colokkan ulang.";
    pub const BUSY: &str = "Keyboard sedang dipakai program lain (tutup OpenHE/AULA HUB lain)";
    pub const BOOTLOADER: &str = "Keyboard sedang dalam mode bootloader. Cabut lalu colokkan lagi.";
    pub const UNSUPPORTED: &str = "Perangkat ini tidak didukung.";
    /// After the model name(s).
    pub const NO_CONFIG_INTERFACE: &str =
        "terdeteksi, tetapi antarmuka pengaturannya tidak ditemukan. Cabut lalu colokkan lagi keyboard.";
    pub const FRAME: &str = "Balasan keyboard tidak dikenali.";
    pub const INVALID: &str = "Nilai di luar batas yang diizinkan untuk switch tombol ini.";
    pub const VERIFY: &str = "Keyboard menyimpan nilai yang berbeda dari yang dikirim.";
    pub const INTERNAL: &str = "Terjadi kesalahan internal di OpenHE. Keyboard disambung ulang.";
    pub const IO: &str = "Kesalahan komunikasi dengan keyboard.";
    pub const LIGHTING_UNKNOWN: &str = "Keyboard melaporkan nilai lampu yang tidak dikenal";
    pub const LIGHTING_RECOVER: &str =
        "Untuk memulihkannya, impor profil yang berisi pengaturan lampu (Pengaturan → Impor…).";
}

/// The raw (English) error text, for a "Detail teknis" tooltip only.
pub fn technical(e: &openhe_device::Error) -> String {
    e.to_string()
}

/// Prefix of the device layer's `Error::Io` text for a lighting reply that
/// does not parse (see `openhe_device::keyboard::lighting`).
const LIGHTING_REPLY: &str = "unexpected lighting reply: ";

/// The lighting field an out-of-range lighting read names, in Indonesian.
/// `None` for other lighting reply problems (e.g. a reply of another
/// command), which stay a plain communication error.
fn lighting_field(detail: &str) -> Option<&'static str> {
    [
        ("brightness ", "kecerahan"),
        ("mode ", "efek"),
        ("speed ", "kecepatan"),
        ("sleep timeout ", "mati otomatis"),
        ("static colour index ", "warna"),
        ("dynamic colour index ", "warna"),
    ]
    .into_iter()
    .find(|(en, _)| detail.starts_with(en))
    .map(|(_, id)| id)
}

/// Indonesian text for a device error. Never includes the English detail.
pub fn device_error(e: &openhe_device::Error) -> String {
    use openhe_device::Error as E;
    if let E::Io(m) = e {
        if let Some(field) = m.strip_prefix(LIGHTING_REPLY).and_then(lighting_field) {
            return format!(
                "{} ({field}); pengaturan lampu belum bisa diubah dari OpenHE. {}",
                error::LIGHTING_UNKNOWN,
                error::LIGHTING_RECOVER
            );
        }
    }
    if let E::NoConfigInterface(models) = e {
        return format!("{models} {}", error::NO_CONFIG_INTERFACE);
    }
    let s = match e {
        E::Timeout => error::TIMEOUT,
        E::Disconnected => common::DISCONNECTED,
        E::Permission(_) if cfg!(target_os = "linux") => error::PERMISSION_LINUX,
        E::Permission(_) => error::PERMISSION,
        E::Busy(_) => error::BUSY,
        E::Unsupported(m) if m == "bootloader" => error::BOOTLOADER,
        E::Unsupported(_) | E::NoConfigInterface(_) => error::UNSUPPORTED,
        E::Frame(_) => error::FRAME,
        E::InvalidInput(_) => error::INVALID,
        E::Io(m) if m.starts_with("verify failed") => error::VERIFY,
        E::Io(m) if m.starts_with(crate::worker::INTERNAL_ERROR) => error::INTERNAL,
        E::Io(_) => error::IO,
    };
    s.to_string()
}

/// Local data file.
pub mod storage {
    pub const RECOVERED: &str =
        "File data OpenHE rusak, jadi memakai pengaturan kosong. File lama disimpan sebagai";
    pub const UNREADABLE: &str =
        "File data OpenHE tidak bisa dibaca. Perubahan lokal (macro, nama profil, aturan aplikasi) TIDAK akan disimpan sampai file itu diperbaiki.";
    pub const SAVE_FAILED: &str = "Pengaturan lokal tidak bisa disimpan.";
}

/// Sidebar navigation.
pub mod nav {
    pub const PERFORMANCE: &str = "Performa";
    pub const ADVANCED: &str = "Tombol Lanjutan";
    pub const REMAP: &str = "Remap";
    pub const MACROS: &str = "Macro";
    pub const LIGHTING: &str = "Lampu";
    pub const CALIBRATION: &str = "Kalibrasi";
    pub const TEMPLATES: &str = "Template";
    pub const SETTINGS: &str = "Pengaturan";
}

/// Top bar.
pub mod topbar {
    pub const PROFILE: &str = "Profil";
    pub const MAC: &str = "Mac";
    pub const WIN: &str = "Win";
}

/// Device card at the bottom of the sidebar.
pub mod device {
    pub const WORKER_STOPPED: &str =
        "Bagian OpenHE yang menghubungi keyboard berhenti. Tutup lalu buka lagi aplikasinya.";
    pub const STALE_RELOAD: &str =
        "Gagal membaca ulang keyboard; nilai di layar mungkin sudah lama. Dicoba lagi otomatis.";
    pub const STALE_EVENTS: &str =
        "Gagal membaca kabar dari keyboard; nilai di layar mungkin sudah lama.";
    pub const KEYS_SKIPPED: &str = "Pengaturan beberapa tombol tidak terbaca dan dilewati:";
    pub const CONNECTED_USB: &str = "Terhubung · USB";
    pub const SEARCHING: &str = "Mencari keyboard…";
    pub const LOADING: &str = "Menyambung…";
    pub const ERROR: &str = "Tidak bisa terhubung";
    pub const NO_DEVICE: &str = "Belum ada keyboard";
    pub const POLLING: &str = "Polling";
    pub const FIRMWARE: &str = "Firmware";
    pub const HZ: &str = "Hz";
}

/// Performa page.
pub mod perf {
    pub const SELECT_ALL: &str = "Pilih semua";
    pub const WASD: &str = "WASD";
    pub const LETTERS: &str = "Huruf";
    pub const CLEAR: &str = "Hapus pilihan";
    pub const VIEW_ACTUATION: &str = "Nilai aktuasi";
    pub const VIEW_LIVE: &str = "Kedalaman live";

    pub const NOTHING_SELECTED: &str = "Pilih tombol";
    pub const KEYS_SELECTED: &str = "tombol dipilih";
    pub const MIXED_VALUES: &str = "Nilai berbeda antar tombol; yang tampil milik tombol pertama.";

    pub const MODE: &str = "Mode";
    pub const MODE_NORMAL: &str = "Normal";
    pub const MODE_RT: &str = "Rapid Trigger";
    pub const MODE_GLOBAL_HINT: &str =
        "Masih mode global; menerapkan aktuasi mengubahnya ke Normal.";

    pub const ACTUATION: &str = "Titik aktuasi";
    pub const SENSITIVE: &str = "sensitif";
    pub const DEEP: &str = "dalam";

    pub const RT_PRESS: &str = "Sensitivitas tekan";
    pub const RT_RELEASE: &str = "Sensitivitas lepas";
    pub const RT_SYNC: &str = "Samakan tekan & lepas";

    pub const DEADZONE: &str = "Deadzone";
    pub const DZ_TOP: &str = "Atas";
    pub const DZ_BOTTOM: &str = "Bawah";

    pub const SWITCH: &str = "Switch";

    pub const APPLY: &str = "Terapkan ke keyboard";
    pub const APPLYING: &str = "Menerapkan…";
    pub const APPLIED: &str = "Tersimpan di keyboard";
    pub const SAVED_IN_PROFILE: &str = "Disimpan di profil";
    pub const APPLY_FAILED: &str = "Gagal menerapkan";
    pub const RELOADED_HINT: &str = "Nilai di layar dibaca ulang dari keyboard.";
    pub const PARTIAL_WRITE: &str =
        "Keyboard terputus. Sebagian perubahan mungkin sudah terkirim; nilai dibaca ulang saat tersambung.";
    pub const CLAMPED: &str =
        "Nilai di atas jarak tekan switch baru disesuaikan ke batasnya dan ikut dikirim. Maks";

    pub const LEGEND: &str = "Keterangan";
    pub const LEGEND_SELECTED: &str = "Dipilih";
    pub const LEGEND_VALUE: &str = "Angka = titik aktuasi (mm)";
    pub const LEGEND_CHANGED: &str = "Berbeda dari bawaan (2.00)";
    pub const LEGEND_RT: &str = "Rapid trigger aktif";
    pub const SELECT_HINT: &str = "Seret untuk memilih banyak, Shift+klik untuk menambah.";

    pub const TRAVEL: &str = "Gerak tombol";
    pub const ACTUATION_MARK: &str = "Aktuasi";
    pub const DZ_SUMMARY_TOP: &str = "Deadzone atas";
    pub const DZ_SUMMARY_BOTTOM: &str = "bawah";
    pub const RT_SUMMARY: &str = "RT: lepas";
    pub const RT_SUMMARY_PRESS: &str = "tekan";
}

/// Shell texts for keyboard events, the interactive top bar, and the
/// background profile switcher (Track F).
pub mod events {
    pub const PROFILE_SWITCH_FAILED: &str = "Gagal mengganti profil.";
    pub const PROFILE_RECONNECTING: &str =
        "Profil diganti; keyboard tersambung ulang sebentar. Menyambung ulang…";
    pub const MODE_SWITCH_FAILED: &str = "Gagal mengganti mode Win/Mac.";
    pub const LOCKED_CALIBRATING: &str = "Hentikan kalibrasi dulu";
    pub const CALIBRATION_OWED: &str =
        "Keyboard mungkin masih dalam mode kalibrasi (tidak bisa mengetik). Perubahan dikunci sampai kalibrasi dihentikan.";
    pub const STOP_CALIBRATION: &str = "Hentikan kalibrasi";
    pub const AUTO_SWITCHED: &str = "Profil diganti otomatis untuk";
    pub const AUTO_SWITCH_FAILED: &str = "Ganti profil otomatis gagal.";
}

/// Updates (Pengaturan → Pembaruan, and the sidebar notice).
pub mod update {
    pub const TITLE: &str = "Pembaruan";
    pub const VERSION: &str = "Versi terpasang";
    pub const CHECK_NOW: &str = "Periksa sekarang";
    pub const CHECKING: &str = "Memeriksa pembaruan…";
    pub const UP_TO_DATE: &str = "OpenHE sudah versi terbaru.";
    pub const LAST_CHECK: &str = "Terakhir diperiksa";
    pub const NEVER_CHECKED: &str = "Belum pernah diperiksa.";
    pub const JUST_NOW: &str = "baru saja";
    pub const MINUTES_AGO: &str = "menit lalu";
    pub const HOURS_AGO: &str = "jam lalu";
    pub const DAYS_AGO: &str = "hari lalu";
    pub const AVAILABLE: &str = "Versi baru tersedia:";
    pub const PRERELEASE: &str = "versi uji";
    pub const NO_NOTES: &str = "Tidak ada catatan rilis.";
    pub const INSTALL: &str = "Unduh & pasang";
    pub const DOWNLOADING: &str = "Mengunduh…";
    pub const INSTALLING: &str = "Memeriksa dan memasang…";
    /// After "✔ Versi 0.3.0".
    pub const INSTALLED: &str = "sudah terpasang.";
    pub const RESTART: &str = "Mulai ulang sekarang";
    pub const RESTART_BUSY: &str = "Tunggu sampai perubahan ke keyboard selesai.";
    pub const RESTART_FAILED: &str =
        "OpenHE versi baru tidak bisa dijalankan. Tutup lalu buka lagi OpenHE.";
    pub const AUTO: &str = "Periksa pembaruan otomatis";
    pub const BETA: &str = "Ikut versi uji (beta)";
    pub const BETA_HINT: &str = "Mungkin belum stabil.";
    pub const RELEASES_LINK: &str = "Halaman rilis di GitHub";
    pub const DOWNLOAD_PAGE: &str = "Buka halaman unduhan";
    pub const LINK_FAILED: &str = "Browser tidak bisa dibuka. Alamatnya:";

    pub const UNSUPPORTED_BUNDLE: &str = "Pemasangan otomatis hanya untuk OpenHE.app.";
    pub const UNSUPPORTED_TRANSLOCATED: &str =
        "Pindahkan OpenHE.app ke folder Aplikasi lalu buka lagi untuk memasang otomatis.";
    pub const UNSUPPORTED_APPIMAGE: &str = "Pemasangan otomatis hanya untuk versi AppImage.";
    pub const UNSUPPORTED_PLATFORM: &str = "Pemasangan otomatis belum tersedia untuk sistem ini.";
    pub const NO_ASSET: &str =
        "Rilis ini belum punya file untuk sistem ini. Unduh manual dari halaman rilis.";

    pub const CHECK_FAILED: &str = "Gagal memeriksa pembaruan.";
    pub const INSTALL_FAILED: &str = "Pembaruan gagal dipasang.";
    pub const ERR_NETWORK: &str =
        "Tidak bisa menghubungi GitHub. Periksa koneksi internet, lalu coba lagi.";
    pub const ERR_RATE: &str = "GitHub sedang membatasi permintaan. Coba lagi nanti.";
    /// Before the HTTP status number.
    pub const ERR_HTTP: &str = "GitHub menjawab dengan kode galat";
    pub const ERR_TOO_LARGE: &str =
        "File dari GitHub lebih besar dari yang diumumkan, jadi unduhan dihentikan. Tidak ada yang diganti.";
    pub const ERR_LIST: &str = "Daftar rilis dari GitHub tidak bisa dibaca.";
    pub const ERR_SIGNATURE: &str = "Tanda tangan pembaruan tidak cocok, jadi pembaruan ditolak.";
    pub const ERR_MANIFEST: &str = "Data pembaruan tidak valid, jadi pembaruan ditolak.";
    pub const ERR_VERSION: &str =
        "Versi di data pembaruan tidak cocok dengan rilisnya, jadi pembaruan ditolak.";
    pub const ERR_CHECKSUM: &str =
        "File unduhan tidak cocok (ukuran atau SHA-256). Tidak ada yang diganti.";
    pub const ERR_DISK: &str = "File pembaruan tidak bisa disimpan. Tidak ada yang diganti.";
    pub const ERR_NOT_WRITABLE: &str =
        "Folder tempat OpenHE berada tidak bisa ditulis. Pindahkan OpenHE ke folder yang bisa kamu ubah (misalnya Aplikasi atau folder pribadi), atau unduh manual.";
    pub const ERR_EXTRACT: &str = "Arsip pembaruan tidak bisa dibuka. Tidak ada yang diganti.";
    pub const ERR_BUNDLE: &str =
        "Isi arsip pembaruan bukan versi yang diumumkan. Tidak ada yang diganti.";
    pub const ERR_REPLACE: &str = "OpenHE tidak bisa diganti; versi lama tetap dipakai.";
    pub const ERR_ROLLBACK: &str =
        "OpenHE tidak bisa diganti dan versi lama tidak bisa dikembalikan otomatis. Unduh ulang dari halaman rilis.";
    pub const INTERNAL: &str = "Terjadi kesalahan internal di pembaru OpenHE.";

    /// Sidebar notice: "Versi 0.3.0 tersedia".
    pub const NOTICE_PREFIX: &str = "Versi";
    pub const NOTICE_SUFFIX: &str = "tersedia";
    pub const NOTICE_OPEN: &str = "Lihat";
    pub const NOTICE_LATER: &str = "Nanti";
}

#[cfg(test)]
mod tests {
    use super::*;
    use openhe_device::Error as E;

    /// macOS: the keyboard is open in another program. That is not a
    /// permission problem and has nothing to do with udev.
    #[test]
    fn a_busy_keyboard_says_another_program_has_it() {
        let msg = device_error(&E::Busy(
            "(0xE00002C5) exclusive access and device already open".into(),
        ));
        assert_eq!(
            msg,
            "Keyboard sedang dipakai program lain (tutup OpenHE/AULA HUB lain)"
        );
        assert_ne!(msg, device_error(&E::Permission("x".into())));
        assert!(!msg.contains("udev"));
    }

    /// The keyboard is there but not its configuration interface: named,
    /// with the advice to reconnect it, not "not supported".
    #[test]
    fn a_missing_configuration_interface_says_to_reconnect() {
        let msg = device_error(&E::NoConfigInterface("WIN 68 HE MAX".into()));
        assert_eq!(
            msg,
            "WIN 68 HE MAX terdeteksi, tetapi antarmuka pengaturannya tidak ditemukan. \
             Cabut lalu colokkan lagi keyboard."
        );
        assert_ne!(msg, device_error(&E::Unsupported("x".into())));
    }
}
