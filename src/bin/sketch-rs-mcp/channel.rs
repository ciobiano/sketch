//! Relay live comment mentions over the sessionful protocol Claude Code uses for channels.
use rmcp::model::{
    ClientConfig, ClientRequest, CustomNotification, CustomRequest, ServerNotification,
    ServerResult,
};
use rmcp::transport::StreamableHttpClientTransport;
use rmcp::{Peer, RoleServer, ServiceExt};
use serde_json::json;
use sketch_rs::mcp::mentions::{Batch, WAIT_METHOD};
use std::time::Duration;

pub const INSTRUCTIONS: &str = "A channel event means the person in Sketch just mentioned you. \
Read the indicated message with get_comment_thread using commentThreadId and fileId when present. \
Use that fileId for follow-up calls and reply on the same thread.";

type Error = Box<dyn std::error::Error + Send + Sync>;

// Establish the cursor before the stdio handshake completes. A mention posted as soon as
// Claude shows the channel as connected must survive the first long poll.
pub async fn initial_cursor(url: &str) -> Option<String> {
    let connect = async {
        let transport = StreamableHttpClientTransport::from_uri(url.to_owned());
        let client = ClientConfig::default().serve(transport).await.ok()?;
        Some(next_batch(&client, None).await.ok()?.cursor)
    };
    tokio::time::timeout(Duration::from_secs(2), connect)
        .await
        .ok()
        .flatten()
}

pub async fn listen(url: String, peer: Peer<RoleServer>, mut cursor: Option<String>) {
    while !peer.is_transport_closed() {
        let transport = StreamableHttpClientTransport::from_uri(url.clone());
        match ClientConfig::default().serve(transport).await {
            Ok(client) => {
                if let Err(e) = relay(&client, &peer, &mut cursor).await {
                    log::debug!("Sketch mention channel disconnected: {e}");
                }
            }
            Err(e) => log::debug!("Sketch mention channel waiting for the app: {e}"),
        }
        tokio::time::sleep(Duration::from_secs(2)).await;
    }
}

async fn relay(
    client: &super::Client,
    peer: &Peer<RoleServer>,
    cursor: &mut Option<String>,
) -> Result<(), Error> {
    while !peer.is_transport_closed() {
        let batch = next_batch(client, cursor.as_deref()).await?;
        for mention in batch.mentions {
            let event = CustomNotification::new(
                "notifications/claude/channel",
                Some(json!({
                    "content": "You were mentioned in a Sketch comment.",
                    "meta": mention,
                })),
            );
            peer.send_notification(ServerNotification::CustomNotification(event))
                .await?;
            *cursor = Some(mention.message_id);
        }
        *cursor = Some(batch.cursor);
    }
    Ok(())
}

async fn next_batch(client: &super::Client, cursor: Option<&str>) -> Result<Batch, Error> {
    let request = CustomRequest::new(WAIT_METHOD, Some(json!({ "cursor": cursor })));
    let result = tokio::time::timeout(
        Duration::from_secs(30),
        client.send_request(ClientRequest::CustomRequest(request)),
    )
    .await??;
    let ServerResult::CustomResult(result) = result else {
        return Err("unexpected response to mention request".into());
    };
    Ok(serde_json::from_value(result.0)?)
}
