//! Comment threads: feedback pinned to a layer or to a spot on a page, so people and agents
//! can discuss a design and mark the feedback done.
//!
//! A thread is a first message plus replies, each by an [`Author`], with an open or resolved
//! status. Messages carry emoji reactions, file attachments (copied into the project's
//! `attachments/` folder) and @mentions of other authors. Comments are saved in the project
//! manifest and are not part of the design's undo history.
//!
//! Authors are named people and agents. The person using this Mac is [`local_author`], saved
//! in the config folder so the same name follows them across projects; anyone else who
//! comments in a shared project folder shows up under their own name. Agents are named after
//! the MCP client they connect with.

use crate::geom::{V2, v2};
use crate::model::{Document, NodeId};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// Folder inside the project that holds attached files.
pub const ATTACHMENTS_DIR: &str = "attachments";
/// Mention id that stands for any agent (`@agent`).
pub const ANY_AGENT: &str = "agent";
pub const CLAUDE_CODE: &str = "agent:claude-code";
/// Reactions offered by the picker. Any emoji is accepted from the tools.
pub const REACTIONS: [&str; 6] = ["👍", "❤", "🎉", "👀", "✅", "😄"];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Status {
    #[default]
    Open,
    Resolved,
}

impl Status {
    pub fn as_str(self) -> &'static str {
        match self {
            Status::Open => "open",
            Status::Resolved => "resolved",
        }
    }
    pub fn parse(s: &str) -> Option<Status> {
        match s {
            "open" => Some(Status::Open),
            "resolved" => Some(Status::Resolved),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AuthorKind {
    #[default]
    Person,
    Agent,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Author {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub kind: AuthorKind,
}

impl Author {
    pub fn person(id: &str, name: &str) -> Author {
        Author {
            id: id.into(),
            name: name.into(),
            kind: AuthorKind::Person,
        }
    }

    /// The author for an MCP client, e.g. `claude-code` becomes "Claude Code" with id
    /// `agent:claude-code`. Without a client name the agent is just "Agent".
    pub fn agent(client: Option<&str>) -> Author {
        let client = client.map(str::trim).filter(|c| !c.is_empty());
        let Some(c) = client else {
            return Author {
                id: "agent:unknown".into(),
                name: "Agent".into(),
                kind: AuthorKind::Agent,
            };
        };
        let slug: String = c
            .chars()
            .map(|ch| {
                if ch.is_alphanumeric() {
                    ch.to_ascii_lowercase()
                } else {
                    '-'
                }
            })
            .collect();
        let name = if c.contains(' ') || c.chars().any(|ch| ch.is_uppercase()) {
            c.to_string()
        } else {
            c.split(['-', '_'])
                .filter(|w| !w.is_empty())
                .map(|w| {
                    let mut cs = w.chars();
                    cs.next()
                        .map(|f| f.to_uppercase().chain(cs).collect::<String>())
                        .unwrap_or_default()
                })
                .collect::<Vec<_>>()
                .join(" ")
        };
        Author {
            id: format!("agent:{}", slug.trim_matches('-')),
            name,
            kind: AuthorKind::Agent,
        }
    }

    /// First letter for a pin or avatar.
    pub fn initial(&self) -> String {
        self.name
            .chars()
            .find(|c| c.is_alphanumeric())
            .map(|c| c.to_uppercase().collect())
            .unwrap_or_else(|| "?".into())
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Attachment {
    pub id: String,
    pub name: String,
    pub mime: String,
    pub size: u64,
    /// Path inside the project folder, e.g. `attachments/3fa9c1d2e4b5a697-shot.png`.
    pub file: String,
}

impl Attachment {
    pub fn is_image(&self) -> bool {
        self.mime.starts_with("image/")
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Message {
    pub id: String,
    pub author: String,
    pub text: String,
    /// Unix milliseconds.
    pub created: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub edited: Option<u64>,
    /// Emoji to the ids of the authors who reacted with it.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub reactions: BTreeMap<String, Vec<String>>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub attachments: Vec<Attachment>,
    /// Author ids mentioned in the text, plus [`ANY_AGENT`] for `@agent`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub mentions: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Thread {
    pub id: String,
    /// Page id.
    pub page: String,
    /// The layer the pin is attached to, by its number on that page.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub node: Option<u64>,
    /// Offset from the layer's top-left corner, or the page position when there is no layer.
    pub at: [f64; 2],
    /// Page position when the pin was placed, used once the layer is gone.
    pub point: [f64; 2],
    #[serde(default)]
    pub status: Status,
    pub created: u64,
    pub updated: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resolved_by: Option<String>,
    pub messages: Vec<Message>,
}

impl Thread {
    pub fn author(&self) -> &str {
        self.messages
            .first()
            .map(|m| m.author.as_str())
            .unwrap_or("")
    }

    pub fn node_id(&self) -> Option<NodeId> {
        self.node.map(NodeId)
    }

    /// Where the pin sits on its page: it follows its layer while the layer exists.
    pub fn world_point(&self, doc: &Document) -> V2 {
        match self.node_id().and_then(|id| doc.world_rect(id)) {
            Some(r) => v2(r.x + self.at[0], r.y + self.at[1]),
            None => v2(self.point[0], self.point[1]),
        }
    }

    pub fn message(&self, id: &str) -> Option<&Message> {
        self.messages.iter().find(|m| m.id == id)
    }

    pub fn participants(&self) -> Vec<&str> {
        let mut v: Vec<&str> = Vec::new();
        for m in &self.messages {
            if !v.contains(&m.author.as_str()) {
                v.push(&m.author);
            }
        }
        v
    }

    pub fn mentions(&self, id: &str) -> bool {
        self.messages
            .iter()
            .any(|m| m.mentions.iter().any(|x| x == id))
    }
}

/// Where a new thread goes.
#[derive(Debug, Clone, PartialEq)]
pub struct Pin {
    pub page: String,
    pub node: Option<NodeId>,
    /// Page position of the click.
    pub point: V2,
}

impl Pin {
    /// A pin at `point`, attached to `node` when there is one.
    pub fn new(doc: &Document, page: &str, node: Option<NodeId>, point: V2) -> Pin {
        let node = node.filter(|id| doc.get(*id).is_some());
        Pin {
            page: page.into(),
            node,
            point,
        }
    }
}

/// Everything a new message carries.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Draft {
    pub text: String,
    pub attachments: Vec<Attachment>,
}

impl Draft {
    pub fn text(text: &str) -> Draft {
        Draft {
            text: text.into(),
            attachments: Vec::new(),
        }
    }
    pub fn is_empty(&self) -> bool {
        self.text.trim().is_empty() && self.attachments.is_empty()
    }
}

/// A project's comment threads and the people and agents who wrote them.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Comments {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub threads: Vec<Thread>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub authors: Vec<Author>,
    #[serde(skip)]
    pub connected_agents: Vec<Author>,
}

impl Comments {
    pub fn is_empty(&self) -> bool {
        self.threads.is_empty() && self.authors.is_empty()
    }

    pub fn thread(&self, id: &str) -> Option<&Thread> {
        self.threads.iter().find(|t| t.id == id)
    }

    fn thread_mut(&mut self, id: &str) -> Result<&mut Thread, String> {
        self.threads
            .iter_mut()
            .find(|t| t.id == id)
            .ok_or_else(|| format!("comment thread `{id}` not found"))
    }

    pub fn author(&self, id: &str) -> Option<&Author> {
        self.authors
            .iter()
            .chain(&self.connected_agents)
            .find(|a| a.id == id)
    }

    /// The author's display name, or the id for an author this project doesn't know.
    pub fn name<'a>(&'a self, id: &'a str) -> &'a str {
        self.author(id)
            .map(|a| a.name.as_str())
            .unwrap_or(if id == CLAUDE_CODE { "Claude Code" } else { id })
    }

    /// Record `a` (or their new name) in the project.
    pub fn touch_author(&mut self, a: &Author) {
        match self.authors.iter_mut().find(|x| x.id == a.id) {
            Some(x) => *x = a.clone(),
            None => self.authors.push(a.clone()),
        }
    }

    /// Authors who can be @mentioned, `me` first and [`ANY_AGENT`] last.
    pub fn mentionable(&self, me: &Author) -> Vec<Author> {
        let mut v = vec![me.clone()];
        for a in self.authors.iter().chain(&self.connected_agents) {
            if !v.iter().any(|other| other.id == a.id) {
                v.push(a.clone());
            }
        }
        if !v.iter().any(|a| a.id == CLAUDE_CODE) {
            v.push(Author::agent(Some("claude-code")));
        }
        v.push(Author {
            id: ANY_AGENT.into(),
            name: ANY_AGENT.into(),
            kind: AuthorKind::Agent,
        });
        v
    }

    fn message(&self, by: &Author, draft: Draft, now: u64) -> Message {
        let known = self.mentionable(by);
        Message {
            id: crate::project::new_id(),
            author: by.id.clone(),
            mentions: parse_mentions(&draft.text, &known),
            text: draft.text.trim().to_string(),
            created: now,
            edited: None,
            reactions: BTreeMap::new(),
            attachments: draft.attachments,
        }
    }

    /// Start a thread at `pin`. Returns its id.
    pub fn add_thread(
        &mut self,
        by: &Author,
        pin: &Pin,
        draft: Draft,
        doc: &Document,
        now: u64,
    ) -> String {
        self.touch_author(by);
        let at = match pin.node.and_then(|id| doc.world_rect(id)) {
            Some(r) => [pin.point.x - r.x, pin.point.y - r.y],
            None => [pin.point.x, pin.point.y],
        };
        let m = self.message(by, draft, now);
        let id = crate::project::new_id();
        self.threads.push(Thread {
            id: id.clone(),
            page: pin.page.clone(),
            node: pin.node.map(|n| n.0),
            at,
            point: [pin.point.x, pin.point.y],
            status: Status::Open,
            created: now,
            updated: now,
            resolved_by: None,
            messages: vec![m],
        });
        id
    }

    /// Add a reply. Returns the new message's id.
    pub fn reply(
        &mut self,
        thread: &str,
        by: &Author,
        draft: Draft,
        now: u64,
    ) -> Result<String, String> {
        if draft.is_empty() {
            return Err("a reply needs text or an attachment".into());
        }
        self.thread_mut(thread)?;
        self.touch_author(by);
        let m = self.message(by, draft, now);
        let id = m.id.clone();
        let t = self.thread_mut(thread)?;
        t.messages.push(m);
        t.updated = now;
        Ok(id)
    }

    pub fn set_status(
        &mut self,
        thread: &str,
        status: Status,
        by: &Author,
        now: u64,
    ) -> Result<(), String> {
        self.touch_author(by);
        let t = self.thread_mut(thread)?;
        if t.status != status {
            t.status = status;
            t.updated = now;
        }
        t.resolved_by = (status == Status::Resolved).then(|| by.id.clone());
        Ok(())
    }

    /// Add (`on`), remove, or with `None` toggle `by`'s `emoji` reaction on a message (the
    /// first message when `message` is None). Returns whether the reaction is now on.
    pub fn react(
        &mut self,
        thread: &str,
        message: Option<&str>,
        emoji: &str,
        by: &Author,
        on: Option<bool>,
    ) -> Result<bool, String> {
        let emoji = emoji.trim();
        if emoji.is_empty() || emoji.chars().count() > 8 || emoji.chars().any(char::is_alphanumeric)
        {
            return Err(format!("`{emoji}` is not an emoji"));
        }
        self.touch_author(by);
        let t = self.thread_mut(thread)?;
        let m = match message {
            Some(id) => t
                .messages
                .iter_mut()
                .find(|m| m.id == id)
                .ok_or_else(|| format!("message `{id}` not found in thread `{thread}`"))?,
            None => t.messages.first_mut().ok_or("the thread has no messages")?,
        };
        let who = m.reactions.entry(emoji.to_string()).or_default();
        let has = who.contains(&by.id);
        let want = on.unwrap_or(!has);
        if want && !has {
            who.push(by.id.clone());
        } else if !want {
            who.retain(|x| x != &by.id);
        }
        if who.is_empty() {
            m.reactions.remove(emoji);
        }
        Ok(want)
    }

    /// Change a message's text (its author only).
    pub fn edit(
        &mut self,
        thread: &str,
        message: &str,
        by: &Author,
        text: &str,
        now: u64,
    ) -> Result<(), String> {
        let known = self.mentionable(by);
        let t = self.thread_mut(thread)?;
        let m = t
            .messages
            .iter_mut()
            .find(|m| m.id == message)
            .ok_or_else(|| format!("message `{message}` not found"))?;
        if m.author != by.id {
            return Err("only the author can edit a message".into());
        }
        m.text = text.trim().to_string();
        m.mentions = parse_mentions(&m.text, &known);
        m.edited = Some(now);
        t.updated = now;
        Ok(())
    }

    /// Delete a message; deleting the first one deletes the thread. Returns the attachments
    /// that went with it, so their files can be removed.
    pub fn delete_message(
        &mut self,
        thread: &str,
        message: &str,
    ) -> Result<Vec<Attachment>, String> {
        let t = self.thread_mut(thread)?;
        let i = t
            .messages
            .iter()
            .position(|m| m.id == message)
            .ok_or_else(|| format!("message `{message}` not found"))?;
        if i == 0 {
            return self.delete_thread(thread);
        }
        Ok(t.messages.remove(i).attachments)
    }

    pub fn delete_thread(&mut self, thread: &str) -> Result<Vec<Attachment>, String> {
        let i = self
            .threads
            .iter()
            .position(|t| t.id == thread)
            .ok_or_else(|| format!("comment thread `{thread}` not found"))?;
        let t = self.threads.remove(i);
        Ok(t.messages.into_iter().flat_map(|m| m.attachments).collect())
    }

    /// Drop the threads of pages that no longer exist.
    pub fn retain_pages(&mut self, pages: &[&str]) -> Vec<Attachment> {
        let mut gone = Vec::new();
        self.threads.retain(|t| {
            let keep = pages.contains(&t.page.as_str());
            if !keep {
                gone.extend(t.messages.iter().flat_map(|m| m.attachments.clone()));
            }
            keep
        });
        gone
    }
}

/// The author ids `@name` mentions in `text` refer to. Longer names win, so "@Ann Lee" is not
/// read as "@Ann"; a name must end at a word boundary.
pub fn parse_mentions(text: &str, known: &[Author]) -> Vec<String> {
    let lower = text.to_lowercase();
    let mut names: Vec<(String, &str)> = known
        .iter()
        .filter(|a| !a.name.trim().is_empty())
        .map(|a| (a.name.to_lowercase(), a.id.as_str()))
        .collect();
    names.sort_by_key(|a| std::cmp::Reverse(a.0.chars().count()));
    let mut out: Vec<String> = Vec::new();
    let mut taken: Vec<std::ops::Range<usize>> = Vec::new();
    for (name, id) in &names {
        let needle = format!("@{name}");
        let mut from = 0;
        while let Some(i) = lower[from..].find(&needle) {
            let start = from + i;
            let end = start + needle.len();
            from = end;
            let before_ok = lower[..start]
                .chars()
                .next_back()
                .is_none_or(|c| !c.is_alphanumeric());
            let after_ok = lower[end..]
                .chars()
                .next()
                .is_none_or(|c| !c.is_alphanumeric() && c != '_');
            if !(before_ok && after_ok) || taken.iter().any(|r| r.start < end && start < r.end) {
                continue;
            }
            taken.push(start..end);
            if !out.iter().any(|x| x == id) {
                out.push(id.to_string());
            }
        }
    }
    out
}

/// Byte ranges of the mentions in `text`, for highlighting.
pub fn mention_spans(text: &str, known: &[Author]) -> Vec<std::ops::Range<usize>> {
    let lower = text.to_lowercase();
    if lower.len() != text.len() {
        // Lowercasing changed byte offsets (rare scripts); skip highlighting.
        return Vec::new();
    }
    let mut spans: Vec<std::ops::Range<usize>> = Vec::new();
    let mut names: Vec<String> = known.iter().map(|a| a.name.to_lowercase()).collect();
    names.sort_by_key(|n| std::cmp::Reverse(n.len()));
    for name in names.iter().filter(|n| !n.is_empty()) {
        let needle = format!("@{name}");
        let mut from = 0;
        while let Some(i) = lower[from..].find(&needle) {
            let start = from + i;
            let end = start + needle.len();
            from = end;
            let after_ok = lower[end..]
                .chars()
                .next()
                .is_none_or(|c| !c.is_alphanumeric() && c != '_');
            let before_ok = lower[..start]
                .chars()
                .next_back()
                .is_none_or(|c| !c.is_alphanumeric());
            if after_ok && before_ok && !spans.iter().any(|r| r.start < end && start < r.end) {
                spans.push(start..end);
            }
        }
    }
    spans.sort_by_key(|r| r.start);
    spans
}

// ---------------------------------------------------------------------------------- files

/// A file's type from its extension.
pub fn mime_of(path: &Path) -> &'static str {
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    match ext.as_str() {
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "svg" => "image/svg+xml",
        "pdf" => "application/pdf",
        "txt" | "md" => "text/plain",
        "json" => "application/json",
        "mp4" => "video/mp4",
        "mov" => "video/quicktime",
        _ => "application/octet-stream",
    }
}

/// Largest file that can be attached.
pub const MAX_ATTACHMENT: u64 = 50 << 20;

/// Copy `src` into the project's attachments folder.
pub fn attach(project: &Path, src: &Path) -> Result<Attachment, String> {
    let meta = std::fs::metadata(src).map_err(|e| format!("{}: {e}", src.display()))?;
    if !meta.is_file() {
        return Err(format!("{} is not a file", src.display()));
    }
    if meta.len() > MAX_ATTACHMENT {
        return Err(format!("{} is larger than 50 MB", src.display()));
    }
    let name = src
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("file")
        .to_string();
    let safe: String = name
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || ".-_".contains(c) {
                c
            } else {
                '-'
            }
        })
        .collect();
    let id = crate::project::new_id();
    let file = format!("{ATTACHMENTS_DIR}/{id}-{safe}");
    let dest = project.join(&file);
    std::fs::create_dir_all(dest.parent().unwrap()).map_err(|e| e.to_string())?;
    std::fs::copy(src, &dest).map_err(|e| format!("copying {}: {e}", src.display()))?;
    Ok(Attachment {
        id,
        name,
        mime: mime_of(src).into(),
        size: meta.len(),
        file,
    })
}

/// Remove attachment files that no longer belong to a message.
pub fn remove_files(project: &Path, gone: &[Attachment]) {
    for a in gone {
        if !a.file.contains("..") {
            let _ = std::fs::remove_file(project.join(&a.file));
        }
    }
}

// ---------------------------------------------------------------------------------- identity

fn author_file() -> Option<PathBuf> {
    dirs::config_dir().map(|d| d.join("sketch-rs").join("author.json"))
}

/// The person using this Mac: loaded from the config folder, or created there on first use
/// with the account's name.
pub fn local_author() -> Author {
    if let Some(a) = author_file()
        .and_then(|p| std::fs::read_to_string(p).ok())
        .and_then(|s| serde_json::from_str::<Author>(&s).ok())
        .filter(|a| !a.id.is_empty())
    {
        return Author {
            kind: AuthorKind::Person,
            ..a
        };
    }
    let a = Author::person(
        &format!("user:{}", crate::project::new_id()),
        &account_name(),
    );
    save_local_author(&a);
    a
}

/// Remember a new display name for the person using this Mac.
pub fn save_local_author(a: &Author) {
    if let Some(p) = author_file() {
        let _ = std::fs::create_dir_all(p.parent().unwrap());
        let _ = std::fs::write(p, serde_json::to_string_pretty(a).unwrap_or_default());
    }
}

fn account_name() -> String {
    #[cfg(target_os = "macos")]
    if let Ok(out) = std::process::Command::new("id").arg("-F").output()
        && out.status.success()
    {
        let s = String::from_utf8_lossy(&out.stdout).trim().to_string();
        if !s.is_empty() {
            return s;
        }
    }
    std::env::var("USER")
        .or_else(|_| std::env::var("USERNAME"))
        .ok()
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "Me".into())
}

// ---------------------------------------------------------------------------------- time

/// Unix milliseconds as an ISO 8601 UTC timestamp, e.g. `2026-11-20T10:45:12Z`.
pub fn iso(ms: u64) -> String {
    let secs = ms / 1000;
    let (days, rem) = (secs / 86_400, secs % 86_400);
    // Civil date from days since 1970-01-01 (Howard Hinnant's algorithm).
    let z = days as i64 + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    format!(
        "{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}Z",
        rem / 3600,
        rem / 60 % 60,
        rem % 60
    )
}

/// "now", "5m", "3h", "2d" or a date, for the thread view.
pub fn ago(ms: u64, now: u64) -> String {
    let s = now.saturating_sub(ms) / 1000;
    match s {
        0..60 => "now".into(),
        60..3600 => format!("{}m", s / 60),
        3600..86_400 => format!("{}h", s / 3600),
        86_400..604_800 => format!("{}d", s / 86_400),
        _ => iso(ms)[..10].to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::geom::Rect;
    use crate::model::{Node, NodeKind};

    fn people() -> Vec<Author> {
        vec![
            Author::person("u1", "Ann"),
            Author::person("u2", "Ann Lee"),
            Author::agent(Some("claude-code")),
        ]
    }

    #[test]
    fn mentions_prefer_the_longest_name_and_need_word_boundaries() {
        let k = people();
        assert_eq!(parse_mentions("hey @ann lee, look", &k), vec!["u2"]);
        assert_eq!(
            parse_mentions("@Ann and @Claude Code", &k),
            vec!["agent:claude-code", "u1"]
        );
        assert!(parse_mentions("mail@ann.com", &k).is_empty());
        assert!(parse_mentions("@annie", &k).is_empty());
        assert_eq!(mention_spans("hi @Ann!", &k), vec![3..7]);
    }

    #[test]
    fn agent_names_come_from_the_client() {
        let a = Author::agent(Some("claude-code"));
        assert_eq!(
            (a.id.as_str(), a.name.as_str()),
            ("agent:claude-code", "Claude Code")
        );
        assert_eq!(Author::agent(Some("Cursor")).name, "Cursor");
        assert_eq!(Author::agent(None).name, "Agent");
    }

    #[test]
    fn pins_follow_their_layer_and_stay_put_once_it_is_gone() {
        let mut doc = Document::default();
        let id = NodeId(1);
        doc.nodes.insert(
            id,
            Node::new(id, NodeKind::Rect, Rect::new(100.0, 100.0, 50.0, 50.0)),
        );
        doc.root.push(id);
        let me = Author::person("u1", "Ann");
        let mut c = Comments::default();
        let pin = Pin::new(&doc, "p", Some(id), v2(110.0, 120.0));
        let t = c.add_thread(&me, &pin, Draft::text("too dark"), &doc, 1);
        doc.nodes.get_mut(&id).unwrap().rect = Rect::new(200.0, 100.0, 50.0, 50.0);
        assert_eq!(c.thread(&t).unwrap().world_point(&doc), v2(210.0, 120.0));
        doc.nodes.remove(&id);
        assert_eq!(c.thread(&t).unwrap().world_point(&doc), v2(110.0, 120.0));
    }

    #[test]
    fn replies_reactions_and_status() {
        let doc = Document::default();
        let ann = Author::person("u1", "Ann");
        let bot = Author::agent(Some("claude-code"));
        let mut c = Comments::default();
        let t = c.add_thread(
            &ann,
            &Pin::new(&doc, "p", None, v2(1.0, 2.0)),
            Draft::text("@agent fix"),
            &doc,
            1,
        );
        assert!(c.thread(&t).unwrap().mentions(ANY_AGENT));
        let r = c.reply(&t, &bot, Draft::text("done @Ann"), 2).unwrap();
        assert_eq!(
            c.thread(&t).unwrap().message(&r).unwrap().mentions,
            vec!["u1"]
        );
        assert!(c.react(&t, None, "👍", &bot, None).unwrap());
        assert!(!c.react(&t, None, "👍", &bot, None).unwrap());
        assert!(c.react(&t, None, "abc", &bot, None).is_err());
        c.set_status(&t, Status::Resolved, &bot, 3).unwrap();
        let th = c.thread(&t).unwrap();
        assert_eq!(
            (th.status, th.resolved_by.as_deref()),
            (Status::Resolved, Some("agent:claude-code"))
        );
        assert_eq!(c.authors.len(), 2);
        assert!(c.edit(&t, &r, &ann, "x", 4).is_err());
        c.delete_message(&t, &r).unwrap();
        assert_eq!(c.thread(&t).unwrap().messages.len(), 1);
        let first = c.thread(&t).unwrap().messages[0].id.clone();
        c.delete_message(&t, &first).unwrap();
        assert!(c.thread(&t).is_none());
    }

    #[test]
    fn iso_dates() {
        assert_eq!(iso(0), "1970-01-01T00:00:00Z");
        assert_eq!(iso(1_795_084_712_000), "2026-11-19T10:38:32Z");
        assert_eq!(ago(0, 90_000), "1m");
    }
}
