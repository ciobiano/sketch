//! Deliver each addressed comment to the listener's chosen host.
use rmcp::model::{
    ClientConfig, ClientRequest, CustomNotification, CustomRequest, ServerNotification,
    ServerResult,
};
use rmcp::transport::StreamableHttpClientTransport;
use rmcp::{Peer, RoleServer, ServiceExt};
use serde_json::json;
use sketch_rs::comments::Author;
use sketch_rs::mcp::mentions::{Batch, Mention, WAIT_METHOD};
use std::{process::Stdio, time::Duration};
use tokio::io::AsyncWriteExt;

pub const INSTRUCTIONS: &str = "A channel event means the person in Sketch sent you a comment, \
or several at once. The event says what each comment says, who wrote it, and which layer \
(nodeId) and page it is on, so you can start on the design directly; call get_comment_thread \
only for attachments or older replies. Pass the event's fileId to the tools. When a comment \
is handled, reply in its thread with reply_to_comment_thread and resolve: true.";

type Error = Box<dyn std::error::Error + Send + Sync>;

pub enum Destination {
    Claude(Peer<RoleServer>),
    Command { agent: String, argv: Vec<String> },
}

impl Destination {
    fn name(&self) -> &str {
        match self {
            Self::Claude(_) => "Claude Code",
            Self::Command { agent, .. } => agent,
        }
    }

    fn open(&self) -> bool {
        match self {
            Self::Claude(peer) => !peer.is_transport_closed(),
            Self::Command { .. } => true,
        }
    }

    async fn deliver(&self, mention: &Mention, url: &str) -> Result<(), Error> {
        match self {
            Self::Claude(peer) => {
                let event = CustomNotification::new(
                    "notifications/claude/channel",
                    Some(json!({ "content": content(mention), "meta": meta(mention) })),
                );
                peer.send_notification(ServerNotification::CustomNotification(event))
                    .await?;
            }
            Self::Command { argv, .. } => {
                let mut child = tokio::process::Command::new(&argv[0])
                    .args(&argv[1..])
                    .stdin(Stdio::piped())
                    .kill_on_drop(true)
                    .spawn()?;
                let event = json!({"event":"sketch.comment_mention", "mcpUrl":url,
                    "message":"Read the indicated message with get_comment_thread and reply in that Sketch thread.",
                    "mention":mention});
                let mut input = child.stdin.take().expect("piped stdin");
                input.write_all(format!("{event}\n").as_bytes()).await?;
                drop(input);
                if !child.wait().await?.success() {
                    return Err("mention delivery command failed".into());
                }
            }
        }
        Ok(())
    }
}

/// What Claude Code reads: one comment, or the list the person sent together.
fn content(m: &Mention) -> String {
    if m.briefs.is_empty() {
        return ids_only(m);
    }
    let n = m.briefs.len();
    let mut s = if n == 1 {
        "New Sketch comment for you.".to_string()
    } else {
        format!("{n} Sketch comments for you, sent together. Work through each one.")
    };
    s.push_str(
        " What they say and where they are is below, so start on the design; call \
         get_comment_thread only to see attachments or older replies. When a comment is \
         handled, say what you changed with reply_to_comment_thread and resolve: true.",
    );
    if let Some(f) = &m.file_id {
        s.push_str(&format!(" Pass fileId {f} to the tools."));
    }
    for (i, b) in m.briefs.iter().enumerate() {
        let place = match (&b.node_name, &b.node_id) {
            (Some(name), Some(id)) => format!("on \"{name}\" (nodeId {id}) in {}", b.page),
            (None, Some(id)) => format!("on a deleted layer ({id}) in {}", b.page),
            _ => format!("on the canvas in {}", b.page),
        };
        s.push_str(&format!(
            "\n\n{}. {place}; commentThreadId {}",
            i + 1,
            b.thread_id
        ));
        if b.older {
            s.push_str("\n   (older replies not shown)");
        }
        for e in &b.earlier {
            s.push_str(&format!("\n   {e}"));
        }
        s.push_str(&format!("\n   {}: {}", b.author, b.text));
        if !b.attachments.is_empty() {
            s.push_str(&format!(
                "\n   Attached: {} (get_comment_thread shows them)",
                b.attachments.join(", ")
            ));
        }
    }
    s
}

/// For apps that send ids only: point at each thread to read.
fn ids_only(m: &Mention) -> String {
    if m.also.is_empty() {
        return "You were mentioned in a Sketch comment.".into();
    }
    let mut s = format!(
        "The person in Sketch sent you {} comments together. Work through each one: read it \
         with get_comment_thread, make the change, then reply in its thread.",
        m.also.len() + 1
    );
    let all = std::iter::once((&m.thread_id, &m.message_id))
        .chain(m.also.iter().map(|r| (&r.thread_id, &r.message_id)));
    for (i, (t, msg)) in all.enumerate() {
        s.push_str(&format!(
            "\n{}. commentThreadId {t}, messageId {msg}",
            i + 1
        ));
    }
    s
}

/// Channel metadata. Claude Code accepts only string values, so a batch adds a count rather
/// than its list (the list is in the content).
fn meta(m: &Mention) -> serde_json::Value {
    let mut v = json!({ "commentThreadId": m.thread_id, "messageId": m.message_id });
    if let Some(f) = &m.file_id {
        v["fileId"] = json!(f);
    }
    if !m.also.is_empty() {
        v["count"] = json!((m.also.len() + 1).to_string());
    }
    v
}

// Establish the cursor before the Claude stdio handshake completes.
async fn initial_cursor(url: &str) -> Option<String> {
    let connect = async {
        let transport = StreamableHttpClientTransport::from_uri(url.to_owned());
        let client = ClientConfig::default().serve(transport).await.ok()?;
        Some(next_batch(&client, None, "Claude Code").await.ok()?.cursor)
    };
    tokio::time::timeout(Duration::from_secs(2), connect)
        .await
        .ok()
        .flatten()
}

/// An open app's MCP endpoint and the cursor to start listening from.
pub type Start = (String, Option<String>);

/// Listen to every open Sketch app: the ones in `initial` (url and starting cursor) at once,
/// then any app that opens later, checked every few seconds. Runs until the destination
/// closes.
pub async fn watch(sink: std::sync::Arc<Destination>, initial: Vec<Start>) {
    let mut started: Vec<String> = Vec::new();
    for (url, cursor) in initial {
        started.push(url.clone());
        tokio::spawn(listen(url, sink.clone(), cursor));
    }
    while sink.open() {
        let ports = tokio::task::spawn_blocking(sketch_rs::mcp::registry::targets)
            .await
            .unwrap_or_default();
        for port in ports {
            let url = sketch_rs::mcp::url(port);
            if !started.contains(&url) && sketch_rs::mcp::registry::answers(port) {
                started.push(url.clone());
                tokio::spawn(listen(url, sink.clone(), None));
            }
        }
        tokio::time::sleep(Duration::from_secs(3)).await;
    }
}

/// The starting cursor of every open app, so nothing sent while Claude Code connects is
/// missed.
pub async fn initial_cursors() -> Vec<Start> {
    let ports = tokio::task::spawn_blocking(sketch_rs::mcp::registry::targets)
        .await
        .unwrap_or_default();
    let mut v = Vec::new();
    for port in ports {
        if sketch_rs::mcp::registry::answers(port) {
            let url = sketch_rs::mcp::url(port);
            let cursor = initial_cursor(&url).await;
            v.push((url, cursor));
        }
    }
    v
}

pub async fn listen(url: String, sink: std::sync::Arc<Destination>, mut cursor: Option<String>) {
    let mut last_error = String::new();
    while sink.open() {
        let transport = StreamableHttpClientTransport::from_uri(url.clone());
        let result = async {
            let client = ClientConfig::default().serve(transport).await?;
            while sink.open() {
                let batch = next_batch(&client, cursor.as_deref(), sink.name()).await?;
                if cursor.is_none() || !last_error.is_empty() {
                    log::info!("listening for @{} at {url}", sink.name());
                    last_error.clear();
                }
                for mention in batch.mentions {
                    tokio::time::timeout(Duration::from_secs(60), sink.deliver(&mention, &url))
                        .await??;
                    cursor = Some(mention.message_id);
                }
                cursor = Some(batch.cursor);
            }
            Ok::<(), Error>(())
        }
        .await;
        if let Err(e) = result {
            let error = e.to_string();
            if error != last_error {
                log::warn!("{} mention delivery: {error}; retrying", sink.name());
                last_error = error;
            }
            tokio::time::sleep(Duration::from_secs(2)).await;
        }
    }
}

async fn next_batch(
    client: &super::Client,
    cursor: Option<&str>,
    name: &str,
) -> Result<Batch, Error> {
    let request = CustomRequest::new(
        WAIT_METHOD,
        Some(json!({ "cursor": cursor, "agentName": name })),
    );
    let result = tokio::time::timeout(
        Duration::from_secs(30),
        client.send_request(ClientRequest::CustomRequest(request)),
    )
    .await??;
    let ServerResult::CustomResult(result) = result else {
        return Err("unexpected response to mention request".into());
    };
    let batch: Batch = serde_json::from_value(result.0)?;
    let id = Author::agent(Some(name)).id;
    // Older apps only publish mentions for Claude Code.
    if batch.recipient.as_ref() != Some(&id)
        && !(batch.recipient.is_none() && id == sketch_rs::comments::CLAUDE_CODE)
    {
        return Err(
            "Sketch does not support delivery to this agent. Start a matching build.".into(),
        );
    }
    Ok(batch)
}

#[cfg(test)]
mod tests {
    use super::*;
    use sketch_rs::mcp::mentions::{Brief, MentionRef};

    #[test]
    fn a_batch_reads_as_one_message_with_everything_to_start() {
        let brief = |t: &str, text: &str, node: Option<(&str, &str)>| Brief {
            thread_id: t.into(),
            message_id: format!("{t}-m"),
            author: "Ann".into(),
            text: text.into(),
            page: "Page 1".into(),
            node_id: node.map(|(id, _)| id.into()),
            node_name: node.map(|(_, n)| n.into()),
            ..Default::default()
        };
        let mut second = brief("t2", "add a forgot password link", None);
        second.earlier = vec!["Claude Code: Done, 48px now".into()];
        second.older = true;
        second.attachments = vec!["ref.png".into()];
        let m = Mention {
            recipients: vec![],
            file_id: Some("f1".into()),
            thread_id: "t1".into(),
            message_id: "t1-m".into(),
            also: vec![MentionRef {
                thread_id: "t2".into(),
                message_id: "t2-m".into(),
            }],
            briefs: vec![
                brief("t1", "make this field taller", Some(("1:23", "Email"))),
                second,
            ],
        };
        let s = content(&m);
        for line in [
            "2 Sketch comments for you",
            "resolve: true",
            "fileId f1",
            "1. on \"Email\" (nodeId 1:23) in Page 1; commentThreadId t1",
            "   Ann: make this field taller",
            "2. on the canvas in Page 1; commentThreadId t2",
            "   (older replies not shown)\n   Claude Code: Done, 48px now\n   Ann: add a forgot",
            "Attached: ref.png",
        ] {
            assert!(s.contains(line), "{line:?} in:\n{s}");
        }
        // An app that sends ids only still gets a usable message.
        let bare = Mention {
            briefs: vec![],
            ..m
        };
        assert!(content(&bare).contains("1. commentThreadId t1, messageId t1-m"));
    }
}
