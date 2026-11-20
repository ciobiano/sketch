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
or several at once (the event then lists each commentThreadId). Read each with \
get_comment_thread using its commentThreadId and the fileId when present, do the work, and \
reply on the same thread. Use that fileId for follow-up calls.";

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
pub async fn initial_cursor(url: &str) -> Option<String> {
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

pub async fn listen(url: String, sink: Destination, mut cursor: Option<String>) {
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
