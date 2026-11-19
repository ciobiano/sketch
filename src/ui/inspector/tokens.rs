//! The Tokens section of the inspector, shown when nothing is selected: the project's design
//! tokens grouped by type, with their names and values editable in place.
//!
//! Edits go through the same rules as the MCP tools (`crate::tokens`), and layers that use a
//! changed token update on every page (`mcp::tokens::refresh`). Errors show in the status bar.

use super::{seal_on_release, separator};
use crate::editor::Editor;
use crate::mcp::{css, tokens::refresh};
use crate::model::Color;
use crate::theme;
use crate::tokens::{Token, TokenType};
use crate::ui::{c32, from32, title_text};
use egui::{RichText, Ui};

/// A single-line text field that reports its text when editing ends with a change.
fn text_cell(ui: &mut Ui, salt: &str, current: &str, width: f32) -> Option<String> {
    let id = ui.id().with(("token-cell", salt));
    let mut buf = ui
        .data_mut(|d| d.get_temp::<String>(id))
        .unwrap_or_else(|| current.to_string());
    let r = ui.add_sized(
        egui::vec2(width, 20.0),
        egui::TextEdit::singleline(&mut buf).font(egui::TextStyle::Monospace),
    );
    if r.has_focus() {
        ui.data_mut(|d| d.insert_temp(id, buf.clone()));
        None
    } else {
        ui.data_mut(|d| d.remove::<String>(id));
        (r.lost_focus() && buf.trim() != current).then(|| buf.trim().to_string())
    }
}

fn hex(c: Color) -> String {
    if c.a == 255 {
        format!("#{:02x}{:02x}{:02x}", c.r, c.g, c.b)
    } else {
        format!("#{:02x}{:02x}{:02x}{:02x}", c.r, c.g, c.b, c.a)
    }
}

/// A fresh name for a new token of `kind`: `--color-1`, `--color-2`, ...
fn new_name(ed: &Editor, kind: TokenType) -> String {
    let stem = match kind {
        TokenType::Color => "color",
        TokenType::Spacing => "space",
        TokenType::Radius => "radius",
        TokenType::FontFamily => "font",
        TokenType::FontSize => "text",
        TokenType::FontWeight => "weight",
        TokenType::LineHeight => "leading",
        TokenType::LetterSpacing => "tracking",
        TokenType::Opacity => "opacity",
        TokenType::Breakpoint => "breakpoint",
        TokenType::Container => "container",
    };
    (1..)
        .map(|i| format!("--{stem}-{i}"))
        .find(|n| ed.tokens.get(n).is_none())
        .unwrap()
}

enum Edit {
    Rename(String, String),
    Value(String, String),
    Delete(String),
}

pub fn section(ui: &mut Ui, ed: &mut Editor) {
    ui.horizontal(|ui| {
        ui.label(title_text("Tokens"));
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.menu_button(RichText::new("+").font(theme::semibold(14.0)), |ui| {
                for kind in TokenType::ALL {
                    if ui.button(kind.label()).clicked() {
                        let name = new_name(ed, kind);
                        if ed
                            .tokens
                            .create(kind, &name, kind.default_value(), "")
                            .is_ok()
                        {
                            ed.meta_dirty = true;
                            ed.status = format!("Added token {name}");
                        }
                        ui.close();
                    }
                }
            })
            .response
            .on_hover_text("Add a design token");
        });
    });
    if ed.tokens.is_empty() {
        ui.label(
            RichText::new("Named colors, sizes and type styles. Layers that use a token update when it changes.")
                .size(theme::SECONDARY)
                .color(theme::pal(ui).secondary),
        );
        return;
    }

    let tokens: Vec<Token> = ed.tokens.iter().cloned().collect();
    let mut edit = None;
    let mut first_group = true;
    for kind in TokenType::ALL {
        let group: Vec<&Token> = tokens.iter().filter(|t| t.kind == kind).collect();
        if group.is_empty() {
            continue;
        }
        if !first_group {
            ui.add_space(4.0);
        }
        first_group = false;
        ui.label(
            RichText::new(kind.label())
                .size(theme::SECONDARY)
                .color(theme::pal(ui).secondary),
        );
        for (i, t) in group.into_iter().enumerate() {
            let salt = format!("{}-{i}", kind.as_str());
            if let Some(e) = token_row(ui, ed, t, &salt) {
                edit = Some(e);
            }
        }
    }
    if let Some(e) = edit {
        apply(ed, e);
    }
    separator(ui);
}

fn token_row(ui: &mut Ui, ed: &mut Editor, t: &Token, salt: &str) -> Option<Edit> {
    let mut out = None;
    let resolved = ed.tokens.resolve(&t.name).ok();
    ui.horizontal(|ui| {
        // Every row keeps a swatch-sized slot so names and values line up across groups.
        const SWATCH: f32 = 22.0;
        if t.kind == TokenType::Color {
            ui.spacing_mut().interact_size = egui::vec2(SWATCH, 20.0);
            let c = resolved
                .as_deref()
                .and_then(css::color)
                .unwrap_or(Color::INK);
            let mut c2 = c32(c);
            let r = ui.color_edit_button_srgba(&mut c2);
            seal_on_release(ed, &r);
            if r.changed() && from32(c2) != c {
                out = Some(Edit::Value(t.name.clone(), hex(from32(c2))));
            }
        } else {
            ui.add_space(SWATCH);
        }
        let name_w = (ui.available_width() * 0.5 - 14.0).max(60.0);
        let r = text_cell(ui, &format!("{salt}-name"), &t.name, name_w);
        if let Some(n) = r {
            out = Some(Edit::Rename(t.name.clone(), n));
        }
        let value_w = (ui.available_width() - 26.0).max(48.0);
        if let Some(v) = text_cell(ui, &format!("{salt}-value"), &t.value, value_w) {
            out = Some(Edit::Value(t.name.clone(), v));
        }
        if theme::icon_button(ui, theme::Icon::Minus, 18.0, false)
            .on_hover_text("Delete token (layers keep their current values)")
            .clicked()
        {
            out = Some(Edit::Delete(t.name.clone()));
        }
    });
    if crate::tokens::has_var(&t.value) || !t.description.is_empty() {
        let mut note = String::new();
        if let Some(r) = resolved
            .as_deref()
            .filter(|_| crate::tokens::has_var(&t.value))
        {
            note.push_str(&format!("= {r}"));
        }
        if !t.description.is_empty() {
            if !note.is_empty() {
                note.push_str("  ·  ");
            }
            note.push_str(&t.description);
        }
        ui.label(
            RichText::new(note)
                .size(theme::SECONDARY)
                .color(theme::pal(ui).secondary),
        );
    }
    out
}

fn apply(ed: &mut Editor, e: Edit) {
    let r = match &e {
        Edit::Rename(old, new) => ed
            .tokens
            .rename(old, new)
            .map(|()| refresh(ed, &[(old.clone(), new.clone())], None)),
        Edit::Value(name, v) => ed
            .tokens
            .set_value(name, v)
            .map(|()| refresh(ed, &[], Some(&format!("token:{name}")))),
        Edit::Delete(name) => {
            ed.tokens.remove(name);
            Ok(refresh(ed, &[], None))
        }
    };
    match r {
        Ok(layers) => {
            ed.meta_dirty = true;
            ed.status = match (&e, layers) {
                (Edit::Delete(n), _) => format!("Deleted token {n}"),
                (_, 0) => "Token updated".into(),
                (_, 1) => "Token updated · 1 layer".into(),
                (_, n) => format!("Token updated · {n} layers"),
            };
        }
        Err(msg) => ed.status = msg,
    }
}
