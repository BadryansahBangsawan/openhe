# OpenHE Phase 3–7 Implementation Plan (parallel tracks)

> **For agentic workers:** each track below is executed by ONE agent in its own git worktree. Read only your track's section, "Global Constraints" and "Shared Contracts". Do not edit files owned by another track. Steps use checkbox (`- [ ]`) syntax.

**Goal:** Finish OpenHE: a native Rust desktop app (egui/eframe) for Windows, Linux and macOS that replaces AULA HUB for WIN 68 HE MAX / WIN 60 HE MAX. It covers all AULA HUB features except firmware update and cloud features.

**Architecture:**
- Phase 1–2 delivered `openhe-proto` (packet formats), `openhe-device` (HID transport, `Session`, `Keyboard` API) and `openhe-cli`.
- This plan adds three things:
  - the remaining protocol and device features, split into three parallel device tracks;
  - a new GUI crate `openhe-app` (track B) with a worker thread and a generic job API, so later pages never edit shared enums;
  - release infrastructure (track C).
- Wave 2 builds the remaining pages in parallel on top of those.

**Tech Stack:**
- Rust stable, edition 2021, `rust-version = "1.85"`.
- `eframe`/`egui`: use the latest version that builds on 1.85, and record which one.
- `hidapi` 2, `thiserror` 1, `serde`/`serde_json` 1, `directories` 5.
- Track F only: `auto-launch` and `active-win-pos-rs`, or equivalent maintained crates.

**Spec:**
- `docs/superpowers/specs/2026-10-28-openhe-design.md`
- Protocol: `docs/protocol/PROTOCOL.md`. Section 7.x holds the hardware findings. The keycode table is in `docs/protocol/keycodes.txt`.
- Mockups: `docs/design/mockups/01-performa.png` … `07-pengaturan.png` and `00-logo-concepts.png`.
- The prettified AULA HUB sources are at `/private/tmp/claude-501/-Users-bbbadry-Downloads-custom-software/d795ce30-8ab7-4f04-bef6-98f14728154b/scratchpad/pretty/`. They are reference only and must never be copied into the repo.

## Global Constraints (bind every track)
- Everything in Phase 1–2's Global Constraints still holds:
  - VID 0x1CA2, PID 0x1901/0x1902, usage page 0xFFA0, usage 1.
  - `[0x00]+64` byte writes, the documented checksum, and the reply matching rules (Rulings 12/13).
  - Rounding mm→µm.
  - No 0x08–0x0E commands, ever.
  - No network access.
  - `#![forbid(unsafe_code)]` in our crates.
  - fmt + clippy `-D warnings` clean, pristine test output.
- **Never run a binary that opens the HID device, and never write to the user's keyboard.** The keyboard is attached. All hardware testing is done by the controller with the user's permission. Tests use `MockTransport`.
- **Mode byte writes:**
  - Every write of KEY LAYOUT 8 (Mode) must preserve the advanced nibble and the high byte (`KeyMode.high`), unless the write's purpose is to change exactly that field.
  - Setting actuation or RT on a key sets its touch mode to Single (normal) or RapidTrigger. This is a controller ruling, and it matches AULA HUB.
- Every device write is followed by a read-back verify, or a reply check where no read-back exists.
- On error, the UI re-reads the affected state from the device. It never trusts its cache after a failed write.
- UI strings are Indonesian and live in `crates/openhe-app/src/i18n.rs`.
- UI matches the mockups:
  - dark theme;
  - colours: background #0F1013, panel #16181C, sidebar #14161A, border #23262C/#2C3038, text #ECEDEF, muted #8B9099, accent #FFB224, accent-soft rgba(255,178,36,0.14);
  - layout: left sidebar 232 px, top bar 64 px, right panel 348 px.
- Real WIN 68 HE MAX layout (from hardware):
  - Matrix rows 1–5 (row 0 empty).
  - The right column is Ins (0x49), Del (0x4C), PgUp (0x4B), PgDn (0x4E). This differs from the mockup's Home/End.
  - Key widths as a 68% board:
    - Backspace 2u
    - Tab 1.5u, `\` 1.5u
    - Caps 1.75u, Enter 2.25u
    - LShift 2.25u, RShift 1.75u
    - Ctrl/Win/Alt 1.25u, Space 6.25u
- Commit messages end with a blank line + `Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>`. Commit only files you own.

## Shared Contracts (the only cross-track coupling — implement exactly)

**P0 (done before any track starts) provides:**
- `openhe_proto::keycodes`:
  - `pub enum Category { Basic, Navigation, Function, Numpad, Modifier, Media, Mouse, Lighting, System, Special }`
  - `pub struct KeyCode { pub code: u16, pub label: &'static str, pub category: Category }`
  - `pub fn all() -> &'static [KeyCode]`, generated from every row of `docs/protocol/keycodes.txt`
  - `pub fn lookup(code: u16) -> Option<&'static KeyCode>`
  - `pub fn legend(default_code: u8) -> &'static str`: a short key-cap label for matrix codes, e.g. 0x29 "Esc", 0x2A "Bksp", 0x49 "Ins", 0x01 "Fn", 0x2C "Space", arrows "←↑→↓".
- `openhe_proto` re-exports `key::{Layer, Slot}` at the crate root.
- Empty modules, registered so tracks never touch `lib.rs`/`mod.rs`:
  - proto: `advanced.rs`, `lighting.rs`, `macros.rs`, `travel.rs`
  - device: `keyboard/mod.rs` (moved from `keyboard.rs`, content unchanged), `keyboard/advanced.rs`, `keyboard/lighting.rs`, `keyboard/macros.rs`, `keyboard/system.rs`, `keyboard/performance.rs`
  - Each file holds only a module doc comment.

**Track A1 → D:**
- In `openhe_proto::advanced`:
  - `Dks { bindings: [u16;4], trps: [u8;4], first_actuation: Micrometers, bottom: Micrometers }`
  - `Mt { hold: u16, tap: u16, delay_ms: u16 }`
  - `Tgl { code: u16, delay_ms: u16 }`
  - `Mpt { codes: [u16;3], depths: [Micrometers;3] }`
  - `End { code: u16 }`
  - `Socd { pos1: u8, pos2: u8, key1: u16, key2: u16, bind_values: bool, mode: SocdMode, delay: u16 }`
  - `SocdMode { LastInput=0, Key1Wins=1, Key2Wins=2, Neutral=3 }`
- In `openhe_device`:
  - `pub enum AdvancedConfig { None, Dks(Dks), Mt(Mt), Tgl(Tgl), Mpt(Mpt), End(End), Socd(Socd), Macro }`
  - `Keyboard::advanced(&mut self, key: u8) -> Result<AdvancedConfig, Error>`
  - `Keyboard::set_advanced(&mut self, key: u8, cfg: &AdvancedConfig) -> Result<(), Error>`. Passing `None` clears the function while preserving touch mode and the high byte.
  - `Keyboard::layer_code(&mut self, key: u8, layer: Layer) -> Result<u16, Error>`
  - `Keyboard::set_layer_codes(&mut self, entries: &[(u8, Layer, u16)]) -> Result<(), Error>`. It splits into ≤14-entry packets (already in `key::set`), verifies, and never writes the Fn key (0x01) on the base layer.

**Track A2 → E:**
- In `openhe_proto::lighting`:
  - `Rgb { r, g, b }`
  - `Lighting { on: bool, reverse: bool, brightness: u8 /*0..=4*/, mode: u8 /*0 static, 1..=20 effects, 21 custom*/, speed: u8 /*0..=4*/, sleep_minutes: u8 /*0,1,2,3,5,10,15,20,25,30,45,60,120*/, static_color: u8 /*0..=7*/, dynamic_color: u8 /*0..=7*/, colors: [Rgb;7] }`
  - `pub const EFFECTS: [(u8, &str);20]`, with the English names from PROTOCOL §3.8.
- In `openhe_proto::macros`:
  - `MacroAction { code: u16, down: bool, delay_ms: u32 }`
  - `PlayMode { Once=0, Toggle=1, WhileHeld=2, WhileHeldFinish=3 }`
- In `openhe_device`:
  - `Keyboard::lighting(&mut self) -> Result<Lighting, Error>`. Send RGBNACK first.
  - `Keyboard::set_lighting(&mut self, l: &Lighting) -> Result<(), Error>`
  - `Keyboard::key_colors(&mut self, keys: &[u8]) -> Result<Vec<(u8, Rgb)>, Error>`
  - `Keyboard::set_key_colors(&mut self, colors: &[(u8, Rgb)]) -> Result<(), Error>`. Commits with `FE` afterwards.
  - `Keyboard::bind_macro(&mut self, key: u8, actions: &[MacroAction], mode: PlayMode, repeat: u16, interval_ms: u32) -> Result<(), Error>`
  - `Keyboard::macro_binding(&mut self, key: u8) -> Result<Option<MacroBinding>, Error>`, with `MacroBinding { len: u8, mode: PlayMode, repeat: u16, interval_ms: u32 }`.

**Track A3 → F and the worker:**
- In `openhe_proto::travel`, RM6X21:
  - `pub struct TravelFrame { pub um: [[u16; 21]; 6] }`
  - `request(sub: u8, part: u8) -> Request`
  - `parse(reports: &[[u8;64]]) -> Result<TravelFrame-half, FrameError>`
- In `openhe_device`:
  - `pub enum DeviceEvent { ProfileChanged(u8), SystemModeChanged(Option<SystemMode>), LightingChanged, KeyDataChanged, Other(u8) }`
  - `Keyboard::poll_events(&mut self) -> Result<Vec<DeviceEvent>, Error>`. This is a non-blocking drain and also converts `take_events()`.
  - `Keyboard::handshake(&mut self) -> Result<DeviceInfo, Error>`. It returns `Error::Unsupported("bootloader")` when the keyboard is in bootloader mode.
  - `Keyboard::set_profile(&mut self, idx: u8)`
  - `Keyboard::set_system_mode(&mut self, m: SystemMode)`
  - `Keyboard::set_polling_rate(&mut self, r: PollingRate) -> Result<WriteOutcome, Error>`, where `WriteOutcome { Done, Reenumerating }`. Disconnect or timeout after the write counts as `Reenumerating`. Sent with exactly one attempt.
  - `Keyboard::factory_reset(&mut self) -> Result<(), Error>`. Exactly one attempt, no retry.
  - `Keyboard::start_calibration` / `stop_calibration`
  - `Keyboard::travel(&mut self) -> Result<TravelFrame, Error>`, built from (2,1)+(2,2)
  - `Keyboard::calibration_values(&mut self) -> Result<TravelFrame, Error>`, built from (6,1)+(6,2)
  - `Session::call_once(&mut self, req) -> Result<Response, Error>` (single attempt)
  - `Session::poll(&mut self) -> Vec<Response>` (read(0) drain)

**Track B → D, E, F:**
- `crates/openhe-app/src/worker.rs`:
  - `pub struct Worker`, with `Worker::spawn() -> Worker`
  - `Worker::submit<T: Send + 'static>(&self, label: &'static str, job: impl FnOnce(&mut Keyboard<HidTransport>) -> Result<T, Error> + Send + 'static) -> Pending<T>`
  - `Pending<T>::poll(&mut self) -> Option<Result<T, Error>>` (non-blocking)
  - `Worker::status(&self) -> ConnectionStatus`, with `ConnectionStatus { Searching, Connected(DeviceSnapshot), Error(String) }`
  - `Worker::snapshot(&self) -> Option<DeviceSnapshot>`
  - `Worker::request_reload(&self)`
  - `DeviceSnapshot { model, info: DeviceInfo, system_mode, profile, polling, matrix: Matrix, keys: BTreeMap<u8, KeySettings> }`
- `crates/openhe-app/src/widgets/keyboard.rs`:
  - `KeyboardView::new(model, &matrix)`
  - `.show(ui, &mut Selection, |code| -> KeyDecoration) -> Response`
  - `KeyDecoration { label2: Option<String>, fill: Option<Color32>, stroke: Option<Color32>, badge: Option<(String, Color32)> }`
  - `Selection`: a set of codes with click, drag and shift-click.
- `crates/openhe-app/src/pages/{performance,advanced,remap,macros,lighting,calibration,settings}.rs`. Each exposes `pub struct Page` with `pub fn ui(&mut self, ctx: &AppCtx, ui: &mut egui::Ui)`. Track B fully implements `performance`. The others are stubs that show "Segera hadir".
- `AppCtx` (in `app.rs`) gives pages `&Worker`, `&DeviceSnapshot` (when connected), the theme and the i18n.
- Widgets: `widgets/{slider_mm.rs, segmented.rs, toggle.rs, card.rs}`.
- `i18n.rs` holds Indonesian strings as `pub const`, grouped by page.
- `storage.rs` provides `AppData` (serde, saved under `directories::ProjectDirs("dev","OpenHE","OpenHE")`) with fields `macros: Vec<SavedMacro>`, `profile_names: [String;4]`, `app_profile_rules: Vec<(String /*app name*/, u8)>`, `launch_at_login: bool`.
  - `SavedMacro { name: String, actions: Vec<MacroAction-serde-mirror>, mode: u8, repeat: u16, interval_ms: u32 }`

## Review Focus
1. **Changing actuation or RT on a key that has DKS or another advanced function.** The advanced nibble must survive, which avoids the AULA bug. Track B tests this.
2. **Keyboard unplugged mid-job.** The worker reports `Searching`, the UI disables write buttons, the pending job ends in `Err(Disconnected)`, and the app reconnects automatically without a restart. Track B tests this.
3. **Profile changed from the keyboard (Fn+key) while the app is open.** The snapshot reloads and the UI shows the new profile. Tracks A3 and F test this.
4. **Polling-rate change re-enumerates USB.** The UI shows "menyambung ulang…", never shows an error, and reconnects. Track F tests this.
5. **Remap of the Fn key, or a remap leaving no Fn key on the base layer.** Fn on the base layer can never be overwritten. Tracks A1 and D test this.

---

## Wave 0 — P0 shared scaffolding (one agent, sequential, small)
- [ ] Generate `openhe_proto::keycodes` from `docs/protocol/keycodes.txt`:
  - Every row becomes an entry.
  - Category follows code ranges and names.
  - `legend()` covers every code that appears in the real matrix.
  - Tests: `lookup(0x0004).label == "A"`; `legend(0x49) == "Ins"`; `all().len()` equals the number of rows in keycodes.txt; codes are unique.
- [ ] Re-export `Layer` and `Slot`.
- [ ] Create the empty modules listed under Shared Contracts. Move `keyboard.rs` to `keyboard/mod.rs` unchanged; all tests still pass.
- [ ] Commit `chore: shared scaffolding for phase 3-7 tracks`.

## Wave 1 — four parallel tracks

### Track A1: advanced keys + layers (proto `advanced.rs`, device `keyboard/advanced.rs`)
- Byte layouts come from PROTOCOL §3.6, using the SOCD ≥1.0.7 format (the user's firmware is App V1.1.3). u16 values are little-endian. Confirm the order against the prettified sources (`high-*.js`, `info-*.js` builders).
- Implement builders and parsers for DKS 0x26, MPT 0x27, MT 0x24, TGL 0x25, END 0x28, SOCD 0x2C. The DKS `trps` bits are 0x01/0x02/0x04/0x18/0x20/0x40/0x80.
- Validation before any I/O:
  - MPT depths strictly increasing, within 0.1–3.4 mm.
  - DKS first actuation within 0.1–3.4 mm, bottom fixed at 3400.
  - MT/TGL delay 1–1000 ms, sent in 10 ms units.
  - SOCD delay 0–50.
- `set_advanced` writes the function packet, then KEY Mode `(touch<<4)|adv` with the high byte preserved, then reads back and verifies.
- `layer_code` / `set_layer_codes` use `KeyParam::Layer`. Writing base-layer 0x01 (Fn) is rejected with `InvalidInput`.
- Tests: exact bytes for every builder (one hand-checked golden vector each); parser round-trips; validation errors; Mode preservation; Fn protection; a >14-entry layer write splits into packets.

### Track A2: lighting + macros (proto `lighting.rs`, `macros.rs`; device `keyboard/lighting.rs`, `keyboard/macros.rs`)
- PRGB 0x18:
  - LEN 44 using the "1.0.9" layout in PROTOCOL §3.8. Always send 7 colours in BGR+0xFF order.
  - Byte [37] flags: bit0 on, bit1 reverse, bit4 always 1.
  - Static/custom modes force speed to 0.
  - GET uses flag 00; the parser uses the same offsets.
- RGBNACK: `order::set(OrderType::RgbNack, 1)` before reading lighting.
- KRGB 0x2A: SET `[01,KEY,R,G,B]` (RGB order), GET `[00,KEY]`, commit `[01,FE,FE,FE,FE]` after every batch.
- MACRO 0x20:
  - At most 9 actions per packet.
  - `addr` starts at 256 and advances by the action count.
  - `word = (flag<<24) | (delay & 0xFFFFFF)`, with flag 1 = down, 8 = up.
- MACRO_MODE 0x21: `[01,KEY,id u16=0,len,mode,num u16,interval u24]`.
- Bind sequence: data, then Mode `(touch<<4)|6` preserving the high byte, then macro mode, then verify by reading macro mode.
- Validation: delay < 2^24; actions ≤ 255; repeat ≥ 1.
- Document in rustdoc that whether several keys can hold different macros is unverified (PROTOCOL Q).
- Tests: exact bytes (golden vectors), BGR order, 9-per-packet split and addresses, Mode preservation, the commit packet after a colour batch.

### Track A3: system, travel, events (proto `travel.rs`; device `keyboard/system.rs`, `session.rs` additions)
- `Session::call_once` and `Session::poll` as in the contract.
- RM6X21 0x12: payload `[SUB, PART, FF, FF]`, 3 reports per request (`call_many(req, 3)`). Concatenate them, drop the first 4 bytes, then parse 3 rows × 21 u16le µm. The framing is flagged [I] in PROTOCOL, so make the parser strict about length and write a test from a synthetic 3-report capture.
- `DeviceEvent` mapping:
  - 0x80 with R[1]=0x70 → `ProfileChanged(R[2])`
  - 0x80 with R[1]=0x21 → `SystemModeChanged`
  - 0x98 → `LightingChanged`
  - 0xA3/0xAB → `KeyDataChanged`
  - anything else → `Other(cmd)`
- `handshake`, `set_profile` (0..=3, verified by `profile()`), `set_system_mode` (`order::set(SetWin|SetMac, 1)`, reply R[2]==1, then re-query), `set_polling_rate` (single attempt, WriteOutcome), `factory_reset` (single attempt), calibration start/stop (0x0C/0x0D), `travel`, `calibration_values`.
- Tests: each command's exact bytes; event mapping; the polling Disconnected→Reenumerating path; factory_reset never retries (mock: no reply → exactly 1 write); travel parse.

### Track B: GUI app + Performa page (new crate `crates/openhe-app`; device `keyboard/performance.rs`)
- `keyboard/performance.rs`:
  - `pub struct PerformanceChange { pub touch: Option<TouchMode>, pub actuation: Option<Micrometers>, pub rt_press: Option<Micrometers>, pub rt_release: Option<Micrometers>, pub top_dz: Option<Micrometers>, pub bottom_dz: Option<Micrometers>, pub switch: Option<u16> }`
  - `Keyboard::set_performance(&mut self, keys: &[u8], change: &PerformanceChange) -> Result<(), Error>`
  - It validates against each key's switch max travel. RT range is 0.005–max; deadzones 0–1.0 mm.
  - It writes all params in batched `set_key_params`, plus Mode with the touch change and the advanced nibble and high byte preserved.
  - Tests cover Review Focus 1.
- The app as described in Shared Contracts:
  - Worker thread with auto-connect every 1 s while disconnected, using `HidTransport::open_first`.
  - On connect it loads the snapshot (handshake when A3 is merged; until then `info()` + matrix + all `key_settings`).
  - Idle event polling arrives later (A3). Use `take_events()` for now behind a small adapter so Track F can switch it to `poll_events`.
  - Sidebar, top bar (profile segmented + Mac/Win shown read-only until F), device card.
  - Keyboard widget with real widths for WIN 68. For WIN 60, a best-effort 61-key layout flagged "belum diuji".
  - Performa page per mockup `01-performa.png`:
    - selection toolbar (Pilih semua / WASD / Huruf / Hapus pilihan);
    - per-key values (actuation) on the keys;
    - right panel with Normal/Rapid Trigger, actuation slider (0.10–max of the selected keys' switch), RT press/release sliders with "Samakan", deadzone fields, switch selector, and "Terapkan ke keyboard" (a job → `set_performance` → reload those keys).
    - The live-depth bar is a placeholder card ("Kedalaman live — tersedia setelah kalibrasi/travel siap"), finished by F.
  - Errors are shown inline with a retry option; the UI never blocks.
- Tests: worker job round-trip using a mock keyboard factory (make the transport generic behind a factory fn so tests use MockTransport); selection logic; keyboard layout geometry (row widths equal 15u ±0.01 for WIN 68); Review Focus 1 and 2.
- Verification: `cargo build -p openhe-app`. The controller runs the GUI.

### Track C: release infrastructure (no Rust crate edits)
- Final logo, concept A "Open Magnet" (`assets/logo/concepts/a-open-magnet.svg`), via the logo-design skill:
  - refined symbol; wordmark lockup with outlined paths, no live text;
  - accent #FFB224 on dark #0F1013, plus mono black/white;
  - app icons: `assets/icons/openhe.icns` (macOS), `openhe.ico` (Windows), PNG 16–1024 (Linux);
  - a short `assets/logo/README.md` usage note.
- `packaging/macos/build-app.sh`: builds `openhe-app` in release and assembles `OpenHE.app` (Info.plist with bundle id `dev.openhe.OpenHE`, icon, LSMinimumSystemVersion 11.0).
- `packaging/linux/openhe.desktop`, `packaging/linux/build-appimage.sh` (linuxdeploy), with a README section for the udev rule `70-openhe.rules`.
- `packaging/windows/README.md`: how the icon is embedded. Add a `build.rs` snippet as text; the controller wires it in during integration.
- `.github/workflows/ci.yml`:
  - fmt, clippy, test on ubuntu-latest / windows-latest / macos-latest;
  - Linux installs libudev-dev, pkg-config, and the egui/eframe system libs (libxkbcommon-dev, libgl1-mesa-dev, libwayland-dev, libx11-dev, libxcursor-dev, libxrandr-dev, libxi-dev).
- `.github/workflows/release.yml`: on `v*` tags, builds artifacts for the 3 OS.
- `LICENSE` (MIT, "OpenHE contributors").
- README: install and usage, in Indonesian and English.

## Wave 2 — three parallel page tracks (after Wave 1 is merged)
- **D: Tombol Lanjutan + Remap** (`pages/advanced.rs`, `pages/remap.rs`), per mockups 02/03:
  - Keys are badged by function colour (DKS #A78BFA, MT #38BDF8, TGL #34D399, MPT #F472B6, SOCD #FFB224, END #F87171).
  - Advanced page: list of assigned functions; editors for DKS (4×4 grid of trps toggles + first-actuation slider), MT, TGL, MPT (3 depths), SOCD (pair picker + mode), END.
  - Remap page: layer tabs Base/Fn1/Fn2/Fn3; key picker by `keycodes::Category` with search; reset to default.
  - Tests: editors' value → config mapping; Fn protection in the UI.
- **E: Lampu + Macro** (`pages/lighting.rs`, `pages/macros.rs`), per mockups 05/04:
  - Lighting: effect grid of 20 + Statis + Kustom, with Indonesian labels mapped to the wire modes; 7-colour palette editor; brightness/speed 1–5; direction; sleep timeout; per-key colour mode painting on the keyboard widget.
  - Macro: library stored in `storage.rs` AppData; recorder captures egui key events and maps them to keycodes with real delays; step editor; play mode; bind to a key.
  - Tests: palette/BGR mapping UI→Lighting; recorder event→MacroAction conversion; storage round-trip.
- **F: Kalibrasi + Pengaturan + live depth + events** (`pages/calibration.rs`, `pages/settings.rs`; `worker.rs` event adapter switched to `poll_events`; Performa live-depth card):
  - Calibration flow per mockup 06: start → poll travel/calibration every 60 ms → a key turns green when `travel >= maxTravel - dp - dr - 0.1 mm` → stop/save.
  - Settings per mockup 07:
    - polling with `Reenumerating` handling;
    - Win/Mac;
    - 4 profile names (stored locally) and switching;
    - import/export of the current profile to JSON: all key settings, layers, advanced, lighting;
    - per-app automatic profile switching;
    - launch at login;
    - factory reset behind a typed confirmation ("RESET").
  - Tests: calibrated-threshold logic; export/import JSON round-trip; event→reload; Reenumerating UI state.

## Wave 3 — integration (controller)
- Merge the tracks. Wire the Windows icon `build.rs`.
- Run the GUI on macOS and screenshot each page against the mockups.
- Final whole-branch review (most capable model) and one fix wave.
- Hardware session, with the user's explicit permission per category:
  - write-and-restore tests for performance, advanced, remap, lighting, macro;
  - calibration;
  - polling change;
  - factory reset is never run without a separate explicit yes.
- Build the `OpenHE.app` bundle locally. Windows/Linux builds need CI; push to GitHub only if the user asks.
