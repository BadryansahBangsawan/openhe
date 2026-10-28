//! From a [`Template`] and the keyboard's current state to the writes.
//!
//! [`plan_template`] is pure. It compares every value the template names
//! with what the keyboard has now (`current`, read just before: the same
//! read becomes the backup) and plans only real changes, as an
//! [`ImportPlan`] that the profile import's `import_job` runs through the
//! verified device API. The profile import's safety rules hold:
//!
//! - the Fn key's own Base code is never written, and 0x0001 is never put
//!   on Base;
//! - a Win/Alt code on LAlt / LWin's Base layer follows the keyboard's
//!   Win/Mac swap (PROTOCOL 7.x finding 16): left alone when the key already
//!   sends Win or Alt, otherwise the code that key has in the current OS
//!   mode;
//! - advanced functions go through the import's checks (packet validation,
//!   SOCD positions, no function on Fn); an SOCD pair is written once, and
//!   the device clears an old partner (shown in the preview);
//! - a key with a macro keeps it (macro contents cannot be read back, so a
//!   backup could not restore it);
//! - touch mode Global cannot be written together with actuation or RT, so
//!   a template asking for both writes the values first and Global after.
//!
//! A value beyond a key's switch travel skips that key (the preview says
//! so); a value no key could take (actuation below 0.1 mm, RT below
//! 0.005 mm, deadzone above 1 mm) refuses the template.

use super::{KeyPerf, Template, TemplateError, TRANSPARENT};
use crate::pages::settings::profile_file::{check_advanced, ImportPlan, KeyData};
use crate::widgets::keyboard::legend_list;
use crate::widgets::slider_mm::mm_short;
use crate::worker::DeviceSnapshot;
use openhe_device::keyboard::performance::{MAX_DEADZONE_UM, MIN_RT_UM};
use openhe_device::{AdvancedConfig, KeySettings, PerformanceChange, FN_KEY};
use openhe_proto::model::MIN_ACTUATION_UM;
use openhe_proto::{Layer, Micrometers, SystemMode, TouchMode};
use std::collections::{BTreeMap, BTreeSet};

mod strings {
    pub const CHANGES: &str = "Mengubah";
    pub const KEYS: &str = "tombol";
    pub const NOTHING: &str = "Tidak ada yang berubah: keyboard sudah sesuai template ini.";
    pub const ALL_KEYS: &str = "Semua tombol";
    pub const GLOBAL: &str = "mode Global";
    pub const NO_RT: &str = "tanpa RT";
    pub const RT: &str = "RT";
    pub const DEADZONE: &str = "deadzone";
    pub const KEYS_CHANGED: &str = "tombol diubah";
    pub const SOCD_LAST: &str = "input terakhir menang";
    pub const FN_CLEARED: &str = "Fungsi lanjutan dihapus:";
    pub const FN_SET: &str = "Fungsi lanjutan diatur:";
    pub const SKIP_MISSING: &str = "Dilewati, tidak ada di keyboard ini:";
    pub const SKIP_TRAVEL: &str = "Dilewati, melebihi jarak tekan switch-nya:";
    pub const SKIP_MACRO: &str = "Tetap memakai macro (tidak diubah):";
    pub const SKIP_UNREAD: &str = "Dilewati, belum terbaca dari keyboard:";
    pub const SKIP_OS: &str = "Dilewati, mode Win/Mac belum diketahui:";
    pub const SKIP_FN: &str = "Tombol Fn di layer Base tidak pernah diubah.";
}
use strings as t;

/// Why a key (or one part of it) is left out.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Skip {
    /// This keyboard does not have the key (e.g. a WIN 68 key on a WIN 60).
    NotOnKeyboard,
    /// A value is beyond the key's switch travel.
    BeyondTravel,
    /// The key has a macro; it is kept.
    MacroKept,
    /// The key's current value was not read.
    NotRead,
    /// LAlt / LWin on Base while the Win/Mac mode is unknown.
    OsModeUnknown,
    /// The Fn key's own Base code.
    FnKey,
}

/// The writes for one template on this keyboard, and what they change.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TemplatePlan {
    pub writes: ImportPlan,
    /// Keys whose performance changes, grouped by the values they get.
    pub perf: Vec<(Vec<u8>, KeyPerf)>,
    /// Layer codes written.
    pub layers: Vec<(u8, Layer, u16)>,
    /// Keys whose function changes and what they get (`None`: cleared,
    /// also an old SOCD partner the device clears).
    pub functions: Vec<(u8, AdvancedConfig)>,
    pub skipped: Vec<(u8, Skip)>,
    pub notes: Vec<&'static str>,
}

impl TemplatePlan {
    /// Every key that changes.
    pub fn changed_keys(&self) -> BTreeSet<u8> {
        self.perf
            .iter()
            .flat_map(|(k, _)| k.iter().copied())
            .chain(self.layers.iter().map(|(k, _, _)| *k))
            .chain(self.functions.iter().map(|(k, _)| *k))
            .collect()
    }

    /// Keys left out.
    pub fn skipped_keys(&self) -> BTreeSet<u8> {
        self.skipped.iter().map(|(k, _)| *k).collect()
    }

    /// Nothing would be written.
    pub fn is_empty(&self) -> bool {
        self.writes.performance.is_empty()
            && self.writes.clear_advanced.is_empty()
            && self.writes.advanced.is_empty()
            && self.writes.layers.is_empty()
    }

    fn skip(&mut self, key: u8, why: Skip) {
        if !self.skipped.contains(&(key, why)) {
            self.skipped.push((key, why));
        }
    }
}

fn bad(msg: String) -> TemplateError {
    TemplateError::BadValue(msg)
}

/// Values no key could take refuse the template.
fn check_values(key: u8, p: &KeyPerf) -> Result<(), TemplateError> {
    if let Some(a) = p.actuation {
        if a.0 < MIN_ACTUATION_UM {
            return Err(bad(format!("0x{key:02X}: actuation {} um", a.0)));
        }
    }
    for v in [p.rt_press, p.rt_release].into_iter().flatten() {
        if v.0 < MIN_RT_UM {
            return Err(bad(format!("0x{key:02X}: RT {} um", v.0)));
        }
    }
    for v in [p.top_dz, p.bottom_dz].into_iter().flatten() {
        if v.0 > MAX_DEADZONE_UM {
            return Err(bad(format!("0x{key:02X}: deadzone {} um", v.0)));
        }
    }
    Ok(())
}

/// `want` where it differs from `now`.
fn differ(want: Option<Micrometers>, now: Micrometers) -> Option<Micrometers> {
    want.filter(|&v| v != now)
}

/// The performance writes for one key: one change, or (Global together
/// with travel values) the values first, then Global.
fn perf_changes(p: &KeyPerf, cur: &KeySettings) -> (PerformanceChange, PerformanceChange) {
    let values = PerformanceChange {
        actuation: differ(p.actuation, cur.actuation),
        rt_press: differ(p.rt_press, cur.rt_press),
        rt_release: differ(p.rt_release, cur.rt_release),
        top_dz: differ(p.top_dz, cur.top_dz),
        bottom_dz: differ(p.bottom_dz, cur.bottom_dz),
        ..Default::default()
    };
    let travel =
        values.actuation.is_some() || values.rt_press.is_some() || values.rt_release.is_some();
    match p.touch {
        Some(TouchMode::Global) if travel => (
            values,
            PerformanceChange {
                touch: Some(TouchMode::Global),
                ..Default::default()
            },
        ),
        touch => (
            PerformanceChange {
                touch: touch.filter(|&t| t != cur.mode.touch),
                ..values
            },
            PerformanceChange::default(),
        ),
    }
}

fn batch(list: &mut Vec<(Vec<u8>, PerformanceChange)>, key: u8, change: PerformanceChange) {
    if change == PerformanceChange::default() {
        return;
    }
    match list.iter_mut().find(|(_, c)| *c == change) {
        Some((keys, _)) => keys.push(key),
        None => list.push((vec![key], change)),
    }
}

/// Plans `tpl` on the keyboard in `snap`, whose keys read `current` just
/// now. See the module docs for the rules.
pub fn plan_template(
    tpl: &Template,
    snap: &DeviceSnapshot,
    current: &[KeyData],
) -> Result<TemplatePlan, TemplateError> {
    if tpl.model != snap.model {
        return Err(TemplateError::WrongModel {
            file: tpl.model.name().to_string(),
            keyboard: snap.model.name().to_string(),
        });
    }
    let present: BTreeSet<u8> = snap.matrix.keys().map(|(_, _, c)| c).collect();
    let now: BTreeMap<u8, &KeyData> = current.iter().map(|k| (k.code, k)).collect();
    let mut plan = TemplatePlan {
        notes: tpl.notes.clone(),
        ..Default::default()
    };

    // ---- performance
    let mut first = Vec::new();
    let mut then = Vec::new();
    for &(key, p) in &tpl.performance {
        check_values(key, &p)?;
        if !present.contains(&key) {
            plan.skip(key, Skip::NotOnKeyboard);
            continue;
        }
        let Some(cur) = now
            .get(&key)
            .map(|k| k.settings)
            .or(snap.keys.get(&key).copied())
        else {
            plan.skip(key, Skip::NotRead);
            continue;
        };
        let Some(sw) = snap.model.switches().iter().find(|s| s.id == cur.switch) else {
            plan.skip(key, Skip::NotRead);
            continue;
        };
        let max = sw.max_travel;
        if [p.actuation, p.rt_press, p.rt_release]
            .into_iter()
            .flatten()
            .any(|v| v > max)
        {
            plan.skip(key, Skip::BeyondTravel);
            continue;
        }
        let (a, b) = perf_changes(&p, &cur);
        if a == PerformanceChange::default() && b == PerformanceChange::default() {
            continue;
        }
        batch(&mut first, key, a);
        batch(&mut then, key, b);
        match plan.perf.iter_mut().find(|(_, v)| *v == p) {
            Some((keys, _)) => keys.push(key),
            None => plan.perf.push((vec![key], p)),
        }
    }
    plan.writes.performance = first.into_iter().chain(then).collect();

    // ---- layers
    for &(key, layer, code) in &tpl.layers {
        if layer == Layer::Base && code == TRANSPARENT {
            return Err(TemplateError::TransparentOnBase(key));
        }
        if !present.contains(&key) {
            plan.skip(key, Skip::NotOnKeyboard);
            continue;
        }
        if key == FN_KEY && layer == Layer::Base {
            plan.skip(key, Skip::FnKey);
            continue;
        }
        let Some(cur) = now.get(&key).map(|k| k.layers[layer as usize]) else {
            plan.skip(key, Skip::NotRead);
            continue;
        };
        let win_alt = |c: u16| matches!(c, 0x00E2 | 0x00E3);
        let code = if layer == Layer::Base && win_alt(u16::from(key)) && win_alt(code) {
            if win_alt(cur) {
                continue;
            }
            match snap.system_mode {
                Some(SystemMode::Windows) => u16::from(key),
                Some(SystemMode::Mac) => u16::from(key) ^ 0x0001,
                None => {
                    plan.skip(key, Skip::OsModeUnknown);
                    continue;
                }
            }
        } else {
            code
        };
        if cur != code {
            plan.layers.push((key, layer, code));
        }
    }
    plan.writes.layers = plan.layers.clone();

    // ---- advanced functions
    let mut pairs_done: Vec<(u8, u8)> = Vec::new();
    let named: BTreeSet<u8> = tpl.advanced.iter().map(|(k, _)| *k).collect();
    for &(key, cfg) in &tpl.advanced {
        if !present.contains(&key) {
            plan.skip(key, Skip::NotOnKeyboard);
            continue;
        }
        check_advanced(key, &cfg, &present).map_err(|e| bad(format!("{e:?}")))?;
        let Some(cur) = now.get(&key).map(|k| k.advanced) else {
            plan.skip(key, Skip::NotRead);
            continue;
        };
        if cur == AdvancedConfig::Macro {
            plan.skip(key, Skip::MacroKept);
            continue;
        }
        let mut touched = vec![key];
        match cfg {
            AdvancedConfig::Socd(s) => {
                let pair = (s.pos1.min(s.pos2), s.pos1.max(s.pos2));
                if pairs_done.contains(&pair) {
                    continue;
                }
                pairs_done.push(pair);
                let partner = if s.pos1 == key { s.pos2 } else { s.pos1 };
                let partner_now = now.get(&partner).map(|k| k.advanced);
                if partner_now == Some(AdvancedConfig::Macro) {
                    plan.skip(key, Skip::MacroKept);
                    plan.skip(partner, Skip::MacroKept);
                    continue;
                }
                if cur == cfg && partner_now == Some(cfg) {
                    continue;
                }
                plan.writes.advanced.push((key, cfg));
                plan.functions.push((key, cfg));
                plan.functions.push((partner, cfg));
                touched.push(partner);
                if let Some(AdvancedConfig::Socd(old)) = partner_now {
                    let other = if old.pos1 == partner {
                        old.pos2
                    } else {
                        old.pos1
                    };
                    if other != key && !named.contains(&other) {
                        plan.functions.push((other, AdvancedConfig::None));
                    }
                }
            }
            AdvancedConfig::None => {
                if cur == AdvancedConfig::None {
                    continue;
                }
                plan.writes.clear_advanced.push(key);
                plan.functions.push((key, cfg));
            }
            _ => {
                if cur == cfg {
                    continue;
                }
                plan.writes.advanced.push((key, cfg));
                plan.functions.push((key, cfg));
            }
        }
        // A key leaving its SOCD pair takes its old partner with it.
        if let AdvancedConfig::Socd(old) = cur {
            let other = if old.pos1 == key { old.pos2 } else { old.pos1 };
            if !touched.contains(&other) && !named.contains(&other) {
                plan.functions.push((other, AdvancedConfig::None));
            }
        }
    }
    let mut seen = BTreeSet::new();
    plan.functions.retain(|(k, _)| seen.insert(*k));
    Ok(plan)
}

// --------------------------------------------------------------- summary

/// The preview text of a plan.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Summary {
    /// "Mengubah 4 tombol", or that nothing changes.
    pub headline: String,
    /// One line per kind of change.
    pub lines: Vec<String>,
    /// Keys left out, and the template's notes.
    pub notes: Vec<String>,
}

fn mm_pair(a: Micrometers, b: Micrometers) -> String {
    if a == b {
        format!("{} mm", mm_short(a.0))
    } else {
        format!("{}/{} mm", mm_short(a.0), mm_short(b.0))
    }
}

/// "0.40 mm, RT 0.15 mm" for a key's target values.
pub fn describe(p: &KeyPerf) -> String {
    let mut parts = Vec::new();
    if p.touch == Some(TouchMode::Global) {
        parts.push(t::GLOBAL.to_string());
    }
    if let Some(a) = p.actuation {
        parts.push(format!("{} mm", mm_short(a.0)));
    }
    match (p.touch, p.rt_press, p.rt_release) {
        (Some(TouchMode::Single), _, _) => parts.push(t::NO_RT.to_string()),
        (_, Some(a), Some(b)) => parts.push(format!("{} {}", t::RT, mm_pair(a, b))),
        (_, Some(a), None) | (_, None, Some(a)) => {
            parts.push(format!("{} {} mm", t::RT, mm_short(a.0)))
        }
        _ => {}
    }
    match (p.top_dz, p.bottom_dz) {
        (Some(a), Some(b)) => parts.push(format!("{} {}", t::DEADZONE, mm_pair(a, b))),
        (Some(a), None) | (None, Some(a)) => {
            parts.push(format!("{} {} mm", t::DEADZONE, mm_short(a.0)))
        }
        _ => {}
    }
    parts.join(", ")
}

fn key_list(keys: &[u8], total: usize) -> String {
    if keys.len() == total && total > 8 {
        format!("{} ({})", t::ALL_KEYS, total)
    } else {
        legend_list(keys, 8)
    }
}

/// The human summary of `plan` on a keyboard with `total` keys.
pub fn summary(plan: &TemplatePlan, total: usize) -> Summary {
    let changed = plan.changed_keys();
    let headline = if plan.is_empty() {
        t::NOTHING.to_string()
    } else {
        format!("{} {} {}", t::CHANGES, changed.len(), t::KEYS)
    };
    let mut lines = Vec::new();
    for (keys, p) in &plan.perf {
        lines.push(format!("{} → {}", key_list(keys, total), describe(p)));
    }
    for (layer, name) in crate::pages::remap::LAYERS {
        let on: Vec<&(u8, Layer, u16)> = plan.layers.iter().filter(|e| e.1 == layer).collect();
        if on.is_empty() {
            continue;
        }
        if on.len() <= 4 {
            let items: Vec<String> = on
                .iter()
                .map(|&&(k, l, c)| {
                    format!(
                        "{} → {}",
                        openhe_proto::keycodes::legend(k),
                        crate::pages::remap::layer_code_name(l, c)
                    )
                })
                .collect();
            lines.push(format!("{name}: {}", items.join(", ")));
        } else {
            lines.push(format!("{name}: {} {}", on.len(), t::KEYS_CHANGED));
        }
    }
    let mut pairs = Vec::new();
    let mut cleared = Vec::new();
    let mut set = Vec::new();
    let mut last_input = true;
    for &(k, cfg) in &plan.functions {
        match cfg {
            AdvancedConfig::Socd(s) => {
                let label = format!(
                    "{}↔{}",
                    openhe_proto::keycodes::legend(s.pos1),
                    openhe_proto::keycodes::legend(s.pos2)
                );
                if !pairs.contains(&label) {
                    pairs.push(label);
                    last_input &= s.mode == openhe_proto::advanced::SocdMode::LastInput;
                }
            }
            AdvancedConfig::None => cleared.push(k),
            _ => set.push(k),
        }
    }
    if !pairs.is_empty() {
        let mut line = format!("SOCD {}", pairs.join(", "));
        if last_input {
            line.push_str(&format!(" ({})", t::SOCD_LAST));
        }
        lines.push(line);
    }
    if !set.is_empty() {
        lines.push(format!("{} {}", t::FN_SET, key_list(&set, total)));
    }
    if !cleared.is_empty() {
        lines.push(format!("{} {}", t::FN_CLEARED, key_list(&cleared, total)));
    }
    let mut notes = Vec::new();
    for (why, text) in [
        (Skip::NotOnKeyboard, t::SKIP_MISSING),
        (Skip::BeyondTravel, t::SKIP_TRAVEL),
        (Skip::MacroKept, t::SKIP_MACRO),
        (Skip::NotRead, t::SKIP_UNREAD),
        (Skip::OsModeUnknown, t::SKIP_OS),
    ] {
        let keys: Vec<u8> = plan
            .skipped
            .iter()
            .filter(|(_, w)| *w == why)
            .map(|(k, _)| *k)
            .collect();
        if !keys.is_empty() {
            notes.push(format!("{text} {}", legend_list(&keys, 8)));
        }
    }
    if plan.skipped.iter().any(|(_, w)| *w == Skip::FnKey) {
        notes.push(t::SKIP_FN.to_string());
    }
    notes.extend(plan.notes.iter().map(|n| n.to_string()));
    Summary {
        headline,
        lines,
        notes,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::test_support::{snapshot60, snapshot68};
    use crate::pages::settings::profile_file::import_job;
    use crate::templates::tests::factory_keys;
    use crate::templates::{Builtin, Parts, FPS_ACTUATION_UM, FPS_RT_UM};
    use openhe_device::{Keyboard, MockTransport};
    use openhe_proto::advanced::{Mt, Socd, SocdMode};
    use openhe_proto::key::Advanced;

    const W: u8 = 0x1A;
    const A: u8 = 0x04;
    const S: u8 = 0x16;
    const D: u8 = 0x07;

    fn keys_of(snap: &DeviceSnapshot) -> Vec<u8> {
        snap.matrix.keys().map(|(_, _, c)| c).collect()
    }

    fn plan(b: Builtin, snap: &DeviceSnapshot, cur: &[KeyData]) -> TemplatePlan {
        plan_template(&b.template(snap.model, &keys_of(snap)), snap, cur)
            .unwrap_or_else(|e| panic!("{}: {e:?}", b.name()))
    }

    /// A WIN 68 / WIN 60 where nothing matches any preset: every key at
    /// 1.00 mm Single, RT 0.05, deadzones 0.10, factory layers.
    fn plain(snap: &mut DeviceSnapshot) -> Vec<KeyData> {
        for s in snap.keys.values_mut() {
            s.mode.touch = TouchMode::Single;
            s.actuation = Micrometers(1000);
            s.rt_press = Micrometers(50);
            s.rt_release = Micrometers(50);
            s.top_dz = Micrometers(100);
            s.bottom_dz = Micrometers(100);
        }
        factory_keys(snap)
    }

    /// Every write of a plan passes the device API's own checks: run it on
    /// a mock keyboard and look at what fails before any I/O
    /// (`InvalidInput`): nothing may.
    fn device_accepts(plan: &TemplatePlan, model: openhe_proto::Model) {
        let mut kb = Keyboard::new(MockTransport::new(), model);
        let report = import_job(&mut kb, &plan.writes).unwrap();
        for (step, e) in report.failed {
            assert!(
                !matches!(e, openhe_device::Error::InvalidInput(_)),
                "{step:?} refused: {e:?}"
            );
        }
    }

    #[test]
    fn every_builtin_plans_on_both_models() {
        for mut snap in [snapshot68(), snapshot60()] {
            let cur = plain(&mut snap);
            for b in Builtin::ALL {
                let p = plan(b, &snap, &cur);
                assert!(!p.is_empty(), "{} changes nothing", b.name());
                assert!(
                    !p.skipped.iter().any(|(_, w)| *w == Skip::NotOnKeyboard),
                    "{}: {:?}",
                    b.name(),
                    p.skipped
                );
                let s = summary(&p, snap.matrix.keys().count());
                assert!(s.headline.starts_with("Mengubah"), "{}", s.headline);
                assert!(!s.lines.is_empty());
                device_accepts(&p, snap.model);
            }
        }
    }

    #[test]
    fn gaming_fps_sets_wasd_and_two_last_input_socd_pairs() {
        let mut snap = snapshot68();
        let cur = plain(&mut snap);
        let p = plan(Builtin::GamingFps, &snap, &cur);
        assert_eq!(p.changed_keys(), BTreeSet::from([W, A, S, D]));
        assert_eq!(p.writes.performance.len(), 1);
        let (keys, c) = &p.writes.performance[0];
        assert_eq!(keys, &vec![W, A, S, D]);
        assert_eq!(c.touch, Some(TouchMode::RapidTrigger));
        assert_eq!(c.actuation, Some(Micrometers(FPS_ACTUATION_UM)));
        assert_eq!(c.rt_press, Some(Micrometers(FPS_RT_UM)));
        assert_eq!(c.rt_release, Some(Micrometers(FPS_RT_UM)));
        assert_eq!(c.top_dz, None, "deadzones are not named");
        assert_eq!(c.switch, None);
        assert_eq!(p.writes.advanced.len(), 2, "each pair once");
        for (_, cfg) in &p.writes.advanced {
            let AdvancedConfig::Socd(s) = cfg else {
                panic!("{cfg:?}")
            };
            assert_eq!(s.mode, SocdMode::LastInput);
        }
        let s = summary(&p, 68);
        assert_eq!(s.headline, "Mengubah 4 tombol");
        assert_eq!(s.lines[0], "W, A, S, D → 0.40 mm, RT 0.15 mm");
        assert_eq!(s.lines[1], "SOCD A↔D, W↔S (input terakhir menang)");
    }

    #[test]
    fn only_differences_are_written() {
        let mut snap = snapshot68();
        let cur = plain(&mut snap);
        let fps = Builtin::GamingFps.template(snap.model, &keys_of(&snap));
        // Already applied: W A S D at the values, pairs in place.
        let mut after = cur.clone();
        for k in after.iter_mut() {
            if [W, A, S, D].contains(&k.code) {
                k.settings.mode.touch = TouchMode::RapidTrigger;
                k.settings.actuation = Micrometers(FPS_ACTUATION_UM);
                k.settings.rt_press = Micrometers(FPS_RT_UM);
                k.settings.rt_release = Micrometers(FPS_RT_UM);
                let pair = fps.advanced.iter().find(|(c, _)| *c == k.code).unwrap().1;
                k.advanced = pair;
            }
        }
        let mut snap2 = snap.clone();
        for k in &after {
            snap2.keys.insert(k.code, k.settings);
        }
        let p = plan_template(&fps, &snap2, &after).unwrap();
        assert!(p.is_empty(), "{p:?}");
        assert_eq!(
            summary(&p, 68).headline,
            summary(&TemplatePlan::default(), 68).headline
        );
    }

    #[test]
    fn values_beyond_the_switch_travel_skip_that_key() {
        let mut snap = snapshot68();
        let mut cur = plain(&mut snap);
        let mut tpl = Builtin::Typing.template(snap.model, &keys_of(&snap));
        for (_, p) in tpl.performance.iter_mut() {
            p.actuation = Some(Micrometers(3350)); // > 3.30 mm switches
        }
        // Q on a Magneto (3.30 mm), the rest Wing Chun (3.40 mm).
        for k in cur.iter_mut().filter(|k| k.code == 0x14) {
            k.settings.switch = 3;
        }
        let p = plan_template(&tpl, &snap, &cur).unwrap();
        assert_eq!(p.skipped, vec![(0x14, Skip::BeyondTravel)]);
        assert!(!p.changed_keys().contains(&0x14));
        assert_eq!(p.changed_keys().len(), 67);
        let s = summary(&p, 68);
        assert!(s.notes[0].contains("melebihi jarak tekan"), "{:?}", s.notes);
        // Below the minimum no key can take: the template is refused.
        tpl.performance[0].1.actuation = Some(Micrometers(50));
        assert!(matches!(
            plan_template(&tpl, &snap, &cur),
            Err(TemplateError::BadValue(_))
        ));
    }

    #[test]
    fn keys_the_keyboard_lacks_are_skipped_and_named() {
        let mut snap = snapshot60();
        let cur = plain(&mut snap);
        // A WIN 68 template moved to a WIN 60: arrows and nav keys.
        let mut tpl = Builtin::Typing.template(snap.model, &[0x50, 0x4C, 0x04]);
        tpl.layers.push((0x49, Layer::Fn1, 0x0004));
        let p = plan_template(&tpl, &snap, &cur).unwrap();
        assert_eq!(p.changed_keys(), BTreeSet::from([0x04]));
        let missing: Vec<u8> = p
            .skipped
            .iter()
            .filter(|(_, w)| *w == Skip::NotOnKeyboard)
            .map(|(k, _)| *k)
            .collect();
        assert_eq!(missing, vec![0x50, 0x4C, 0x49]);
        assert!(summary(&p, 61).notes[0].starts_with("Dilewati, tidak ada"));
    }

    #[test]
    fn another_models_template_is_refused() {
        let snap = snapshot60();
        let tpl = Builtin::CapsCtrl.template(openhe_proto::Model::Win68HeMax, &[0x39]);
        assert!(matches!(
            plan_template(&tpl, &snap, &factory_keys(&snap)),
            Err(TemplateError::WrongModel { .. })
        ));
    }

    #[test]
    fn factory_writes_global_after_the_values_and_keeps_fn_and_macros() {
        let mut snap = snapshot68();
        let mut cur = plain(&mut snap);
        for k in cur.iter_mut() {
            match k.code {
                0x08 => {
                    k.advanced = AdvancedConfig::Mt(Mt {
                        hold: 0xE0,
                        tap: 8,
                        delay_ms: 200,
                    })
                }
                0x10 => k.advanced = AdvancedConfig::Macro,
                0x39 => k.layers[0] = 0x00E0,
                0x0B => k.layers[1] = 0x0050,
                _ => {}
            }
        }
        let p = plan(Builtin::Factory, &snap, &cur);
        // Values first (keys leave Single for... nothing yet), Global last.
        let n = p.writes.performance.len();
        assert_eq!(n, 2, "{:?}", p.writes.performance);
        let (k1, c1) = &p.writes.performance[0];
        assert_eq!(k1.len(), 68);
        assert_eq!(c1.touch, None);
        assert_eq!(c1.actuation, Some(Micrometers(2000)));
        assert_eq!(c1.rt_press, Some(Micrometers(20)));
        assert_eq!(c1.top_dz, Some(Micrometers(200)));
        assert_eq!(c1.switch, None, "the switch type is never touched");
        assert_eq!(p.writes.performance[1].1.touch, Some(TouchMode::Global));
        assert_eq!(p.writes.performance[1].1.actuation, None);
        // Layers: only the two that differ; Fn's Base code never.
        assert_eq!(
            p.writes.layers,
            vec![(0x39, Layer::Base, 0x0039), (0x0B, Layer::Fn1, 0x0001)]
        );
        // MT cleared, the macro kept.
        assert_eq!(p.writes.clear_advanced, vec![0x08]);
        assert!(p.skipped.contains(&(0x10, Skip::MacroKept)));
        let s = summary(&p, 68);
        assert!(
            s.lines[0].starts_with("Semua tombol (68) → mode Global, 2.00 mm"),
            "{:?}",
            s.lines
        );
    }

    #[test]
    fn factory_on_win60_leaves_the_fn_layers_and_says_so() {
        let mut snap = snapshot60();
        let mut cur = plain(&mut snap);
        for k in cur.iter_mut() {
            k.layers[1] = 0x0004;
        }
        let p = plan(Builtin::Factory, &snap, &cur);
        assert!(p.layers.iter().all(|(_, l, _)| *l == Layer::Base));
        let s = summary(&p, 61);
        assert!(
            s.notes.iter().any(|n| n.contains("WIN 60")),
            "{:?}",
            s.notes
        );
    }

    #[test]
    fn win_and_alt_follow_the_os_mode() {
        let mut snap = snapshot68();
        let mut cur = plain(&mut snap);
        let tpl = Builtin::Factory.template(snap.model, &keys_of(&snap));
        // Mac mode, untouched: LAlt reads 0xE3, LWin 0xE2. Nothing to do.
        snap.system_mode = Some(SystemMode::Mac);
        for k in cur.iter_mut() {
            match k.code {
                0xE2 => k.layers[0] = 0x00E3,
                0xE3 => k.layers[0] = 0x00E2,
                _ => {}
            }
        }
        let p = plan_template(&tpl, &snap, &cur).unwrap();
        assert!(!p.layers.iter().any(|(k, _, _)| matches!(k, 0xE2 | 0xE3)));
        // LAlt remapped to A in Mac mode: back to what Mac mode reads.
        cur.iter_mut().find(|k| k.code == 0xE2).unwrap().layers[0] = 0x0004;
        let p = plan_template(&tpl, &snap, &cur).unwrap();
        assert!(p.layers.contains(&(0xE2, Layer::Base, 0x00E3)));
        snap.system_mode = Some(SystemMode::Windows);
        let p = plan_template(&tpl, &snap, &cur).unwrap();
        assert!(p.layers.contains(&(0xE2, Layer::Base, 0x00E2)));
        snap.system_mode = None;
        let p = plan_template(&tpl, &snap, &cur).unwrap();
        assert!(p.skipped.contains(&(0xE2, Skip::OsModeUnknown)));
    }

    #[test]
    fn vim_and_caps_name_their_codes() {
        let mut snap = snapshot68();
        let cur = plain(&mut snap);
        let p = plan(Builtin::VimArrows, &snap, &cur);
        assert_eq!(
            p.writes.layers,
            vec![
                (0x0B, Layer::Fn1, 0x0050),
                (0x0D, Layer::Fn1, 0x0051),
                (0x0E, Layer::Fn1, 0x0052),
                (0x0F, Layer::Fn1, 0x004F)
            ]
        );
        assert!(p.writes.performance.is_empty() && p.writes.advanced.is_empty());
        let p = plan(Builtin::CapsCtrl, &snap, &cur);
        assert_eq!(p.writes.layers, vec![(0x39, Layer::Base, 0x00E0)]);
        let s = summary(&p, 68);
        assert_eq!(s.headline, "Mengubah 1 tombol");
        assert!(s.lines[0].starts_with("Base: Caps"), "{:?}", s.lines);
    }

    #[test]
    fn a_backup_restores_exactly_including_global_and_socd() {
        // Before: factory snapshot68 (W A S D on 0.40 RT, rest Global).
        let snap = snapshot68();
        let before = factory_keys(&snap);
        let backup = super::super::Template::from_current("b", snap.model, &before, Parts::ALL);
        // After Gaming FPS: W A S D 0.40 / 0.15 RT and two SOCD pairs.
        let fps = Builtin::GamingFps.template(snap.model, &keys_of(&snap));
        let mut after = before.clone();
        let mut snap_after = snap.clone();
        for k in after.iter_mut() {
            if let Some((_, p)) = fps.performance.iter().find(|(c, _)| *c == k.code) {
                k.settings.rt_press = p.rt_press.unwrap();
                k.settings.rt_release = p.rt_release.unwrap();
                k.settings.mode.advanced = Advanced::Socd;
                k.advanced = fps.advanced.iter().find(|(c, _)| *c == k.code).unwrap().1;
                snap_after.keys.insert(k.code, k.settings);
            }
        }
        let p = plan_template(&backup, &snap_after, &after).unwrap();
        assert_eq!(p.changed_keys(), BTreeSet::from([W, A, S, D]));
        assert_eq!(p.writes.performance.len(), 1);
        assert_eq!(p.writes.performance[0].1.rt_press, Some(Micrometers(20)));
        assert_eq!(p.writes.clear_advanced, vec![A, D, S, W]);
        assert!(p.writes.layers.is_empty());
        // A Global key that had travel values: values first, Global last.
        let mut g = after.clone();
        let q = g.iter_mut().find(|k| k.code == 0x14).unwrap();
        q.settings.mode.touch = TouchMode::RapidTrigger;
        q.settings.actuation = Micrometers(700);
        let p = plan_template(&backup, &snap_after, &g).unwrap();
        let q_writes: Vec<&PerformanceChange> = p
            .writes
            .performance
            .iter()
            .filter(|(k, _)| k.contains(&0x14))
            .map(|(_, c)| c)
            .collect();
        assert_eq!(q_writes.len(), 2);
        assert_eq!(q_writes[0].actuation, Some(Micrometers(2000)));
        assert_eq!(q_writes[1].touch, Some(TouchMode::Global));
    }

    #[test]
    fn a_new_socd_pair_names_the_old_partner_it_clears() {
        let mut snap = snapshot68();
        let mut cur = plain(&mut snap);
        // A is paired with J; Gaming FPS pairs A with D.
        let aj = AdvancedConfig::Socd(Socd {
            pos1: A,
            pos2: 0x0D,
            key1: 4,
            key2: 0x0D,
            bind_values: false,
            mode: SocdMode::Neutral,
            delay: 0,
        });
        for k in cur.iter_mut().filter(|k| matches!(k.code, A | 0x0D)) {
            k.advanced = aj;
        }
        let p = plan(Builtin::GamingFps, &snap, &cur);
        assert!(p.changed_keys().contains(&0x0D), "{:?}", p.functions);
        let s = summary(&p, 68);
        assert!(
            s.lines.iter().any(|l| l.contains("dihapus: J")),
            "{:?}",
            s.lines
        );
    }

    #[test]
    fn a_function_on_fn_or_a_bad_function_refuses_the_template() {
        let mut snap = snapshot68();
        let cur = plain(&mut snap);
        let mut tpl = Builtin::CapsCtrl.template(snap.model, &[]);
        tpl.advanced.push((
            FN_KEY,
            AdvancedConfig::Mt(Mt {
                hold: 0xE0,
                tap: 8,
                delay_ms: 200,
            }),
        ));
        assert!(matches!(
            plan_template(&tpl, &snap, &cur),
            Err(TemplateError::BadValue(_))
        ));
        let mut tpl = Builtin::CapsCtrl.template(snap.model, &[]);
        tpl.advanced.push((
            0x08,
            AdvancedConfig::Mt(Mt {
                hold: 0xE0,
                tap: 8,
                delay_ms: 0,
            }),
        ));
        assert!(plan_template(&tpl, &snap, &cur).is_err());
    }
}
