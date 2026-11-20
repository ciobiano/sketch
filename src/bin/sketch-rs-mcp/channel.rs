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

pub const INSTRUCTIONS: &str = "A channel event means the person in Sketch just mentioned you. \
Read the indicated message with get_comment_thread using commentThreadId and fileId when present. \
Use that fileId for follow-up calls and reply on the same thread.";

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
                    Some(json!({
                        "content": "You were mentioned in a Sketch comment.", "meta": mention
                    })),
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
