//! The pages list at the top of the left sidebar (as in Figma): switch, add, rename,
//! duplicate, delete and drag to reorder.

use super::{UiState, section};
use crate::editor::Editor;
use crate::theme::{self, Icon};
use egui::{Color32, RichText, Ui};

#[derive(Default)]
pub struct PagesUi {
    pub collapsed: Option<bool>,
    pub renaming: Option<(String, String)>,
    /// Row being dragged, and the index it would drop at.
    drag: Option<(usize, usize)>,
}

const ROW_H: f32 = 24.0;

#[derive(Clone, Copy)]
enum PageAction {
    Duplicate(usize),
    Delete(usize),
}

impl PageAction {
    fn apply(self, ed: &mut Editor) {
        match self {
            Self::Duplicate(index) => {
                ed.duplicate_page(index);
            }
            Self::Delete(index) => {
                ed.delete_page(index);
            }
        }
    }
}

pub fn panel(ui: &mut Ui, ed: &mut Editor, st: &mut UiState, compact: bool, query: &str) {
    if st
        .pages
        .renaming
        .as_ref()
        .is_some_and(|(id, _)| !ed.pages.iter().any(|page| &page.id == id))
    {
        st.pages.renaming = None;
    }
    let collapsed = query.is_empty() && st.pages.collapsed.unwrap_or(compact);
    let p = theme::pal(ui);
    ui.horizontal(|ui| {
        let chevron = if collapsed {
            Icon::ChevronRight
        } else {
            Icon::ChevronDown
        };
        if ui
            .add_enabled_ui(query.is_empty(), |ui| {
                theme::icon_button(ui, chevron, 18.0, false)
            })
            .inner
            .on_hover_text(if collapsed {
                "Show pages"
            } else {
                "Collapse pages"
            })
            .clicked()
        {
            st.pages.collapsed = Some(!collapsed);
        }
        section(ui, "Pages");
        if collapsed {
            let max_width = (ui.available_width() - 31.0).max(0.0);
            ui.add_sized(
                [max_width, 18.0],
                egui::Label::new(
                    RichText::new(ed.page_name())
                        .size(theme::SECONDARY)
                        .color(p.secondary),
                )
                .truncate(),
            );
        }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if theme::icon_button(ui, Icon::Plus, 20.0, false)
                .on_hover_text("Add page")
                .clicked()
            {
                ed.add_page();
            }
        });
    });
    if collapsed {
        return;
    }
    let n = ed.pages.len();
    let shown: Vec<_> = (0..n)
        .filter(|&i| query.is_empty() || ed.pages[i].name.to_lowercase().contains(query))
        .collect();
    if shown.is_empty() {
        ui.label(RichText::new("No matching pages").small().weak());
        return;
    }
    let max_h = shown.len().min(6) as f32 * (ROW_H + 1.0);
    let mut action = None;
    egui::ScrollArea::vertical()
        .id_salt("pages")
        .max_height(max_h)
        .auto_shrink([false, true])
        .show(ui, |ui| {
            ui.spacing_mut().item_spacing.y = 1.0;
            let top = ui.cursor().top();
            for i in shown {
                ui.push_id(ed.pages[i].id.clone(), |ui| {
                    action = page_row(ui, ed, st, i, top, query.is_empty()).or(action);
                });
            }
        });
    if let Some(action) = action {
        st.pages.drag = None;
        action.apply(ed);
        return;
    }
    // Drop: reorder.
    if let Some((from, to)) = st.pages.drag
        && ui.input(|i| i.pointer.any_released())
    {
        st.pages.drag = None;
        ed.move_page(from, to.min(n - 1));
    }
}

fn page_row(
    ui: &mut Ui,
    ed: &mut Editor,
    st: &mut UiState,
    i: usize,
    top: f32,
    reorder: bool,
) -> Option<PageAction> {
    let id = ed.pages[i].id.clone();
    let p = theme::pal(ui);
    let active = i == ed.active_page;
    let (row, resp) = ui.allocate_exact_size(
        egui::vec2(ui.available_width(), ROW_H),
        if reorder {
            egui::Sense::click_and_drag()
        } else {
            egui::Sense::click()
        },
    );
    if active {
        ui.painter().rect_filled(row, 6.0, p.pressed);
    } else if resp.hovered() {
        ui.painter().rect_filled(row, 6.0, p.hover);
    }
    // Drop indicator while dragging.
    if let Some((from, to)) = st.pages.drag
        && from != to
        && to == i
    {
        let y = if to > from { row.bottom() } else { row.top() };
        ui.painter()
            .hline(row.x_range(), y, egui::Stroke::new(2.0, theme::accent()));
    }
    theme::paint_icon(
        ui.painter(),
        egui::Rect::from_center_size(
            egui::pos2(row.left() + 14.0, row.center().y),
            egui::vec2(13.0, 13.0),
        ),
        Icon::Page,
        if active { p.text } else { p.secondary },
    );
    let name_rect = egui::Rect::from_min_max(
        egui::pos2(row.left() + 28.0, row.top()),
        egui::pos2(row.right() - 4.0, row.bottom()),
    );
    if let Some((target, name)) = &mut st.pages.renaming
        && *target == id
    {
        let escape = ui.input(|i| i.key_pressed(egui::Key::Escape));
        let response = ui.put(
            name_rect.shrink2(egui::vec2(0.0, 2.0)),
            egui::TextEdit::singleline(name).id(ui.id().with("rename")),
        );
        if let Some(confirmed) = super::finish_rename(ui, &response, escape) {
            let (_, name) = st.pages.renaming.take().unwrap();
            if confirmed {
                ed.rename_page(i, &name);
            }
        }
        return None;
    } else {
        let font = if active {
            theme::medium(theme::BODY)
        } else {
            egui::FontId::proportional(theme::BODY)
        };
        let g = ui
            .painter()
            .layout(ed.pages[i].name.clone(), font, p.text, f32::INFINITY);
        ui.painter()
            .with_clip_rect(name_rect.intersect(ui.clip_rect()))
            .galley(
                egui::pos2(name_rect.left(), row.center().y - g.size().y / 2.0),
                g,
                Color32::PLACEHOLDER,
            );
    }
    if resp.double_clicked() {
        st.pages.renaming = Some((id.clone(), ed.pages[i].name.clone()));
    } else if resp.clicked() {
        ed.switch_page(i);
    }
    if reorder && resp.drag_started() {
        st.pages.drag = Some((i, i));
    }
    if reorder
        && resp.dragged()
        && let Some(pos) = ui.input(|inp| inp.pointer.interact_pos())
        && let Some((from, _)) = st.pages.drag
    {
        let to = ((pos.y - top) / (ROW_H + 1.0)).floor().max(0.0) as usize;
        st.pages.drag = Some((from, to.min(ed.pages.len() - 1)));
    }
    let mut action = None;
    resp.context_menu(|ui| {
        if ui.button("Rename").clicked() {
            st.pages.renaming = Some((id.clone(), ed.pages[i].name.clone()));
            ui.close();
        }
        if ui.button("Duplicate").clicked() {
            action = Some(PageAction::Duplicate(i));
            ui.close();
        }
        if ui
            .add_enabled(ed.pages.len() > 1, egui::Button::new("Delete"))
            .clicked()
        {
            action = Some(PageAction::Delete(i));
            ui.close();
        }
    });
    action
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui::{Event, Modifiers, PointerButton, Pos2, Rect, Shape, pos2};

    #[test]
    fn deleting_first_page_keeps_other_rows_valid_and_rename_commits() {
        let ctx = egui::Context::default();
        theme::apply(&ctx);
        let mut ed = Editor::default();
        ed.rename_page(0, "First");
        ed.add_page_in_background(Some("Second"));
        ed.add_page_in_background(Some("Third"));
        let mut st = UiState::default();
        let run = |ed: &mut Editor, st: &mut UiState, events| {
            let mut output = ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(Rect::from_min_size(Pos2::ZERO, egui::vec2(256.0, 400.0))),
                    events,
                    ..Default::default()
                },
                |ui| panel(ui, ed, st, false, ""),
            );
            output.textures_delta.clear();
            output
        };
        let click = |pos, button, pressed| Event::PointerButton {
            pos,
            button,
            pressed,
            modifiers: Modifiers::NONE,
        };
        let text_position = |output: &egui::FullOutput, value: &str| {
            output
                .shapes
                .iter()
                .find_map(|entry| {
                    if let Shape::Text(text) = &entry.shape
                        && text.galley.job.text == value
                    {
                        return Some(text.pos + text.galley.rect.center().to_vec2());
                    }
                    None
                })
                .unwrap_or_else(|| panic!("missing text {value}"))
        };
        run(&mut ed, &mut st, vec![]);
        let output = run(&mut ed, &mut st, vec![]);
        let row = text_position(&output, "First");
        run(
            &mut ed,
            &mut st,
            vec![
                Event::PointerMoved(row),
                click(row, PointerButton::Secondary, true),
            ],
        );
        run(
            &mut ed,
            &mut st,
            vec![click(row, PointerButton::Secondary, false)],
        );
        let menu = run(&mut ed, &mut st, vec![]);
        let delete = text_position(&menu, "Delete");
        run(
            &mut ed,
            &mut st,
            vec![
                Event::PointerMoved(delete),
                click(delete, PointerButton::Primary, true),
            ],
        );
        run(
            &mut ed,
            &mut st,
            vec![click(delete, PointerButton::Primary, false)],
        );
        assert_eq!(
            ed.pages.iter().map(|p| p.name.as_str()).collect::<Vec<_>>(),
            ["Second", "Third"]
        );
        assert_eq!(ed.page_name(), "Second");

        st.pages.renaming = Some((ed.pages[0].id.clone(), "Updated".into()));
        run(
            &mut ed,
            &mut st,
            vec![Event::PointerMoved(pos2(200.0, 300.0))],
        );
        run(
            &mut ed,
            &mut st,
            vec![Event::Key {
                key: egui::Key::Enter,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: Modifiers::NONE,
            }],
        );
        assert_eq!(ed.page_name(), "Updated");
        assert!(st.pages.renaming.is_none());

        st.pages.renaming = Some((ed.pages[0].id.clone(), "Discarded".into()));
        run(&mut ed, &mut st, vec![]);
        run(
            &mut ed,
            &mut st,
            vec![Event::Key {
                key: egui::Key::Escape,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: Modifiers::NONE,
            }],
        );
        assert_eq!(ed.page_name(), "Updated");
        assert!(st.pages.renaming.is_none());
    }
}
