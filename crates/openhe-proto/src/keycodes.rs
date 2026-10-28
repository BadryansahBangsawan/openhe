//! The u16 keycode table the keyboard accepts for remapping, plus short key-cap legends.
//!
//! The table mirrors `docs/protocol/keycodes.txt` row for row. Labels are the vendor's own
//! (including their spelling); `Category` is our grouping for pickers.

/// Grouping of a keycode for the UI's key picker.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Category {
    /// Letters, digits, punctuation, Space, Enter, Esc, Tab, Backspace, lock keys.
    Basic,
    /// Arrows, Ins/Del/Home/End/PgUp/PgDn, PrtSc/ScrLk/Pause.
    Navigation,
    /// F1 to F24.
    Function,
    /// Keypad keys.
    Numpad,
    /// 0xE0 to 0xE7.
    Modifier,
    /// Consumer and media controls.
    Media,
    /// Mouse buttons, movement and wheel.
    Mouse,
    /// Lighting controls (main and logo light).
    Lighting,
    /// Profile, Win/Mac mode, Win lock, power and similar device functions.
    System,
    /// Fn, "none" and reserved or international codes.
    Special,
}

/// One entry of the keycode table.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct KeyCode {
    pub code: u16,
    pub label: &'static str,
    pub category: Category,
}

const fn k(code: u16, label: &'static str, category: Category) -> KeyCode {
    KeyCode {
        code,
        label,
        category,
    }
}

static TABLE: [KeyCode; 288] = [
    k(0x0000, "△", Category::Special),
    k(0x0001, "Fn", Category::Special),
    k(0x0004, "A", Category::Basic),
    k(0x0005, "B", Category::Basic),
    k(0x0006, "C", Category::Basic),
    k(0x0007, "D", Category::Basic),
    k(0x0008, "E", Category::Basic),
    k(0x0009, "F", Category::Basic),
    k(0x000A, "G", Category::Basic),
    k(0x000B, "H", Category::Basic),
    k(0x000C, "I", Category::Basic),
    k(0x000D, "J", Category::Basic),
    k(0x000E, "K", Category::Basic),
    k(0x000F, "L", Category::Basic),
    k(0x0010, "M", Category::Basic),
    k(0x0011, "N", Category::Basic),
    k(0x0012, "O", Category::Basic),
    k(0x0013, "P", Category::Basic),
    k(0x0014, "Q", Category::Basic),
    k(0x0015, "R", Category::Basic),
    k(0x0016, "S", Category::Basic),
    k(0x0017, "T", Category::Basic),
    k(0x0018, "U", Category::Basic),
    k(0x0019, "V", Category::Basic),
    k(0x001A, "W", Category::Basic),
    k(0x001B, "X", Category::Basic),
    k(0x001C, "Y", Category::Basic),
    k(0x001D, "Z", Category::Basic),
    k(0x001E, "1", Category::Basic),
    k(0x001F, "2", Category::Basic),
    k(0x0020, "3", Category::Basic),
    k(0x0021, "4", Category::Basic),
    k(0x0022, "5", Category::Basic),
    k(0x0023, "6", Category::Basic),
    k(0x0024, "7", Category::Basic),
    k(0x0025, "8", Category::Basic),
    k(0x0026, "9", Category::Basic),
    k(0x0027, "0", Category::Basic),
    k(0x0028, "Enter", Category::Basic),
    k(0x0029, "ESC", Category::Basic),
    k(0x002A, "Back Space", Category::Basic),
    k(0x002B, "Tab", Category::Basic),
    k(0x002C, "Space", Category::Basic),
    k(0x002D, "-", Category::Basic),
    k(0x002E, "=", Category::Basic),
    k(0x002F, "[", Category::Basic),
    k(0x0030, "]", Category::Basic),
    k(0x0031, "\\", Category::Basic),
    k(0x0032, "no", Category::Special),
    k(0x0033, ";", Category::Basic),
    k(0x0034, "'", Category::Basic),
    k(0x0035, "`", Category::Basic),
    k(0x0036, ",", Category::Basic),
    k(0x0037, ".", Category::Basic),
    k(0x0038, "/", Category::Basic),
    k(0x0039, "Cap", Category::Basic),
    k(0x003A, "F1", Category::Function),
    k(0x003B, "F2", Category::Function),
    k(0x003C, "F3", Category::Function),
    k(0x003D, "F4", Category::Function),
    k(0x003E, "F5", Category::Function),
    k(0x003F, "F6", Category::Function),
    k(0x0040, "F7", Category::Function),
    k(0x0041, "F8", Category::Function),
    k(0x0042, "F9", Category::Function),
    k(0x0043, "F10", Category::Function),
    k(0x0044, "F11", Category::Function),
    k(0x0045, "F12", Category::Function),
    k(0x0046, "PrtSc", Category::Navigation),
    k(0x0047, "ScrLc", Category::Navigation),
    k(0x0048, "Pause", Category::Navigation),
    k(0x0049, "Ins", Category::Navigation),
    k(0x004A, "Home", Category::Navigation),
    k(0x004B, "PgUp", Category::Navigation),
    k(0x004C, "Del", Category::Navigation),
    k(0x004D, "End", Category::Navigation),
    k(0x004E, "PgDn", Category::Navigation),
    k(0x004F, "→", Category::Navigation),
    k(0x0050, "←", Category::Navigation),
    k(0x0051, "↓", Category::Navigation),
    k(0x0052, "↑", Category::Navigation),
    k(0x0053, "NumLc", Category::Numpad),
    k(0x0054, "/", Category::Numpad),
    k(0x0055, "*", Category::Numpad),
    k(0x0056, "-", Category::Numpad),
    k(0x0057, "+", Category::Numpad),
    k(0x0058, "Enter", Category::Numpad),
    k(0x0059, "1", Category::Numpad),
    k(0x005A, "2", Category::Numpad),
    k(0x005B, "3", Category::Numpad),
    k(0x005C, "4", Category::Numpad),
    k(0x005D, "5", Category::Numpad),
    k(0x005E, "6", Category::Numpad),
    k(0x005F, "7", Category::Numpad),
    k(0x0060, "8", Category::Numpad),
    k(0x0061, "9", Category::Numpad),
    k(0x0062, "0", Category::Numpad),
    k(0x0063, ".", Category::Numpad),
    k(0x0064, "no2", Category::Special),
    k(0x0065, "App", Category::Basic),
    k(0x0066, "Power", Category::System),
    k(0x0067, "=", Category::Numpad),
    k(0x0068, "F13", Category::Function),
    k(0x0069, "F14", Category::Function),
    k(0x006A, "F15", Category::Function),
    k(0x006B, "F16", Category::Function),
    k(0x006C, "F17", Category::Function),
    k(0x006D, "F18", Category::Function),
    k(0x006E, "F19", Category::Function),
    k(0x006F, "F20", Category::Function),
    k(0x0070, "F21", Category::Function),
    k(0x0071, "F22", Category::Function),
    k(0x0072, "F23", Category::Function),
    k(0x0073, "F24", Category::Function),
    k(0x0074, "Execute", Category::Special),
    k(0x0075, "Help", Category::Special),
    k(0x0076, "Menu", Category::Basic),
    k(0x0077, "Select", Category::Special),
    k(0x0078, "Stop", Category::Special),
    k(0x0079, "Again", Category::Special),
    k(0x007A, "Undo", Category::Special),
    k(0x007B, "Cut", Category::Special),
    k(0x007C, "Copy", Category::Special),
    k(0x007D, "Paste", Category::Special),
    k(0x007E, "Find", Category::Special),
    k(0x007F, "Mute", Category::Media),
    k(0x0080, "Vol+", Category::Media),
    k(0x0081, "Vol-", Category::Media),
    k(0x0082, "Caps Lock", Category::Basic),
    k(0x0083, "Num Lock", Category::Basic),
    k(0x0084, "Scroll Lock", Category::Basic),
    k(0x0085, ".", Category::Numpad),
    k(0x0086, "=", Category::Numpad),
    k(0x0087, "In1", Category::Special),
    k(0x0088, "In2", Category::Special),
    k(0x0089, "In3", Category::Special),
    k(0x008A, "In4", Category::Special),
    k(0x008B, "In5", Category::Special),
    k(0x008C, "In6", Category::Special),
    k(0x008D, "In7", Category::Special),
    k(0x008E, "In8", Category::Special),
    k(0x008F, "In9", Category::Special),
    k(0x0090, "Lang1", Category::Special),
    k(0x0091, "Lang2", Category::Special),
    k(0x0092, "Lang3", Category::Special),
    k(0x0093, "Lang4", Category::Special),
    k(0x0094, "Lang5", Category::Special),
    k(0x0095, "Lang6", Category::Special),
    k(0x0096, "Lang7", Category::Special),
    k(0x0097, "Lang8", Category::Special),
    k(0x0098, "Lang9", Category::Special),
    k(0x0099, "Alt Erase", Category::Special),
    k(0x009A, "Sys Request", Category::Special),
    k(0x009B, "Cancel", Category::Special),
    k(0x009C, "Clear", Category::Special),
    k(0x009D, "Prior", Category::Special),
    k(0x009E, "Return", Category::Special),
    k(0x009F, "Sepaeator", Category::Special),
    k(0x00A0, "Out", Category::Special),
    k(0x00A1, "Oper", Category::Special),
    k(0x00A2, "Clear Again", Category::Special),
    k(0x00A3, "Crsel", Category::Special),
    k(0x00A4, "Exsel", Category::Special),
    k(0x00A5, "Desktop Power Down", Category::System),
    k(0x00A6, "Desktop Sleep", Category::System),
    k(0x00A7, "-", Category::Special),
    k(0x00A8, "-", Category::Special),
    k(0x00A9, "-", Category::Special),
    k(0x00AA, "-", Category::Special),
    k(0x00AB, "-", Category::Special),
    k(0x00AC, "-", Category::Special),
    k(0x00AD, "Play Pause", Category::Media),
    k(0x00AE, "Next Track", Category::Media),
    k(0x00AF, "Ptrack", Category::Media),
    k(0x00B0, "Stop", Category::Media),
    k(0x00B1, "Mute", Category::Media),
    k(0x00B2, "Vol+", Category::Media),
    k(0x00B3, "Vol-", Category::Media),
    k(0x00B4, "Media", Category::Media),
    k(0x00B5, "Email", Category::Media),
    k(0x00B6, "Calc", Category::Media),
    k(0x00B7, "PC", Category::Media),
    k(0x00B8, "WWW", Category::Media),
    k(0x00B9, "Search", Category::Media),
    k(0x00BA, "AC Back", Category::Media),
    k(0x00BB, "Forward", Category::Media),
    k(0x00BC, "Refresh", Category::Media),
    k(0x00BD, "Bookmarks", Category::Media),
    k(0x00BE, "MACT", Category::Special),
    k(0x00BF, "MACC", Category::Special),
    k(0x00C0, "SCDE", Category::Special),
    k(0x00C1, "SCIN", Category::Special),
    k(0x00C2, "-", Category::Special),
    k(0x00C3, "-", Category::Special),
    k(0x00C4, "-", Category::Special),
    k(0x00C5, "-", Category::Special),
    k(0x00C6, "-", Category::Special),
    k(0x00C7, "i18n:key1", Category::Special),
    k(0x00C8, "i18n:key2", Category::Special),
    k(0x00C9, "-", Category::Special),
    k(0x00CA, "-", Category::Special),
    k(0x00CB, "-", Category::Special),
    k(0x00CC, "-", Category::Special),
    k(0x00CD, "-", Category::Special),
    k(0x00CE, "-", Category::Special),
    k(0x00CF, "-", Category::Special),
    k(0x00D0, "-", Category::Special),
    k(0x00D1, "-", Category::Special),
    k(0x00D2, "-", Category::Special),
    k(0x00D3, "-", Category::Special),
    k(0x00D4, "-", Category::Special),
    k(0x00D5, "-", Category::Special),
    k(0x00D6, "-", Category::Special),
    k(0x00D7, "-", Category::Special),
    k(0x00D8, "-", Category::Special),
    k(0x00D9, "-", Category::Special),
    k(0x00DA, "-", Category::Special),
    k(0x00DB, "-", Category::Special),
    k(0x00DC, "-", Category::Special),
    k(0x00DD, "-", Category::Special),
    k(0x00DE, "-", Category::Special),
    k(0x00DF, "-", Category::Special),
    k(0x00E0, "Ctrl", Category::Modifier),
    k(0x00E1, "Shift", Category::Modifier),
    k(0x00E2, "Alt", Category::Modifier),
    k(0x00E3, "Win", Category::Modifier),
    k(0x00E4, "Ctrl", Category::Modifier),
    k(0x00E5, "Shift", Category::Modifier),
    k(0x00E6, "Alt", Category::Modifier),
    k(0x00E7, "Win", Category::Modifier),
    k(0x00E8, "-", Category::Special),
    k(0x106F, "i18n:key3", Category::Media),
    k(0x1070, "i18n:key4", Category::Media),
    k(0x10B5, "i18n:key5", Category::Media),
    k(0x10B6, "i18n:key6", Category::Media),
    k(0x10B7, "i18n:key7", Category::Media),
    k(0x10CD, "i18n:key8", Category::Media),
    k(0x10E2, "i18n:key9", Category::Media),
    k(0x10E9, "i18n:key10", Category::Media),
    k(0x10EA, "i18n:key11", Category::Media),
    k(0x1183, "Media", Category::Media),
    k(0x118A, "Email", Category::Media),
    k(0x1192, "Calc", Category::Media),
    k(0x1194, "File", Category::Media),
    k(0x1221, "search", Category::Media),
    k(0x1223, "www", Category::Media),
    k(0x129F, "desktop", Category::Media),
    k(0x6235, "~", Category::Basic),
    k(0x7301, "i18n:key12", Category::Mouse),
    k(0x7302, "i18n:key13", Category::Mouse),
    k(0x7303, "i18n:key14", Category::Mouse),
    k(0x7304, "i18n:key15", Category::Mouse),
    k(0x7305, "i18n:key16", Category::Mouse),
    k(0x7306, "i18n:key17", Category::Mouse),
    k(0x7307, "i18n:key18", Category::Mouse),
    k(0x7308, "i18n:key19", Category::Mouse),
    k(0x7309, "i18n:key20", Category::Mouse),
    k(0xF001, "Fn1", Category::Special),
    k(0xF002, "Fn2", Category::Special),
    k(0xF003, "Fn3", Category::Special),
    k(0xF100, "i18n:key21", Category::System),
    k(0xF101, "Win", Category::System),
    k(0xF102, "Mac", Category::System),
    k(0xF103, "i18n:key22", Category::System),
    k(0xF108, "i18n:key23", Category::System),
    k(0xF109, "i18n:key24", Category::System),
    k(0xF10A, "i18n:key25", Category::System),
    k(0xF10C, "i18n:key26", Category::System),
    k(0xF10B, "i18n:key27", Category::System),
    k(0xF309, "LockWin", Category::System),
    k(0xF317, "i18n:key28", Category::Lighting),
    k(0xF311, "i18n:key29", Category::Lighting),
    k(0xF310, "i18n:key30", Category::Lighting),
    k(0xF32F, "i18n:key31", Category::Lighting),
    k(0xF312, "i18n:key32", Category::Lighting),
    k(0xF313, "i18n:key33", Category::Lighting),
    k(0xF314, "i18n:key34", Category::Lighting),
    k(0xF315, "i18n:key35", Category::Lighting),
    k(0xF326, "i18n:key36", Category::Lighting),
    k(0xF327, "i18n:key37", Category::Lighting),
    k(0xF325, "i18n:key38", Category::Lighting),
    k(0xF32C, "i18n:key39", Category::Lighting),
    k(0xF32A, "i18n:key40", Category::Lighting),
    k(0xF32B, "i18n:key41", Category::Lighting),
    k(0xF328, "i18n:key42", Category::Lighting),
    k(0xF329, "i18n:key43", Category::Lighting),
    k(0xF336, "Static-Light-Change", Category::Lighting),
    k(0xF339, "snap tap on", Category::System),
];

/// Every keycode in the table, in the order of `keycodes.txt`.
pub fn all() -> &'static [KeyCode] {
    &TABLE
}

/// Look a keycode up by its u16 value.
pub fn lookup(code: u16) -> Option<&'static KeyCode> {
    TABLE.iter().find(|k| k.code == code)
}

const LETTERS: [&str; 26] = [
    "A", "B", "C", "D", "E", "F", "G", "H", "I", "J", "K", "L", "M", "N", "O", "P", "Q", "R", "S",
    "T", "U", "V", "W", "X", "Y", "Z",
];

const DIGITS: [&str; 10] = ["1", "2", "3", "4", "5", "6", "7", "8", "9", "0"];

/// A short key-cap label for a key's default (matrix) code, as used in per-key packets.
///
/// Falls back to the table label and then to `"?"`.
pub fn legend(default_code: u8) -> &'static str {
    match default_code {
        0x04..=0x1D => LETTERS[usize::from(default_code - 0x04)],
        0x1E..=0x27 => DIGITS[usize::from(default_code - 0x1E)],
        0x28 => "Enter",
        0x29 => "Esc",
        0x2A => "Bksp",
        0x2B => "Tab",
        0x2C => "Space",
        0x2D => "-",
        0x2E => "=",
        0x2F => "[",
        0x30 => "]",
        0x31 => "\\",
        0x33 => ";",
        0x34 => "'",
        0x35 => "`",
        0x36 => ",",
        0x37 => ".",
        0x38 => "/",
        0x39 => "Caps",
        0x49 => "Ins",
        0x4A => "Home",
        0x4B => "PgUp",
        0x4C => "Del",
        0x4D => "End",
        0x4E => "PgDn",
        0x4F => "→",
        0x50 => "←",
        0x51 => "↓",
        0x52 => "↑",
        0xE0 | 0xE4 => "Ctrl",
        0xE1 | 0xE5 => "Shift",
        0xE2 | 0xE6 => "Alt",
        0xE3 | 0xE7 => "Win",
        0x01 => "Fn",
        other => lookup(u16::from(other)).map_or("?", |k| k.label),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    /// The real WIN 68 HE MAX matrix (docs/protocol/PROTOCOL.md section 4.1), zeros included.
    fn matrix_codes() -> Vec<u8> {
        let json = include_str!("../../../docs/protocol/captures/win68-dump.json");
        let start = json.find("\"matrix\"").expect("matrix key");
        let rest = &json[start + "\"matrix\"".len()..];
        let open = rest.find('[').expect("matrix array");
        let mut depth = 0usize;
        let mut end = open;
        for (i, ch) in rest[open..].char_indices() {
            match ch {
                '[' => depth += 1,
                ']' => {
                    depth -= 1;
                    if depth == 0 {
                        end = open + i;
                        break;
                    }
                }
                _ => {}
            }
        }
        rest[open..end]
            .split('"')
            .skip(1)
            .step_by(2)
            .map(|s| u8::from_str_radix(s, 16).expect("hex byte"))
            .collect()
    }

    #[test]
    fn lookup_letter_a() {
        assert_eq!(lookup(0x0004).unwrap().label, "A");
    }

    #[test]
    fn legend_examples() {
        assert_eq!(legend(0x49), "Ins");
        assert_eq!(legend(0x01), "Fn");
        assert_eq!(legend(0x04), "A");
        assert_eq!(legend(0x27), "0");
        assert_eq!(legend(0x31), "\\");
        assert_eq!(legend(0x52), "↑");
        assert_eq!(legend(0xE6), "Alt");
    }

    #[test]
    fn table_has_one_entry_per_row() {
        let rows = include_str!("../../../docs/protocol/keycodes.txt")
            .lines()
            .filter(|l| !l.trim().is_empty())
            .count();
        assert_eq!(all().len(), rows);
    }

    #[test]
    fn codes_are_unique() {
        let set: HashSet<u16> = all().iter().map(|k| k.code).collect();
        assert_eq!(set.len(), all().len());
    }

    #[test]
    fn table_codes_match_file_order() {
        let codes: Vec<u16> = include_str!("../../../docs/protocol/keycodes.txt")
            .lines()
            .filter(|l| !l.trim().is_empty())
            .map(|l| {
                let hex = l
                    .split_whitespace()
                    .nth(1)
                    .unwrap()
                    .trim_start_matches("0x");
                u16::from_str_radix(hex, 16).unwrap()
            })
            .collect();
        let table: Vec<u16> = all().iter().map(|k| k.code).collect();
        assert_eq!(table, codes);
    }

    #[test]
    fn every_matrix_key_has_a_legend() {
        let codes = matrix_codes();
        assert_eq!(codes.len(), 6 * 21);
        let keys: Vec<u8> = codes.into_iter().filter(|&c| c != 0).collect();
        assert_eq!(keys.len(), 68);
        for c in keys {
            assert_ne!(legend(c), "?", "no legend for 0x{c:02X}");
        }
    }

    #[test]
    fn unknown_code_has_question_mark_legend() {
        assert_eq!(legend(0xFF), "?");
    }

    #[test]
    fn categories_follow_ranges() {
        let cat = |c| lookup(c).unwrap().category;
        assert_eq!(cat(0x0004), Category::Basic);
        assert_eq!(cat(0x0049), Category::Navigation);
        assert_eq!(cat(0x003A), Category::Function);
        assert_eq!(cat(0x0059), Category::Numpad);
        assert_eq!(cat(0x00E0), Category::Modifier);
        assert_eq!(cat(0x10E2), Category::Media);
        assert_eq!(cat(0x7301), Category::Mouse);
        assert_eq!(cat(0xF312), Category::Lighting);
        assert_eq!(cat(0xF101), Category::System);
        assert_eq!(cat(0x0001), Category::Special);
    }
}
