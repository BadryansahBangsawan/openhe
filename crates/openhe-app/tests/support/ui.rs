//! UI queries and input for [`World`]: by AccessKit label, value or role.

use super::{collect, World};
use eframe::egui::{Event, Key, Modifiers, PointerButton, Pos2, Rect};
use eframe::egui::accesskit::{Role, Toggled};
use egui_kittest::kittest::{NodeT, Queryable};

impl World {
    /// Whether some node has exactly this label.
    pub fn has(&self, label: &str) -> bool {
        self.h.query_all_by_label(label).next().is_some()
    }

    /// Whether some node's label (or value) contains `text`.
    pub fn has_text(&self, text: &str) -> bool {
        self.labels().iter().any(|l| l.contains(text))
    }

    /// All labels on screen (values prefixed with "="), for messages.
    pub fn labels(&self) -> Vec<String> {
        let mut out = Vec::new();
        collect(&self.h.root(), &mut out);
        out
    }

    fn first<'a>(&'a self, label: &'a str) -> egui_kittest::Node<'a> {
        match self.h.query_all_by_label(label).next() {
            Some(n) => n,
            None => panic!("no node {label:?}; on screen: {:?}", self.labels()),
        }
    }

    /// Whether the first node labelled `label` is enabled.
    pub fn enabled(&self, label: &str) -> bool {
        !self.first(label).accesskit_node().is_disabled()
    }

    /// Whether the first node labelled `label` is selected or checked.
    pub fn selected(&self, label: &str) -> bool {
        let n = self.first(label);
        let a = n.accesskit_node();
        a.is_selected() == Some(true) || a.toggled() == Some(Toggled::True)
    }

    /// The rect of the first node labelled `label`.
    pub fn rect(&self, label: &str) -> Rect {
        self.first(label).rect()
    }

    /// Clicks the node labelled `label` (exact; the first one if several).
    pub fn click(&mut self, label: &str) {
        self.click_nth(label, 0);
    }

    pub fn click_nth(&mut self, label: &str, n: usize) {
        {
            let all: Vec<_> = self.h.query_all_by_label(label).collect();
            let Some(node) = all.get(n) else {
                panic!("no node {n} labelled {label:?}; on screen: {:?}", self.labels());
            };
            assert!(
                !node.accesskit_node().is_disabled(),
                "{label:?} is disabled; on screen: {:?}",
                self.labels()
            );
            node.click();
        }
        self.frames(2);
    }

    /// Clicks the first node whose label contains `text`.
    pub fn click_text(&mut self, text: &str) {
        {
            let Some(node) = self.h.query_all_by_label_contains(text).next() else {
                panic!("no node containing {text:?}; on screen: {:?}", self.labels());
            };
            node.click();
        }
        self.frames(2);
    }

    /// Clicks the n-th node with `role` (e.g. a combo box or a toggle,
    /// which carry no label of their own).
    pub fn click_role(&mut self, role: Role, n: usize) {
        {
            let all: Vec<_> = self.h.query_all_by_role(role).collect();
            let Some(node) = all.get(n) else {
                panic!("no {role:?} #{n}; on screen: {:?}", self.labels());
            };
            node.click();
        }
        self.frames(2);
    }

    /// Whether the n-th node with `role` is checked.
    pub fn role_checked(&self, role: Role, n: usize) -> bool {
        self.h
            .query_all_by_role(role)
            .nth(n)
            .is_some_and(|x| x.accesskit_node().toggled() == Some(Toggled::True))
    }

    /// The label of on-screen key `code` ("W · 0x1A"), if shown.
    fn key_node(&self, code: u8) -> Option<egui_kittest::Node<'_>> {
        let tag = format!(" · 0x{code:02X}");
        self.h
            .query_all_by(move |n| {
                n.role() == Role::Button && n.label().is_some_and(|l| l.ends_with(&tag))
            })
            .next()
    }

    /// Clicks key `code` on the (first) on-screen keyboard; `add` holds
    /// Shift to add it to the selection.
    pub fn click_key(&mut self, code: u8, add: bool) {
        {
            let Some(node) = self.key_node(code) else {
                panic!("no key 0x{code:02X}; on screen: {:?}", self.labels());
            };
            if add {
                node.click_modifiers(Modifiers::SHIFT);
            } else {
                node.click();
            }
        }
        self.frames(2);
    }

    /// Whether key `code` is shown selected.
    pub fn key_selected(&self, code: u8) -> bool {
        self.key_node(code)
            .is_some_and(|n| n.accesskit_node().is_selected() == Some(true))
    }

    /// Clicks at a point (for parts of custom widgets without a node).
    pub fn click_at(&mut self, pos: Pos2) {
        let ev = &mut self.h.input_mut().events;
        ev.push(Event::PointerMoved(pos));
        ev.push(Event::PointerButton {
            pos,
            button: PointerButton::Primary,
            pressed: true,
            modifiers: Modifiers::NONE,
        });
        self.h.step();
        self.h.input_mut().events.push(Event::PointerButton {
            pos,
            button: PointerButton::Primary,
            pressed: false,
            modifiers: Modifiers::NONE,
        });
        self.frames(2);
    }

    /// Clicks the n-th node whose value is `value` (a DragValue shows its
    /// number; a click starts editing it), then types `text` and Enter.
    pub fn edit_value(&mut self, value: &str, n: usize, text: &str) {
        {
            let all: Vec<_> = self.h.query_all_by_value(value).collect();
            let Some(node) = all.get(n) else {
                panic!("no node {n} with value {value:?}; on screen: {:?}", self.labels());
            };
            node.click();
        }
        self.frames(2);
        self.replace_text(text);
    }

    /// Focuses the n-th text field labelled `label` and replaces its text.
    pub fn type_into(&mut self, label: &str, n: usize, text: &str) {
        {
            let all: Vec<_> = self.h.query_all_by_label(label).collect();
            let Some(node) = all.get(n) else {
                panic!("no field {n} labelled {label:?}; on screen: {:?}", self.labels());
            };
            node.click();
        }
        self.frames(2);
        self.replace_text(text);
    }

    /// Replaces the text of the focused field with `text`, then Enter.
    pub fn replace_text(&mut self, text: &str) {
        let ev = &mut self.h.input_mut().events;
        ev.push(Event::Key {
            key: Key::A,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: Modifiers::COMMAND,
        });
        ev.push(Event::Text(text.to_string()));
        ev.push(Event::Key {
            key: Key::Enter,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: Modifiers::NONE,
        });
        self.frames(3);
    }

    /// Types `text` into the focused field (no Enter).
    pub fn type_text(&mut self, text: &str) {
        self.h.input_mut().events.push(Event::Text(text.to_string()));
        self.frames(2);
    }

    /// One key press (down and up) with modifiers.
    pub fn key(&mut self, key: Key, modifiers: Modifiers) {
        for pressed in [true, false] {
            self.h.input_mut().events.push(Event::Key {
                key,
                physical_key: None,
                pressed,
                repeat: false,
                modifiers,
            });
            self.h.step();
        }
        self.frames(1);
    }

    /// Goes to a sidebar page by its title.
    pub fn page(&mut self, title: &str) {
        self.click(title);
        self.frames(3);
    }

    /// Clicks `label` and waits until the worker is idle again.
    pub fn apply(&mut self, label: &str) {
        self.click(label);
        self.frames(2);
        self.idle();
    }
}
