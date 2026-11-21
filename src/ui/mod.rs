//! egui chrome: menu bar, layers panel, design (properties) panel, floating toolbar, and the
//! projects home screen.

pub mod comments;
pub mod home;
pub mod inspector;
pub mod library;
pub mod pages;
pub mod palette;

pub mod workspace;

use crate::editor::shortcuts::{self, Action};
use crate::editor::{Editor, Tool};
use crate::model::{Color, NodeId, NodeKind};
use crate::theme::{self, Icon};
use egui::{Align2, Color32, RichText, Ui};
use std::collections::HashSet;

#[derive(Default)]
pub struct UiState {
    pub collapsed: HashSet<NodeId>,
    pub renaming: Option<(NodeId, String)>,
    pub show_help: bool,
    pub show_grid_off: bool,
    /// Export settings (inspector Export section).
    pub export: crate::io::export::Spec,
    /// The window has native chrome: a system menu bar (so no egui menu) and traffic lights
    /// inset into the toolbar / sidebar.
    pub native_chrome: bool,
    /// The sidebar sits over an NSVisualEffectView, so paint it transparent.
    pub translucent_sidebar: bool,
    /// When set, the projects home screen is shown instead of the editor.
    pub home: Option<home::HomeView>,
    /// Name of the open project (shown in the toolbar).
    pub project_name: String,
    /// Autosave state for the toolbar subtitle, e.g. "Saved" or "Saving…".
    pub save_state: String,
    pub pages: pages::PagesUi,
    pub library: library::LibraryUi,
    /// Left sidebar shows the library instead of the layers.
    pub show_library: bool,
    /// Left sidebar shows the comment threads instead of the layers.
    pub show_comments: bool,
    pub comments: comments::CommentsUi,
    pub palette: palette::PaletteUi,
    /// All panels hidden (⌘\\): just the canvas.
    pub hide_ui: bool,
    pub preview: bool,
    pub light_canvas: bool,
    pub reduced_motion: bool,
    pub workspace: workspace::WorkspaceUi,
    /// Layer being dragged in the layers panel, and where it would drop.
    pub layer_drag: Option<(NodeId, Option<LayerDrop>)>,
    /// Row rects of the last layers-panel frame (for drop targeting).
    pub layer_rows: Vec<(NodeId, egui::Rect, usize)>,
}

/// Where a dragged layer lands relative to a row.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum LayerDrop {
    /// Above the row (in front of it in paint order).
    Above(NodeId),
    Below(NodeId),
    /// Inside a frame or group (on top of its children).
    Into(NodeId),
}

pub fn setup_style(ctx: &egui::Context) {
    theme::apply(ctx);
}

pub(crate) fn c32(c: Color) -> Color32 {
    Color32::from_rgba_unmultiplied(c.r, c.g, c.b, c.a)
}
pub(crate) fn from32(c: Color32) -> Color {
    let [r, g, b, a] = c.to_srgba_unmultiplied();
    Color::rgba(r, g, b, a)
}

/// Section header, as in a macOS sidebar or inspector: 11pt semibold, secondary colour.
pub(crate) fn section(ui: &mut Ui, title: &str) {
    ui.label(
        RichText::new(title)
            .font(theme::semibold(theme::SECONDARY))
            .color(theme::pal(ui).secondary),
    );
}

pub(crate) fn title_text(text: impl Into<String>) -> RichText {
    RichText::new(text).font(theme::semibold(theme::BODY))
}

/// Return whether to save the edit once the text field closes.
pub(crate) fn finish_rename(ui: &Ui, response: &egui::Response, escape: bool) -> Option<bool> {
    let id = response.id.with("rename-focus");
    let frame = ui.ctx().cumulative_frame_nr();
    let previous = ui.data_mut(|data| {
        let previous = data.get_temp::<u64>(id);
        data.insert_temp(id, frame);
        previous
    });
    if !escape && previous.is_none_or(|last| frame.saturating_sub(last) > 1) {
        response.request_focus();
        return None;
    }
    let enter = ui.input(|i| i.key_pressed(egui::Key::Enter));
    if escape || response.lost_focus() || (response.has_focus() && enter) {
        ui.data_mut(|data| data.remove::<u64>(id));
        response.surrender_focus();
        Some(!escape)
    } else {
        None
    }
}

/// Lay out the chrome and return the remaining canvas rectangle (logical px).
pub fn draw(ui: &mut Ui, ed: &mut Editor, st: &mut UiState) -> egui::Rect {
    if let Some(hv) = &mut st.home {
        home::draw(ui, hv, st.native_chrome, st.translucent_sidebar);
        return egui::Rect::NOTHING;
    }
    sync_layer_page(ed, st);
    if st
        .renaming
        .as_ref()
        .is_some_and(|(id, _)| ed.doc.get(*id).is_none())
    {
        st.renaming = None;
    }
    for ev in std::mem::take(&mut ed.ui_events) {
        use crate::editor::commands::UiEvent;
        match ev {
            UiEvent::TogglePalette => st.palette.toggle(),
            UiEvent::Rename(id) => {
                if let Some(n) = ed.doc.get(id) {
                    st.renaming = Some((id, n.name.clone()));
                    st.show_library = false;
                    st.show_comments = false;
                    st.workspace.show_layers();
                    st.hide_ui = false;
                }
            }
            UiEvent::ToggleUi => st.hide_ui = !st.hide_ui,
            UiEvent::ToggleHelp => st.show_help = !st.show_help,
            UiEvent::ToggleGrid => st.show_grid_off = !st.show_grid_off,
            UiEvent::ToggleComments => {
                st.comments.hidden = !st.comments.hidden;
                ed.status = if st.comments.hidden {
                    "Comments hidden. Shift+C shows them again".into()
                } else {
                    "Comments shown".into()
                };
            }
            UiEvent::Comment { node, world } => st.comments.start(ed, node, world),
        }
    }
    if st.preview {
        let canvas = ui.available_rect_before_wrap();
        workspace::preview_controls(ui.ctx(), canvas, ed, st);
        return canvas;
    }
    if st.hide_ui {
        let canvas = ui.available_rect_before_wrap();
        palette::draw(ui.ctx(), ed, st, canvas);
        return canvas;
    }
    let canvas = workspace::panels(ui, ed, st);
    comments::canvas(ui.ctx(), ed, &mut st.comments, canvas);
    workspace::toolbar(ui.ctx(), ed, st, canvas);
    status(ui.ctx(), ed, canvas);
    palette::draw(ui.ctx(), ed, st, canvas);
    if st.show_help {
        help(ui.ctx(), st, canvas);
    }
    canvas
}

fn menu_item(ui: &mut Ui, ed: &mut Editor, label: &str, action: Action) {
    if ui
        .add(egui::Button::new(label).shortcut_text(shortcuts::chord_text(action)))
        .clicked()
    {
        ed.run(action);
        ui.close();
    }
}

fn menu_entries(ui: &mut Ui, ed: &mut Editor, entries: &[shortcuts::MenuEntry]) {
    for e in entries {
        match e {
            shortcuts::MenuEntry::Item(label, a) => menu_item(ui, ed, label, *a),
            shortcuts::MenuEntry::Separator => {
                ui.separator();
            }
            shortcuts::MenuEntry::Submenu(label, items) => {
                ui.menu_button(*label, |ui| menu_entries(ui, ed, items));
            }
        }
    }
}

/// egui menus for platforms without a native menu bar (inline, so the toolbar row keeps
/// the title and zoom controls).
fn menu_bar(ui: &mut Ui, ed: &mut Editor) {
    for (title, entries) in shortcuts::menus() {
        ui.menu_button(title, |ui| {
            menu_entries(ui, ed, &entries);
        });
    }
}

fn sync_layer_page(ed: &Editor, st: &mut UiState) {
    let page = ed.active_page_id();
    if st.workspace.page_id != page {
        st.workspace.page_id = page.to_owned();
        st.renaming = None;
        st.layer_drag = None;
    }
}

fn layers(ui: &mut Ui, ed: &mut Editor, st: &mut UiState, compact: bool) {
    let query = st.workspace.search.trim().to_lowercase();
    pages::panel(ui, ed, st, compact, &query);
    sync_layer_page(ed, st);
    ui.add_space(4.0);
    let r = ui.available_rect_before_wrap();
    ui.painter().hline(
        r.x_range(),
        r.top(),
        egui::Stroke::new(1.0, theme::pal(ui).separator),
    );
    ui.add_space(6.0);
    let p = theme::pal(ui);
    ui.horizontal(|ui| {
        // 0 layers, 1 library, 2 comments.
        let tab = if st.show_comments {
            2
        } else {
            usize::from(st.show_library)
        };
        for (i, label) in ["Layers", "Library", "Comments"].into_iter().enumerate() {
            let on = tab == i;
            let r = ui.add(
                egui::Label::new(
                    RichText::new(label)
                        .font(theme::semibold(theme::SECONDARY))
                        .color(if on { p.text } else { p.secondary }),
                )
                .sense(egui::Sense::click()),
            );
            if r.clicked() {
                st.show_library = i == 1;
                st.show_comments = i == 2;
            }
        }
        if st.show_library || st.show_comments {
            return;
        }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.spacing_mut().item_spacing.x = 0.0;
            let some = !ed.selection.is_empty();
            ui.add_enabled_ui(some, |ui| {
                if theme::icon_button(ui, Icon::ChevronDown, 20.0, false)
                    .on_hover_text("Send backward")
                    .clicked()
                {
                    ed.run(Action::Backward);
                }
                if theme::icon_button(ui, Icon::ChevronUp, 20.0, false)
                    .on_hover_text("Bring forward")
                    .clicked()
                {
                    ed.run(Action::Forward);
                }
            });
        });
    });
    ui.add_space(2.0);
    if st.show_library {
        library::panel(ui, ed, st);
        return;
    }
    if st.show_comments {
        comments::panel(ui, ed, &mut st.comments);
        return;
    }
    let filter = (!query.is_empty()).then(|| {
        let mut visible = HashSet::new();
        for node in ed
            .doc
            .nodes
            .values()
            .filter(|n| n.name.to_lowercase().contains(&query))
        {
            let mut id = Some(node.id);
            while let Some(current) = id.filter(|id| visible.insert(*id)) {
                id = ed.doc.get(current).and_then(|n| n.parent);
            }
        }
        visible
    });
    egui::ScrollArea::vertical()
        .id_salt("layers-scroll")
        .auto_shrink(false)
        .show(ui, |ui| {
            ui.spacing_mut().item_spacing.y = 1.0;
            let roots = ed.doc.root.clone();
            if filter.as_ref().is_some_and(HashSet::is_empty) {
                ui.label(RichText::new("No matching layers").small().weak());
            } else if roots.is_empty() {
                ui.label(
                    RichText::new("Nothing here yet. Pick a tool below\nand draw on the canvas.")
                        .size(theme::SECONDARY)
                        .color(theme::pal(ui).secondary),
                );
            }
            st.layer_rows.clear();
            for id in roots.iter().rev() {
                layer_row(ui, ed, st, *id, 0, filter.as_ref());
            }
            if filter.is_none() {
                layer_drag(ui, ed, st);
            }
        });
}

/// Drag-and-drop reordering in the layers tree: above / below a row, or into a frame/group.
fn layer_drag(ui: &mut Ui, ed: &mut Editor, st: &mut UiState) {
    let Some((dragged, _)) = st.layer_drag else {
        return;
    };
    let Some(pos) = ui.input(|i| i.pointer.interact_pos()) else {
        return;
    };
    let target = st
        .layer_rows
        .iter()
        .find(|(_, r, _)| r.y_range().contains(pos.y))
        .copied();
    let drop = target.and_then(|(id, r, _)| {
        if id == dragged || ed.doc.is_ancestor(dragged, id) {
            return None;
        }
        let n = ed.doc.get(id)?;
        let t = (pos.y - r.top()) / r.height();
        Some(if n.is_container() && (0.3..0.7).contains(&t) {
            LayerDrop::Into(id)
        } else if t < 0.5 {
            LayerDrop::Above(id)
        } else {
            LayerDrop::Below(id)
        })
    });
    st.layer_drag = Some((dragged, drop));
    // Indicator.
    if let (Some(d), Some((_, r, depth))) = (drop, target) {
        let x0 = r.left() + 4.0 + depth as f32 * 14.0 + 16.0;
        let stroke = egui::Stroke::new(2.0, theme::accent());
        match d {
            LayerDrop::Above(_) => {
                ui.painter().hline(x0..=r.right(), r.top(), stroke);
            }
            LayerDrop::Below(_) => {
                ui.painter().hline(x0..=r.right(), r.bottom(), stroke);
            }
            LayerDrop::Into(_) => {
                ui.painter()
                    .rect_stroke(r, 6.0, stroke, egui::StrokeKind::Inside);
            }
        }
    }
    if ui.input(|i| i.pointer.any_released()) {
        st.layer_drag = None;
        if let Some(d) = drop {
            drop_layer(ed, dragged, d);
        }
    }
}

/// Move a layer in the tree (keeping its position on the canvas).
pub fn drop_layer(ed: &mut Editor, id: NodeId, d: LayerDrop) {
    let (parent, index) = match d {
        LayerDrop::Into(c) => (Some(c), None),
        LayerDrop::Above(t) | LayerDrop::Below(t) => {
            let parent = ed.doc.get(t).and_then(|n| n.parent);
            let list: Vec<NodeId> = ed
                .doc
                .children(parent)
                .iter()
                .copied()
                .filter(|x| *x != id)
                .collect();
            let i = list.iter().position(|x| *x == t).unwrap_or(0);
            (
                parent,
                Some(if matches!(d, LayerDrop::Above(_)) {
                    i + 1
                } else {
                    i
                }),
            )
        }
    };
    if parent.is_some_and(|p| p == id || ed.doc.is_ancestor(id, p)) {
        return;
    }
    ed.edit("Move layer", |tx| {
        let same = tx.doc().get(id).and_then(|n| n.parent) == parent;
        if same {
            if let Some(i) = index {
                tx.move_to_index(id, i);
            } else {
                let len = tx.doc().children(parent).len();
                tx.move_to_index(id, len);
            }
        } else {
            tx.reparent(id, parent, index);
        }
    });
    ed.selection = vec![id];
}

fn node_icon(n: &crate::model::Node) -> Icon {
    if n.component {
        return Icon::Component;
    }
    if n.instance.is_some() && n.parent.is_none_or(|_| n.is_frame()) && n.is_frame() {
        return Icon::Instance;
    }
    if n.style
        .fills
        .iter()
        .any(|p| matches!(p, crate::model::Paint::Image { .. }))
        && matches!(n.kind, NodeKind::Rect)
    {
        return Icon::Image;
    }
    kind_icon(&n.kind)
}

fn kind_icon(k: &NodeKind) -> Icon {
    match k {
        NodeKind::Group => Icon::Group,
        NodeKind::Connector { .. } => Icon::Arrow,
        NodeKind::Sticky { .. } => Icon::Sticky,
        NodeKind::Path { .. } => Icon::Pen,
        NodeKind::Polygon { .. } => Icon::Polygon,
        NodeKind::Star { .. } => Icon::Star,
        NodeKind::Frame { .. } => Icon::Frame,
        NodeKind::Rect => Icon::Rect,
        NodeKind::Ellipse => Icon::Ellipse,
        NodeKind::Line { .. } => Icon::Line,
        NodeKind::Text { .. } => Icon::Text,
        NodeKind::Stroke { .. } => Icon::Ink,
    }
}

/// One sidebar row, drawn like an NSOutlineView row: 24pt tall, 6pt radius selection in the
/// accent colour, subtle hover, disclosure chevron, symbol, name, and eye/lock on hover.
fn layer_row(
    ui: &mut Ui,
    ed: &mut Editor,
    st: &mut UiState,
    id: NodeId,
    depth: usize,
    filter: Option<&HashSet<NodeId>>,
) {
    if filter.is_some_and(|ids| !ids.contains(&id)) {
        return;
    }
    let Some(n) = ed.doc.get(id).cloned() else {
        return;
    };
    let p = theme::pal(ui);
    let selected = ed.selection.contains(&id);
    let width = ui.available_width();
    let (row, resp) =
        ui.allocate_exact_size(egui::vec2(width, 24.0), egui::Sense::click_and_drag());
    st.layer_rows.push((id, row, depth));
    if resp.drag_started() && !n.locked && filter.is_none() {
        st.layer_drag = Some((id, None));
    }
    let hovered = ui.rect_contains_pointer(row);
    if selected {
        ui.painter().rect_filled(row, 6.0, p.pressed);
    } else if hovered {
        ui.painter().rect_filled(row, 6.0, p.hover);
    }
    let fg = if selected {
        Color32::WHITE
    } else if n.visible {
        p.text
    } else {
        p.secondary
    };
    let fg2 = if selected {
        Color32::from_white_alpha(200)
    } else {
        p.secondary
    };
    let mut x = (row.left() + 4.0 + depth as f32 * 14.0).min(row.right() - 76.0);
    let cy = row.center().y;
    let sq = |x: f32, s: f32| {
        egui::Rect::from_center_size(egui::pos2(x + s / 2.0, cy), egui::vec2(s, s))
    };

    // Disclosure chevron.
    if !n.children.is_empty() && filter.is_none() {
        let open = !st.collapsed.contains(&id);
        let r = sq(x, 14.0);
        let cr = ui.interact(
            r.expand(2.0),
            ui.id().with(("disclose", id)),
            egui::Sense::click(),
        );
        theme::paint_icon(
            ui.painter(),
            r.shrink(2.0),
            if open {
                Icon::ChevronDown
            } else {
                Icon::ChevronRight
            },
            fg2,
        );
        if cr.clicked() {
            if open {
                st.collapsed.insert(id);
            } else {
                st.collapsed.remove(&id);
            }
        }
    }
    x += 16.0;
    theme::paint_icon(ui.painter(), sq(x, 14.0), node_icon(&n), fg2);
    x += 20.0;

    // Trailing eye / lock buttons: always shown when hidden or locked, otherwise on hover.
    let mut right = row.right() - 4.0;
    let mut trailing = |ui: &mut Ui, icon: Icon, tip: &str, show: bool| -> bool {
        let r = egui::Rect::from_center_size(egui::pos2(right - 9.0, cy), egui::vec2(18.0, 18.0));
        if !show {
            return false;
        }
        right -= 20.0;
        let b = ui.interact(r, ui.id().with((tip, id)), egui::Sense::click());
        if b.hovered() {
            ui.painter().rect_filled(
                r,
                5.0,
                if selected {
                    Color32::from_white_alpha(40)
                } else {
                    p.hover
                },
            );
        }
        theme::paint_icon(ui.painter(), r.shrink(3.0), icon, fg2);
        b.on_hover_text(tip).clicked()
    };
    let eye = if n.visible { Icon::Eye } else { Icon::EyeSlash };
    if trailing(
        ui,
        eye,
        if n.visible { "Hide" } else { "Show" },
        hovered || !n.visible,
    ) {
        ed.edit(if n.visible { "Hide" } else { "Show" }, |tx| {
            tx.node_mut(id).unwrap().visible = !n.visible
        });
    }
    let lock = if n.locked { Icon::Lock } else { Icon::LockOpen };
    if trailing(
        ui,
        lock,
        if n.locked { "Unlock" } else { "Lock" },
        hovered || n.locked,
    ) {
        ed.edit(if n.locked { "Unlock" } else { "Lock" }, |tx| {
            tx.node_mut(id).unwrap().locked = !n.locked
        });
    }

    // Name, or an inline rename field.
    let name_rect =
        egui::Rect::from_min_max(egui::pos2(x, row.top()), egui::pos2(right, row.bottom()));
    let editing = st
        .renaming
        .as_ref()
        .is_some_and(|(target, _)| *target == id);
    if let Some((rid, name)) = &mut st.renaming
        && *rid == id
    {
        let escape = ui.input(|i| i.key_pressed(egui::Key::Escape));
        let response = ui.put(
            name_rect.shrink2(egui::vec2(0.0, 2.0)),
            egui::TextEdit::singleline(name).id(ui.id().with(("rename-layer", id))),
        );
        if let Some(confirmed) = finish_rename(ui, &response, escape) {
            let (_, name) = st.renaming.take().unwrap();
            let name = name.trim();
            if confirmed && !name.is_empty() && name != n.name {
                ed.edit("Rename", |tx| {
                    if let Some(node) = tx.node_mut(id) {
                        node.name = name.to_owned();
                    }
                });
            }
        }
    } else {
        let font = if n.is_frame() {
            theme::medium(theme::BODY)
        } else {
            egui::FontId::proportional(theme::BODY)
        };
        let mut job = egui::text::LayoutJob::simple_singleline(n.name.clone(), font, fg);
        job.wrap.max_width = name_rect.width().max(0.0);
        job.wrap.max_rows = 1;
        let galley = ui.painter().layout_job(job);
        let clip = ui
            .painter()
            .with_clip_rect(name_rect.intersect(ui.clip_rect()));
        clip.galley(egui::pos2(x, cy - galley.size().y / 2.0), galley, fg);
    }

    if !editing {
        resp.context_menu(|ui| {
            if ui.button("Rename").clicked() {
                st.renaming = Some((id, n.name.clone()));
                ui.close();
            }
            if ui
                .add_enabled(
                    !ed.doc.is_effectively_locked(id),
                    egui::Button::new("Delete"),
                )
                .clicked()
            {
                if !selected {
                    ed.select(vec![id]);
                }
                ed.run(Action::Delete);
                ui.close();
            }
        });
    }
    if !editing && resp.double_clicked() {
        st.renaming = Some((id, n.name.clone()));
    } else if !editing && resp.clicked() {
        ed.end_text_edit();
        if ui.input(|i| i.modifiers.shift || i.modifiers.command) {
            if selected {
                ed.selection.retain(|s| *s != id);
            } else {
                ed.selection.push(id);
            }
        } else {
            ed.selection = vec![id];
        }
    }
    if hovered {
        ed.hover = Some(id);
    }
    if filter.is_some() || !st.collapsed.contains(&id) {
        for c in n.children.iter().rev() {
            layer_row(ui, ed, st, *c, depth + 1, filter);
        }
    }
}

fn tool_icon(t: Tool) -> Icon {
    match t {
        Tool::Select => Icon::Pointer,
        Tool::Hand => Icon::Hand,
        Tool::Frame => Icon::Frame,
        Tool::Rect => Icon::Rect,
        Tool::Ellipse => Icon::Ellipse,
        Tool::Polygon => Icon::Polygon,
        Tool::Diamond => Icon::Diamond,
        Tool::Arrow => Icon::Arrow,
        Tool::Sticky => Icon::Sticky,
        Tool::Laser => Icon::Laser,
        Tool::Star => Icon::Star,
        Tool::Pen => Icon::Pen,
        Tool::Line => Icon::Line,
        Tool::Text => Icon::Text,
        Tool::Pencil => Icon::Pencil,
        Tool::Eraser => Icon::Eraser,
        Tool::Comment => Icon::Comment,
    }
}

fn status(ctx: &egui::Context, ed: &Editor, canvas: egui::Rect) {
    if ed.status.is_empty() {
        return;
    }
    egui::Area::new("status".into())
        .pivot(Align2::LEFT_BOTTOM)
        .fixed_pos(egui::pos2(canvas.left() + 12.0, canvas.bottom() - 12.0))
        .interactable(false)
        .show(ctx, |ui| {
            ui.label(RichText::new(&ed.status).weak().small());
        });
}

fn help(ctx: &egui::Context, st: &mut UiState, canvas: egui::Rect) {
    let width = 820.0f32.min(canvas.width() - 40.0);
    egui::Area::new("help".into())
        .order(egui::Order::Foreground)
        .fade_in(false)
        .pivot(Align2::CENTER_CENTER)
        .fixed_pos(canvas.center())
        .show(ctx, |ui| {
            let p = theme::pal(ui);
            egui::Frame::popup(ui.style())
                .fill(p.popover)
                .corner_radius(12.0)
                .inner_margin(16.0)
                .show(ui, |ui| {
                    ui.set_width(width);
                    ui.horizontal(|ui| {
                        ui.label(RichText::new("Keyboard shortcuts").font(theme::semibold(15.0)));
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if ui.button("Done").on_hover_text("Close (Esc)").clicked() {
                                st.show_help = false;
                            }
                        });
                    });
                    ui.add_space(6.0);
                    let sections = shortcuts::help_sections();
                    egui::ScrollArea::vertical()
                        .max_height((canvas.height() - 160.0).max(200.0))
                        .show(ui, |ui| {
                            ui.columns(3, |cols| {
                                // Balance columns by row count.
                                let total: usize = sections.iter().map(|(_, r)| r.len() + 2).sum();
                                let mut acc = 0;
                                for (i, (title, rows)) in sections.iter().enumerate() {
                                    let col = (acc * 3 / total.max(1)).min(2);
                                    acc += rows.len() + 2;
                                    let ui = &mut cols[col];
                                    ui.label(title_text(*title));
                                    egui::Grid::new(("keys", i)).num_columns(2).spacing([10.0, 3.0]).show(ui, |ui| {
                                        for (k, d) in rows {
                                            ui.label(
                                                RichText::new(k)
                                                    .font(theme::medium(11.5))
                                                    .color(p.secondary),
                                            );
                                            ui.label(RichText::new(*d).size(12.0));
                                            ui.end_row();
                                        }
                                    });
                                    ui.add_space(10.0);
                                }
                            });
                        });
                    ui.label(
                        RichText::new("Space-drag pans · wheel pans · ⌘/Ctrl+wheel or pinch zooms · ⌥-drag duplicates · hold ⌥ to measure")
                            .size(theme::SECONDARY)
                            .color(p.secondary),
                    );
                });
        });
    if ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
        st.show_help = false;
    }
}
