//! The editor workspace: document toolbar, sidebars and floating tool palette.

use super::{UiState, comments, inspector, layers, library, menu_bar, tool_icon};
use crate::editor::{AppRequest, Editor, Tool, shortcuts::Action};
use crate::theme::{self, Icon};
use egui::{
    Align, Align2, Button, Color32, Frame, Id, Layout, Rect, Response, RichText, Sense, Stroke,
    StrokeKind, Ui, UiBuilder, WidgetInfo, WidgetType, emath::easing, pos2, vec2,
};

const COMPACT_WIDTH: f32 = 1200.0;
const NARROW_WIDTH: f32 = 960.0;
const CONTROL_H: f32 = 34.0;
const PRIMARY_TOOLS: [Tool; 6] = [
    Tool::Pen,
    Tool::Hand,
    Tool::Frame,
    Tool::Rect,
    Tool::Text,
    Tool::Comment,
];

#[derive(Clone, Copy, Default, PartialEq, Eq)]
enum LeftTab {
    #[default]
    Design,
    Theme,
    Assets,
}

#[derive(Clone, Copy, Default, PartialEq, Eq)]
enum RightTab {
    #[default]
    Agent,
    Style,
}

#[derive(Default)]
pub struct WorkspaceUi {
    left_tab: LeftTab,
    right_tab: RightTab,
    left_hidden: bool,
    right_open: Option<bool>,
    preview_tool: Option<Tool>,
    pub(super) page_id: String,
    pub search: String,
}

impl WorkspaceUi {
    pub(super) fn show_layers(&mut self) {
        self.left_tab = LeftTab::Design;
        self.left_hidden = false;
        self.search.clear();
    }
}

pub(super) fn panels(ui: &mut Ui, ed: &mut Editor, st: &mut UiState) -> Rect {
    let p = theme::pal(ui);
    let width = ui.available_width();
    let compact = width < COMPACT_WIDTH;
    let narrow = width < NARROW_WIDTH;
    egui::Panel::top("workspace-top")
        .exact_size(theme::TOOLBAR_H)
        .frame(Frame::NONE.fill(p.window))
        .show(ui, |ui| top_bar(ui, ed, st, narrow));

    let mut left_open = !st.workspace.left_hidden;
    let left = egui::Panel::left(Id::new(("workspace-left", compact)))
        .resizable(true)
        .default_size(if compact { 210.0 } else { 256.0 })
        .size_range(180.0..=(width * 0.30).clamp(210.0, 400.0))
        .frame(Frame::NONE.fill(p.sidebar).inner_margin(12));
    sidebar(left, ui, &mut left_open, st.reduced_motion, |ui| {
        vertical_rule(ui, ui.max_rect().right() + 11.5);
        left_sidebar(ui, ed, st, compact);
    });
    st.workspace.left_hidden = !left_open;

    let right_default = st.workspace.right_open.unwrap_or(!narrow);
    let mut right_open = right_default;
    let right = egui::Panel::right(Id::new(("workspace-right", compact)))
        .resizable(true)
        .default_size(if compact { 232.0 } else { 312.0 })
        .size_range(220.0..=(width * 0.32).clamp(232.0, 420.0))
        .frame(Frame::NONE.fill(p.inspector).inner_margin(12));
    sidebar(right, ui, &mut right_open, st.reduced_motion, |ui| {
        vertical_rule(ui, ui.max_rect().left() - 11.5);
        right_sidebar(ui, ed, st);
    });
    if right_open != right_default {
        st.workspace.right_open = Some(right_open);
    }
    ui.available_rect_before_wrap()
}

fn sidebar(
    panel: egui::Panel,
    ui: &mut Ui,
    open: &mut bool,
    reduced: bool,
    draw: impl FnOnce(&mut Ui),
) {
    if reduced || ui.style().animation_time == 0.0 {
        if *open {
            panel.show(ui, draw);
        } else {
            ui.skip_ahead_auto_ids(1);
        }
    } else {
        panel.show_collapsible(ui, open, draw);
    }
}

fn vertical_rule(ui: &Ui, x: f32) {
    let rect = ui.max_rect().expand(12.0);
    ui.painter()
        .with_clip_rect(rect.intersect(ui.ctx().content_rect()))
        .vline(
            x,
            rect.y_range(),
            Stroke::new(1.0, theme::pal(ui).separator),
        );
}

fn horizontal_rule(ui: &mut Ui) {
    let rect = ui.available_rect_before_wrap();
    ui.painter().hline(
        rect.x_range(),
        rect.top(),
        Stroke::new(1.0, theme::pal(ui).separator),
    );
    ui.add_space(1.0);
}

fn group(ui: &mut Ui, rect: Rect, layout: Layout, draw: impl FnOnce(&mut Ui)) {
    let mut child = ui.new_child(UiBuilder::new().max_rect(rect).layout(layout));
    child.spacing_mut().item_spacing.x = 8.0;
    child.set_clip_rect(rect.intersect(ui.clip_rect()));
    draw(&mut child);
}

fn top_bar(ui: &mut Ui, ed: &mut Editor, st: &mut UiState, narrow: bool) {
    let p = theme::pal(ui);
    let r = ui.max_rect();
    let (reduced, left_on, right_on) = (
        st.reduced_motion,
        !st.workspace.left_hidden,
        st.workspace.right_open.unwrap_or(!narrow),
    );
    let leading = r.left()
        + if st.native_chrome {
            theme::TRAFFIC_LIGHTS_W
        } else {
            12.0
        };
    let center_width = if narrow { 148.0 } else { 220.0 };
    let center = Rect::from_center_size(r.center(), vec2(center_width, r.height()));

    group(
        ui,
        Rect::from_min_max(
            pos2(leading, r.top()),
            pos2(center.left() - 8.0, r.bottom()),
        ),
        Layout::left_to_right(Align::Center),
        |ui| {
            let dashboard = if narrow {
                icon_control(ui, Icon::Dashboard, "Dashboard", false, true, reduced)
            } else {
                let button =
                    ui.add(Button::new(title("      Dashboard")).min_size(vec2(126.0, CONTROL_H)));
                theme::paint_icon(
                    ui.painter(),
                    Rect::from_center_size(
                        pos2(button.rect.left() + 18.0, button.rect.center().y),
                        vec2(19.0, 19.0),
                    ),
                    Icon::Dashboard,
                    p.secondary,
                );
                button
            };
            if dashboard.clicked() {
                ed.run(Action::Home);
            }
            if icon_control(
                ui,
                Icon::Sidebar,
                if left_on {
                    "Hide navigator"
                } else {
                    "Show navigator"
                },
                false,
                true,
                reduced,
            )
            .clicked()
            {
                st.workspace.left_hidden = left_on;
            }
            let insert = icon_control(ui, Icon::Plus, "Insert", false, false, reduced);
            egui::Popup::menu(&insert).show(|ui| {
                for tool in [
                    Tool::Frame,
                    Tool::Rect,
                    Tool::Ellipse,
                    Tool::Text,
                    Tool::Pen,
                    Tool::Sticky,
                ] {
                    tool_menu_item(ui, ed, tool);
                }
                ui.separator();
                super::menu_item(ui, ed, "Place image…", Action::PlaceImage);
            });
            if !narrow {
                for tool in [Tool::Frame, Tool::Rect, Tool::Comment] {
                    tool_control(ui, ed, tool, false, reduced);
                }
            }
            if !st.native_chrome {
                let menus =
                    icon_control(ui, Icon::More, "Application menus", false, false, reduced);
                egui::Popup::menu(&menus).show(|ui| menu_bar(ui, ed));
            }
        },
    );

    group(ui, center, Layout::left_to_right(Align::Center), |ui| {
        let label = if st.project_name.is_empty() {
            &ed.doc.name
        } else {
            &st.project_name
        };
        let local_width = if narrow { 0.0 } else { 76.0 };
        let document = ui
            .add_sized(
                [ui.available_width() - local_width, CONTROL_H],
                Button::new(
                    RichText::new(format!("{label}  ⌄"))
                        .font(theme::semibold(theme::DOCUMENT_TITLE)),
                )
                .truncate(),
            )
            .on_hover_text(label);
        egui::Popup::menu(&document).show(|ui| {
            for (label, action) in [
                ("Open project…", Action::Open),
                ("Save", Action::Save),
                ("Save a copy…", Action::SaveAs),
                ("Keyboard shortcuts", Action::ToggleHelp),
            ] {
                super::menu_item(ui, ed, label, action);
            }
            ui.separator();
            for i in 0..ed.pages.len() {
                if ui
                    .selectable_label(ed.active_page == i, &ed.pages[i].name)
                    .clicked()
                {
                    ed.switch_page(i);
                    ui.close();
                }
            }
        });
        if !narrow {
            let state = if st.save_state.is_empty() {
                "Local"
            } else {
                &st.save_state
            };
            ui.label(
                RichText::new("●")
                    .color(Color32::from_rgb(72, 177, 96))
                    .size(8.0),
            );
            ui.add(
                egui::Label::new(
                    RichText::new(state)
                        .size(theme::SECONDARY)
                        .color(p.tertiary),
                )
                .truncate(),
            );
        }
    });

    group(
        ui,
        Rect::from_min_max(
            pos2(center.right() + 8.0, r.top()),
            pos2(r.right() - 12.0, r.bottom()),
        ),
        Layout::right_to_left(Align::Center),
        |ui| {
            if ui
                .add(
                    Button::new(title("Export").color(p.primary_text))
                        .fill(p.primary)
                        .stroke(Stroke::NONE)
                        .min_size(vec2(56.0, CONTROL_H)),
                )
                .clicked()
            {
                ed.requests.push(AppRequest::Export(st.export));
            }
            let share = ui.add(Button::new("Share").min_size(vec2(54.0, CONTROL_H)));
            egui::Popup::menu(&share).show(|ui| {
                super::menu_item(ui, ed, "Save a copy…", Action::SaveAs);
                super::menu_item(ui, ed, "Copy as PNG", Action::CopyAsPng);
                super::menu_item(ui, ed, "Copy as SVG", Action::CopyAsSvg);
                if let Some(path) = &ed.project_dir
                    && ui.button("Copy local project path").clicked()
                {
                    ui.ctx().copy_text(path.to_string_lossy().into_owned());
                    ed.status = "Copied project path".into();
                    ui.close();
                }
            });
            let preview = if r.width() >= COMPACT_WIDTH {
                ui.add(Button::new("Preview").min_size(vec2(72.0, CONTROL_H)))
            } else {
                icon_control(ui, Icon::Play, "Preview", false, true, reduced)
            };
            if preview.clicked() {
                st.workspace.preview_tool = Some(ed.tool);
                ed.run(Action::Tool(Tool::Hand));
                st.preview = true;
            }
            if icon_control(
                ui,
                Icon::SidebarRight,
                if right_on {
                    "Hide inspector"
                } else {
                    "Show inspector"
                },
                false,
                false,
                reduced,
            )
            .clicked()
            {
                st.workspace.right_open = Some(!right_on);
            }
        },
    );
    ui.painter()
        .hline(r.x_range(), r.bottom() - 0.5, Stroke::new(1.0, p.separator));
    ui.allocate_rect(r, Sense::hover());
}

fn left_sidebar(ui: &mut Ui, ed: &mut Editor, st: &mut UiState, compact: bool) {
    let mut tab = st.workspace.left_tab as usize;
    segments(
        ui,
        "navigator-tabs",
        &mut tab,
        &["Design", "Theme", "Assets"],
        st.reduced_motion,
    );
    st.workspace.left_tab = [LeftTab::Design, LeftTab::Theme, LeftTab::Assets][tab];
    ui.add_space(4.0);
    horizontal_rule(ui);
    ui.add_space(5.0);

    let content = ui.available_rect_before_wrap();
    let footer = Rect::from_min_max(pos2(content.left(), content.bottom() - 31.0), content.max);
    let mut body = ui.new_child(
        UiBuilder::new()
            .max_rect(Rect::from_min_max(
                content.min,
                pos2(content.right(), footer.top() - 9.0),
            ))
            .layout(Layout::top_down(Align::Min)),
    );
    body.set_clip_rect(body.max_rect().intersect(ui.clip_rect()));
    match st.workspace.left_tab {
        LeftTab::Design => {
            search_field(&mut body, &mut st.workspace.search, compact);
            layers(&mut body, ed, st, compact);
        }
        LeftTab::Theme => inspector::tokens_panel(&mut body, ed),
        LeftTab::Assets => library::panel(&mut body, ed, st),
    }
    group(ui, footer, Layout::left_to_right(Align::Center), |ui| {
        let p = theme::pal(ui);
        ui.painter().hline(
            footer.x_range(),
            footer.top() + 0.5,
            Stroke::new(1.0, p.separator),
        );
        let button = ui
            .add_enabled_ui(ed.project_dir.is_some(), |ui| {
                let icon = icon_control(
                    ui,
                    Icon::Folder,
                    "Project files",
                    false,
                    false,
                    st.reduced_motion,
                );
                let label = ui.add(
                    Button::new(RichText::new("Project files").color(p.secondary))
                        .frame(false)
                        .min_size(vec2(0.0, CONTROL_H)),
                );
                icon.union(label)
            })
            .inner
            .on_disabled_hover_text("Project files are available after saving a project");
        if button.clicked()
            && let Some(dir) = &ed.project_dir
        {
            ed.requests.push(AppRequest::OpenFile(dir.clone()));
        }
    });
}

fn search_field(ui: &mut Ui, query: &mut String, compact: bool) {
    let p = theme::pal(ui);
    Frame::NONE
        .fill(p.field)
        .stroke(Stroke::new(1.0, p.control_border))
        .corner_radius(theme::CONTROL_RADIUS)
        .inner_margin(egui::Margin::symmetric(7, 3))
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 6.0;
                let r = ui.allocate_exact_size(vec2(16.0, 24.0), Sense::hover()).0;
                theme::paint_icon(ui.painter(), r, Icon::Search, p.secondary);
                let hint = if compact {
                    "Search"
                } else {
                    "Search pages and layers"
                };
                let edit = ui.add(
                    egui::TextEdit::singleline(query)
                        .hint_text(hint)
                        .desired_width(f32::INFINITY)
                        .frame(Frame::NONE),
                );
                if edit.has_focus() && ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                    query.clear();
                    edit.surrender_focus();
                }
            });
        });
    ui.add_space(4.0);
}

fn right_sidebar(ui: &mut Ui, ed: &mut Editor, st: &mut UiState) {
    let mut tab = st.workspace.right_tab as usize;
    ui.scope(|ui| {
        ui.set_max_width(112.0);
        segments(
            ui,
            "inspector-tabs",
            &mut tab,
            &["Agent", "Style"],
            st.reduced_motion,
        );
    });
    st.workspace.right_tab = [RightTab::Agent, RightTab::Style][tab];
    ui.add_space(4.0);
    horizontal_rule(ui);
    ui.add_space(5.0);
    match st.workspace.right_tab {
        RightTab::Agent => comments::agent_panel(ui, ed, &mut st.comments),
        RightTab::Style => inspector::panel(ui, ed, st),
    }
}

#[derive(Clone, Copy)]
struct TabMotion {
    start: f64,
    from: f32,
    to: usize,
}

impl TabMotion {
    const SECONDS: f64 = 0.22;

    fn position(self, now: f64) -> Option<f32> {
        let t = ((now - self.start) / Self::SECONDS).clamp(0.0, 1.0) as f32;
        (t < 1.0).then(|| egui::lerp(self.from..=self.to as f32, easing::cubic_in_out(t)))
    }
}

fn segments(ui: &mut Ui, key: &str, selected: &mut usize, labels: &[&str], reduced: bool) {
    let p = theme::pal(ui);
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), CONTROL_H), Sense::hover());
    let inside = rect.shrink(3.0);
    let width = inside.width() / labels.len() as f32;
    let cell = |i: f32| {
        Rect::from_min_size(
            inside.min + vec2(width * i, 0.0),
            vec2(width, inside.height()),
        )
    };
    let id = ui.id().with(key);
    let time = ui.input(|i| i.time);
    ui.painter()
        .rect_filled(rect, theme::PANEL_RADIUS, p.window);
    let mut motion = ui
        .data(|d| d.get_temp::<TabMotion>(id))
        .filter(|m| !reduced && m.to == *selected);
    for (index, label) in labels.iter().enumerate() {
        let resp = ui.interact(cell(index as f32), id.with(index), Sense::click());
        resp.widget_info(|| {
            WidgetInfo::selected(
                WidgetType::SelectableLabel,
                true,
                *selected == index,
                *label,
            )
        });
        if *selected != index && resp.clicked() {
            let from = motion
                .and_then(|m| m.position(time))
                .unwrap_or(*selected as f32);
            *selected = index;
            motion =
                (!reduced && resp.clicked_by(egui::PointerButton::Primary)).then_some(TabMotion {
                    start: time,
                    from,
                    to: index,
                });
        }
        if !touch_active(ui) && resp.hovered() && index != *selected {
            ui.painter().rect_filled(
                cell(index as f32).shrink(1.0),
                theme::CONTROL_RADIUS,
                p.hover,
            );
        }
        if resp.has_focus() {
            ui.painter().rect_stroke(
                cell(index as f32).shrink(1.0),
                theme::CONTROL_RADIUS,
                Stroke::new(1.0, p.focus),
                StrokeKind::Outside,
            );
        }
    }
    let pos = motion.and_then(|m| m.position(time));
    ui.data_mut(|d| {
        if let Some(m) = motion.filter(|_| pos.is_some()) {
            d.insert_temp(id, m);
        } else {
            d.remove::<TabMotion>(id);
        }
    });
    if pos.is_some() {
        ui.ctx().request_repaint();
    }
    let active = cell(pos.unwrap_or(*selected as f32)).shrink(1.0);
    ui.painter()
        .rect_filled(active, theme::CONTROL_RADIUS, p.selected);
    ui.painter().rect_stroke(
        active,
        theme::CONTROL_RADIUS,
        Stroke::new(1.0, p.control_border),
        StrokeKind::Inside,
    );
    for (index, label) in labels.iter().enumerate() {
        let fg = if index == *selected {
            p.text
        } else {
            p.secondary
        };
        ui.painter().text(
            cell(index as f32).center(),
            Align2::CENTER_CENTER,
            label,
            if index == *selected {
                theme::semibold(theme::CONTROL_LABEL)
            } else {
                egui::FontId::proportional(theme::CONTROL_LABEL)
            },
            fg,
        );
    }
}

fn touch_active(ui: &Ui) -> bool {
    ui.input(|i| i.any_touches())
}

fn title(text: impl Into<String>) -> RichText {
    RichText::new(text).font(theme::semibold(theme::CONTROL_LABEL))
}

fn icon_control(
    ui: &mut Ui,
    icon: Icon,
    label: &str,
    selected: bool,
    framed: bool,
    reduced: bool,
) -> Response {
    let p = theme::pal(ui);
    let (rect, reply) = ui.allocate_exact_size(vec2(CONTROL_H, CONTROL_H), Sense::click());
    reply
        .widget_info(|| WidgetInfo::selected(WidgetType::Button, ui.is_enabled(), selected, label));
    let down = reply.is_pointer_button_down_on();
    let duration = if reduced || down || ui.style().animation_time == 0.0 {
        0.0
    } else {
        0.10
    };
    let press = ui.ctx().animate_bool_with_time_and_easing(
        reply.id.with("press"),
        down,
        duration,
        easing::cubic_out,
    );
    let base = if selected {
        p.primary
    } else if framed {
        p.control
    } else {
        Color32::TRANSPARENT
    };
    ui.painter().rect_filled(rect, theme::CONTROL_RADIUS, base);
    if !touch_active(ui) && (reply.hovered() || reply.has_focus()) {
        ui.painter()
            .rect_filled(rect, theme::CONTROL_RADIUS, p.hover);
    }
    if press > 0.0 {
        ui.painter()
            .rect_filled(rect, theme::CONTROL_RADIUS, p.pressed.gamma_multiply(press));
    }
    if framed || reply.has_focus() {
        ui.painter().rect_stroke(
            rect,
            theme::CONTROL_RADIUS,
            Stroke::new(
                1.0,
                if reply.has_focus() {
                    p.focus
                } else {
                    p.control_border
                },
            ),
            StrokeKind::Inside,
        );
    }
    let fg = if selected {
        p.primary_text
    } else if ui.is_enabled() {
        p.secondary
    } else {
        p.tertiary
    };
    let glyph = Rect::from_center_size(rect.center(), egui::Vec2::splat(theme::TOOL_GLYPH));
    theme::paint_icon(ui.painter(), glyph, icon, fg);
    reply.on_hover_text(label)
}

fn tool_control(ui: &mut Ui, ed: &mut Editor, tool: Tool, framed: bool, reduced: bool) {
    let label = format!("{} ({})", tool.label(), tool.shortcut());
    if icon_control(
        ui,
        tool_icon(tool),
        &label,
        ed.tool == tool,
        framed,
        reduced,
    )
    .clicked()
    {
        ed.run(Action::Tool(tool));
    }
}

fn tool_menu_item(ui: &mut Ui, ed: &mut Editor, tool: Tool) {
    super::menu_item(ui, ed, tool.label(), Action::Tool(tool));
}

fn zoom_control(ui: &mut Ui, ed: &mut Editor, light_canvas: &mut bool) {
    let zoom = ui
        .add(
            Button::new(title(format!("{:.0}%  ⌄", ed.camera.zoom * 100.0)))
                .frame(false)
                .min_size(vec2(65.0, CONTROL_H)),
        )
        .on_hover_text("Canvas zoom");
    egui::Popup::menu(&zoom).show(|ui| {
        for (label, action) in [
            ("Zoom in", Action::ZoomIn),
            ("Zoom out", Action::ZoomOut),
            ("Zoom to 100%", Action::Zoom100),
            ("Zoom to fit", Action::ZoomFit),
            ("Zoom to selection", Action::ZoomSelection),
        ] {
            super::menu_item(ui, ed, label, action);
        }
        ui.separator();
        if ui.checkbox(light_canvas, "Light canvas").changed() {
            ui.close();
        }
    });
}

pub(super) fn toolbar(ctx: &egui::Context, ed: &mut Editor, st: &mut UiState, canvas: Rect) {
    let roomy = canvas.width() >= 355.0;
    egui::Area::new(Id::new("workspace-tools"))
        .fade_in(false)
        .pivot(Align2::CENTER_BOTTOM)
        .fixed_pos(pos2(canvas.center().x, canvas.bottom() - 21.0))
        .show(ctx, |ui| {
            let p = theme::pal(ui);
            Frame::NONE
                .fill(p.popover)
                .stroke(Stroke::new(1.0, p.separator))
                .corner_radius(theme::PANEL_RADIUS)
                .inner_margin(6)
                .show(ui, |ui| {
                    ui.horizontal_centered(|ui| {
                        ui.spacing_mut().item_spacing.x = 4.0;
                        for tool in PRIMARY_TOOLS {
                            if roomy || matches!(tool, Tool::Pen | Tool::Hand | Tool::Frame) {
                                tool_control(ui, ed, tool, true, st.reduced_motion);
                            }
                        }
                        let extra_tool = !PRIMARY_TOOLS.contains(&ed.tool)
                            || (!roomy
                                && matches!(ed.tool, Tool::Rect | Tool::Text | Tool::Comment));
                        // Select is the default tool. Keep it visible in the menu and put the
                        // Pen glyph in the first slot, as in the workspace design.
                        let extra = icon_control(
                            ui,
                            Icon::More,
                            "More tools",
                            extra_tool && ed.tool != Tool::Select,
                            true,
                            st.reduced_motion,
                        );
                        egui::Popup::menu(&extra).show(|ui| {
                            for tool in Tool::ALL {
                                if !PRIMARY_TOOLS.contains(&tool)
                                    || (!roomy
                                        && matches!(tool, Tool::Rect | Tool::Text | Tool::Comment))
                                {
                                    tool_menu_item(ui, ed, tool);
                                }
                            }
                        });
                        let (r, _) = ui.allocate_exact_size(vec2(1.0, CONTROL_H), Sense::hover());
                        ui.painter().vline(
                            r.center().x,
                            r.y_range().shrink(2.0),
                            Stroke::new(1.0, p.separator),
                        );
                        zoom_control(ui, ed, &mut st.light_canvas);
                    });
                });
        });
}

pub(super) fn preview_controls(
    ctx: &egui::Context,
    canvas: Rect,
    ed: &mut Editor,
    st: &mut UiState,
) {
    egui::Area::new(Id::new("workspace-preview"))
        .fade_in(false)
        .pivot(Align2::CENTER_BOTTOM)
        .fixed_pos(pos2(canvas.center().x, canvas.bottom() - 21.0))
        .show(ctx, |ui| {
            Frame::popup(ui.style()).show(ui, |ui| {
                ui.horizontal(|ui| {
                    if ui.button("Exit preview").on_hover_text("Escape").clicked() {
                        st.preview = false;
                    }
                    zoom_control(ui, ed, &mut st.light_canvas);
                });
            });
        });
    if ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
        st.preview = false;
    }
    if !st.preview
        && let Some(tool) = st.workspace.preview_tool.take()
    {
        ed.run(Action::Tool(tool));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn navigator_slide_reverses_and_respects_reduced_motion() {
        let ctx = egui::Context::default();
        theme::apply(&ctx);
        let mut ed = Editor::default();
        let mut st = UiState::default();
        let mut x = |time, st: &mut UiState| {
            let mut layout = crate::render::frame::layout(
                &ctx,
                egui::RawInput {
                    screen_rect: Some(Rect::from_min_size(egui::Pos2::ZERO, vec2(1440.0, 900.0))),
                    time: Some(time),
                    ..Default::default()
                },
                &mut ed,
                st,
            );
            layout.output.textures_delta.clear();
            layout.canvas.left()
        };
        let open = x(0.0, &mut st);
        assert!(open > 240.0);
        st.workspace.left_hidden = true;
        x(0.016, &mut st);
        let closing = x(0.09, &mut st);
        assert!(closing > 0.0 && closing < open, "closing x = {closing}");
        st.workspace.left_hidden = false;
        let reversing = x(0.106, &mut st);
        assert!(
            reversing > 0.0 && reversing < open,
            "reversing x = {reversing}"
        );
        assert_eq!(x(0.40, &mut st), open);
        st.reduced_motion = true;
        st.workspace.left_hidden = true;
        assert_eq!(x(0.416, &mut st), 0.0);
    }
}
