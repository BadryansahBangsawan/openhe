//! One module per sidebar page. Each exposes `pub struct Page` (built with
//! `Default`) and `pub fn ui(&mut self, ctx: &AppCtx, ui: &mut egui::Ui)`.
//! A page that wants the 348 pt right panel uses
//! [`crate::app::page_with_right_panel`].

pub mod advanced;
pub mod calibration;
pub mod lighting;
pub mod macros;
pub mod performance;
pub mod remap;
pub mod settings;
pub mod templates;
