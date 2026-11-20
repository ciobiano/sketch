//! `sketch-rs-mcp`: MCP over stdio for clients that can't speak HTTP, relayed to the running
//! Sketch app (`http://127.0.0.1:29980/mcp`, or `$SKETCH_RS_MCP_PORT`).
//!
//! The tool list is answered locally, so the agent sees every tool even before the app is
//! started; calls are forwarded to the app and report a clear error while it isn't running.
//! The connection is opened lazily and re-opened after the app restarts.

#[path = "sketch-rs-mcp/channel.rs"]
mod channel;

use rmcp::model::{
    CallToolRequestParams, CallToolResponse, CallToolResult, ClientConfig, ContentBlock,
    Implementation, ListToolsResult, PaginatedRequestParams, ProtocolVersion, ResultType,
    ServerConfig,
};
use rmcp::service::{NotificationContext, RequestContext, RunningService};
use rmcp::transport::StreamableHttpClientTransport;
use rmcp::{ErrorData, RoleClient, RoleServer, ServerHandler, ServiceExt};
use sketch_rs::mcp::{self, server};
use tokio::sync::Mutex;

const USAGE: &str = "\
sketch-rs-mcp: stdio MCP bridge to the running Sketch app

USAGE:
  sketch-rs-mcp           serve MCP on stdin/stdout (what agent harnesses run)
  sketch-rs-mcp --claude-channel   also deliver new comment mentions to Claude Code
  sketch-rs-mcp --deliver-mentions NAME -- PROGRAM [ARGS...]
                         send each new mention as JSON to the program on stdin
  sketch-rs-mcp --url     print the HTTP endpoints it would use, in order
  sketch-rs-mcp --help

Each open Sketch app serves MCP on its own port (29980 unless SKETCH_RS_MCP_PORT says
otherwise) and lists itself for this bridge, so it finds the installed app and dev builds on
other ports without setup. Tool calls go to the newest open app; comments are delivered from
all of them. Set SKETCH_RS_MCP_PORT in the bridge's environment to use one port only.
";

type Client = RunningService<RoleClient, ClientConfig>;

struct Bridge {
    /// Starting cursors per open app, when this bridge also delivers comments to Claude Code.
    channel_cursor: Option<tokio::sync::OnceCell<Vec<channel::Start>>>,
    listener_started: std::sync::Once,
    client: Mutex<Option<Client>>,
}

impl Bridge {
    /// Connect as `client`, the stdio client's identity, so the app knows which agent is
    /// calling (it signs that agent's comments with its name).
    ///
    /// Tools go to the newest open Sketch (a dev build you just started wins over the
    /// installed app), or to `$SKETCH_RS_MCP_PORT` when it is set.
    async fn connect(&self, client: Option<Implementation>) -> Result<(), String> {
        let ports = tokio::task::spawn_blocking(mcp::registry::targets)
            .await
            .unwrap_or_else(|_| vec![mcp::port()]);
        let mut errors = Vec::new();
        for port in ports {
            let url = mcp::url(port);
            let transport = StreamableHttpClientTransport::from_uri(url.clone());
            let mut config = ClientConfig::default();
            if let Some(c) = &client {
                config.client_info = c.clone();
            }
            match config.serve(transport).await {
                Ok(c) => {
                    *self.client.lock().await = Some(c);
                    return Ok(());
                }
                Err(e) => errors.push(format!("{url}: {e}")),
            }
        }
        Err(format!("cannot reach Sketch ({})", errors.join("; ")))
    }

    async fn forward(
        &self,
        params: CallToolRequestParams,
        client: Option<Implementation>,
    ) -> Result<CallToolResult, String> {
        // One retry with a fresh connection covers an app restart between calls.
        for attempt in 0..2 {
            if self.client.lock().await.is_none() {
                self.connect(client.clone()).await?;
            }
            let res = {
                let guard = self.client.lock().await;
                guard.as_ref().unwrap().call_tool(params.clone()).await
            };
            match res {
                Ok(r) => return Ok(r),
                Err(e) if attempt == 0 => {
                    log::info!("sketch-rs-mcp: reconnecting after {e}");
                    *self.client.lock().await = None;
                }
                Err(e) => return Err(e.to_string()),
            }
        }
        unreachable!()
    }
}

impl ServerHandler for Bridge {
    fn get_info(&self) -> ServerConfig {
        let mut info = server::info();
        if self.channel_cursor.is_some() {
            info.capabilities
                .experimental
                .get_or_insert_with(Default::default)
                .insert("claude/channel".into(), Default::default());
            info = info.with_instructions(format!(
                "{}\n{}",
                mcp::tools::INSTRUCTIONS,
                channel::INSTRUCTIONS
            ));
        }
        info
    }

    fn supported_protocol_versions(&self) -> std::borrow::Cow<'static, [ProtocolVersion]> {
        std::borrow::Cow::Borrowed(if self.channel_cursor.is_some() {
            ProtocolVersion::known_up_to(&ProtocolVersion::V_2025_11_25)
        } else {
            ProtocolVersion::KNOWN_VERSIONS
        })
    }

    async fn initialize(
        &self,
        request: rmcp::model::InitializeRequestParams,
        context: RequestContext<RoleServer>,
    ) -> Result<rmcp::model::InitializeResult, ErrorData> {
        context.peer.set_peer_info(request.clone());
        let info = self.negotiate_initialize(&request)?;
        if let Some(cursor) = &self.channel_cursor {
            cursor.get_or_init(channel::initial_cursors).await;
        }
        Ok(info)
    }

    async fn on_initialized(&self, context: NotificationContext<RoleServer>) {
        if let Some(cursor) = &self.channel_cursor {
            self.listener_started.call_once(|| {
                tokio::spawn(channel::watch(
                    std::sync::Arc::new(channel::Destination::Claude(context.peer)),
                    cursor.get().cloned().unwrap_or_default(),
                ));
            });
        }
    }

    async fn list_tools(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> Result<ListToolsResult, ErrorData> {
        Ok(ListToolsResult::with_all_items(server::tool_list()))
    }

    async fn call_tool(
        &self,
        request: CallToolRequestParams,
        context: RequestContext<RoleServer>,
    ) -> Result<CallToolResponse, ErrorData> {
        let client = context.client_info();
        let r = match self.forward(request, client).await {
            Ok(mut r) => {
                // The app may have answered over an older protocol revision (no resultType);
                // 2026-07-28 clients require it. rmcp drops it again for older-revision peers.
                if r.result_type.is_none() {
                    r.result_type = Some(ResultType::COMPLETE);
                }
                r
            }
            Err(e) => CallToolResult::error(vec![ContentBlock::text(format!(
                "{e}\nThe Sketch app is not running or its MCP server is off. Ask the user to \
                 start Sketch (it serves MCP while open), then try again."
            ))]),
        };
        Ok(r.into())
    }
}

fn main() -> std::process::ExitCode {
    // Logs go to stderr: stdout carries the protocol.
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("warn"))
        .target(env_logger::Target::Stderr)
        .init();
    let args: Vec<String> = std::env::args().skip(1).collect();
    if matches!(args.first().map(String::as_str), Some("--help" | "-h")) {
        print!("{USAGE}");
        return std::process::ExitCode::SUCCESS;
    }
    if args.first().is_some_and(|a| a == "--url") {
        for port in mcp::registry::targets() {
            println!("{}", mcp::url(port));
        }
        return std::process::ExitCode::SUCCESS;
    }
    let rt = match tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
    {
        Ok(rt) => rt,
        Err(e) => {
            eprintln!("sketch-rs-mcp: {e}");
            return std::process::ExitCode::FAILURE;
        }
    };
    let r = rt.block_on(async move {
        if args.first().is_some_and(|a| a == "--deliver-mentions") {
            let [_, name, separator, program, rest @ ..] = args.as_slice() else {
                return Err(USAGE.to_owned());
            };
            if separator != "--" || program.is_empty() || !mcp::mentions::valid_listener_name(name)
            {
                return Err(USAGE.to_owned());
            }
            let mut argv = vec![program.clone()];
            argv.extend_from_slice(rest);
            channel::watch(
                std::sync::Arc::new(channel::Destination::Command {
                    agent: name.clone(),
                    argv,
                }),
                Vec::new(),
            )
            .await;
            return Ok(());
        }
        let bridge = Bridge {
            channel_cursor: args
                .iter()
                .any(|a| a == "--claude-channel")
                .then(Default::default),
            listener_started: Default::default(),
            client: Mutex::new(None),
        };
        let service = bridge
            .serve(rmcp::transport::stdio())
            .await
            .map_err(|e| e.to_string())?;
        service.waiting().await.map_err(|e| e.to_string())?;
        Ok::<(), String>(())
    });
    match r {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("sketch-rs-mcp: {e}");
            std::process::ExitCode::FAILURE
        }
    }
}
