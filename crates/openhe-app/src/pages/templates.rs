//! Template: ready-made and saved sets of changes for the active profile.
//!
//! Flow (every write goes through the worker):
//!
//! 1. The user picks a template (built-in, saved, imported file or a
//!    backup). One job reads every key of the active profile
//!    ([`read_keys`]); the template is planned against that state
//!    ([`plan_template`]) and the preview shows what changes: a summary in
//!    the right panel and the affected keys on the keyboard.
//! 2. "Terapkan": if the profile, connection or snapshot moved since the
//!    read, nothing is written and the preview is read again. Otherwise the
//!    state read in step 1 is saved as a local backup first (no backup, no
//!    write), then the plan runs ([`import_job`]: every write verified by
//!    read-back) and the snapshot is reloaded.
//! 3. "Tersimpan" offers "Urungkan": the backup is applied back the same
//!    way (it is itself backed up first). Recent backups are listed and can
//!    be restored with a preview.
//!
//! A profile switch or a reconnect drops a pending preview. While a
//! preview or a write is under way, automatic per-app profile switches
//! wait ([`Page::holds_profile`]).

use crate::app::{page_with_right_panel, AppCtx};
use crate::i18n;
use crate::pages::settings::profile_file::{import_job, read_keys, ImportReport, KeyData};
use crate::storage::AppData;
use crate::templates::plan::{plan_template, summary, Summary, TemplatePlan};
use crate::templates::store::{self, Backup, Saved};
use crate::templates::{self, error_text, Builtin, Parts, Template, TemplateFile};
use crate::theme::{color, font, line, size};
use crate::widgets::card::{self, Card};
use crate::widgets::keyboard::{reference_matrix, KeyDecoration, KeyboardView, Selection};
use crate::widgets::slider_mm::mm_short;
use crate::worker::Pending;
use eframe::egui::{self, CornerRadius, RichText, Sense, Ui};
use openhe_device::{AdvancedConfig, Error};
use openhe_proto::Model;
use std::path::{Path, PathBuf};

mod strings {
    pub const BUILTIN: &str = "Template bawaan";
    pub const BUILTIN_HINT: &str =
        "Hanya mengubah yang disebut template, di profil yang aktif. Pilih satu untuk melihat pratinjau.";
    pub const MINE: &str = "Template saya";
    pub const MINE_EMPTY: &str = "Belum ada template tersimpan.";
    pub const SAVE_CURRENT: &str = "Simpan setting sekarang…";
    pub const IMPORT: &str = "Impor file…";
    pub const EXPORT: &str = "Ekspor…";
    pub const DELETE: &str = "Hapus";
    pub const YES_DELETE: &str = "Ya, hapus";
    pub const CANCEL: &str = "Batal";
    pub const SAVE: &str = "Simpan";
    pub const NAME: &str = "Nama template";
    pub const NAME_HINT: &str = "mis. Valorant";
    pub const INCLUDE: &str = "Ikut disimpan (seluruh keyboard):";
    pub const PART_PERF: &str = "Performa";
    pub const PART_REMAP: &str = "Remap";
    pub const PART_ADV: &str = "Tombol lanjutan";
    pub const SAVING: &str = "Membaca setting dari keyboard…";
    pub const SAVED: &str = "Template disimpan.";
    pub const SAVE_FAILED: &str = "Template tidak bisa disimpan.";
    pub const EXPORTED: &str = "Template diekspor ke";
    pub const DELETED: &str = "Template dihapus.";
    pub const FILE_FAILED: &str = "File tidak bisa dibaca atau ditulis.";

    pub const KEYBOARD: &str = "Tombol yang berubah";
    pub const KEYBOARD_IDLE: &str = "Pilih template untuk melihat tombol yang berubah.";
    pub const LEGEND_CHANGED: &str = "berubah";
    pub const LEGEND_SKIPPED: &str = "dilewati";

    pub const TITLE: &str = "Template";
    pub const INTRO: &str =
        "Template mengubah profil yang aktif saja. Sebelum menulis, OpenHE menyimpan cadangan setting sekarang di komputer ini.";
    pub const READING: &str = "Membaca keyboard…";
    pub const PREVIEW: &str = "Pratinjau";
    pub const ON_PROFILE: &str = "Untuk Profil";
    pub const BACKUP_FIRST: &str = "Cadangan otomatis disimpan sebelum menulis.";
    pub const APPLY: &str = "Terapkan";
    pub const APPLYING: &str = "Menerapkan…";
    pub const KEEP_IMPORTED: &str = "Simpan ke Template saya";
    pub const REFRESHED: &str =
        "Keyboard berubah sejak pratinjau dibuat. Pratinjau sudah diperbarui; periksa lalu terapkan lagi.";
    pub const DONE: &str = "Tersimpan di keyboard";
    pub const DONE_BODY: &str = "Nilai sudah dibaca ulang dari keyboard.";
    pub const UNDO: &str = "Urungkan";
    pub const UNDO_PREFIX: &str = "Urungkan";
    pub const RESTORE_PREFIX: &str = "Pulihkan cadangan";
    pub const NOTHING_TO_UNDO: &str = "Tidak ada yang perlu dikembalikan.";
    pub const PARTIAL: &str =
        "Sebagian perubahan gagal ditulis; nilai sudah dibaca ulang. Cadangan tetap tersimpan.";
    pub const DISCONNECTED: &str =
        "Keyboard terputus saat menulis. Setelah tersambung lagi, periksa nilainya atau pulihkan cadangan.";
    pub const FAILED: &str = "Gagal";
    pub const READ_FAILED: &str = "Keyboard tidak bisa dibaca, jadi tidak ada yang ditulis.";
    pub const CANNOT_USE: &str = "Template tidak bisa dipakai";
    pub const BACKUP_FAILED: &str =
        "Cadangan tidak bisa disimpan, jadi tidak ada yang ditulis ke keyboard.";
    pub const NO_DATA_DIR: &str = "Folder data OpenHE tidak tersedia.";
    pub const DROPPED_PROFILE: &str = "Profil atau koneksi berganti, jadi pratinjau dibatalkan.";

    pub const BACKUPS: &str = "Cadangan terakhir";
    pub const BACKUPS_EMPTY: &str =
        "Belum ada cadangan. Cadangan dibuat setiap kali template diterapkan.";
    pub const BEFORE: &str = "Sebelum";
    pub const PROFILE: &str = "Profil";
    pub const RESTORE: &str = "Pulihkan";
    pub const BACKUP_GONE: &str = "Cadangan ini tidak bisa dibaca lagi.";
}
use strings as t;

/// What a job or plan was made for: a profile switch or a reconnect makes
/// it stale; a moved generation (a reload) means the keys read may be old.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Stamp {
    profile: u8,
    connects: u64,
    generation: u64,
}

impl Stamp {
    fn now(ctx: &AppCtx) -> Option<Stamp> {
        ctx.snapshot.map(|s| Stamp {
            profile: s.profile,
            connects: ctx.worker.connects(),
            generation: ctx.worker.generation(),
        })
    }

    /// Same profile on the same connection.
    fn same_place(&self, ctx: &AppCtx) -> bool {
        ctx.snapshot.is_some_and(|s| s.profile == self.profile)
            && ctx.worker.connects() == self.connects
    }
}

/// Where the template being previewed came from.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Source {
    Builtin(Builtin),
    Saved(PathBuf),
    /// An imported file (its name), not in the library yet.
    Imported(String),
    Backup(PathBuf),
    /// "Urungkan" of the last apply.
    Undo,
}

/// After the read: show the preview, or apply at once ("Urungkan").
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Then {
    Preview,
    Apply,
}

enum Flow {
    Idle,
    Reading {
        tpl: Box<Template>,
        source: Source,
        then: Then,
        pending: Pending<Vec<KeyData>>,
        on: Stamp,
        /// Read again because the keyboard moved after a preview.
        refreshed: bool,
    },
    Preview {
        tpl: Box<Template>,
        source: Source,
        current: Vec<KeyData>,
        plan: Box<TemplatePlan>,
        summary: Summary,
        on: Stamp,
        refreshed: bool,
    },
    Applying {
        name: String,
        pending: Pending<ImportReport>,
        backup: PathBuf,
        on: Stamp,
    },
    Done {
        ok: bool,
        text: String,
        tech: Option<String>,
        /// The backup "Urungkan" applies (none when nothing was written).
        backup: Option<PathBuf>,
        on: Stamp,
    },
    Failed {
        title: &'static str,
        detail: String,
        tech: Option<String>,
    },
}

/// "Simpan setting sekarang" form.
struct SaveForm {
    name: String,
    parts: Parts,
    pending: Option<Pending<Vec<KeyData>>>,
}

pub struct Page {
    flow: Flow,
    /// The user's templates (read on first show and after changes).
    library: Option<Vec<Saved>>,
    /// Backups of the connected keyboard, keyed by its storage key.
    backups: Option<(String, Vec<Backup>)>,
    save_form: Option<SaveForm>,
    confirm_delete: Option<PathBuf>,
    /// The last library action's result: (ok, text).
    lib_note: Option<(bool, String)>,
    /// A preview was dropped (profile switch, reconnect).
    dropped: bool,
    selection: Selection,
    /// The app data directory (tests point it elsewhere through
    /// `OPENHE_DATA_DIR`).
    base: Option<PathBuf>,
}

impl Default for Page {
    fn default() -> Self {
        Page {
            flow: Flow::Idle,
            library: None,
            backups: None,
            save_form: None,
            confirm_delete: None,
            lib_note: None,
            dropped: false,
            selection: Selection::default(),
            base: AppData::default_dir(),
        }
    }
}

fn keys_of(ctx: &AppCtx) -> Vec<u8> {
    ctx.snapshot
        .map(|s| s.matrix.keys().map(|(_, _, c)| c).collect())
        .unwrap_or_default()
}

fn submit_read(ctx: &AppCtx, label: &'static str) -> Pending<Vec<KeyData>> {
    let keys = keys_of(ctx);
    ctx.worker.submit(label, move |kb| read_keys(kb, &keys))
}

impl Page {
    // ------------------------------------------------------- state

    /// A preview is shown or a template is being read or written: automatic
    /// profile switches must wait (they would change the profile the
    /// preview was made for).
    pub fn holds_profile(&self) -> bool {
        matches!(
            self.flow,
            Flow::Reading { .. } | Flow::Preview { .. } | Flow::Applying { .. }
        )
    }

    /// Nothing of this page runs on the keyboard.
    fn idle(&self) -> bool {
        !matches!(self.flow, Flow::Reading { .. } | Flow::Applying { .. })
            && self.save_form.as_ref().is_none_or(|f| f.pending.is_none())
    }

    fn can_start(&self, ctx: &AppCtx) -> bool {
        ctx.can_write() && self.idle()
    }

    /// The keys highlighted on the keyboard: (changed, skipped).
    pub fn preview_keys(&self) -> Option<(Vec<u8>, Vec<u8>)> {
        match &self.flow {
            Flow::Preview { plan, .. } => Some((
                plan.changed_keys().into_iter().collect(),
                plan.skipped_keys()
                    .difference(&plan.changed_keys())
                    .copied()
                    .collect(),
            )),
            _ => None,
        }
    }

    fn library(&mut self) -> &Vec<Saved> {
        if self.library.is_none() {
            self.library = Some(
                self.base
                    .as_deref()
                    .map(store::list_templates)
                    .unwrap_or_default(),
            );
        }
        self.library.get_or_insert_with(Vec::new)
    }

    fn backups(&mut self, ctx: &AppCtx) -> Vec<Backup> {
        let (Some(snap), Some(key)) = (ctx.snapshot, ctx.keyboard_key()) else {
            return Vec::new();
        };
        if self.backups.as_ref().is_none_or(|(k, _)| *k != key) {
            let list = self
                .base
                .as_deref()
                .map(|b| store::list_backups(b, snap.model, &snap.info.serial))
                .unwrap_or_default();
            self.backups = Some((key, list));
        }
        self.backups
            .as_ref()
            .map(|(_, l)| l.clone())
            .unwrap_or_default()
    }

    // --------------------------------------------------------- flow

    /// Starts a preview (or, for `Then::Apply`, an apply) of `tpl`.
    fn start(&mut self, ctx: &AppCtx, tpl: Template, source: Source, then: Then) {
        let Some(on) = Stamp::now(ctx) else { return };
        self.dropped = false;
        self.lib_note = None;
        self.flow = Flow::Reading {
            tpl: Box::new(tpl),
            source,
            then,
            pending: submit_read(ctx, "template-read"),
            on,
            refreshed: false,
        };
    }

    fn start_builtin(&mut self, ctx: &AppCtx, b: Builtin) {
        let Some(snap) = ctx.snapshot else { return };
        let tpl = b.template(snap.model, &keys_of(ctx));
        self.start(ctx, tpl, Source::Builtin(b), Then::Preview);
    }

    /// Opens a template file (the import dialog's choice, or a test). A
    /// file that cannot be used is reported and nothing is read or written.
    pub fn import_path(&mut self, ctx: &AppCtx, path: &Path) {
        let name = path.file_name().map_or_else(
            || path.display().to_string(),
            |f| f.to_string_lossy().into_owned(),
        );
        match std::fs::read_to_string(path) {
            Ok(text) => match templates::parse(&text) {
                Ok(tpl) => self.start(ctx, tpl, Source::Imported(name), Then::Preview),
                Err(e) => {
                    self.flow = Flow::Failed {
                        title: t::CANNOT_USE,
                        detail: error_text(&e),
                        tech: Some(format!("{name}: {e:?}")),
                    }
                }
            },
            Err(e) => {
                self.flow = Flow::Failed {
                    title: t::CANNOT_USE,
                    detail: t::FILE_FAILED.to_string(),
                    tech: Some(e.to_string()),
                }
            }
        }
    }

    fn pick_import(&mut self, ctx: &AppCtx) {
        let Some(path) = rfd::FileDialog::new()
            .add_filter("OpenHE template", &[templates::EXTENSION])
            .pick_file()
        else {
            return;
        };
        self.import_path(ctx, &path);
    }

    fn restore_backup(&mut self, ctx: &AppCtx, path: &Path, then: Then, source: Source) {
        match store::read_backup(path) {
            Some(b) => {
                let name = match then {
                    Then::Apply => format!("{} {}", t::UNDO_PREFIX, b.reason),
                    Then::Preview => format!("{} ({})", t::RESTORE_PREFIX, b.reason),
                };
                let tpl = Template::from_current(&name, b.model, &b.keys, Parts::ALL);
                self.start(ctx, tpl, source, then);
            }
            None => {
                self.backups = None;
                self.flow = Flow::Failed {
                    title: t::FAILED,
                    detail: t::BACKUP_GONE.to_string(),
                    tech: Some(path.display().to_string()),
                }
            }
        }
    }

    /// Writes a planned template: backup first, then the plan.
    fn apply(
        &mut self,
        ctx: &AppCtx,
        tpl: &Template,
        current: &[KeyData],
        plan: &TemplatePlan,
        on: Stamp,
    ) {
        let Some(snap) = ctx.snapshot else { return };
        if plan.is_empty() {
            self.flow = Flow::Done {
                ok: true,
                text: t::NOTHING_TO_UNDO.to_string(),
                tech: None,
                backup: None,
                on,
            };
            return;
        }
        let Some(base) = self.base.clone() else {
            self.flow = Flow::Failed {
                title: t::FAILED,
                detail: format!("{} {}", t::BACKUP_FAILED, t::NO_DATA_DIR),
                tech: None,
            };
            return;
        };
        let file = store::BackupFile::new(
            &tpl.name,
            snap.profile,
            &snap.info.serial,
            snap.model,
            current,
            store::now_ms(),
        );
        let backup = match store::save_backup(&base, &file) {
            Ok(p) => p,
            Err(e) => {
                self.flow = Flow::Failed {
                    title: t::FAILED,
                    detail: t::BACKUP_FAILED.to_string(),
                    tech: Some(e.to_string()),
                };
                return;
            }
        };
        self.backups = None;
        let writes = plan.writes.clone();
        self.flow = Flow::Applying {
            name: tpl.name.clone(),
            pending: ctx
                .worker
                .submit("template-apply", move |kb| import_job(kb, &writes)),
            backup,
            on,
        };
    }

    /// "Terapkan" on the preview.
    fn confirm(&mut self, ctx: &AppCtx) {
        let Flow::Preview {
            tpl,
            source,
            current,
            plan,
            on,
            ..
        } = std::mem::replace(&mut self.flow, Flow::Idle)
        else {
            return;
        };
        match Stamp::now(ctx) {
            Some(now) if now == on => self.apply(ctx, &tpl, &current, &plan, on),
            Some(now) => {
                // The keyboard moved since the read: read and plan again,
                // and show the new preview instead of writing.
                self.flow = Flow::Reading {
                    tpl,
                    source,
                    then: Then::Preview,
                    pending: submit_read(ctx, "template-read"),
                    on: now,
                    refreshed: true,
                };
            }
            None => {}
        }
    }

    /// Runs every frame, whichever page is shown: finishes jobs and drops a
    /// preview made for another profile or connection.
    pub fn pump(&mut self, ctx: &AppCtx) {
        self.poll_save(ctx);
        // A profile switch or reconnect drops a preview (and a read for one).
        let stale = match &self.flow {
            Flow::Reading { on, .. } | Flow::Preview { on, .. } => !on.same_place(ctx),
            _ => false,
        };
        if stale {
            self.flow = Flow::Idle;
            self.dropped = true;
            return;
        }
        if let Flow::Done { on, .. } = &self.flow {
            if !on.same_place(ctx) {
                self.flow = Flow::Idle;
            }
        }
        match std::mem::replace(&mut self.flow, Flow::Idle) {
            Flow::Reading {
                tpl,
                source,
                then,
                mut pending,
                on,
                refreshed,
            } => match pending.poll() {
                None => {
                    self.flow = Flow::Reading {
                        tpl,
                        source,
                        then,
                        pending,
                        on,
                        refreshed,
                    }
                }
                Some(Err(e)) => {
                    self.flow = Flow::Failed {
                        title: t::FAILED,
                        detail: format!("{} {}", t::READ_FAILED, i18n::device_error(&e)),
                        tech: Some(i18n::technical(&e)),
                    }
                }
                Some(Ok(current)) => {
                    let Some(snap) = ctx.snapshot else { return };
                    // A reload while reading: the keys may be older than
                    // the snapshot; read again.
                    if Stamp::now(ctx) != Some(on) {
                        self.flow = Flow::Reading {
                            tpl,
                            source,
                            then,
                            pending: submit_read(ctx, "template-read"),
                            on: Stamp::now(ctx).unwrap_or(on),
                            refreshed,
                        };
                        return;
                    }
                    match plan_template(&tpl, snap, &current) {
                        Err(e) => {
                            self.flow = Flow::Failed {
                                title: t::CANNOT_USE,
                                detail: error_text(&e),
                                tech: Some(format!("{e:?}")),
                            }
                        }
                        Ok(plan) => match then {
                            Then::Apply => self.apply(ctx, &tpl, &current, &plan, on),
                            Then::Preview => {
                                let summary = summary(&plan, snap.matrix.keys().count());
                                self.flow = Flow::Preview {
                                    tpl,
                                    source,
                                    current,
                                    plan: Box::new(plan),
                                    summary,
                                    on,
                                    refreshed,
                                };
                            }
                        },
                    }
                }
            },
            Flow::Applying {
                name,
                mut pending,
                backup,
                on,
            } => match pending.poll() {
                None => {
                    self.flow = Flow::Applying {
                        name,
                        pending,
                        backup,
                        on,
                    }
                }
                Some(r) => {
                    // The keyboard is the truth: re-read it (after a
                    // disconnect the worker reloads on reconnect).
                    if r != Err(Error::Disconnected) {
                        ctx.worker.request_reload();
                    }
                    let (ok, text, tech) = match &r {
                        Ok(rep) if rep.failed.is_empty() => {
                            (true, format!("{} · {name}", t::DONE), None)
                        }
                        Ok(rep) => (
                            false,
                            t::PARTIAL.to_string(),
                            rep.failed.first().map(|(_, e)| i18n::technical(e)),
                        ),
                        Err(Error::Disconnected) => (false, t::DISCONNECTED.to_string(), None),
                        Err(e) => (false, i18n::device_error(e), Some(i18n::technical(e))),
                    };
                    self.backups = None;
                    self.flow = Flow::Done {
                        ok,
                        text,
                        tech,
                        backup: Some(backup),
                        on,
                    };
                }
            },
            other => self.flow = other,
        }
    }

    fn poll_save(&mut self, ctx: &AppCtx) {
        let Some(form) = self.save_form.as_mut() else {
            return;
        };
        let Some(r) = form.pending.as_mut().and_then(|p| p.poll()) else {
            return;
        };
        form.pending = None;
        let note = match (r, ctx.snapshot, self.base.as_deref()) {
            (Ok(keys), Some(snap), Some(base)) => {
                let tpl = Template::from_current(form.name.trim(), snap.model, &keys, form.parts);
                match store::save_template(base, &tpl) {
                    Ok(_) => (true, t::SAVED.to_string()),
                    Err(e) => (false, format!("{} {e}", t::SAVE_FAILED)),
                }
            }
            (Ok(_), _, None) => (false, format!("{} {}", t::SAVE_FAILED, t::NO_DATA_DIR)),
            (Ok(_), None, _) => (false, t::SAVE_FAILED.to_string()),
            (Err(e), _, _) => (
                false,
                format!("{} {}", t::SAVE_FAILED, i18n::device_error(&e)),
            ),
        };
        if note.0 {
            self.save_form = None;
            self.library = None;
        }
        self.lib_note = Some(note);
    }

    fn keep_imported(&mut self) {
        let Flow::Preview { tpl, source, .. } = &mut self.flow else {
            return;
        };
        let Some(base) = self.base.as_deref() else {
            return;
        };
        self.lib_note = Some(match store::save_template(base, tpl) {
            Ok(path) => {
                *source = Source::Saved(path);
                self.library = None;
                (true, t::SAVED.to_string())
            }
            Err(e) => (false, format!("{} {e}", t::SAVE_FAILED)),
        });
    }

    fn export(&mut self, saved: &Saved) {
        let name = format!(
            "{}.{}",
            store::slug(&saved.template.name),
            templates::EXTENSION
        );
        let Some(path) = rfd::FileDialog::new()
            .add_filter("OpenHE template", &[templates::EXTENSION])
            .set_file_name(name)
            .save_file()
        else {
            return;
        };
        let json = TemplateFile::from_template(&saved.template).to_json();
        self.lib_note = Some(match std::fs::write(&path, json) {
            Ok(()) => (
                true,
                format!(
                    "{} {}.",
                    t::EXPORTED,
                    path.file_name()
                        .map_or_else(String::new, |f| f.to_string_lossy().into_owned())
                ),
            ),
            Err(e) => (false, format!("{} {e}", t::FILE_FAILED)),
        });
    }

    /// Screenshot hook: a Gaming FPS preview on the snapshot's keys (as
    /// read now), without reading or writing the keyboard.
    #[cfg(feature = "preview")]
    pub fn preview_state(&mut self, ctx: &AppCtx) {
        let Some(snap) = ctx.snapshot else { return };
        let current = crate::templates::snapshot_keys(snap);
        let tpl = Builtin::GamingFps.template(snap.model, &keys_of(ctx));
        if let Ok(plan) = plan_template(&tpl, snap, &current) {
            let summary = summary(&plan, snap.matrix.keys().count());
            self.flow = Flow::Preview {
                tpl: Box::new(tpl),
                source: Source::Builtin(Builtin::GamingFps),
                current,
                plan: Box::new(plan),
                summary,
                on: Stamp::now(ctx).unwrap_or(Stamp {
                    profile: snap.profile,
                    connects: 0,
                    generation: 0,
                }),
                refreshed: false,
            };
        }
    }

    // ----------------------------------------------------------- UI

    pub fn ui(&mut self, ctx: &AppCtx, ui: &mut Ui) {
        page_with_right_panel(
            ui,
            "templates-right",
            self,
            |page, ui| {
                egui::ScrollArea::vertical()
                    .id_salt("templates-center")
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        egui::Frame::new()
                            .inner_margin(egui::Margin {
                                right: 10,
                                ..Default::default()
                            })
                            .show(ui, |ui| {
                                page.keyboard_card(ctx, ui);
                                ui.add_space(20.0);
                                page.builtins(ctx, ui);
                                ui.add_space(20.0);
                                page.mine(ctx, ui);
                            });
                    });
            },
            |page, ui| {
                egui::ScrollArea::vertical()
                    .id_salt("templates-right-scroll")
                    .auto_shrink([false, false])
                    .show(ui, |ui| page.right(ctx, ui));
            },
        );
    }

    fn keyboard_card(&mut self, ctx: &AppCtx, ui: &mut Ui) {
        let (model, matrix) = match ctx.snapshot {
            Some(s) => (s.model, s.matrix.clone()),
            None => (Model::Win68HeMax, reference_matrix(Model::Win68HeMax)),
        };
        let preview = self.preview_keys();
        let labels = self.cap_labels();
        Card::new().show(ui, |ui| {
            ui.set_width(ui.available_width());
            card::title(ui, t::KEYBOARD);
            ui.add_space(10.0);
            ui.vertical_centered(|ui| {
                KeyboardView::new(model, &matrix)
                    .skin(ctx.skin())
                    .max_unit(52.0)
                    .picture()
                    .id_salt("templates-keyboard")
                    .show(ui, &mut self.selection, |key| match &preview {
                        Some((changed, _)) if changed.contains(&key) => KeyDecoration {
                            label2: labels
                                .iter()
                                .find(|(k, _)| *k == key)
                                .map(|(_, l)| l.clone().into()),
                            fill: Some(color::ACCENT_SOFT),
                            stroke: Some(color::ACCENT),
                            badge: None,
                        },
                        Some((_, skipped)) if skipped.contains(&key) => KeyDecoration {
                            stroke: Some(color::DANGER),
                            ..Default::default()
                        },
                        _ => KeyDecoration::default(),
                    });
            });
            ui.add_space(8.0);
            match &preview {
                Some((_, skipped)) => {
                    ui.horizontal_wrapped(|ui| {
                        legend_chip(ui, color::ACCENT, t::LEGEND_CHANGED);
                        if !skipped.is_empty() {
                            ui.add_space(12.0);
                            legend_chip(ui, color::DANGER, t::LEGEND_SKIPPED);
                        }
                    });
                }
                None => card::hint(ui, t::KEYBOARD_IDLE),
            }
        });
    }

    /// The second line on each changed key cap: its new actuation, layer
    /// code or "SOCD".
    fn cap_labels(&self) -> Vec<(u8, String)> {
        let Flow::Preview { plan, .. } = &self.flow else {
            return Vec::new();
        };
        let mut out: Vec<(u8, String)> = Vec::new();
        for (keys, p) in &plan.perf {
            if let Some(a) = p.actuation {
                out.extend(keys.iter().map(|&k| (k, mm_short(a.0))));
            }
        }
        for &(k, l, c) in &plan.layers {
            if !out.iter().any(|(x, _)| *x == k) {
                out.push((k, crate::pages::remap::layer_short_code_name(l, c)));
            }
        }
        for &(k, cfg) in &plan.functions {
            if !out.iter().any(|(x, _)| *x == k) {
                let s = match cfg {
                    AdvancedConfig::Socd(_) => "SOCD".to_string(),
                    AdvancedConfig::None => "—".to_string(),
                    _ => "Fn+".to_string(),
                };
                out.push((k, s));
            }
        }
        out
    }

    fn selected_builtin(&self) -> Option<Builtin> {
        match &self.flow {
            Flow::Reading {
                source: Source::Builtin(b),
                ..
            }
            | Flow::Preview {
                source: Source::Builtin(b),
                ..
            } => Some(*b),
            _ => None,
        }
    }

    fn builtins(&mut self, ctx: &AppCtx, ui: &mut Ui) {
        card::title(ui, t::BUILTIN);
        card::hint(ui, t::BUILTIN_HINT);
        ui.add_space(10.0);
        let enabled = self.can_start(ctx);
        let selected = self.selected_builtin();
        let gap = 12.0;
        let cols = if ui.available_width() >= 600.0 { 3 } else { 2 };
        let w = ((ui.available_width() - gap * (cols as f32 - 1.0)) / cols as f32).floor();
        let mut clicked = None;
        for row in Builtin::ALL.chunks(cols) {
            let h = row
                .iter()
                .map(|b| tile_height(ui, b.name(), b.blurb(), w))
                .fold(0.0, f32::max);
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = gap;
                for &b in row {
                    if tile(
                        ui,
                        b.name(),
                        b.blurb(),
                        (w, h),
                        selected == Some(b),
                        enabled,
                    ) {
                        clicked = Some(b);
                    }
                }
            });
            ui.add_space(gap);
        }
        if let Some(b) = clicked {
            self.start_builtin(ctx, b);
        }
    }

    fn mine(&mut self, ctx: &AppCtx, ui: &mut Ui) {
        let enabled = self.can_start(ctx);
        card::title(ui, t::MINE);
        ui.add_space(10.0);
        Card::new().show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal_wrapped(|ui| {
                if ui
                    .add_enabled(
                        enabled && self.save_form.is_none(),
                        secondary(t::SAVE_CURRENT),
                    )
                    .clicked()
                {
                    self.save_form = Some(SaveForm {
                        name: String::new(),
                        parts: Parts::ALL,
                        pending: None,
                    });
                    self.lib_note = None;
                }
                if ui.add_enabled(enabled, secondary(t::IMPORT)).clicked() {
                    self.pick_import(ctx);
                }
            });
            if self.save_form.is_some() {
                ui.add_space(12.0);
                self.save_form_ui(ctx, ui);
            }
            if let Some((ok, text)) = &self.lib_note {
                ui.add_space(8.0);
                let c = if *ok { color::SUCCESS } else { color::DANGER };
                ui.add(egui::Label::new(RichText::new(text).font(font::small()).color(c)).wrap());
            }
            ui.add_space(12.0);
            let list = self.library().clone();
            if list.is_empty() {
                card::hint(ui, t::MINE_EMPTY);
            }
            let mut start = None;
            let mut export = None;
            let mut delete = None;
            let selected = match &self.flow {
                Flow::Reading {
                    source: Source::Saved(p),
                    ..
                }
                | Flow::Preview {
                    source: Source::Saved(p),
                    ..
                } => Some(p.clone()),
                _ => None,
            };
            for (i, s) in list.iter().enumerate() {
                if i > 0 {
                    ui.add_space(6.0);
                }
                let wrong_model = ctx
                    .snapshot
                    .is_some_and(|snap| snap.model != s.template.model);
                let on = selected.as_ref() == Some(&s.path);
                let row = egui::Frame::new()
                    .fill(if on { color::ACCENT_SOFT } else { color::BG })
                    .stroke(line(1.0, if on { color::ACCENT } else { color::BORDER }))
                    .corner_radius(CornerRadius::same(size::CONTROL_RADIUS))
                    .inner_margin(egui::Margin::symmetric(12, 10))
                    .show(ui, |ui| {
                        ui.set_width(ui.available_width());
                        ui.horizontal(|ui| {
                            ui.vertical(|ui| {
                                ui.set_max_width((ui.available_width() - 190.0).max(120.0));
                                ui.add(
                                    egui::Label::new(
                                        RichText::new(&s.template.name).color(color::TEXT).strong(),
                                    )
                                    .wrap(),
                                );
                                ui.add(
                                    egui::Label::new(
                                        RichText::new(parts_text(&s.template))
                                            .font(font::small())
                                            .color(color::MUTED),
                                    )
                                    .wrap(),
                                );
                            });
                            ui.with_layout(
                                egui::Layout::right_to_left(egui::Align::Center),
                                |ui| {
                                    if self.confirm_delete.as_ref() == Some(&s.path) {
                                        if ui.button(t::CANCEL).clicked() {
                                            self.confirm_delete = None;
                                        }
                                        if ui
                                            .button(
                                                RichText::new(t::YES_DELETE).color(color::DANGER),
                                            )
                                            .clicked()
                                        {
                                            delete = Some(s.path.clone());
                                        }
                                        return;
                                    }
                                    if ui.button(t::DELETE).clicked() {
                                        self.confirm_delete = Some(s.path.clone());
                                    }
                                    if ui.button(t::EXPORT).clicked() {
                                        export = Some(s.clone());
                                    }
                                    if ui
                                        .add_enabled(
                                            enabled && !wrong_model,
                                            egui::Button::new(t::PREVIEW),
                                        )
                                        .clicked()
                                    {
                                        start = Some(s.clone());
                                    }
                                },
                            );
                        });
                    });
                let _ = row;
            }
            if let Some(s) = start {
                self.start(
                    ctx,
                    s.template.clone(),
                    Source::Saved(s.path.clone()),
                    Then::Preview,
                );
            }
            if let Some(s) = export {
                self.export(&s);
            }
            if let Some(p) = delete {
                self.confirm_delete = None;
                self.lib_note = Some(match store::delete_template(&p) {
                    Ok(()) => (true, t::DELETED.to_string()),
                    Err(e) => (false, format!("{} {e}", t::FILE_FAILED)),
                });
                self.library = None;
            }
        });
    }

    fn save_form_ui(&mut self, ctx: &AppCtx, ui: &mut Ui) {
        let enabled = ctx.can_write();
        let Some(form) = self.save_form.as_mut() else {
            return;
        };
        let busy = form.pending.is_some();
        ui.label(RichText::new(t::NAME).color(color::MUTED));
        ui.add_enabled(
            !busy,
            egui::TextEdit::singleline(&mut form.name)
                .hint_text(t::NAME_HINT)
                .desired_width(280.0),
        );
        ui.add_space(8.0);
        ui.label(RichText::new(t::INCLUDE).color(color::MUTED));
        ui.horizontal_wrapped(|ui| {
            ui.add_enabled(
                !busy,
                egui::Checkbox::new(&mut form.parts.performance, t::PART_PERF),
            );
            ui.add_enabled(
                !busy,
                egui::Checkbox::new(&mut form.parts.remap, t::PART_REMAP),
            );
            ui.add_enabled(
                !busy,
                egui::Checkbox::new(&mut form.parts.advanced, t::PART_ADV),
            );
        });
        ui.add_space(8.0);
        let mut save = false;
        let mut cancel = false;
        ui.horizontal(|ui| {
            let ok = enabled && !busy && !form.name.trim().is_empty() && form.parts.any();
            save = ui.add_enabled(ok, egui::Button::new(t::SAVE)).clicked();
            cancel = ui
                .add_enabled(!busy, egui::Button::new(t::CANCEL))
                .clicked();
            if busy {
                ui.spinner();
                ui.label(
                    RichText::new(t::SAVING)
                        .font(font::small())
                        .color(color::MUTED),
                );
            }
        });
        if save {
            form.pending = Some(submit_read(ctx, "template-save"));
        }
        if cancel {
            self.save_form = None;
        }
    }

    fn right(&mut self, ctx: &AppCtx, ui: &mut Ui) {
        ui.set_width(ui.available_width());
        let can = self.can_start(ctx);
        let mut action = None;
        match &self.flow {
            Flow::Idle => {
                card::title(ui, t::TITLE);
                ui.add_space(6.0);
                card::hint(ui, t::INTRO);
                if self.dropped {
                    ui.add_space(10.0);
                    note_box(ui, t::DROPPED_PROFILE);
                }
            }
            Flow::Reading { tpl, .. } => {
                card::title(ui, &tpl.name);
                ui.add_space(10.0);
                ui.horizontal(|ui| {
                    ui.spinner();
                    ui.label(RichText::new(t::READING).color(color::MUTED));
                });
            }
            Flow::Preview {
                tpl,
                source,
                summary,
                plan,
                on,
                refreshed,
                ..
            } => {
                ui.label(
                    RichText::new(format!(
                        "{} · {} {}",
                        t::PREVIEW,
                        t::ON_PROFILE,
                        on.profile + 1
                    ))
                    .font(font::small())
                    .color(color::MUTED),
                );
                ui.add(
                    egui::Label::new(
                        RichText::new(&tpl.name)
                            .font(font::heading())
                            .color(color::TEXT)
                            .strong(),
                    )
                    .wrap(),
                );
                ui.add_space(10.0);
                if *refreshed {
                    note_box(ui, t::REFRESHED);
                    ui.add_space(10.0);
                }
                Card::new().padding(14).show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.add(
                        egui::Label::new(
                            RichText::new(&summary.headline)
                                .font(font::bold(15.0))
                                .color(if plan.is_empty() {
                                    color::MUTED
                                } else {
                                    color::ACCENT
                                }),
                        )
                        .wrap(),
                    );
                    for l in &summary.lines {
                        ui.add_space(6.0);
                        ui.add(
                            egui::Label::new(RichText::new(format!("• {l}")).color(color::TEXT))
                                .wrap(),
                        );
                    }
                    for n in &summary.notes {
                        ui.add_space(6.0);
                        ui.add(
                            egui::Label::new(
                                RichText::new(n).font(font::small()).color(color::MUTED),
                            )
                            .wrap(),
                        );
                    }
                });
                ui.add_space(10.0);
                if !plan.is_empty() {
                    card::hint(ui, t::BACKUP_FIRST);
                    ui.add_space(10.0);
                }
                ui.horizontal(|ui| {
                    let cancel = egui::Button::new(RichText::new(t::CANCEL).color(color::TEXT))
                        .fill(color::BG)
                        .stroke(line(1.0, color::BORDER_STRONG))
                        .corner_radius(CornerRadius::same(size::CONTROL_RADIUS + 2))
                        .min_size(egui::vec2(90.0, 46.0));
                    if ui.add(cancel).clicked() {
                        action = Some(RightAction::Cancel);
                    }
                    if card::primary_button(ui, t::APPLY, can && !plan.is_empty()).clicked() {
                        action = Some(RightAction::Apply);
                    }
                });
                if matches!(source, Source::Imported(_)) {
                    ui.add_space(6.0);
                    if ui.add(secondary(t::KEEP_IMPORTED)).clicked() {
                        action = Some(RightAction::KeepImported);
                    }
                }
            }
            Flow::Applying { name, .. } => {
                card::title(ui, name);
                ui.add_space(10.0);
                ui.horizontal(|ui| {
                    ui.spinner();
                    ui.label(RichText::new(t::APPLYING).color(color::MUTED));
                });
            }
            Flow::Done {
                ok,
                text,
                tech,
                backup,
                ..
            } => {
                card::title(ui, t::TITLE);
                ui.add_space(10.0);
                let resp = Card::new()
                    .fill(if *ok {
                        color::PANEL
                    } else {
                        color::DANGER_SOFT
                    })
                    .stroke(if *ok {
                        color::SUCCESS.gamma_multiply(0.6)
                    } else {
                        color::DANGER.gamma_multiply(0.6)
                    })
                    .padding(14)
                    .show(ui, |ui| {
                        ui.set_width(ui.available_width());
                        let c = if *ok { color::SUCCESS } else { color::DANGER };
                        ui.add(
                            egui::Label::new(RichText::new(format!("✔ {text}")).color(c).strong())
                                .wrap(),
                        );
                        if *ok && backup.is_some() {
                            ui.add(
                                egui::Label::new(
                                    RichText::new(t::DONE_BODY)
                                        .font(font::small())
                                        .color(color::MUTED),
                                )
                                .wrap(),
                            );
                        }
                    })
                    .response;
                if let Some(tech) = tech {
                    resp.on_hover_text(format!("{}: {tech}", i18n::common::TECH_DETAIL));
                }
                if backup.is_some() {
                    ui.add_space(10.0);
                    if ui.add_enabled(can, secondary(t::UNDO)).clicked() {
                        action = Some(RightAction::Undo);
                    }
                }
            }
            Flow::Failed {
                title,
                detail,
                tech,
            } => {
                card::title(ui, t::TITLE);
                ui.add_space(10.0);
                card::error_box(ui, title, detail, None, tech.as_deref());
            }
        }
        match action {
            Some(RightAction::Cancel) => self.flow = Flow::Idle,
            Some(RightAction::Apply) => self.confirm(ctx),
            Some(RightAction::KeepImported) => self.keep_imported(),
            Some(RightAction::Undo) => {
                if let Flow::Done {
                    backup: Some(p), ..
                } = &self.flow
                {
                    let p = p.clone();
                    self.restore_backup(ctx, &p, Then::Apply, Source::Undo);
                }
            }
            None => {}
        }
        ui.add_space(24.0);
        self.backups_ui(ctx, ui, can);
    }

    fn backups_ui(&mut self, ctx: &AppCtx, ui: &mut Ui, can: bool) {
        ui.label(
            RichText::new(t::BACKUPS)
                .font(font::bold(15.0))
                .color(color::TEXT),
        );
        ui.add_space(8.0);
        let list = self.backups(ctx);
        if list.is_empty() {
            card::hint(ui, t::BACKUPS_EMPTY);
            return;
        }
        let now = store::now_ms();
        let mut restore = None;
        for (i, b) in list.iter().enumerate() {
            if i > 0 {
                ui.add_space(6.0);
            }
            egui::Frame::new()
                .fill(color::PANEL)
                .stroke(line(1.0, color::BORDER))
                .corner_radius(CornerRadius::same(size::CONTROL_RADIUS))
                .inner_margin(egui::Margin::symmetric(10, 8))
                .show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.horizontal(|ui| {
                        ui.vertical(|ui| {
                            ui.set_max_width(ui.available_width() - 84.0);
                            ui.add(
                                egui::Label::new(
                                    RichText::new(format!("{} {}", t::BEFORE, b.reason))
                                        .color(color::TEXT),
                                )
                                .wrap(),
                            );
                            ui.label(
                                RichText::new(format!(
                                    "{} · {} {}",
                                    store::age_text(b.created_ms, now),
                                    t::PROFILE,
                                    b.profile + 1
                                ))
                                .font(font::small())
                                .color(color::MUTED),
                            );
                        });
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if ui.add_enabled(can, egui::Button::new(t::RESTORE)).clicked() {
                                restore = Some(b.path.clone());
                            }
                        });
                    });
                });
        }
        if let Some(p) = restore {
            self.restore_backup(ctx, &p, Then::Preview, Source::Backup(p.clone()));
        }
    }
}

enum RightAction {
    Cancel,
    Apply,
    KeepImported,
    Undo,
}

/// "Performa · Remap · WIN 68 HE MAX".
fn parts_text(t: &Template) -> String {
    let p = t.parts();
    let mut parts = Vec::new();
    if p.performance {
        parts.push(strings::PART_PERF);
    }
    if p.remap {
        parts.push(strings::PART_REMAP);
    }
    if p.advanced {
        parts.push(strings::PART_ADV);
    }
    parts.push(t.model.name());
    parts.join(" · ")
}

fn secondary(text: &str) -> egui::Button<'_> {
    egui::Button::new(RichText::new(text).color(color::TEXT))
        .fill(color::BG)
        .stroke(line(1.0, color::BORDER_STRONG))
        .corner_radius(CornerRadius::same(size::CONTROL_RADIUS))
        .min_size(egui::vec2(0.0, 32.0))
}

fn note_box(ui: &mut Ui, text: &str) {
    Card::new()
        .fill(color::ACCENT_SOFT)
        .stroke(color::ACCENT.gamma_multiply(0.5))
        .padding(10)
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.add(egui::Label::new(RichText::new(text).color(color::TEXT)).wrap());
        });
}

fn legend_chip(ui: &mut Ui, c: egui::Color32, text: &str) {
    let (rect, _) = ui.allocate_exact_size(egui::vec2(14.0, 14.0), Sense::hover());
    ui.painter().rect_stroke(
        rect.shrink(1.0),
        CornerRadius::same(3),
        line(2.0, c),
        egui::StrokeKind::Inside,
    );
    ui.label(RichText::new(text).font(font::small()).color(color::MUTED));
}

const TILE_PAD: f32 = 14.0;
const TILE_MIN_H: f32 = 84.0;

fn tile_galleys(
    ui: &Ui,
    name: &str,
    blurb: &str,
    w: f32,
    colors: (egui::Color32, egui::Color32),
) -> (std::sync::Arc<egui::Galley>, std::sync::Arc<egui::Galley>) {
    let inner = (w - 2.0 * TILE_PAD).max(40.0);
    let p = ui.painter();
    (
        p.layout(name.to_string(), font::bold(15.0), colors.0, inner),
        p.layout(blurb.to_string(), font::small(), colors.1, inner),
    )
}

fn tile_height(ui: &Ui, name: &str, blurb: &str, w: f32) -> f32 {
    let (n, b) = tile_galleys(ui, name, blurb, w, (color::TEXT, color::MUTED));
    (2.0 * TILE_PAD + n.size().y + 6.0 + b.size().y).max(TILE_MIN_H)
}

/// A preset card: name and one line of what it does; one button for
/// accessibility (labelled with the name). True when clicked.
fn tile(
    ui: &mut Ui,
    name: &str,
    blurb: &str,
    (w, h): (f32, f32),
    selected: bool,
    enabled: bool,
) -> bool {
    let sense = if enabled {
        Sense::click()
    } else {
        Sense::hover()
    };
    let (rect, resp) = ui.allocate_exact_size(egui::vec2(w, h), sense);
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, enabled, name));
    let hovered = enabled && resp.hovered();
    let (fill, stroke) = if selected {
        (color::ACCENT_SOFT, color::ACCENT)
    } else if hovered {
        (color::PANEL, color::BORDER_STRONG)
    } else {
        (color::PANEL, color::BORDER)
    };
    let dim = |c: egui::Color32| if enabled { c } else { c.gamma_multiply(0.6) };
    let name_c = if selected { color::ACCENT } else { color::TEXT };
    let (n, b) = tile_galleys(ui, name, blurb, w, (dim(name_c), dim(color::MUTED)));
    let p = ui.painter();
    p.rect(
        rect,
        CornerRadius::same(size::CARD_RADIUS),
        fill,
        line(if selected { 1.5 } else { 1.0 }, stroke),
        egui::StrokeKind::Inside,
    );
    let top = rect.min + egui::vec2(TILE_PAD, TILE_PAD);
    let name_h = n.size().y;
    p.galley(top, n, color::TEXT);
    p.galley(top + egui::vec2(0.0, name_h + 6.0), b, color::MUTED);
    let resp = if enabled {
        resp.on_hover_cursor(egui::CursorIcon::PointingHand)
    } else {
        resp
    };
    resp.clicked()
}
