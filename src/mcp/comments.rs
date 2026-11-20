//! Comment tools: agents read the feedback people pin to the design, reply, react and mark
//! threads resolved once the design is fixed.
//!
//! `list_comment_threads`, `get_comment_thread`, `list_comment_thread_authors` and
//! `set_comment_thread_status` take the same arguments as Paper's tools of those names, so
//! prompts written for Paper work here. `reply_to_comment_thread` and `set_comment_reaction`
//! are Sketch's own. Writes are made as the agent that called, named after its MCP client.

use super::exec::{find_page, node_key, resolve};
use super::{Out, ToolResult};
use crate::comments::{self, ANY_AGENT, Author, AuthorKind, Draft, Status, Thread};
use crate::editor::Editor;
use crate::project::now_ms;
use serde_json::{Map, Value, json};
use std::cell::RefCell;
use std::collections::BTreeMap;

thread_local! {
    static CLIENT: RefCell<Option<String>> = const { RefCell::new(None) };
}

/// Restores the previous client name when dropped.
pub struct Scope(Option<String>);

impl Drop for Scope {
    fn drop(&mut self) {
        CLIENT.with(|c| *c.borrow_mut() = self.0.take());
    }
}

/// Make comment writes during this call as the agent named `client`.
pub fn scope(client: Option<&str>) -> Scope {
    Scope(CLIENT.with(|c| c.replace(client.map(str::to_string))))
}

fn agent() -> Author {
    CLIENT.with(|c| Author::agent(c.borrow().as_deref()))
}

/// Biggest image attachment get_comment_thread also returns as an image.
const INLINE_IMAGE: u64 = 5 << 20;

fn s<'a>(a: &'a Map<String, Value>, k: &str) -> Option<&'a str> {
    a.get(k)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|v| !v.is_empty())
}

fn n(a: &Map<String, Value>, k: &str) -> Option<u64> {
    a.get(k)
        .and_then(|v| v.as_u64().or_else(|| v.as_str()?.trim().parse().ok()))
}

fn out(v: Value) -> ToolResult {
    Ok(vec![Out::Text(
        serde_json::to_string_pretty(&v).unwrap_or_default(),
    )])
}

/// "current-user" is the person using the app, whom the agent works for.
fn user_arg(ed: &Editor, a: &Map<String, Value>, k: &str) -> Option<String> {
    s(a, k).map(|u| {
        if u == "current-user" {
            ed.me.id.clone()
        } else {
            u.to_string()
        }
    })
}

fn page_of(ed: &Editor, t: &Thread) -> Option<usize> {
    ed.pages.iter().position(|p| p.id == t.page)
}

fn name(ed: &Editor, id: &str) -> String {
    if id == ed.me.id {
        return ed.me.name.clone();
    }
    if id == ANY_AGENT {
        return "agent".into();
    }
    ed.comments.name(id).to_string()
}

fn kind(ed: &Editor, id: &str) -> &'static str {
    match ed.comments.author(id).map(|a| a.kind) {
        Some(AuthorKind::Agent) => "agent",
        _ if id.starts_with("agent:") || id == ANY_AGENT => "agent",
        _ => "person",
    }
}

fn node_json(ed: &Editor, page: usize, t: &Thread) -> Value {
    let Some(id) = t.node_id() else {
        return Value::Null;
    };
    match ed.page_doc(page).get(id) {
        Some(n) => json!({
            "nodeId": node_key(ed, page, id),
            "name": n.name,
            "type": n.kind.type_name(),
            "exists": true,
        }),
        None => json!({"nodeId": node_key(ed, page, id), "exists": false}),
    }
}

/// A message as a pushed notification carries it: who wrote it, what it says, where it is, and
/// the replies just before it. `None` when the thread or message is gone.
pub fn brief(ed: &Editor, thread: &str, message: &str) -> Option<super::mentions::Brief> {
    let t = ed.comments.thread(thread)?;
    let i = t.messages.iter().position(|m| m.id == message)?;
    let page = page_of(ed, t)?;
    let m = &t.messages[i];
    let node = t.node_id().map(|id| {
        (
            node_key(ed, page, id),
            ed.page_doc(page).get(id).map(|n| n.name.clone()),
        )
    });
    let from = i.saturating_sub(3);
    Some(super::mentions::Brief {
        thread_id: t.id.clone(),
        message_id: m.id.clone(),
        author: ed.comments.name(&m.author).to_string(),
        text: preview(&m.text, 1500),
        page: ed.pages[page].name.clone(),
        node_id: node.as_ref().map(|(k, _)| k.clone()),
        node_name: node.and_then(|(_, n)| n),
        earlier: t.messages[from..i]
            .iter()
            .map(|e| format!("{}: {}", ed.comments.name(&e.author), preview(&e.text, 280)))
            .collect(),
        older: from > 0,
        attachments: t
            .messages
            .iter()
            .flat_map(|m| m.attachments.iter().map(|a| a.name.clone()))
            .collect(),
    })
}

fn preview(text: &str, len: usize) -> String {
    let t: String = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if t.chars().count() <= len {
        t
    } else {
        format!("{}…", t.chars().take(len).collect::<String>())
    }
}

fn summary(ed: &Editor, page: usize, t: &Thread, preview_len: usize) -> Value {
    let first = &t.messages[0];
    json!({
        "commentThreadId": t.id,
        "status": t.status.as_str(),
        "pageId": t.page,
        "pageName": ed.pages[page].name,
        "node": node_json(ed, page, t),
        "authorUserId": first.author,
        "authorName": name(ed, &first.author),
        "preview": preview(&first.text, preview_len),
        "replyCount": t.messages.len() - 1,
        "participants": t.participants().iter().map(|p| json!({"userId": p, "name": name(ed, p)})).collect::<Vec<_>>(),
        "mentions": mentions(ed, t.messages.iter().flat_map(|m| m.mentions.iter())),
        "reactionCount": t.messages.iter().map(|m| m.reactions.values().map(Vec::len).sum::<usize>()).sum::<usize>(),
        "attachmentCount": t.messages.iter().map(|m| m.attachments.len()).sum::<usize>(),
        "createdAt": comments::iso(t.created),
        "updatedAt": comments::iso(t.updated),
    })
}

fn mentions<'a>(ed: &Editor, ids: impl Iterator<Item = &'a String>) -> Vec<Value> {
    let mut seen: Vec<&str> = Vec::new();
    for id in ids {
        if !seen.contains(&id.as_str()) {
            seen.push(id);
        }
    }
    seen.into_iter()
        .map(|id| json!({"userId": id, "name": name(ed, id), "kind": kind(ed, id)}))
        .collect()
}

pub fn list_threads(ed: &Editor, a: &Map<String, Value>) -> ToolResult {
    let status = match s(a, "status").unwrap_or("open") {
        "all" => None,
        st => Some(
            Status::parse(st)
                .ok_or_else(|| format!("status must be open, resolved or all, not `{st}`"))?,
        ),
    };
    let page = match s(a, "pageId") {
        Some(p) => Some(find_page(ed, p)?),
        None if a.get("currentPageOnly").and_then(Value::as_bool) == Some(true) => {
            Some(ed.active_page)
        }
        None => None,
    };
    let node = match s(a, "nodeId") {
        Some(k) => Some(resolve(ed, k, page.unwrap_or(ed.active_page))?),
        None => None,
    };
    let author = user_arg(ed, a, "threadAuthorUserId");
    let participant = user_arg(ed, a, "participantUserId");
    let mentioned = user_arg(ed, a, "mentionedUserId");
    let search = s(a, "search").map(str::to_lowercase);
    let first_only = s(a, "searchScope") == Some("first-message");
    let preview_len = n(a, "previewLength").unwrap_or(240).clamp(20, 1000) as usize;
    let limit = n(a, "limit").unwrap_or(50).clamp(1, 200) as usize;
    let offset = n(a, "offset").unwrap_or(0) as usize;

    let mut hits: Vec<(usize, &Thread)> = ed
        .comments
        .threads
        .iter()
        .filter(|t| !t.messages.is_empty())
        .filter_map(|t| Some((page_of(ed, t)?, t)))
        .filter(|(p, t)| {
            status.is_none_or(|s| t.status == s)
                && page.is_none_or(|q| *p == q)
                && node.is_none_or(|(q, id)| *p == q && t.node_id() == Some(id))
                && author.as_deref().is_none_or(|u| t.author() == u)
                && participant
                    .as_deref()
                    .is_none_or(|u| t.participants().contains(&u))
                && mentioned.as_deref().is_none_or(|u| {
                    t.mentions(u)
                        || (kind(ed, u) == "agent" && u != ANY_AGENT && t.mentions(ANY_AGENT))
                })
                && search.as_deref().is_none_or(|q| {
                    let msgs = if first_only {
                        &t.messages[..1]
                    } else {
                        &t.messages[..]
                    };
                    msgs.iter().any(|m| {
                        m.text.to_lowercase().contains(q)
                            || name(ed, &m.author).to_lowercase().contains(q)
                    })
                })
        })
        .collect();
    let field = s(a, "sortField").unwrap_or("page");
    hits.sort_by(|(pa, a), (pb, b)| match field {
        "createdAt" => a.created.cmp(&b.created),
        "updatedAt" => a.updated.cmp(&b.updated),
        // Page order, then top to bottom and left to right, like reading the page.
        _ => pa.cmp(pb).then_with(|| {
            let (da, db) = (ed.page_doc(*pa), ed.page_doc(*pb));
            let (wa, wb) = (a.world_point(da), b.world_point(db));
            wa.y.total_cmp(&wb.y).then(wa.x.total_cmp(&wb.x))
        }),
    });
    if s(a, "sortDirection") == Some("desc") {
        hits.reverse();
    }
    let total = hits.len();
    let threads: Vec<Value> = hits
        .iter()
        .skip(offset)
        .take(limit)
        .map(|(p, t)| summary(ed, *p, t, preview_len))
        .collect();
    out(json!({
        "threads": threads,
        "total": total,
        "offset": offset,
        "hasMore": offset + threads.len() < total,
        "currentUserId": ed.me.id,
    }))
}

fn thread_arg<'a>(ed: &'a Editor, a: &Map<String, Value>) -> Result<(usize, &'a Thread), String> {
    let id = s(a, "commentThreadId").ok_or("missing required string argument `commentThreadId`")?;
    let t = ed
        .comments
        .thread(id)
        .filter(|t| !t.messages.is_empty())
        .ok_or_else(|| format!("comment thread `{id}` not found (see list_comment_threads)"))?;
    let page = page_of(ed, t).ok_or_else(|| format!("comment thread `{id}` not found"))?;
    Ok((page, t))
}

/// A `file://` URL with everything but unreserved characters and `/` percent-encoded.
fn file_url(p: &std::path::Path) -> String {
    let mut out = String::from("file://");
    for b in p.to_string_lossy().bytes() {
        if b.is_ascii_alphanumeric() || b"/-._~".contains(&b) {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

pub fn get_thread(ed: &mut Editor, a: &Map<String, Value>) -> ToolResult {
    // Reading a thread counts as picking it up, so it stops waiting to be sent.
    let id = thread_arg(ed, a)?.1.id.clone();
    if ed.comments.clear_pending(&id) {
        written(ed);
    }
    let ed = &*ed;
    let (page, t) = thread_arg(ed, a)?;
    let dir = ed.project_dir.as_deref();
    let mut images = Vec::new();
    let messages: Vec<Value> = t
        .messages
        .iter()
        .map(|m| {
            let attachments: Vec<Value> = m
                .attachments
                .iter()
                .map(|f| {
                    let path = dir.map(|d| d.join(&f.file));
                    if f.is_image()
                        && f.size <= INLINE_IMAGE
                        && f.mime != "image/svg+xml"
                        && let Some(bytes) = path.as_ref().and_then(|p| std::fs::read(p).ok())
                    {
                        images.push(Out::Image { data: bytes, mime: f.mime.clone() });
                    }
                    json!({
                        "attachmentId": f.id,
                        "name": f.name,
                        "mimeType": f.mime,
                        "size": f.size,
                        "path": path.as_ref().map(|p| p.display().to_string()),
                        "url": path.as_ref().map(|p| file_url(p)),
                    })
                })
                .collect();
            let reactions: Vec<Value> = m
                .reactions
                .iter()
                .map(|(e, who)| {
                    json!({
                        "emoji": e,
                        "count": who.len(),
                        "users": who.iter().map(|u| json!({"userId": u, "name": name(ed, u)})).collect::<Vec<_>>(),
                    })
                })
                .collect();
            json!({
                "messageId": m.id,
                "authorUserId": m.author,
                "authorName": name(ed, &m.author),
                "authorKind": kind(ed, &m.author),
                "text": m.text,
                "createdAt": comments::iso(m.created),
                "editedAt": m.edited.map(comments::iso),
                "mentions": mentions(ed, m.mentions.iter()),
                "reactions": reactions,
                "attachments": attachments,
            })
        })
        .collect();
    let at = t.world_point(ed.page_doc(page));
    let v = json!({
        "commentThreadId": t.id,
        "status": t.status.as_str(),
        "resolvedBy": t.resolved_by.as_deref().map(|u| json!({"userId": u, "name": name(ed, u)})),
        "pageId": t.page,
        "pageName": ed.pages[page].name,
        "node": node_json(ed, page, t),
        "position": {"x": (at.x * 100.0).round() / 100.0, "y": (at.y * 100.0).round() / 100.0},
        "createdAt": comments::iso(t.created),
        "updatedAt": comments::iso(t.updated),
        "messages": messages,
        "note": if images.is_empty() { Value::Null } else { json!("Image attachments follow as images, in message order.") },
    });
    let mut r = out(v)?;
    r.extend(images);
    Ok(r)
}

pub fn list_authors(ed: &Editor, a: &Map<String, Value>) -> ToolResult {
    let page = match s(a, "pageId") {
        Some(p) => Some(find_page(ed, p)?),
        None => None,
    };
    #[derive(Default)]
    struct Count {
        threads: usize,
        messages: usize,
        last: u64,
    }
    let mut counts: BTreeMap<String, Count> = BTreeMap::new();
    for t in &ed.comments.threads {
        let Some(p) = page_of(ed, t) else { continue };
        if page.is_some_and(|q| q != p) || t.messages.is_empty() {
            continue;
        }
        counts.entry(t.author().to_string()).or_default().threads += 1;
        for m in &t.messages {
            let c = counts.entry(m.author.clone()).or_default();
            c.messages += 1;
            c.last = c.last.max(m.created);
        }
    }
    let mut authors: Vec<(String, Count)> = counts.into_iter().collect();
    authors.sort_by_key(|a| std::cmp::Reverse(a.1.last));
    let list: Vec<Value> = authors
        .iter()
        .map(|(id, c)| {
            json!({
                "userId": id,
                "displayName": name(ed, id),
                "kind": kind(ed, id),
                "isCurrentUser": *id == ed.me.id,
                "threadsStarted": c.threads,
                "messagesWritten": c.messages,
                "lastActivityAt": comments::iso(c.last),
            })
        })
        .collect();
    out(json!({"authors": list, "currentUserId": ed.me.id}))
}

fn written(ed: &mut Editor) {
    ed.meta_dirty = true;
    ed.revision += 1;
}

pub fn set_status(ed: &mut Editor, a: &Map<String, Value>) -> ToolResult {
    let id = thread_arg(ed, a)?.1.id.clone();
    let st = s(a, "status").ok_or("missing required string argument `status`")?;
    let status =
        Status::parse(st).ok_or_else(|| format!("status must be open or resolved, not `{st}`"))?;
    let by = agent();
    ed.comments.set_status(&id, status, &by, now_ms())?;
    ed.comments.clear_pending(&id);
    written(ed);
    out(json!({"commentThreadId": id, "status": status.as_str()}))
}

pub fn reply(ed: &mut Editor, a: &Map<String, Value>) -> ToolResult {
    let id = thread_arg(ed, a)?.1.id.clone();
    let text = s(a, "text").ok_or("missing required string argument `text`")?;
    let by = agent();
    let resolve = match a.get("resolve") {
        None | Some(Value::Null) => false,
        Some(Value::Bool(b)) => *b,
        Some(_) => return Err("`resolve` must be true or false".into()),
    };
    let now = now_ms();
    let mid = ed.comments.reply(&id, &by, Draft::text(text), now)?;
    // Replying and closing the thread in one call saves the agent a round trip.
    if resolve {
        ed.comments.set_status(&id, Status::Resolved, &by, now)?;
    }
    ed.comments.clear_pending(&id);
    let t = ed.comments.thread(&id).expect("thread exists");
    let m = t.message(&mid).expect("message exists");
    let v = json!({
        "commentThreadId": id,
        "messageId": mid,
        "authorUserId": by.id,
        "authorName": by.name,
        "mentions": mentions(ed, m.mentions.iter()),
        "status": t.status.as_str(),
    });
    written(ed);
    out(v)
}

pub fn react(ed: &mut Editor, a: &Map<String, Value>) -> ToolResult {
    let id = thread_arg(ed, a)?.1.id.clone();
    let emoji = s(a, "emoji").ok_or("missing required string argument `emoji`")?;
    let on = a.get("on").and_then(Value::as_bool).unwrap_or(true);
    let by = agent();
    let message = s(a, "messageId");
    let now = ed.comments.react(&id, message, emoji, &by, Some(on))?;
    ed.comments.clear_pending(&id);
    written(ed);
    out(json!({"commentThreadId": id, "emoji": emoji, "on": now}))
}
