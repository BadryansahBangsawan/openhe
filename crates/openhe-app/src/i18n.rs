//! Indonesian UI strings, grouped by page. Every user-facing string lives
//! here; pages add their own group.

/// App-wide strings.
pub mod common {
    pub const APP_NAME: &str = "OpenHE";
    pub const COMING_SOON: &str = "Segera hadir";
    pub const COMING_SOON_BODY: &str =
        "Halaman ini sedang dibuat. Pengaturan lain sudah bisa dipakai.";
    pub const RETRY: &str = "Coba lagi";
    pub const CLOSE: &str = "Tutup";
    pub const MM: &str = "mm";
    pub const UNTESTED_LAYOUT: &str = "Tata letak WIN 60 belum diuji";
    pub const WAITING_DEVICE: &str = "Menunggu keyboard…";
    pub const WAITING_DEVICE_BODY: &str =
        "Colokkan WIN 68 HE MAX atau WIN 60 HE MAX lewat USB. OpenHE mencari setiap detik.";
    pub const LOADING_DEVICE: &str = "Membaca pengaturan keyboard…";
    pub const DISCONNECTED_WRITE: &str = "Keyboard terputus. Perubahan belum dikirim.";
}

/// Indonesian text for a device error, with the technical detail kept.
pub fn device_error(e: &openhe_device::Error) -> String {
    use openhe_device::Error as E;
    match e {
        E::Timeout => "Keyboard tidak menjawab. Coba lagi.".to_string(),
        E::Disconnected => common::DISCONNECTED_WRITE.to_string(),
        E::Permission(m) => format!("Akses ke keyboard ditolak: {m}"),
        E::Unsupported(m) => format!("Tidak didukung: {m}"),
        E::Frame(f) => format!("Balasan keyboard tidak dikenali: {f}"),
        E::InvalidInput(m) => format!("Nilai tidak valid: {m}"),
        E::Io(m) if m.starts_with("verify failed") => {
            format!("Keyboard menyimpan nilai lain dari yang dikirim ({m}).")
        }
        E::Io(m) => format!("Kesalahan komunikasi: {m}"),
    }
}

/// Sidebar navigation.
pub mod nav {
    pub const PERFORMANCE: &str = "Performa";
    pub const ADVANCED: &str = "Tombol Lanjutan";
    pub const REMAP: &str = "Remap";
    pub const MACROS: &str = "Macro";
    pub const LIGHTING: &str = "Lampu";
    pub const CALIBRATION: &str = "Kalibrasi";
    pub const SETTINGS: &str = "Pengaturan";
}

/// Page subtitles in the top bar.
pub mod subtitle {
    pub const PERFORMANCE: &str = "Titik aktuasi, rapid trigger, deadzone";
    pub const ADVANCED: &str = "DKS, MT, TGL, MPT, SOCD, END";
    pub const REMAP: &str = "Ubah fungsi tombol di 4 layer";
    pub const MACROS: &str = "Rekam urutan tombol, pasang ke tombol mana saja";
    pub const LIGHTING: &str = "Efek, warna per tombol, kecerahan";
    pub const CALIBRATION: &str = "Ukur ulang jarak tekan semua switch";
    pub const SETTINGS: &str = "Polling rate, profil, app, reset";
}

/// Top bar.
pub mod topbar {
    pub const PROFILE: &str = "Profil";
    pub const MAC: &str = "Mac";
    pub const WIN: &str = "Win";
    pub const READ_ONLY_HINT: &str = "Ganti dari halaman Pengaturan (segera hadir)";
}

/// Device card at the bottom of the sidebar.
pub mod device {
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
    pub const NOTHING_SELECTED_BODY: &str =
        "Klik tombol di keyboard untuk mengatur titik aktuasi dan rapid trigger.";
    pub const KEYS_SELECTED: &str = "tombol dipilih";
    pub const SWITCH_PREFIX: &str = "switch";
    pub const MIXED_VALUES: &str = "Nilai berbeda antar tombol; yang tampil milik tombol pertama.";

    pub const MODE: &str = "Mode";
    pub const MODE_NORMAL: &str = "Normal";
    pub const MODE_RT: &str = "Rapid Trigger";
    pub const MODE_GLOBAL_HINT: &str =
        "Tombol ini masih mode global bawaan. Menerapkan aktuasi mengubahnya ke Normal.";

    pub const ACTUATION: &str = "Titik aktuasi";
    pub const SENSITIVE: &str = "sensitif";
    pub const DEEP: &str = "dalam";

    pub const RAPID_TRIGGER: &str = "Rapid Trigger";
    pub const RT_PRESS: &str = "Sensitivitas tekan";
    pub const RT_RELEASE: &str = "Sensitivitas lepas";
    pub const RT_SYNC: &str = "Samakan tekan & lepas";

    pub const DEADZONE: &str = "Deadzone";
    pub const DZ_TOP: &str = "Atas";
    pub const DZ_BOTTOM: &str = "Bawah";

    pub const SWITCH: &str = "Switch";
    pub const SWITCH_HINT: &str = "Menentukan jarak tekan maksimum.";

    pub const APPLY: &str = "Terapkan ke keyboard";
    pub const APPLYING: &str = "Menerapkan…";
    pub const APPLIED: &str = "Tersimpan di keyboard";
    pub const NO_CHANGES: &str = "Belum ada perubahan";
    pub const SAVED_IN_PROFILE: &str = "Disimpan di profil";
    pub const WORKS_WITHOUT_APP: &str = "tetap aktif tanpa app";
    pub const APPLY_FAILED: &str = "Gagal menerapkan";
    pub const RELOADED_HINT: &str = "Nilai di layar sudah dibaca ulang dari keyboard.";

    pub const LEGEND: &str = "Keterangan";
    pub const LEGEND_SELECTED: &str = "Dipilih";
    pub const LEGEND_VALUE: &str = "Angka = titik aktuasi (mm)";
    pub const LEGEND_CHANGED: &str = "Berbeda dari bawaan (2.00)";
    pub const LEGEND_RT: &str = "RT = rapid trigger aktif";
    pub const SELECT_HINT: &str =
        "Klik atau seret untuk memilih beberapa tombol. Shift+klik untuk menambah.";

    pub const TRAVEL: &str = "Gerak tombol";
    pub const LIVE_SOON: &str = "Kedalaman live — segera hadir";
    pub const ACTUATION_MARK: &str = "Aktuasi";
    pub const DZ_SUMMARY_TOP: &str = "Deadzone atas";
    pub const DZ_SUMMARY_BOTTOM: &str = "bawah";
    pub const RT_SUMMARY: &str = "RT: lepas setelah naik";
    pub const RT_SUMMARY_PRESS: &str = "tekan lagi setelah turun";
}
