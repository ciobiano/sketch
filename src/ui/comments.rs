//! Comments in the window: pins that stay on the canvas (a preview card opens on hover),
//! the thread popover (replies, reactions, attachments, @mentions) and the Comments panel.

use crate::comments::{
    self, Attachment, Author, AuthorKind, Draft, Pin, REACTIONS, Status, Thread,
};
use crate::editor::{AppRequest, Editor, Tool};
use crate::geom::V2;
use crate::model::NodeId;
use crate::project::now_ms;
use crate::theme::{self, Icon};
use egui::{
    Align2, Color32, CornerRadius, Id, Order, Pos2, Rect, RichText, Sense, Stroke, StrokeKind, Ui,
    pos2, vec2,
};
use std::collections::HashMap;

/// Pin size. The pin's bottom-left corner is the point it marks.
const PIN: f32 = 28.0;
const POPOVER_W: f32 = 320.0;
const CARD_W: f32 = 260.0;

/// Pins and their preview cards share one shape: round, with the corner on the point left
/// sharp.
fn bubble(r: f32) -> CornerRadius {
    let r = r.round().clamp(0.0, 255.0) as u8;
    CornerRadius {
        nw: r,
        ne: r,
        se: r,
        sw: 3,
    }
}

fn shadow() -> egui::epaint::Shadow {
    egui::epaint::Shadow {
        offset: [0, 3],
        blur: 10,
        spread: 0,
        color: Color32::from_black_alpha(48),
    }
}

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
    /// The highlighted @mention suggestion.
    pick: usize,
}

impl Composer {
    fn take(&mut self) -> Draft {
        self.pick = 0;
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
    /// Pins are hidden on the canvas (Shift+C). The Comment tool and an open thread still
    /// show them.
    pub hidden: bool,
    /// Display name being edited in the panel.
    pub name_edit: Option<String>,
    /// Focus the name field on the next frame.
    name_focus: bool,
    /// Focus the composer on the next frame.
    focus: bool,
    /// Unsent reply text per thread, kept when its popover closes.
    drafts: HashMap<String, String>,
    /// The pin whose preview card is showing, and where the card was last frame.
    peek: Option<(String, Rect)>,
    /// Pins drawn in the last frame, with the screen point each one marks.
    pub drawn: Vec<(String, Pos2)>,
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
            self.reply.text = self.drafts.remove(id).unwrap_or_default();
            self.focus = true;
            self.peek = None;
        }
    }

    pub fn close(&mut self, ed: &Editor) {
        if let Some(mut n) = self.new.take() {
            n.composer.discard(ed);
        }
        if let Some(id) = self.open.take() {
            let text = std::mem::take(&mut self.reply.text);
            if !text.trim().is_empty() {
                self.drafts.insert(id, text);
            }
        }
        self.reply.discard(ed);
    }

    /// A reply was started in this thread and not sent.
    pub fn has_draft(&self, id: &str) -> bool {
        self.drafts.contains_key(id)
    }
}

/// Avatar colours, picked per author. Each one keeps white initials readable.
const AVATAR_COLORS: [Color32; 8] = [
    Color32::from_rgb(0xe5, 0x48, 0x4d),
    Color32::from_rgb(0xe8, 0x6a, 0x10),
    Color32::from_rgb(0x2b, 0x9a, 0x66),
    Color32::from_rgb(0x0d, 0x94, 0x88),
    Color32::from_rgb(0x25, 0x63, 0xeb),
    Color32::from_rgb(0x6e, 0x56, 0xcf),
    Color32::from_rgb(0xc0, 0x3a, 0x8f),
    Color32::from_rgb(0x02, 0x84, 0xc7),
];

/// A stable colour per author for pins and avatars. Agents use the accent colour.
fn author_color(a: &Author) -> Color32 {
    if a.kind == AuthorKind::Agent {
        return theme::accent();
    }
    let h =
        a.id.bytes()
            .fold(5381u32, |h, b| h.wrapping_mul(33) ^ u32::from(b));
    AVATAR_COLORS[h as usize % AVATAR_COLORS.len()]
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

/// A person's avatar is round; an agent's is a rounded square, so the two read apart
/// without relying on colour.
fn paint_avatar(p: &egui::Painter, r: Rect, a: &Author, alpha: f32) {
    let fill = author_color(a).gamma_multiply(alpha);
    if a.kind == AuthorKind::Agent {
        p.rect_filled(r, r.width() * 0.28, fill);
    } else {
        p.circle_filled(r.center(), r.width() / 2.0, fill);
    }
    p.text(
        r.center(),
        Align2::CENTER_CENTER,
        a.initial(),
        theme::semibold(r.width() * 0.5),
        Color32::WHITE.gamma_multiply(alpha),
    );
}

fn avatar(ui: &mut Ui, a: &Author, size: f32) -> egui::Response {
    let (r, resp) = ui.allocate_exact_size(vec2(size, size), Sense::hover());
    paint_avatar(ui.painter(), r, a, 1.0);
    resp
}

fn size_label(n: u64) -> String {
    match n {
        0..1024 => format!("{n} B"),
        1024..1_048_576 => format!("{:.0} KB", n as f64 / 1024.0),
        _ => format!("{:.1} MB", n as f64 / 1_048_576.0),
    }
}

fn plural(n: usize, one: &str, many: &str) -> String {
    format!("{n} {}", if n == 1 { one } else { many })
}

/// Message text with @mentions in the accent colour, wrapped at `width`, cut after
/// `rows` lines (0 = no limit).
fn rich_text(
    text: &str,
    known: &[Author],
    size: f32,
    color: Color32,
    width: f32,
    rows: usize,
) -> egui::text::LayoutJob {
    let plain = egui::TextFormat {
        font_id: egui::FontId::proportional(size),
        color,
        line_height: Some(size * 1.38),
        ..Default::default()
    };
    let mention = egui::TextFormat {
        font_id: theme::semibold(size),
        color: theme::accent(),
        line_height: Some(size * 1.38),
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
    job.wrap.max_width = width;
    if rows > 0 {
        job.wrap.max_rows = rows;
        job.wrap.overflow_character = Some('…');
    }
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
    Show(String),
    Select(NodeId),
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

/// How a pin looks.
struct Look {
    author: Author,
    resolved: bool,
    replies: usize,
}

/// Pins, the hover card, the Comment tool's ghost pin and the popover, over the canvas.
pub fn canvas(ctx: &egui::Context, ed: &mut Editor, st: &mut CommentsUi, canvas: Rect) {
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
    let commenting = ed.tool == Tool::Comment;
    let show_all = !st.hidden || commenting;
    let mut acts = Vec::new();
    st.drawn.clear();
    let pins: Vec<(String, Pos2, Look)> = ed
        .comments
        .threads
        .iter()
        .filter(|t| t.page == page && !t.messages.is_empty())
        // Resolved threads keep their pin (muted), so earlier discussions stay one click
        // away. The panel's filter only narrows the list.
        .filter(|t| show_all || st.open.as_deref() == Some(&t.id))
        .map(|t| {
            (
                t.id.clone(),
                to_screen(t.world_point(&ed.doc), ed),
                Look {
                    author: author(ed, t.author()),
                    resolved: t.status == Status::Resolved,
                    replies: t.messages.len() - 1,
                },
            )
        })
        .collect();
    let pointer = ctx.pointer_hover_pos();
    // Hover is worked out from geometry rather than egui's hover state, because the card
    // covers the pin it opened from. Nothing peeks from under a popover or a panel.
    let peek_layer = egui::LayerId::new(Order::Foreground, Id::new("comment-peek"));
    let free = pointer.filter(|p| {
        canvas.contains(*p)
            && ctx.layer_id_at(*p).is_none_or(|l| {
                matches!(l.order, Order::Background | Order::Middle) || l == peek_layer
            })
    });
    let mut peek = st
        .peek
        .take()
        .filter(|(_, r)| free.is_some_and(|p| r.expand(2.0).contains(p)))
        .map(|(id, _)| id);
    for (id, at, look) in &pins {
        if !canvas.expand(PIN).contains(*at) {
            continue;
        }
        st.drawn.push((id.clone(), *at));
        let open = st.open.as_deref() == Some(id.as_str());
        let r = pin(ctx, id, *at, look, open, false);
        if r.clicked() {
            if open {
                acts.push(Act::Close);
            } else {
                acts.push(Act::Show(id.clone()));
            }
        }
        let hit = Rect::from_min_max(*at - vec2(0.0, PIN), *at + vec2(PIN, 0.0));
        if peek.is_none() && !open && free.is_some_and(|p| hit.contains(p)) {
            peek = Some(id.clone());
        }
    }
    if let Some(id) = peek.filter(|id| st.open.as_deref() != Some(id.as_str()))
        && let Some(t) = ed.comments.thread(&id).cloned()
        && let Some((_, at, _)) = pins.iter().find(|(p, _, _)| *p == id)
    {
        let (clicked, rect) = peek_card(ctx, ed, &t, *at, canvas);
        if clicked {
            acts.push(Act::Show(id.clone()));
        } else {
            st.peek = Some((id, rect));
        }
    }
    // Where a click with the Comment tool will put the comment.
    if commenting
        && st.new.is_none()
        && st.peek.is_none()
        && let Some(p) = pointer
        && canvas.contains(p)
        && ctx
            .layer_id_at(p)
            .is_none_or(|l| l.order == Order::Background)
    {
        let look = Look {
            author: ed.me.clone(),
            resolved: false,
            replies: 0,
        };
        pin(ctx, "ghost", p, &look, false, true);
    }
    if let Some(n) = &st.new {
        let at = to_screen(n.pin.point, ed);
        let look = Look {
            author: ed.me.clone(),
            resolved: false,
            replies: 0,
        };
        pin(ctx, "new", at, &look, true, false);
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

/// One pin. `active` marks the open thread; a `ghost` pin follows the pointer and takes no
/// clicks.
fn pin(
    ctx: &egui::Context,
    key: &str,
    at: Pos2,
    look: &Look,
    active: bool,
    ghost: bool,
) -> egui::Response {
    let pad = 8.0;
    egui::Area::new(Id::new(("comment-pin", key)))
        .fixed_pos(at - vec2(pad, PIN + pad))
        .order(Order::Middle)
        .fade_in(false)
        .interactable(!ghost)
        .show(ctx, |ui| {
            let (outer, resp) =
                ui.allocate_exact_size(vec2(PIN + 2.0 * pad, PIN + 2.0 * pad), Sense::click());
            let grow = ctx.animate_bool_with_time(
                Id::new(("comment-pin-grow", key)),
                resp.hovered() || active,
                0.12,
            );
            let s = PIN + 4.0 * grow;
            // Grow up and to the right, so the marked point stays put.
            let r = Rect::from_min_size(pos2(outer.left() + pad, at.y - s), vec2(s, s));
            let shape = bubble(s / 2.0);
            let alpha = if ghost {
                0.7
            } else if look.resolved && !active {
                0.8
            } else {
                1.0
            };
            let p = ui.painter();
            if !ghost {
                p.add(shadow().as_shape(r, shape));
            }
            if active {
                p.rect_stroke(
                    r.expand(2.0),
                    bubble(s / 2.0 + 2.0),
                    Stroke::new(2.0, theme::accent()),
                    StrokeKind::Outside,
                );
            }
            let fill = if look.resolved {
                Color32::from_gray(150)
            } else {
                author_color(&look.author)
            };
            p.rect(
                r,
                shape,
                fill.gamma_multiply(alpha),
                Stroke::new(2.0, Color32::WHITE.gamma_multiply(alpha)),
                StrokeKind::Inside,
            );
            if look.resolved {
                theme::paint_icon(
                    p,
                    r.shrink(s * 0.27),
                    Icon::Check,
                    Color32::WHITE.gamma_multiply(alpha),
                );
            } else {
                p.text(
                    r.center(),
                    Align2::CENTER_CENTER,
                    look.author.initial(),
                    theme::semibold(s * 0.42),
                    Color32::WHITE.gamma_multiply(alpha),
                );
            }
            if look.replies > 0 {
                let c = r.right_top() + vec2(-1.0, 1.0);
                let n = if look.replies > 9 {
                    "9+".to_string()
                } else {
                    look.replies.to_string()
                };
                let w = if n.len() > 1 { 9.0 } else { 7.5 };
                let badge = Rect::from_center_size(c, vec2(w * 2.0, 15.0));
                p.rect(
                    badge,
                    7.5,
                    Color32::from_gray(28),
                    Stroke::new(1.5, Color32::WHITE),
                    StrokeKind::Outside,
                );
                p.text(
                    c,
                    Align2::CENTER_CENTER,
                    n,
                    theme::semibold(9.5),
                    Color32::WHITE,
                );
            }
            resp.on_hover_cursor(egui::CursorIcon::PointingHand)
        })
        .inner
}

/// The preview a pin opens into on hover: who, when, the first lines and the reply count.
/// Returns whether it was clicked, and where it is.
fn peek_card(ctx: &egui::Context, ed: &Editor, t: &Thread, at: Pos2, canvas: Rect) -> (bool, Rect) {
    let a = author(ed, t.author());
    let known = ed.comments.mentionable(&ed.me);
    let now = now_ms();
    let r = egui::Area::new(Id::new("comment-peek"))
        .order(Order::Foreground)
        .pivot(Align2::LEFT_BOTTOM)
        .fixed_pos(at)
        .constrain_to(canvas.shrink(8.0))
        .sense(Sense::click())
        .show(ctx, |ui| {
            let p = theme::pal(ui);
            egui::Frame::NONE
                .fill(p.popover)
                .stroke(Stroke::new(1.0, p.separator))
                .shadow(shadow())
                .corner_radius(bubble(14.0))
                .inner_margin(egui::Margin::symmetric(12, 10))
                .show(ui, |ui| {
                    ui.set_width(CARD_W - 24.0);
                    ui.spacing_mut().item_spacing = vec2(6.0, 4.0);
                    ui.horizontal(|ui| {
                        avatar(ui, &a, 20.0);
                        ui.label(RichText::new(&a.name).font(theme::semibold(theme::BODY)));
                        ui.label(
                            RichText::new(comments::ago(t.created, now))
                                .color(p.tertiary)
                                .size(theme::SECONDARY),
                        );
                        if t.status == Status::Resolved {
                            ui.with_layout(
                                egui::Layout::right_to_left(egui::Align::Center),
                                |ui| tag(ui, "Resolved", p.secondary),
                            );
                        }
                    });
                    let first = &t.messages[0];
                    if !first.text.is_empty() {
                        ui.label(rich_text(
                            &first.text,
                            &known,
                            theme::BODY,
                            p.text,
                            CARD_W - 24.0,
                            3,
                        ));
                    }
                    let mut meta = Vec::new();
                    let replies = t.messages.len() - 1;
                    if replies > 0 {
                        meta.push(plural(replies, "reply", "replies"));
                    }
                    let files: usize = t.messages.iter().map(|m| m.attachments.len()).sum();
                    if files > 0 {
                        meta.push(plural(files, "file", "files"));
                    }
                    if let Some(n) = t.node_id().and_then(|id| ed.doc.get(id)) {
                        meta.push(format!("on {}", n.name));
                    }
                    if !meta.is_empty() {
                        ui.label(
                            RichText::new(meta.join("  ·  "))
                                .color(p.secondary)
                                .size(theme::SECONDARY),
                        );
                    }
                });
        });
    let resp = r.response.on_hover_cursor(egui::CursorIcon::PointingHand);
    (resp.clicked(), resp.rect)
}

/// A small rounded label, like "Agent" or "Resolved".
fn tag(ui: &mut Ui, text: &str, color: Color32) {
    egui::Frame::NONE
        .fill(color.gamma_multiply(0.14))
        .corner_radius(4.0)
        .inner_margin(egui::Margin::symmetric(5, 1))
        .show(ui, |ui| {
            ui.label(RichText::new(text).font(theme::medium(10.0)).color(color));
        });
}

fn popover(ctx: &egui::Context, at: Pos2, canvas: Rect, body: impl FnOnce(&mut Ui)) {
    // Right of the pin, or left of it when the canvas has no room there, so it never covers
    // the pin it belongs to.
    let width = POPOVER_W + 16.0;
    let right = at.x + PIN + 14.0;
    let x = if right + width <= canvas.right() - 8.0 {
        right
    } else {
        at.x - width - 10.0
    };
    egui::Area::new("comment-popover".into())
        .fixed_pos(pos2(x, at.y - PIN - 4.0))
        .order(Order::Foreground)
        .constrain_to(canvas.shrink(8.0))
        .show(ctx, |ui| {
            let p = theme::pal(ui);
            egui::Frame::NONE
                .fill(p.popover)
                .stroke(Stroke::new(1.0, p.separator))
                .shadow(egui::epaint::Shadow {
                    offset: [0, 8],
                    blur: 24,
                    spread: 0,
                    color: Color32::from_black_alpha(40),
                })
                .corner_radius(12.0)
                .show(ui, |ui| {
                    ui.set_width(POPOVER_W);
                    ui.spacing_mut().item_spacing = vec2(6.0, 4.0);
                    body(ui)
                });
        });
}

/// A padded block of the popover.
fn section<R>(ui: &mut Ui, x: i8, y: i8, body: impl FnOnce(&mut Ui) -> R) -> R {
    egui::Frame::NONE
        .inner_margin(egui::Margin::symmetric(x, y))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            body(ui)
        })
        .inner
}

fn rule(ui: &mut Ui) {
    let p = theme::pal(ui);
    let (r, _) = ui.allocate_exact_size(vec2(ui.available_width(), 1.0), Sense::hover());
    ui.painter()
        .hline(r.x_range(), r.center().y, Stroke::new(1.0, p.separator));
}

/// The layer a comment is pinned to; clicking selects it.
fn node_chip(ui: &mut Ui, ed: &Editor, node: Option<NodeId>, acts: &mut Vec<Act>) {
    let p = theme::pal(ui);
    let Some((id, name)) = node.and_then(|id| Some((id, ed.doc.get(id)?.name.clone()))) else {
        ui.label(
            RichText::new(ed.pages[ed.active_page].name.as_str())
                .color(p.secondary)
                .size(theme::SECONDARY),
        );
        return;
    };
    let r = ui.add(
        egui::Button::new(
            RichText::new(format!("◇ {name}"))
                .color(p.secondary)
                .size(theme::SECONDARY),
        )
        .fill(p.control)
        .stroke(Stroke::NONE)
        .corner_radius(5.0)
        .min_size(vec2(0.0, 20.0)),
    );
    if r.on_hover_text("Select this layer").clicked() {
        acts.push(Act::Select(id));
    }
}

fn new_thread(ui: &mut Ui, ed: &mut Editor, st: &mut CommentsUi, acts: &mut Vec<Act>) {
    let node = st.new.as_ref().and_then(|n| n.pin.node);
    section(ui, 12, 10, |ui| {
        ui.horizontal(|ui| {
            ui.label(RichText::new("New comment").font(theme::semibold(theme::BODY)));
            node_chip(ui, ed, node, acts);
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if theme::icon_button(ui, Icon::Close, 22.0, false)
                    .on_hover_text("Discard (Esc)")
                    .clicked()
                {
                    acts.push(Act::Close);
                }
            });
        });
    });
    let focus = std::mem::take(&mut st.focus);
    let known = ed.comments.mentionable(&ed.me);
    let can_attach = ed.project_dir.is_some();
    let me = ed.me.clone();
    let Some(n) = st.new.as_mut() else { return };
    section(ui, 12, 0, |ui| {
        let sent = composer(
            ui,
            ed,
            &mut n.composer,
            ComposerOpts {
                id: Id::new("comment-composer-new"),
                me: &me,
                known: &known,
                hint: "Add a comment. Type @ to mention someone",
                focus,
                can_attach,
            },
        );
        if sent {
            acts.push(Act::Post);
        }
        ui.add_space(10.0);
    });
}

fn thread_view(ui: &mut Ui, ed: &mut Editor, st: &mut CommentsUi, t: &Thread, acts: &mut Vec<Act>) {
    let p = theme::pal(ui);
    let resolved = t.status == Status::Resolved;
    section(ui, 12, 10, |ui| {
        ui.horizontal(|ui| {
            node_chip(ui, ed, t.node_id(), acts);
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.spacing_mut().item_spacing.x = 2.0;
                if theme::icon_button(ui, Icon::Close, 22.0, false)
                    .on_hover_text("Close (Esc)")
                    .clicked()
                {
                    acts.push(Act::Close);
                }
                let more = theme::icon_button(ui, Icon::More, 22.0, false).on_hover_text("More");
                egui::Popup::menu(&more).show(|ui| {
                    if ui.button("Delete thread").clicked() {
                        acts.push(Act::DeleteThread(t.id.clone()));
                    }
                });
                ui.add_space(4.0);
                let (label, next, tip) = if resolved {
                    ("Reopen", Status::Open, "Move back to open comments")
                } else {
                    ("✓ Resolve", Status::Resolved, "Mark as done")
                };
                let b = egui::Button::new(
                    RichText::new(label)
                        .font(theme::medium(theme::SECONDARY))
                        .color(if resolved { p.text } else { Color32::WHITE }),
                )
                .fill(if resolved { p.control } else { theme::accent() })
                .stroke(Stroke::NONE)
                .corner_radius(11.0)
                .min_size(vec2(0.0, 22.0));
                if ui.add(b).on_hover_text(tip).clicked() {
                    acts.push(Act::Status(t.id.clone(), next));
                }
            });
        });
        if resolved {
            ui.add_space(6.0);
            let by = t
                .resolved_by
                .as_deref()
                .map(|by| format!("Resolved by {}", author(ed, by).name))
                .unwrap_or_else(|| "Resolved".into());
            egui::Frame::NONE
                .fill(p.control)
                .corner_radius(6.0)
                .inner_margin(egui::Margin::symmetric(8, 5))
                .show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.horizontal(|ui| {
                        let (r, _) = ui.allocate_exact_size(vec2(12.0, 12.0), Sense::hover());
                        theme::paint_icon(ui.painter(), r, Icon::Check, p.secondary);
                        ui.label(RichText::new(by).color(p.secondary).size(theme::SECONDARY));
                    });
                });
        }
    });
    rule(ui);
    let known = ed.comments.mentionable(&ed.me);
    let now = now_ms();
    egui::ScrollArea::vertical()
        .max_height(360.0)
        .auto_shrink([false, true])
        .stick_to_bottom(true)
        .show(ui, |ui| {
            section(ui, 12, 10, |ui| {
                for i in 0..t.messages.len() {
                    if i > 0 {
                        ui.add_space(12.0);
                    }
                    message(ui, ed, t, i, &known, now, acts);
                }
            });
        });
    rule(ui);
    let focus = std::mem::take(&mut st.focus);
    let can_attach = ed.project_dir.is_some();
    let me = ed.me.clone();
    section(ui, 12, 10, |ui| {
        let sent = composer(
            ui,
            ed,
            &mut st.reply,
            ComposerOpts {
                id: Id::new(("comment-composer", &t.id)),
                me: &me,
                known: &known,
                hint: if resolved {
                    "Reply to pick this up again"
                } else {
                    "Reply"
                },
                focus,
                can_attach,
            },
        );
        if sent {
            acts.push(Act::Reply(t.id.clone()));
        }
    });
}

/// One message: avatar, name, time, text, files and reactions. Reacting and (for your own
/// messages) deleting sit at the end of the name row.
fn message(
    ui: &mut Ui,
    ed: &Editor,
    t: &Thread,
    i: usize,
    known: &[Author],
    now: u64,
    acts: &mut Vec<Act>,
) {
    let p = theme::pal(ui);
    let m = &t.messages[i];
    let a = author(ed, &m.author);
    ui.horizontal_top(|ui| {
        ui.spacing_mut().item_spacing.x = 8.0;
        avatar(ui, &a, 24.0);
        ui.vertical(|ui| {
            let width = ui.available_width();
            ui.spacing_mut().item_spacing.y = 3.0;
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 5.0;
                ui.label(RichText::new(&a.name).font(theme::semibold(theme::BODY)));
                if a.kind == AuthorKind::Agent {
                    tag(ui, "Agent", theme::accent());
                }
                let edited = if m.edited.is_some() { " · edited" } else { "" };
                ui.label(
                    RichText::new(format!("{}{edited}", comments::ago(m.created, now)))
                        .color(p.tertiary)
                        .size(theme::SECONDARY),
                )
                .on_hover_text(comments::iso(m.created));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.spacing_mut().item_spacing.x = 0.0;
                    if m.author == ed.me.id {
                        let more = small_icon(ui, Icon::More, "More");
                        egui::Popup::menu(&more).show(|ui| {
                            let label = if i == 0 {
                                "Delete thread"
                            } else {
                                "Delete reply"
                            };
                            if ui.button(label).clicked() {
                                acts.push(Act::DeleteMessage(t.id.clone(), m.id.clone()));
                            }
                        });
                    }
                    reaction_picker(ui, t, &m.id, acts);
                });
            });
            if !m.text.is_empty() {
                ui.label(rich_text(&m.text, known, theme::BODY, p.text, width, 0));
            }
            for f in &m.attachments {
                if attachment_chip(ui, f, None).clicked()
                    && let Some(dir) = &ed.project_dir
                {
                    acts.push(Act::Open(dir.join(&f.file)));
                }
            }
            if !m.reactions.is_empty() {
                ui.add_space(2.0);
                ui.horizontal_wrapped(|ui| {
                    ui.spacing_mut().item_spacing.x = 4.0;
                    for (emoji, who) in &m.reactions {
                        let mine = who.contains(&ed.me.id);
                        let names: Vec<String> = who.iter().map(|w| author(ed, w).name).collect();
                        let fill = if mine {
                            theme::accent().gamma_multiply(0.16)
                        } else {
                            p.control
                        };
                        let stroke = if mine {
                            Stroke::new(1.0, theme::accent())
                        } else {
                            Stroke::NONE
                        };
                        let r = ui.add(
                            egui::Button::new(
                                RichText::new(format!("{emoji} {}", who.len()))
                                    .size(13.0)
                                    .color(p.text),
                            )
                            .fill(fill)
                            .stroke(stroke)
                            .corner_radius(11.0)
                            .min_size(vec2(0.0, 22.0)),
                        );
                        if r.on_hover_text(names.join(", ")).clicked() {
                            acts.push(Act::React(t.id.clone(), m.id.clone(), emoji.clone()));
                        }
                    }
                });
            }
        });
    });
}

/// A quiet 20 px icon button.
fn small_icon(ui: &mut Ui, icon: Icon, tip: &str) -> egui::Response {
    let p = theme::pal(ui);
    let (r, resp) = ui.allocate_exact_size(vec2(20.0, 20.0), Sense::click());
    if resp.hovered() {
        ui.painter().rect_filled(r, 5.0, p.hover);
    }
    let color = if resp.hovered() { p.text } else { p.tertiary };
    theme::paint_icon(ui.painter(), r.shrink(4.0), icon, color);
    resp.on_hover_text(tip)
}

fn reaction_picker(ui: &mut Ui, t: &Thread, message: &str, acts: &mut Vec<Act>) {
    let p = theme::pal(ui);
    let (r, resp) = ui.allocate_exact_size(vec2(20.0, 20.0), Sense::click());
    if resp.hovered() {
        ui.painter().rect_filled(r, 5.0, p.hover);
    }
    let color = if resp.hovered() { p.text } else { p.tertiary };
    paint_smiley(ui.painter(), r.shrink(4.0), color);
    let resp = resp.on_hover_text("Add reaction");
    egui::Popup::menu(&resp).show(|ui| {
        ui.horizontal(|ui| {
            for e in REACTIONS {
                if ui
                    .add(egui::Button::new(RichText::new(e).size(18.0)).frame(false))
                    .clicked()
                {
                    acts.push(Act::React(t.id.clone(), message.to_string(), e.to_string()));
                    ui.close();
                }
            }
        });
    });
}

fn paint_smiley(p: &egui::Painter, r: Rect, color: Color32) {
    let c = r.center();
    let rad = r.width() / 2.0;
    let s = Stroke::new(1.3, color);
    p.circle_stroke(c, rad, s);
    p.circle_filled(c + vec2(-rad * 0.35, -rad * 0.25), 1.0, color);
    p.circle_filled(c + vec2(rad * 0.35, -rad * 0.25), 1.0, color);
    let mouth: Vec<Pos2> = (0..=8)
        .map(|i| {
            let a = std::f32::consts::PI * (0.2 + 0.6 * i as f32 / 8.0);
            c + vec2(a.cos() * rad * 0.5, a.sin() * rad * 0.5)
        })
        .collect();
    p.add(egui::Shape::line(mouth, s));
}

/// A file: paperclip, name and size. With `remove`, an x button that sets it.
fn attachment_chip(ui: &mut Ui, f: &Attachment, remove: Option<&mut bool>) -> egui::Response {
    let p = theme::pal(ui);
    let r = egui::Frame::NONE
        .fill(p.control)
        .corner_radius(6.0)
        .inner_margin(egui::Margin::symmetric(6, 4))
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 4.0;
                let (r, _) = ui.allocate_exact_size(vec2(14.0, 14.0), Sense::hover());
                theme::paint_icon(ui.painter(), r, Icon::Attach, p.secondary);
                ui.add(
                    egui::Label::new(RichText::new(&f.name).size(theme::SECONDARY))
                        .truncate()
                        .selectable(false),
                );
                ui.label(
                    RichText::new(size_label(f.size))
                        .color(p.tertiary)
                        .size(theme::SECONDARY),
                );
                if let Some(remove) = remove
                    && small_icon(ui, Icon::Close, "Remove").clicked()
                {
                    *remove = true;
                }
            });
        })
        .response
        .interact(Sense::click());
    r.on_hover_cursor(egui::CursorIcon::PointingHand)
}

struct ComposerOpts<'a> {
    id: Id,
    me: &'a Author,
    known: &'a [Author],
    hint: &'a str,
    focus: bool,
    can_attach: bool,
}

/// Names that complete the @word being typed at the end of the text, and where that @ is.
fn suggestions<'a>(text: &str, known: &'a [Author]) -> Option<(usize, Vec<&'a Author>)> {
    let at = text.rfind('@')?;
    let tail = &text[at + 1..];
    let starts_word = text[..at]
        .chars()
        .next_back()
        .is_none_or(char::is_whitespace);
    if !starts_word || tail.len() > 24 || tail.contains('\n') {
        return None;
    }
    let q = tail.to_lowercase();
    let hits: Vec<&Author> = known
        .iter()
        .filter(|a| {
            let n = a.name.to_lowercase();
            n.starts_with(&q) && n != q
        })
        .take(6)
        .collect();
    (!hits.is_empty()).then_some((at, hits))
}

/// Put the caret at the end of the text and keep the box focused.
fn caret_to_end(ctx: &egui::Context, id: Id, text: &str) {
    let mut s = egui::TextEdit::load_state(ctx, id).unwrap_or_default();
    let end = egui::text::CCursor::new(text.chars().count());
    s.cursor
        .set_char_range(Some(egui::text::CCursorRange::one(end)));
    egui::TextEdit::store_state(ctx, id, s);
    ctx.memory_mut(|m| m.request_focus(id));
}

/// Replace the @word at `at` with the full name, then carry on typing after it.
fn complete(ctx: &egui::Context, id: Id, c: &mut Composer, at: usize, a: &Author) {
    c.text.truncate(at);
    c.text.push('@');
    c.text.push_str(&a.name);
    c.text.push(' ');
    c.pick = 0;
    caret_to_end(ctx, id, &c.text);
}

/// The message box: text, @mention suggestions (arrow keys, then Return or Tab, pick one),
/// attached files, and a row with attach, mention and send. Returns true when the message
/// should be sent (send button, or Return; Shift+Return adds a line).
fn composer(ui: &mut Ui, ed: &mut Editor, c: &mut Composer, o: ComposerOpts) -> bool {
    let p = theme::pal(ui);
    let ctx = ui.ctx().clone();
    let had_focus = ctx.memory(|m| m.has_focus(o.id));
    // Keys for the suggestion list, taken before the text box sees them.
    if had_focus && let Some((at, hits)) = suggestions(&c.text, o.known) {
        c.pick = c.pick.min(hits.len() - 1);
        let (down, up, pick) = ui.input_mut(|i| {
            (
                i.consume_key(egui::Modifiers::NONE, egui::Key::ArrowDown),
                i.consume_key(egui::Modifiers::NONE, egui::Key::ArrowUp),
                i.consume_key(egui::Modifiers::NONE, egui::Key::Enter)
                    || i.consume_key(egui::Modifiers::NONE, egui::Key::Tab),
            )
        });
        if down {
            c.pick = (c.pick + 1) % hits.len();
        }
        if up {
            c.pick = (c.pick + hits.len() - 1) % hits.len();
        }
        if pick {
            let a = hits[c.pick].clone();
            complete(&ctx, o.id, c, at, &a);
        }
    }
    let border = if had_focus {
        theme::accent()
    } else {
        p.control_border
    };
    let mut sent = false;
    let mut remove = None;
    egui::Frame::NONE
        .fill(p.field)
        .stroke(Stroke::new(if had_focus { 1.5 } else { 1.0 }, border))
        .corner_radius(10.0)
        .inner_margin(egui::Margin {
            left: 10,
            right: 6,
            top: 8,
            bottom: 6,
        })
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal_top(|ui| {
                ui.spacing_mut().item_spacing.x = 8.0;
                avatar(ui, o.me, 22.0);
                let edit = egui::TextEdit::multiline(&mut c.text)
                    .id(o.id)
                    .hint_text(o.hint)
                    .frame(egui::Frame::NONE)
                    .margin(egui::Margin::symmetric(0, 3))
                    .desired_rows(1)
                    .desired_width(f32::INFINITY)
                    .font(egui::FontId::proportional(theme::BODY))
                    .return_key(egui::KeyboardShortcut::new(
                        egui::Modifiers::SHIFT,
                        egui::Key::Enter,
                    ));
                let r = ui.add(edit);
                if o.focus {
                    r.request_focus();
                }
                sent = r.has_focus()
                    && ui.input(|i| i.key_pressed(egui::Key::Enter) && !i.modifiers.shift)
                    && !Draft::text(&c.text).is_empty();
            });
            for (i, f) in c.files.iter().enumerate() {
                let mut gone = false;
                attachment_chip(ui, f, Some(&mut gone));
                if gone {
                    remove = Some(i);
                }
            }
            ui.add_space(2.0);
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 2.0;
                let attach = ui
                    .add_enabled_ui(o.can_attach, |ui| {
                        theme::icon_button(ui, Icon::Attach, 24.0, false)
                    })
                    .inner;
                let attach = if o.can_attach {
                    attach.on_hover_text("Attach files")
                } else {
                    attach.on_disabled_hover_text("Attachments need a saved project")
                };
                if attach.clicked() {
                    ed.requests.push(AppRequest::PickAttachments);
                }
                let mention = ui
                    .add(
                        egui::Button::new(
                            RichText::new("@").font(theme::medium(14.0)).color(p.text),
                        )
                        .frame(false)
                        .min_size(vec2(24.0, 24.0)),
                    )
                    .on_hover_text("Mention someone");
                if mention.clicked() {
                    if !(c.text.is_empty() || c.text.ends_with(char::is_whitespace)) {
                        c.text.push(' ');
                    }
                    c.text.push('@');
                    caret_to_end(&ctx, o.id, &c.text);
                }
                if had_focus {
                    ui.add_space(4.0);
                    ui.label(RichText::new("⇧↩ new line").color(p.tertiary).size(10.0));
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let ready = !(c.text.trim().is_empty() && c.files.is_empty());
                    if send_button(ui, ready).clicked() && ready {
                        sent = true;
                    }
                });
            });
        });
    if let Some((at, hits)) = suggestions(&c.text, o.known).filter(|_| had_focus || o.focus) {
        c.pick = c.pick.min(hits.len() - 1);
        ui.add_space(4.0);
        let mut chosen = None;
        egui::Frame::NONE
            .fill(p.popover)
            .stroke(Stroke::new(1.0, p.separator))
            .corner_radius(8.0)
            .inner_margin(4.0)
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.spacing_mut().item_spacing.y = 0.0;
                for (i, a) in hits.iter().enumerate() {
                    let (r, resp) =
                        ui.allocate_exact_size(vec2(ui.available_width(), 28.0), Sense::click());
                    if resp.hovered() {
                        c.pick = i;
                    }
                    if i == c.pick {
                        ui.painter()
                            .rect_filled(r, 6.0, theme::accent().gamma_multiply(0.14));
                    }
                    let av = Rect::from_center_size(
                        pos2(r.left() + 16.0, r.center().y),
                        vec2(18.0, 18.0),
                    );
                    paint_avatar(ui.painter(), av, a, 1.0);
                    ui.painter().text(
                        pos2(r.left() + 32.0, r.center().y),
                        Align2::LEFT_CENTER,
                        &a.name,
                        theme::medium(theme::BODY),
                        p.text,
                    );
                    let note = if a.id == comments::ANY_AGENT {
                        "any connected agent"
                    } else if a.kind == AuthorKind::Agent {
                        "agent"
                    } else {
                        ""
                    };
                    ui.painter().text(
                        pos2(r.right() - 8.0, r.center().y),
                        Align2::RIGHT_CENTER,
                        note,
                        egui::FontId::proportional(theme::SECONDARY),
                        p.tertiary,
                    );
                    if resp
                        .on_hover_cursor(egui::CursorIcon::PointingHand)
                        .clicked()
                    {
                        chosen = Some((*a).clone());
                    }
                }
            });
        if let Some(a) = chosen {
            complete(&ctx, o.id, c, at, &a);
        }
    }
    if let Some(i) = remove {
        let f = c.files.remove(i);
        if let Some(dir) = &ed.project_dir {
            comments::remove_files(dir, &[f]);
        }
    }
    sent
}

/// A round send button with an up arrow; grey until there is something to send.
fn send_button(ui: &mut Ui, ready: bool) -> egui::Response {
    let p = theme::pal(ui);
    let (r, resp) = ui.allocate_exact_size(vec2(26.0, 26.0), Sense::click());
    let fill = if !ready {
        p.control
    } else if resp.hovered() {
        theme::blend(theme::accent(), Color32::from_black_alpha(30))
    } else {
        theme::accent()
    };
    let ink = if ready { Color32::WHITE } else { p.tertiary };
    let c = r.center();
    ui.painter().circle_filled(c, 13.0, fill);
    let s = Stroke::new(1.8, ink);
    ui.painter()
        .line_segment([c + vec2(0.0, 5.5), c + vec2(0.0, -5.0)], s);
    ui.painter()
        .line_segment([c + vec2(-4.5, -0.5), c + vec2(0.0, -5.0)], s);
    ui.painter()
        .line_segment([c + vec2(4.5, -0.5), c + vec2(0.0, -5.0)], s);
    let resp = if ready {
        resp.on_hover_cursor(egui::CursorIcon::PointingHand)
    } else {
        resp
    };
    resp.on_hover_text("Send (Return)")
}

fn notify_mention(ed: &mut Editor, thread_id: &str) {
    if let Some(message) = ed
        .comments
        .thread(thread_id)
        .and_then(|t| t.messages.last())
        && message
            .mentions
            .iter()
            .any(|id| id == comments::ANY_AGENT || id.starts_with("agent:"))
    {
        ed.requests.push(AppRequest::CommentMention {
            recipients: message.mentions.clone(),
            thread_id: thread_id.to_owned(),
            message_id: message.id.clone(),
        });
    }
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
            Act::Show(id) => {
                st.show(ed, &id);
                Ok(())
            }
            Act::Select(id) => {
                ed.selection = vec![id];
                ed.revision += 1;
                Ok(())
            }
            Act::Post => {
                if let Some(mut n) = st.new.take() {
                    let draft = n.composer.take();
                    let id = ed.comments.add_thread(&me, &n.pin, draft, &ed.doc, now);
                    notify_mention(ed, &id);
                    st.open = Some(id);
                    st.focus = true;
                    changed = true;
                }
                Ok(())
            }
            Act::Reply(t) => {
                let draft = st.reply.take();
                changed = true;
                ed.comments
                    .reply(&t, &me, draft, now)
                    .map(|_| notify_mention(ed, &t))
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
                st.drafts.remove(&t);
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

/// A row of options, one selected, on a sunken track.
fn segmented<T: Copy + PartialEq>(ui: &mut Ui, value: &mut T, options: &[(T, String)]) {
    let p = theme::pal(ui);
    egui::Frame::NONE
        .fill(p.control)
        .corner_radius(7.0)
        .inner_margin(2.0)
        .show(ui, |ui| {
            ui.spacing_mut().item_spacing.x = 2.0;
            ui.horizontal(|ui| {
                for (v, label) in options {
                    let on = *value == *v;
                    let b = egui::Button::new(
                        RichText::new(label)
                            .font(if on {
                                theme::semibold(theme::SECONDARY)
                            } else {
                                theme::medium(theme::SECONDARY)
                            })
                            .color(if on { p.text } else { p.secondary }),
                    )
                    .fill(if on { p.popover } else { Color32::TRANSPARENT })
                    .stroke(Stroke::NONE)
                    .corner_radius(5.0)
                    .min_size(vec2(0.0, 20.0));
                    if ui.add(b).clicked() {
                        *value = *v;
                    }
                }
            });
        });
}

/// The Comments panel: a status filter, every thread in the project as a card, and who you
/// comment as.
pub fn panel(ui: &mut Ui, ed: &mut Editor, st: &mut CommentsUi) {
    let p = theme::pal(ui);
    let live = |t: &&Thread| !t.messages.is_empty() && ed.pages.iter().any(|p| p.id == t.page);
    let open = ed
        .comments
        .threads
        .iter()
        .filter(live)
        .filter(|t| t.status == Status::Open)
        .count();
    let resolved = ed
        .comments
        .threads
        .iter()
        .filter(live)
        .filter(|t| t.status == Status::Resolved)
        .count();
    ui.horizontal(|ui| {
        segmented(
            ui,
            &mut st.filter,
            &[
                (Filter::Open, format!("Open {open}")),
                (Filter::Resolved, format!("Resolved {resolved}")),
                (Filter::All, "All".into()),
            ],
        );
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let (icon, tip) = if st.hidden {
                (Icon::EyeSlash, "Show comments on the canvas (Shift+C)")
            } else {
                (Icon::Eye, "Hide comments on the canvas (Shift+C)")
            };
            if theme::icon_button(ui, icon, 24.0, false)
                .on_hover_text(tip)
                .clicked()
            {
                st.hidden = !st.hidden;
            }
        });
    });
    ui.add_space(6.0);
    let mut threads: Vec<(usize, &Thread)> = ed
        .comments
        .threads
        .iter()
        .filter(|t| st.filter.shows(t.status) && !t.messages.is_empty())
        .filter_map(|t| Some((ed.pages.iter().position(|p| p.id == t.page)?, t)))
        .collect();
    threads.sort_by(|(pa, a), (pb, b)| pa.cmp(pb).then(b.updated.cmp(&a.updated)));
    let footer_h = 40.0;
    let list_h = (ui.available_height() - footer_h).max(60.0);
    let mut go = None;
    let mut comment_tool = false;
    if threads.is_empty() {
        let (title, body) = match st.filter {
            Filter::Open if resolved > 0 => (
                "All caught up",
                "Every comment is resolved. Press C and click the canvas to add another.",
            ),
            Filter::Open | Filter::All => (
                "No comments yet",
                "Press C, then click the canvas to pin a comment. Mention @agent to hand it to an AI agent.",
            ),
            Filter::Resolved => (
                "Nothing resolved yet",
                "Resolved comments move here, so you can look back at them.",
            ),
        };
        ui.allocate_ui(vec2(ui.available_width(), list_h), |ui| {
            ui.vertical_centered(|ui| {
                ui.add_space(28.0);
                let (r, _) = ui.allocate_exact_size(vec2(28.0, 28.0), Sense::hover());
                theme::paint_icon(ui.painter(), r, Icon::Comment, p.tertiary);
                ui.add_space(8.0);
                ui.label(RichText::new(title).font(theme::semibold(theme::BODY)));
                ui.add_space(2.0);
                ui.label(
                    RichText::new(body)
                        .color(p.secondary)
                        .size(theme::SECONDARY),
                );
                if st.filter != Filter::Resolved {
                    ui.add_space(10.0);
                    if ui.button("Add a comment").clicked() {
                        comment_tool = true;
                    }
                }
            });
        });
    } else {
        let known = ed.comments.mentionable(&ed.me);
        let now = now_ms();
        egui::ScrollArea::vertical()
            .max_height(list_h)
            .min_scrolled_height(list_h)
            .auto_shrink([false, false])
            .show(ui, |ui| {
                let mut last_page = None;
                for (page, t) in &threads {
                    if last_page != Some(*page) && ed.pages.len() > 1 {
                        ui.add_space(6.0);
                        ui.label(
                            RichText::new(&ed.pages[*page].name)
                                .font(theme::semibold(theme::SECONDARY))
                                .color(p.secondary),
                        );
                        last_page = Some(*page);
                    }
                    let selected = st.open.as_deref() == Some(t.id.as_str());
                    if card(ui, ed, st, t, &known, now, selected).clicked() {
                        go = Some((*page, t.id.clone()));
                    }
                }
            });
    }
    // Who you comment as.
    ui.add_space(4.0);
    rule(ui);
    ui.add_space(6.0);
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 6.0;
        let me = ed.me.clone();
        avatar(ui, &me, 20.0);
        match &mut st.name_edit {
            Some(name) => {
                let r = ui.add(egui::TextEdit::singleline(name).desired_width(130.0));
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
                ui.label(RichText::new(&me.name).font(theme::semibold(theme::SECONDARY)));
                ui.label(
                    RichText::new("(you)")
                        .color(p.tertiary)
                        .size(theme::SECONDARY),
                );
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let rename = egui::Button::new(
                        RichText::new("Rename")
                            .color(p.secondary)
                            .size(theme::SECONDARY),
                    )
                    .frame(false);
                    if ui
                        .add(rename)
                        .on_hover_text("Change the name others see on your comments")
                        .clicked()
                    {
                        st.name_edit = Some(ed.me.name.clone());
                        st.name_focus = true;
                    }
                });
            }
        }
    });
    if comment_tool {
        ed.run(crate::editor::shortcuts::Action::Tool(Tool::Comment));
    }
    if let Some((page, id)) = go {
        reveal(ed, st, page, &id);
    }
}

/// One thread in the panel.
fn card(
    ui: &mut Ui,
    ed: &Editor,
    st: &CommentsUi,
    t: &Thread,
    known: &[Author],
    now: u64,
    selected: bool,
) -> egui::Response {
    let p = theme::pal(ui);
    let first = &t.messages[0];
    let a = author(ed, &first.author);
    let resolved = t.status == Status::Resolved;
    let id = ui.id().with(("comment-card", &t.id));
    let hovered = ui.ctx().read_response(id).is_some_and(|r| r.hovered());
    let fill = if selected {
        theme::accent().gamma_multiply(0.12)
    } else if hovered {
        p.hover
    } else {
        Color32::TRANSPARENT
    };
    let text_color = if resolved { p.secondary } else { p.text };
    let r = egui::Frame::NONE
        .fill(fill)
        .corner_radius(8.0)
        .inner_margin(egui::Margin::symmetric(8, 8))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.spacing_mut().item_spacing.y = 4.0;
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 6.0;
                avatar(ui, &a, 20.0);
                ui.label(
                    RichText::new(&a.name)
                        .font(theme::semibold(theme::SECONDARY))
                        .color(text_color),
                );
                ui.label(
                    RichText::new(comments::ago(t.updated, now))
                        .color(p.tertiary)
                        .size(theme::SECONDARY),
                );
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if resolved {
                        let (r, _) = ui.allocate_exact_size(vec2(14.0, 14.0), Sense::hover());
                        theme::paint_icon(ui.painter(), r, Icon::Check, p.secondary);
                    } else if st.has_draft(&t.id) {
                        tag(ui, "Draft", p.secondary);
                    }
                });
            });
            let width = ui.available_width();
            ui.label(rich_text(
                &first.text,
                known,
                theme::SECONDARY + 1.0,
                text_color,
                width,
                2,
            ));
            let replies = t.messages.len() - 1;
            let files: usize = t.messages.iter().map(|m| m.attachments.len()).sum();
            let node = t
                .node_id()
                .and_then(|id| ed.doc.get(id))
                .map(|n| n.name.clone());
            // Everyone else in the thread, as overlapping avatars.
            let others: Vec<Author> = t
                .participants()
                .into_iter()
                .filter(|id| *id != first.author)
                .take(3)
                .map(|id| author(ed, id))
                .collect();
            if replies > 0 || files > 0 || node.is_some() || !others.is_empty() {
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 8.0;
                    if !others.is_empty() {
                        let w = 16.0 + 10.0 * (others.len() - 1) as f32;
                        let (r, _) = ui.allocate_exact_size(vec2(w, 16.0), Sense::hover());
                        for (i, o) in others.iter().enumerate() {
                            let c = Rect::from_min_size(
                                pos2(r.left() + 10.0 * i as f32, r.top()),
                                vec2(16.0, 16.0),
                            );
                            ui.painter().circle_filled(c.center(), 9.5, p.sidebar);
                            paint_avatar(ui.painter(), c, o, 1.0);
                        }
                    }
                    let mut meta = Vec::new();
                    if replies > 0 {
                        meta.push(plural(replies, "reply", "replies"));
                    }
                    if files > 0 {
                        meta.push(plural(files, "file", "files"));
                    }
                    if let Some(n) = node {
                        meta.push(format!("on {n}"));
                    }
                    ui.add(
                        egui::Label::new(
                            RichText::new(meta.join("  ·  "))
                                .color(p.secondary)
                                .size(theme::SECONDARY),
                        )
                        .truncate()
                        .selectable(false),
                    );
                });
            }
        })
        .response;
    let r = ui.interact(r.rect, id, Sense::click());
    r.on_hover_cursor(egui::CursorIcon::PointingHand)
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
    fn only_new_local_agent_mentions_notify_the_channel() {
        let mut ed = Editor::default();
        let mut st = CommentsUi::default();
        st.start(&mut ed, None, crate::geom::v2(1.0, 2.0));
        st.new.as_mut().unwrap().composer.text = "@Claude Code check the title".into();
        apply(&mut ed, &mut st, vec![Act::Post]);
        let thread = st.open.clone().unwrap();
        let event = |message_id: &String, recipient: &str| AppRequest::CommentMention {
            recipients: vec![recipient.to_owned()],
            thread_id: thread.clone(),
            message_id: message_id.clone(),
        };
        assert_eq!(
            ed.requests.pop(),
            Some(event(
                &ed.comments.thread(&thread).unwrap().messages[0].id,
                comments::CLAUDE_CODE
            ))
        );
        st.reply.text = "plain reply".into();
        apply(&mut ed, &mut st, vec![Act::Reply(thread.clone())]);
        assert!(ed.requests.is_empty());
        st.reply.text = "another look @agent".into();
        apply(&mut ed, &mut st, vec![Act::Reply(thread.clone())]);
        assert_eq!(
            ed.requests.pop(),
            Some(event(
                &ed.comments.thread(&thread).unwrap().messages[2].id,
                comments::ANY_AGENT
            ))
        );
        st.reply.text = "@Codex only".into();
        ed.comments
            .connected_agents
            .push(Author::agent(Some("Codex")));
        apply(
            &mut ed,
            &mut st,
            vec![Act::Reply(thread.clone()), Act::Close, Act::Show(thread)],
        );
        assert_eq!(
            ed.requests.len(),
            1,
            "a named Codex mention must be delivered"
        );
    }

    #[test]
    fn closing_a_draft_forgets_it() {
        let mut ed = Editor::default();
        let mut st = CommentsUi::default();
        st.start(&mut ed, None, crate::geom::v2(10.0, 10.0));
        assert!(st.new.is_some());
        st.close(&ed);
        assert!(st.new.is_none() && st.open.is_none());
    }

    #[test]
    fn an_unsent_reply_comes_back_when_the_thread_reopens() {
        let mut ed = Editor::default();
        let mut st = CommentsUi::default();
        st.show(&mut ed, "t1");
        st.reply.text = "half a thought".into();
        st.close(&ed);
        assert!(st.has_draft("t1"));
        st.show(&mut ed, "t2");
        assert!(st.reply.text.is_empty());
        st.show(&mut ed, "t1");
        assert_eq!(st.reply.text, "half a thought");
        assert!(!st.has_draft("t1"));
    }

    #[test]
    fn suggestions_complete_the_last_at_word() {
        let known = vec![
            Author::person("agent", "agent"),
            Author::person("u1", "Ann"),
            Author::agent(Some("claude-code")),
        ];
        let (at, hits) = suggestions("hi @c", &known).unwrap();
        assert_eq!(at, 3);
        assert_eq!(hits[0].name, "Claude Code");
        assert!(suggestions("mail@ann", &known).is_none());
        assert_eq!(suggestions("@", &known).unwrap().1.len(), 3);
        assert!(suggestions("@Ann", &known).is_none(), "already complete");
    }
}
