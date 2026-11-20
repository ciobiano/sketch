//! Comments in the window: pins on the canvas, the thread popover (replies, reactions,
//! attachments, @mentions) and the Comments panel in the left sidebar.

use crate::comments::{
    self, Attachment, Author, AuthorKind, Draft, Pin, REACTIONS, Status, Thread,
};
use crate::editor::{AppRequest, Editor};
use crate::geom::V2;
use crate::model::NodeId;
use crate::project::now_ms;
use crate::theme::{self, Icon};
use egui::{Color32, RichText, Ui, pos2, vec2};

const PIN: f32 = 26.0;
const POPOVER_W: f32 = 300.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Filter {
    #[default]
    Open,
    Resolved,
    All,
}

impl Filter {
    fn shows(self, s: Status) -> bool {
        match self {
            Filter::Open => s == Status::Open,
            Filter::Resolved => s == Status::Resolved,
            Filter::All => true,
        }
    }
}

/// Text and files of a message being written.
#[derive(Debug, Default)]
pub struct Composer {
    pub text: String,
    pub files: Vec<Attachment>,
}

impl Composer {
    fn take(&mut self) -> Draft {
        Draft {
            text: std::mem::take(&mut self.text),
            attachments: std::mem::take(&mut self.files),
        }
    }
    /// Throw the draft away, with the files already copied into the project.
    fn discard(&mut self, ed: &Editor) {
        if let Some(dir) = &ed.project_dir {
            comments::remove_files(dir, &self.files);
        }
        *self = Composer::default();
    }
}

#[derive(Debug)]
pub struct NewThread {
    pub pin: Pin,
    pub composer: Composer,
}

#[derive(Debug, Default)]
pub struct CommentsUi {
    /// Thread whose popover is open.
    pub open: Option<String>,
    /// Thread being started with the Comment tool.
    pub new: Option<NewThread>,
    /// Reply being written in the open thread.
    pub reply: Composer,
    pub filter: Filter,
    /// Display name being edited in the panel.
    pub name_edit: Option<String>,
    /// Focus the name field on the next frame.
    name_focus: bool,
    /// Focus the composer on the next frame.
    focus: bool,
}

impl CommentsUi {
    /// The Comment tool clicked the canvas: start a thread there.
    pub fn start(&mut self, ed: &mut Editor, node: Option<NodeId>, world: V2) {
        self.close(ed);
        let page = ed.active_page_id().to_string();
        self.new = Some(NewThread {
            pin: Pin::new(&ed.doc, &page, node, world),
            composer: Composer::default(),
        });
        self.focus = true;
    }

    /// Open a thread's popover.
    pub fn show(&mut self, ed: &mut Editor, id: &str) {
        if self.open.as_deref() != Some(id) {
            self.close(ed);
            self.open = Some(id.to_string());
            self.focus = true;
        }
    }

    pub fn close(&mut self, ed: &Editor) {
        if let Some(mut n) = self.new.take() {
            n.composer.discard(ed);
        }
        self.reply.discard(ed);
        self.open = None;
    }
}

/// A stable colour per author for pins and avatars.
fn author_color(id: &str) -> Color32 {
    let h = id
        .bytes()
        .fold(5381u32, |h, b| h.wrapping_mul(33) ^ u32::from(b));
    egui::ecolor::Hsva::new((h % 360) as f32 / 360.0, 0.55, 0.82, 1.0).into()
}

fn author(ed: &Editor, id: &str) -> Author {
    if id == ed.me.id {
        return ed.me.clone();
    }
    ed.comments
        .author(id)
        .cloned()
        .unwrap_or_else(|| Author::person(id, id))
}

fn avatar(ui: &mut Ui, a: &Author, size: f32) {
    let (r, _) = ui.allocate_exact_size(vec2(size, size), egui::Sense::hover());
    let fill = if a.kind == AuthorKind::Agent {
        theme::accent()
    } else {
        author_color(&a.id)
    };
    ui.painter().circle_filled(r.center(), size / 2.0, fill);
    ui.painter().text(
        r.center(),
        egui::Align2::CENTER_CENTER,
        a.initial(),
        theme::semibold(size * 0.55),
        Color32::WHITE,
    );
}

fn size_label(n: u64) -> String {
    match n {
        0..1024 => format!("{n} B"),
        1024..1_048_576 => format!("{:.0} KB", n as f64 / 1024.0),
        _ => format!("{:.1} MB", n as f64 / 1_048_576.0),
    }
}

/// Message text with @mentions in the accent colour.
fn rich_text(ui: &Ui, text: &str, known: &[Author]) -> egui::text::LayoutJob {
    let p = theme::pal(ui);
    let plain = egui::TextFormat {
        font_id: egui::FontId::proportional(theme::BODY),
        color: p.text,
        ..Default::default()
    };
    let mention = egui::TextFormat {
        font_id: theme::semibold(theme::BODY),
        color: theme::accent(),
        ..Default::default()
    };
    let mut job = egui::text::LayoutJob::default();
    let mut at = 0;
    for r in comments::mention_spans(text, known) {
        job.append(&text[at..r.start], 0.0, plain.clone());
        job.append(&text[r.clone()], 0.0, mention.clone());
        at = r.end;
    }
    job.append(&text[at..], 0.0, plain);
    job.wrap.max_width = POPOVER_W - 40.0;
    job
}

enum Act {
    React(String, String, String),
    DeleteMessage(String, String),
    DeleteThread(String),
    Status(String, Status),
    Reply(String),
    Post,
    Open(std::path::PathBuf),
    Close,
}

/// Add files the user picked to the composer that asked for them.
fn take_picked(ed: &mut Editor, st: &mut CommentsUi) {
    if ed.picked_files.is_empty() {
        return;
    }
    let files = std::mem::take(&mut ed.picked_files);
    let Some(dir) = ed.project_dir.clone() else {
        ed.status = "Attachments need a saved project".into();
        return;
    };
    let target = match &mut st.new {
        Some(n) => &mut n.composer,
        None => &mut st.reply,
    };
    for f in files {
        match comments::attach(&dir, &f) {
            Ok(a) => target.files.push(a),
            Err(e) => ed.status = e,
        }
    }
    st.focus = true;
}

/// Pins and popovers over the canvas.
pub fn canvas(ctx: &egui::Context, ed: &mut Editor, st: &mut CommentsUi, canvas: egui::Rect) {
    take_picked(ed, st);
    if st
        .open
        .as_deref()
        .is_some_and(|id| ed.comments.thread(id).is_none())
        || st
            .new
            .as_ref()
            .is_some_and(|n| n.pin.page != ed.active_page_id())
    {
        st.close(ed);
    }
    let page = ed.active_page_id().to_string();
    let to_screen = |w: V2, ed: &Editor| {
        let s = ed.camera.world_to_screen(w);
        canvas.min + vec2(s.x as f32, s.y as f32)
    };
    let mut acts = Vec::new();
    let pins: Vec<(String, egui::Pos2, Author, Status, String)> = ed
        .comments
        .threads
        .iter()
        .filter(|t| t.page == page && !t.messages.is_empty())
        .filter(|t| st.filter.shows(t.status) || st.open.as_deref() == Some(&t.id))
        .map(|t| {
            let preview: String = t.messages[0].text.chars().take(80).collect();
            (
                t.id.clone(),
                to_screen(t.world_point(&ed.doc), ed),
                author(ed, t.author()),
                t.status,
                preview,
            )
        })
        .collect();
    for (id, at, a, status, preview) in &pins {
        if !canvas.expand(PIN).contains(*at) {
            continue;
        }
        let open = st.open.as_deref() == Some(id.as_str());
        let fill = match status {
            Status::Resolved => Color32::from_gray(150),
            Status::Open if a.kind == AuthorKind::Agent => theme::accent(),
            Status::Open => author_color(&a.id),
        };
        let r = pin(ctx, id, *at, &a.initial(), fill, open);
        if r.clicked() {
            if open {
                acts.push(Act::Close);
            } else {
                st.show(ed, id);
            }
        } else if !open {
            r.on_hover_text(format!("{}: {preview}", a.name));
        }
    }
    if let Some(n) = &st.new {
        let at = to_screen(n.pin.point, ed);
        pin(
            ctx,
            "new",
            at,
            &ed.me.initial(),
            author_color(&ed.me.id),
            true,
        );
        popover(ctx, at, canvas, |ui| new_thread(ui, ed, st, &mut acts));
    } else if let Some(id) = st.open.clone()
        && let Some(t) = ed.comments.thread(&id).cloned()
    {
        let at = to_screen(t.world_point(&ed.doc), ed);
        popover(ctx, at, canvas, |ui| thread_view(ui, ed, st, &t, &mut acts));
    }
    if (st.new.is_some() || st.open.is_some()) && ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
        acts.push(Act::Close);
    }
    apply(ed, st, acts);
}

fn pin(
    ctx: &egui::Context,
    id: &str,
    at: egui::Pos2,
    initial: &str,
    fill: Color32,
    open: bool,
) -> egui::Response {
    egui::Area::new(egui::Id::new(("comment-pin", id)))
        .fixed_pos(at - vec2(0.0, PIN))
        .order(egui::Order::Middle)
        .show(ctx, |ui| {
            let (r, resp) = ui.allocate_exact_size(vec2(PIN, PIN), egui::Sense::click());
            let c = r.center();
            let rad = PIN / 2.0 - 1.0;
            let p = ui.painter();
            // A round pin whose bottom-left corner is the point it marks.
            p.add(egui::Shape::convex_polygon(
                vec![r.left_bottom(), c + vec2(-rad, 0.0), c + vec2(0.0, rad)],
                fill,
                egui::Stroke::NONE,
            ));
            p.circle_filled(c, rad, fill);
            let ring = if open || resp.hovered() { 2.5 } else { 1.5 };
            p.circle_stroke(c, rad, egui::Stroke::new(ring, Color32::WHITE));
            p.text(
                c,
                egui::Align2::CENTER_CENTER,
                initial,
                theme::semibold(11.0),
                Color32::WHITE,
            );
            resp.on_hover_cursor(egui::CursorIcon::PointingHand)
        })
        .inner
}

fn popover(ctx: &egui::Context, at: egui::Pos2, canvas: egui::Rect, body: impl FnOnce(&mut Ui)) {
    // Right of the pin, or left of it when the canvas has no room there, so it never covers
    // the pin it belongs to.
    let width = POPOVER_W + 24.0;
    let right = at.x + PIN + 8.0;
    let x = if right + width <= canvas.right() - 8.0 {
        right
    } else {
        at.x - width - 8.0
    };
    egui::Area::new("comment-popover".into())
        .fixed_pos(pos2(x, at.y - PIN - 4.0))
        .order(egui::Order::Foreground)
        .constrain_to(canvas.shrink(8.0))
        .show(ctx, |ui| {
            let p = theme::pal(ui);
            egui::Frame::popup(ui.style())
                .fill(p.popover)
                .corner_radius(10.0)
                .inner_margin(12.0)
                .show(ui, |ui| {
                    ui.set_width(POPOVER_W);
                    body(ui)
                });
        });
}

fn new_thread(ui: &mut Ui, ed: &mut Editor, st: &mut CommentsUi, acts: &mut Vec<Act>) {
    let p = theme::pal(ui);
    ui.horizontal(|ui| {
        ui.label(RichText::new("New comment").font(theme::semibold(theme::BODY)));
        if let Some(n) = st
            .new
            .as_ref()
            .and_then(|n| n.pin.node)
            .and_then(|id| ed.doc.get(id))
        {
            ui.label(
                RichText::new(format!("on {}", n.name))
                    .color(p.secondary)
                    .small(),
            );
        }
        close_button(ui, acts);
    });
    ui.add_space(6.0);
    let focus = std::mem::take(&mut st.focus);
    let known = ed.comments.mentionable(&ed.me);
    let can_attach = ed.project_dir.is_some();
    let Some(n) = st.new.as_mut() else { return };
    if composer(
        ui,
        ed,
        &mut n.composer,
        &known,
        "Add a comment… @ to mention",
        "Comment",
        focus,
        can_attach,
    ) {
        acts.push(Act::Post);
    }
}

fn close_button(ui: &mut Ui, acts: &mut Vec<Act>) {
    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
        if theme::icon_button(ui, Icon::Close, 20.0, false)
            .on_hover_text("Close (Esc)")
            .clicked()
        {
            acts.push(Act::Close);
        }
    });
}

fn thread_view(ui: &mut Ui, ed: &mut Editor, st: &mut CommentsUi, t: &Thread, acts: &mut Vec<Act>) {
    let p = theme::pal(ui);
    ui.horizontal(|ui| {
        let resolved = t.status == Status::Resolved;
        ui.label(
            RichText::new(if resolved { "Resolved" } else { "Comment" })
                .font(theme::semibold(theme::BODY)),
        );
        if let Some(n) = t.node_id().and_then(|id| ed.doc.get(id)) {
            ui.label(
                RichText::new(format!("on {}", n.name))
                    .color(p.secondary)
                    .small(),
            );
        }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.spacing_mut().item_spacing.x = 2.0;
            if theme::icon_button(ui, Icon::Close, 20.0, false)
                .on_hover_text("Close (Esc)")
                .clicked()
            {
                acts.push(Act::Close);
            }
            if theme::icon_button(ui, Icon::Trash, 20.0, false)
                .on_hover_text("Delete thread")
                .clicked()
            {
                acts.push(Act::DeleteThread(t.id.clone()));
            }
            let (tip, next) = if resolved {
                ("Reopen", Status::Open)
            } else {
                ("Resolve", Status::Resolved)
            };
            if theme::icon_button(ui, Icon::Check, 20.0, resolved)
                .on_hover_text(tip)
                .clicked()
            {
                acts.push(Act::Status(t.id.clone(), next));
            }
        });
    });
    if let Some(by) = &t.resolved_by
        && t.status == Status::Resolved
    {
        ui.label(
            RichText::new(format!("Resolved by {}", author(ed, by).name))
                .color(p.secondary)
                .small(),
        );
    }
    ui.add_space(4.0);
    let known = ed.comments.mentionable(&ed.me);
    let now = now_ms();
    egui::ScrollArea::vertical()
        .max_height(340.0)
        .auto_shrink([false, true])
        .stick_to_bottom(true)
        .show(ui, |ui| {
            for (i, m) in t.messages.iter().enumerate() {
                if i > 0 {
                    ui.add_space(8.0);
                }
                let a = author(ed, &m.author);
                let row = ui.vertical(|ui| {
                    ui.horizontal(|ui| {
                        avatar(ui, &a, 18.0);
                        ui.label(RichText::new(&a.name).font(theme::semibold(theme::BODY)));
                        if a.kind == AuthorKind::Agent {
                            ui.label(RichText::new("agent").color(p.secondary).small());
                        }
                        let when = comments::ago(m.created, now);
                        let edited = if m.edited.is_some() { " · edited" } else { "" };
                        ui.label(
                            RichText::new(format!("{when}{edited}"))
                                .color(p.tertiary)
                                .small(),
                        )
                        .on_hover_text(comments::iso(m.created));
                    });
                    if !m.text.is_empty() {
                        ui.label(rich_text(ui, &m.text, &known));
                    }
                    for f in &m.attachments {
                        let label = format!("📎 {} · {}", f.name, size_label(f.size));
                        let r = ui.add(
                            egui::Button::new(RichText::new(label).small()).corner_radius(6.0),
                        );
                        if r.on_hover_text("Open").clicked()
                            && let Some(dir) = &ed.project_dir
                        {
                            acts.push(Act::Open(dir.join(&f.file)));
                        }
                    }
                    ui.horizontal_wrapped(|ui| {
                        ui.spacing_mut().item_spacing.x = 4.0;
                        for (emoji, who) in &m.reactions {
                            let mine = who.contains(&ed.me.id);
                            let names: Vec<String> =
                                who.iter().map(|w| author(ed, w).name).collect();
                            let r = ui.add(
                                egui::Button::new(
                                    RichText::new(format!("{emoji} {}", who.len())).size(14.0),
                                )
                                .selected(mine)
                                .corner_radius(10.0),
                            );
                            if r.on_hover_text(names.join(", ")).clicked() {
                                acts.push(Act::React(t.id.clone(), m.id.clone(), emoji.clone()));
                            }
                        }
                        ui.menu_button(RichText::new("☺ +").small().color(p.secondary), |ui| {
                            ui.horizontal(|ui| {
                                for e in REACTIONS {
                                    if ui.button(RichText::new(e).size(20.0)).clicked() {
                                        acts.push(Act::React(
                                            t.id.clone(),
                                            m.id.clone(),
                                            e.to_string(),
                                        ));
                                        ui.close();
                                    }
                                }
                            });
                        })
                        .response
                        .on_hover_text("Add reaction");
                    });
                });
                if m.author == ed.me.id {
                    row.response.context_menu(|ui| {
                        let label = if i == 0 {
                            "Delete thread"
                        } else {
                            "Delete reply"
                        };
                        if ui.button(label).clicked() {
                            acts.push(Act::DeleteMessage(t.id.clone(), m.id.clone()));
                            ui.close();
                        }
                    });
                }
            }
        });
    ui.add_space(8.0);
    ui.separator();
    let focus = std::mem::take(&mut st.focus);
    let can_attach = ed.project_dir.is_some();
    if composer(
        ui,
        ed,
        &mut st.reply,
        &known,
        "Reply… @ to mention",
        "Reply",
        focus,
        can_attach,
    ) {
        acts.push(Act::Reply(t.id.clone()));
    }
}

/// The text box, mention suggestions, attachments and send button. Returns true when the
/// message should be sent (button, or Return; Shift+Return adds a line).
#[allow(clippy::too_many_arguments)]
fn composer(
    ui: &mut Ui,
    ed: &mut Editor,
    c: &mut Composer,
    known: &[Author],
    hint: &str,
    send: &str,
    focus: bool,
    can_attach: bool,
) -> bool {
    let p = theme::pal(ui);
    let edit = egui::TextEdit::multiline(&mut c.text)
        .hint_text(hint)
        .desired_rows(2)
        .desired_width(f32::INFINITY)
        .return_key(egui::KeyboardShortcut::new(
            egui::Modifiers::SHIFT,
            egui::Key::Enter,
        ));
    let r = ui.add(edit);
    if focus {
        r.request_focus();
    }
    let mut sent = r.has_focus()
        && ui.input(|i| i.key_pressed(egui::Key::Enter) && !i.modifiers.shift)
        && !Draft::text(&c.text).is_empty();
    // Suggest names while an @word is being typed at the end of the text.
    if let Some(at) = c.text.rfind('@') {
        let tail = &c.text[at + 1..];
        let starts_word = c.text[..at]
            .chars()
            .next_back()
            .is_none_or(char::is_whitespace);
        if starts_word && tail.len() <= 24 && !tail.contains('\n') {
            let q = tail.to_lowercase();
            let hits: Vec<&Author> = known
                .iter()
                .filter(|a| a.name.to_lowercase().starts_with(&q) && a.name.to_lowercase() != q)
                .take(5)
                .collect();
            if !hits.is_empty() {
                ui.horizontal_wrapped(|ui| {
                    for a in hits {
                        let label = if a.id == comments::ANY_AGENT {
                            "@agent (any agent)".to_string()
                        } else {
                            format!("@{}", a.name)
                        };
                        if ui.small_button(label).clicked() {
                            c.text.truncate(at);
                            c.text.push('@');
                            c.text.push_str(&a.name);
                            c.text.push(' ');
                            r.request_focus();
                            // Carry on typing after the name, not where the caret was.
                            if let Some(mut s) = egui::TextEdit::load_state(ui.ctx(), r.id) {
                                let end = egui::text::CCursor::new(c.text.chars().count());
                                s.cursor
                                    .set_char_range(Some(egui::text::CCursorRange::one(end)));
                                egui::TextEdit::store_state(ui.ctx(), r.id, s);
                            }
                        }
                    }
                });
            }
        }
    }
    let mut remove = None;
    for (i, f) in c.files.iter().enumerate() {
        ui.horizontal(|ui| {
            ui.label(RichText::new(format!("📎 {} · {}", f.name, size_label(f.size))).small());
            if ui.small_button("✕").on_hover_text("Remove").clicked() {
                remove = Some(i);
            }
        });
    }
    if let Some(i) = remove {
        let f = c.files.remove(i);
        if let Some(dir) = &ed.project_dir {
            comments::remove_files(dir, &[f]);
        }
    }
    ui.horizontal(|ui| {
        let attach = ui
            .add_enabled_ui(can_attach, |ui| {
                theme::icon_button(ui, Icon::Attach, 22.0, false)
            })
            .inner;
        let attach = if can_attach {
            attach.on_hover_text("Attach files")
        } else {
            attach.on_disabled_hover_text("Attachments need a saved project")
        };
        if attach.clicked() {
            ed.requests.push(AppRequest::PickAttachments);
        }
        ui.label(RichText::new("⇧↩ new line").color(p.tertiary).small());
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let ready = !(c.text.trim().is_empty() && c.files.is_empty());
            if ui.add_enabled(ready, egui::Button::new(send)).clicked() {
                sent = true;
            }
        });
    });
    sent
}

fn apply(ed: &mut Editor, st: &mut CommentsUi, acts: Vec<Act>) {
    let now = now_ms();
    let me = ed.me.clone();
    let mut changed = false;
    for act in acts {
        let r: Result<(), String> = match act {
            Act::Close => {
                st.close(ed);
                Ok(())
            }
            Act::Post => {
                if let Some(mut n) = st.new.take() {
                    let draft = n.composer.take();
                    let id = ed.comments.add_thread(&me, &n.pin, draft, &ed.doc, now);
                    st.open = Some(id);
                    changed = true;
                }
                Ok(())
            }
            Act::Reply(t) => {
                let draft = st.reply.take();
                changed = true;
                ed.comments.reply(&t, &me, draft, now).map(|_| ())
            }
            Act::React(t, m, e) => {
                changed = true;
                ed.comments.react(&t, Some(&m), &e, &me, None).map(|_| ())
            }
            Act::Status(t, s) => {
                changed = true;
                ed.comments.set_status(&t, s, &me, now)
            }
            Act::DeleteMessage(t, m) => {
                changed = true;
                ed.comments.delete_message(&t, &m).map(|gone| {
                    if let Some(dir) = &ed.project_dir {
                        comments::remove_files(dir, &gone);
                    }
                })
            }
            Act::DeleteThread(t) => {
                changed = true;
                st.open = None;
                ed.comments.delete_thread(&t).map(|gone| {
                    if let Some(dir) = &ed.project_dir {
                        comments::remove_files(dir, &gone);
                    }
                })
            }
            Act::Open(path) => {
                ed.requests.push(AppRequest::OpenFile(path));
                Ok(())
            }
        };
        if let Err(e) = r {
            ed.status = e;
        }
    }
    if changed {
        ed.meta_dirty = true;
        ed.revision += 1;
    }
}

/// The Comments panel: who you comment as, a status filter and every thread in the project.
pub fn panel(ui: &mut Ui, ed: &mut Editor, st: &mut CommentsUi) {
    let p = theme::pal(ui);
    ui.horizontal(|ui| {
        ui.label(RichText::new("Commenting as").color(p.secondary).small());
        match &mut st.name_edit {
            Some(name) => {
                let r = ui.add(egui::TextEdit::singleline(name).desired_width(110.0));
                if std::mem::take(&mut st.name_focus) {
                    r.request_focus();
                }
                if r.lost_focus() {
                    let n = name.trim().to_string();
                    st.name_edit = None;
                    if !n.is_empty() && n != ed.me.name {
                        ed.me.name = n;
                        comments::save_local_author(&ed.me);
                        if ed.comments.author(&ed.me.id).is_some() {
                            let me = ed.me.clone();
                            ed.comments.touch_author(&me);
                            ed.meta_dirty = true;
                        }
                    }
                }
            }
            None => {
                let r = ui.add(
                    egui::Label::new(
                        RichText::new(&ed.me.name).font(theme::semibold(theme::SECONDARY)),
                    )
                    .sense(egui::Sense::click()),
                );
                if r.on_hover_text("Click to change your name").clicked() {
                    st.name_edit = Some(ed.me.name.clone());
                    st.name_focus = true;
                }
            }
        }
    });
    ui.add_space(4.0);
    let open = ed
        .comments
        .threads
        .iter()
        .filter(|t| t.status == Status::Open && ed.pages.iter().any(|p| p.id == t.page))
        .count();
    ui.horizontal(|ui| {
        for (f, label) in [
            (Filter::Open, format!("Open {open}")),
            (Filter::Resolved, "Resolved".to_string()),
            (Filter::All, "All".to_string()),
        ] {
            if ui
                .selectable_label(st.filter == f, RichText::new(label).small())
                .clicked()
            {
                st.filter = f;
            }
        }
    });
    ui.add_space(4.0);
    let mut threads: Vec<(usize, &Thread)> = ed
        .comments
        .threads
        .iter()
        .filter(|t| st.filter.shows(t.status) && !t.messages.is_empty())
        .filter_map(|t| Some((ed.pages.iter().position(|p| p.id == t.page)?, t)))
        .collect();
    threads.sort_by(|(pa, a), (pb, b)| pa.cmp(pb).then(b.updated.cmp(&a.updated)));
    if threads.is_empty() {
        ui.add_space(12.0);
        ui.label(
            RichText::new(match st.filter {
                Filter::Open => "No open comments. Press C, then click the canvas to add one.",
                _ => "No comments here.",
            })
            .color(p.secondary)
            .small(),
        );
        return;
    }
    let now = now_ms();
    let mut go = None;
    egui::ScrollArea::vertical()
        .auto_shrink(false)
        .show(ui, |ui| {
            let mut last_page = None;
            for (page, t) in &threads {
                if last_page != Some(*page) && ed.pages.len() > 1 {
                    ui.add_space(4.0);
                    ui.label(
                        RichText::new(&ed.pages[*page].name)
                            .color(p.secondary)
                            .small(),
                    );
                    last_page = Some(*page);
                }
                let first = &t.messages[0];
                let a = author(ed, &first.author);
                let selected = st.open.as_deref() == Some(t.id.as_str());
                let r = egui::Frame::NONE
                    .fill(if selected {
                        p.pressed
                    } else {
                        Color32::TRANSPARENT
                    })
                    .corner_radius(6.0)
                    .inner_margin(6.0)
                    .show(ui, |ui| {
                        ui.set_width(ui.available_width());
                        ui.horizontal(|ui| {
                            avatar(ui, &a, 16.0);
                            ui.label(
                                RichText::new(&a.name).font(theme::semibold(theme::SECONDARY)),
                            );
                            ui.label(
                                RichText::new(comments::ago(t.updated, now))
                                    .color(p.tertiary)
                                    .small(),
                            );
                            if t.status == Status::Resolved {
                                ui.label(RichText::new("✓").color(p.secondary).small());
                            }
                        });
                        let preview: String = first.text.chars().take(90).collect();
                        ui.label(RichText::new(preview).small());
                        let replies = t.messages.len() - 1;
                        let files: usize = t.messages.iter().map(|m| m.attachments.len()).sum();
                        let mut meta = Vec::new();
                        if replies > 0 {
                            meta.push(format!(
                                "{replies} repl{}",
                                if replies == 1 { "y" } else { "ies" }
                            ));
                        }
                        if files > 0 {
                            meta.push(format!("📎 {files}"));
                        }
                        if !meta.is_empty() {
                            ui.label(RichText::new(meta.join(" · ")).color(p.secondary).small());
                        }
                    })
                    .response
                    .interact(egui::Sense::click());
                if r.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
                    go = Some((*page, t.id.clone()));
                }
            }
        });
    if let Some((page, id)) = go {
        reveal(ed, st, page, &id);
    }
}

/// Switch to the thread's page, centre its pin and open it.
pub fn reveal(ed: &mut Editor, st: &mut CommentsUi, page: usize, id: &str) {
    if page != ed.active_page {
        st.close(ed);
        ed.switch_page(page);
    }
    if let Some(t) = ed.comments.thread(id) {
        let w = t.world_point(&ed.doc);
        ed.camera.pan = w - ed.viewport * (0.5 / ed.camera.zoom);
        ed.revision += 1;
    }
    st.show(ed, id);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn closing_a_draft_forgets_it() {
        let mut ed = Editor::default();
        let mut st = CommentsUi::default();
        st.start(&mut ed, None, crate::geom::v2(10.0, 10.0));
        assert!(st.new.is_some());
        st.close(&ed);
        assert!(st.new.is_none() && st.open.is_none());
    }
}
