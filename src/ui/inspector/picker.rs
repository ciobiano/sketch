//! The design token picker beside inspector fields. A field linked to a token shows the
//! token's name as a chip; its menu switches to another token of the same type or unlinks.
//! An unlinked field gets a small button that lists the tokens it can use.
//!
//! Links are the same per-property `var(--name)` references agents write
//! (`mcp::tokens::link`), so a layer linked here follows token changes like any other.

use crate::editor::Editor;
use crate::mcp::{css, tokens as mcp_tokens};
use crate::model::{Node, NodeId};
use crate::theme;
use crate::tokens::{self, TokenType};
use crate::ui::c32;
use egui::{RichText, Ui};

enum Pick {
    Link(String),
    Unlink,
}

/// The tokens of type `kind`, with `current` ticked, and an Unlink item when `linked`.
fn menu(
    ui: &mut Ui,
    ed: &Editor,
    kind: TokenType,
    current: Option<&str>,
    linked: bool,
) -> Option<Pick> {
    let mut out = None;
    ui.set_min_width(180.0);
    let list: Vec<_> = ed.tokens.iter().filter(|t| t.kind == kind).collect();
    if list.is_empty() {
        ui.label(
            RichText::new(format!(
                "No {} tokens yet. Deselect everything to add them in the Tokens section.",
                kind.as_str()
            ))
            .size(theme::SECONDARY)
            .color(theme::pal(ui).secondary),
        );
    }
    for t in list {
        let value = ed.tokens.resolve(&t.name).unwrap_or_else(|e| e);
        let on = current == Some(t.name.as_str());
        let r = ui
            .horizontal(|ui| {
                if kind == TokenType::Color {
                    let (rect, _) =
                        ui.allocate_exact_size(egui::vec2(12.0, 12.0), egui::Sense::hover());
                    if let Some(c) = css::color(&value) {
                        ui.painter().rect_filled(rect, 3.0, c32(c));
                    }
                }
                let r = ui.selectable_label(on, RichText::new(&t.name).monospace());
                ui.label(RichText::new(&value).monospace().weak());
                r
            })
            .inner;
        if r.clicked() && !on {
            out = Some(Pick::Link(t.name.clone()));
        }
    }
    if linked {
        ui.separator();
        if ui
            .button("Unlink")
            .on_hover_text("Keep the current value without the token")
            .clicked()
        {
            out = Some(Pick::Unlink);
        }
    }
    out
}

/// The control for property `prop` (camelCase CSS) of the selection. When `first` has a
/// token link on it, a chip with the token name stands in for the field; otherwise `field`
/// draws the usual control, followed by a token button.
pub(super) fn field(
    ui: &mut Ui,
    ed: &mut Editor,
    ids: &[NodeId],
    first: &Node,
    prop: &str,
    width: f32,
    field: impl FnOnce(&mut Ui, &mut Editor),
) {
    let Some(kind) = mcp_tokens::picker_type(prop) else {
        field(ui, ed);
        return;
    };
    let link = tokens::link_for(&first.tokens, prop).map(|(_, r)| r.clone());
    let pick = match &link {
        Some(r) => {
            let label = tokens::link_label(r);
            let name = (label.starts_with("--")).then_some(label.as_str());
            let p = theme::pal(ui);
            if let Some(c) = (kind == TokenType::Color)
                .then(|| css::color(&r.applied))
                .flatten()
            {
                let (rect, _) =
                    ui.allocate_exact_size(egui::vec2(18.0, 18.0), egui::Sense::hover());
                ui.painter().rect_filled(rect, 4.0, c32(c));
                ui.painter().rect_stroke(
                    rect,
                    4.0,
                    egui::Stroke::new(1.0, p.separator),
                    egui::StrokeKind::Inside,
                );
            }
            let button = egui::Button::new(
                RichText::new(&label)
                    .monospace()
                    .size(theme::SECONDARY + 1.0)
                    .color(p.text),
            )
            .fill(theme::accent().gamma_multiply(0.18))
            .stroke(egui::Stroke::new(1.0, theme::accent().gamma_multiply(0.6)))
            .corner_radius(10.0)
            .truncate()
            .min_size(egui::vec2(width, 20.0));
            let (resp, inner) = egui::containers::menu::MenuButton::from_button(button)
                .ui(ui, |ui| menu(ui, ed, kind, name, true));
            resp.on_hover_text(format!("{} = {}", r.raw, r.applied));
            inner.and_then(|i| i.inner)
        }
        None => {
            field(ui, ed);
            let resp = theme::icon_button(ui, theme::Icon::Diamond, 18.0, false)
                .on_hover_text("Use a design token");
            egui::Popup::menu(&resp)
                .show(|ui| menu(ui, ed, kind, None, false))
                .and_then(|i| i.inner)
        }
    };
    match pick {
        Some(Pick::Link(name)) => match mcp_tokens::link(ed, ids, prop, &name) {
            Ok(0) => ed.status = format!("{name} doesn't apply to the selection"),
            Ok(_) => ed.status = format!("Linked {name}"),
            Err(e) => ed.status = e,
        },
        Some(Pick::Unlink) => {
            mcp_tokens::unlink(ed, ids, prop);
            ed.status = "Unlinked token".into();
        }
        None => {}
    }
}
