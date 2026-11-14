//! A border between two places the user drags to make them wider or narrower (user 2026-11-14:
//! "da se paneli mogu širiti po korisnikovim potrebama, a ne da budu fiksni"). It only reports
//! how far it was dragged; the caller keeps the sizes.

use eframe::egui::{self, Rect};

/// How wide the grip of a border is on each side of its line.
const GRIP: f32 = 4.0;

/// A border along `line` (a vertical border when `across` is false: dragged left and right; a
/// horizontal one when it is true: dragged up and down); the pixels it was dragged this frame.
/// The pointer shows the resize cursor over it.
pub fn drag_edge(ui: &mut egui::Ui, id: egui::Id, line: Rect, across: bool) -> f32 {
    let grip = if across { line.expand2(egui::vec2(0.0, GRIP)) } else { line.expand2(egui::vec2(GRIP, 0.0)) };
    let response = ui.interact(grip, id, egui::Sense::drag());
    if response.hovered() || response.dragged() {
        ui.ctx().set_cursor_icon(if across { egui::CursorIcon::ResizeVertical } else { egui::CursorIcon::ResizeHorizontal });
    }
    if response.dragged() {
        let delta = response.drag_delta();
        if across { delta.y } else { delta.x }
    } else {
        0.0
    }
}
