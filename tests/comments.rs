//! Comments: people pin feedback to layers, agents find it over MCP, reply, react and
//! resolve it, and everything is saved with the project.

mod common;

use rmcp::ServiceExt;
use rmcp::model::{CallToolRequestParams, ClientConfig, ContentBlock};
use serde_json::{Value, json};
use sketch_rs::comments::{Author, Draft, Pin, Status};
use sketch_rs::geom::v2;
use sketch_rs::mcp::{self, Out, exec};
use sketch_rs::model::NodeId;
use sketch_rs::project::Store;

fn store() -> Store {
    Store::open(std::env::temp_dir().join(format!("sketch-rs-comments-{:016x}", fastrand::u64(..))))
        .unwrap()
}

fn host() -> exec::SessionHost {
    let mut h = exec::SessionHost::new(store());
    h.session.create(&mut h.ed, "Comments test").unwrap();
    h.ed.me = Author::person("user:ann", "Ann");
    h
}

fn call_as(h: &mut exec::SessionHost, name: &str, args: Value, client: Option<&str>) -> Vec<Out> {
    exec::call_as(h, name, &args, client).unwrap_or_else(|e| panic!("{name} failed: {e}"))
}

fn call(h: &mut exec::SessionHost, name: &str, args: Value) -> Value {
    match call_as(h, name, args, Some("claude-code"))
        .into_iter()
        .next()
    {
        Some(Out::Text(t)) => serde_json::from_str(&t).unwrap(),
        other => panic!("{name}: {other:?}"),
    }
}

fn num(id: &Value) -> NodeId {
    NodeId(
        id.as_str()
            .unwrap()
            .rsplit(':')
            .next()
            .unwrap()
            .parse()
            .unwrap(),
    )
}

/// A button on an artboard, with a comment from Ann pinned to it.
fn board(h: &mut exec::SessionHost) -> (NodeId, String) {
    let ab = call(
        h,
        "create_artboard",
        json!({"name": "Home", "styles": {"width": "400px", "height": "300px"}}),
    );
    let w = call(
        h,
        "write_html",
        json!({"targetNodeId": ab["id"], "mode": "insert-children",
               "html": "<div layer-name=\"Button\" style=\"width:120px;height:40px;background:#888\"></div>"}),
    );
    let button = num(&w["createdNodeIds"][0]);
    let r = h.ed.doc.world_rect(button).unwrap();
    let page = h.ed.active_page_id().to_string();
    let pin = Pin::new(&h.ed.doc, &page, Some(button), v2(r.x + 10.0, r.y + 10.0));
    let me = h.ed.me.clone();
    let t = h.ed.comments.add_thread(
        &me,
        &pin,
        Draft::text("@agent make this button blue"),
        &h.ed.doc,
        1_000,
    );
    (button, t)
}

#[test]
fn an_agent_finds_feedback_fixes_it_and_resolves_it() {
    let mut h = host();
    let (button, t) = board(&mut h);
    let me = h.ed.me.clone();
    let other = h.ed.comments.add_thread(
        &me,
        &Pin::new(&h.ed.doc, h.ed.active_page_id(), None, v2(5.0, 5.0)),
        Draft::text("Ship it?"),
        &h.ed.doc,
        2_000,
    );

    let list = call(&mut h, "list_comment_threads", json!({}));
    assert_eq!(list["total"], 2);
    let mine = call(
        &mut h,
        "list_comment_threads",
        json!({"mentionedUserId": "agent"}),
    );
    assert_eq!(mine["total"], 1);
    let s = &mine["threads"][0];
    assert_eq!(s["commentThreadId"], t.as_str());
    assert_eq!(s["authorName"], "Ann");
    assert_eq!(s["node"]["name"], "Button");
    // An agent's own id also finds threads addressed to @agent.
    let as_me = call(
        &mut h,
        "list_comment_threads",
        json!({"mentionedUserId": "agent:claude-code"}),
    );
    assert_eq!(as_me["total"], 1);
    let by_user = call(
        &mut h,
        "list_comment_threads",
        json!({"threadAuthorUserId": "current-user", "search": "ship"}),
    );
    assert_eq!(by_user["threads"][0]["commentThreadId"], other.as_str());

    let full = call(&mut h, "get_comment_thread", json!({"commentThreadId": t}));
    assert_eq!(full["messages"][0]["mentions"][0]["userId"], "agent");
    assert_eq!(
        full["node"]["nodeId"]
            .as_str()
            .unwrap()
            .rsplit(':')
            .next()
            .unwrap(),
        button.0.to_string()
    );

    // The agent does the work, says so and closes the thread.
    call(
        &mut h,
        "set_comment_reaction",
        json!({"commentThreadId": t, "emoji": "👀"}),
    );
    let key = full["node"]["nodeId"].clone();
    call(
        &mut h,
        "update_styles",
        json!({"updates": [{"nodeIds": [key], "styles": {"background": "#2563eb"}}]}),
    );
    let r = call(
        &mut h,
        "reply_to_comment_thread",
        json!({"commentThreadId": t, "text": "Done, it is blue now @Ann"}),
    );
    assert_eq!(r["authorName"], "Claude Code");
    assert_eq!(r["mentions"][0]["userId"], "user:ann");
    call(
        &mut h,
        "set_comment_thread_status",
        json!({"commentThreadId": t, "status": "resolved"}),
    );

    let th = h.ed.comments.thread(&t).unwrap();
    assert_eq!(th.status, Status::Resolved);
    assert_eq!(th.resolved_by.as_deref(), Some("agent:claude-code"));
    assert_eq!(th.messages[0].reactions["👀"], vec!["agent:claude-code"]);
    assert_eq!(th.messages[1].author, "agent:claude-code");
    // Resolved threads drop out of the default listing.
    assert_eq!(call(&mut h, "list_comment_threads", json!({}))["total"], 1);
    assert_eq!(
        call(
            &mut h,
            "list_comment_threads",
            json!({"status": "resolved"})
        )["total"],
        1
    );

    let authors = call(&mut h, "list_comment_thread_authors", json!({}));
    let names: Vec<&str> = authors["authors"]
        .as_array()
        .unwrap()
        .iter()
        .map(|a| a["displayName"].as_str().unwrap())
        .collect();
    assert_eq!(names.len(), 2, "{authors}");
    assert!(names.contains(&"Ann") && names.contains(&"Claude Code"));
    let basic = call(&mut h, "get_basic_info", json!({}));
    assert_eq!(basic["openCommentThreads"], 1);
}

#[test]
fn attachments_reach_the_agent_and_comments_are_saved() {
    let mut h = host();
    let (_, t) = board(&mut h);
    let dir = h.session.project.as_ref().unwrap().dir.clone();
    // A screenshot attached to a reply: the file is copied into the project.
    let src = std::env::temp_dir().join(format!("shot-{:08x}.png", fastrand::u32(..)));
    let mut png = Vec::new();
    image::DynamicImage::new_rgba8(4, 4)
        .write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
        .unwrap();
    std::fs::write(&src, &png).unwrap();
    let file = sketch_rs::comments::attach(&dir, &src).unwrap();
    assert!(dir.join(&file.file).is_file());
    let me = h.ed.me.clone();
    h.ed.comments
        .reply(
            &t,
            &me,
            Draft {
                text: "like this".into(),
                attachments: vec![file],
            },
            3_000,
        )
        .unwrap();

    let out = call_as(
        &mut h,
        "get_comment_thread",
        json!({"commentThreadId": t}),
        None,
    );
    let Out::Text(text) = &out[0] else { panic!() };
    let v: Value = serde_json::from_str(text).unwrap();
    let a = &v["messages"][1]["attachments"][0];
    assert_eq!(a["mimeType"], "image/png");
    assert!(std::path::Path::new(a["path"].as_str().unwrap()).is_file());
    // The project folder name has a space; the URL encodes it.
    let url = a["url"].as_str().unwrap();
    assert!(
        url.starts_with("file://") && url.contains("%20") && !url.contains(' '),
        "{url}"
    );
    assert!(
        matches!(&out[1], Out::Image { mime, .. } if mime == "image/png"),
        "the image comes back to look at"
    );

    // Saved with the project, and back after reopening it.
    h.ed.meta_dirty = true;
    h.session.save_all(&mut h.ed).unwrap();
    let before = h.ed.comments.clone();
    let mut again = exec::SessionHost::new(h.session.store.clone());
    again.session.open(&mut again.ed, &dir).unwrap();
    assert_eq!(again.ed.comments, before);
    assert_eq!(again.ed.comments.thread(&t).unwrap().messages.len(), 2);

    // Deleting the reply removes its file.
    let reply = again.ed.comments.thread(&t).unwrap().messages[1].id.clone();
    let gone = again.ed.comments.delete_message(&t, &reply).unwrap();
    sketch_rs::comments::remove_files(&dir, &gone);
    assert!(!dir.join(&gone[0].file).exists());
}

#[test]
fn errors_name_the_problem() {
    let mut h = host();
    let e = exec::call(
        &mut h,
        "get_comment_thread",
        &json!({"commentThreadId": "nope"}),
    )
    .unwrap_err();
    assert!(e.contains("not found"), "{e}");
    let (_, t) = board(&mut h);
    let e = exec::call(
        &mut h,
        "set_comment_thread_status",
        &json!({"commentThreadId": t, "status": "done"}),
    )
    .unwrap_err();
    assert!(e.contains("open or resolved"), "{e}");
    let e = exec::call(
        &mut h,
        "set_comment_reaction",
        &json!({"commentThreadId": t, "emoji": "ok"}),
    )
    .unwrap_err();
    assert!(e.contains("not an emoji"), "{e}");
}

/// Over HTTP the reply is signed with the name the client gave in its handshake.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn replies_over_http_carry_the_clients_name() {
    let (tx, rx) = std::sync::mpsc::channel::<mcp::Call>();
    let (ready_tx, ready_rx) = std::sync::mpsc::channel();
    let worker = std::thread::spawn(move || {
        let mut h = host();
        let (_, t) = board(&mut h);
        ready_tx.send(t).unwrap();
        mcp::serve_host(&mut h, rx);
        h.ed.comments
    });
    let t = ready_rx.recv().unwrap();
    let tx = std::sync::Mutex::new(tx);
    let running = mcp::server::spawn(
        0,
        std::sync::Arc::new(move |c| tx.lock().unwrap().send(c).is_ok()),
    )
    .unwrap();
    let mut config = ClientConfig::default();
    config.client_info.name = "test-agent".into();
    let client = config
        .serve(rmcp::transport::StreamableHttpClientTransport::from_uri(
            running.url(),
        ))
        .await
        .unwrap();
    let mut p = CallToolRequestParams::new("reply_to_comment_thread");
    p.arguments = json!({"commentThreadId": t, "text": "On it"})
        .as_object()
        .cloned();
    let r = client.call_tool(p).await.unwrap();
    assert_ne!(r.is_error, Some(true), "{r:?}");
    let ContentBlock::Text(text) = &r.content[0] else {
        panic!()
    };
    let v: Value = serde_json::from_str(&text.text).unwrap();
    assert_eq!(v["authorName"], "Test Agent");
    // Sessionless clients send their identity on each request.
    let body = json!({
        "jsonrpc": "2.0", "id": 1, "method": "tools/call", "params": {
            "name": "reply_to_comment_thread",
            "arguments": {"commentThreadId": t, "text": "Checked"},
            "_meta": {
                "io.modelcontextprotocol/protocolVersion": "2026-07-28",
                "io.modelcontextprotocol/clientInfo": {"name": "claude-code", "version": "1"},
                "io.modelcontextprotocol/clientCapabilities": {}
            }
        }
    });
    let response = reqwest::Client::new()
        .post(running.url())
        .header("content-type", "application/json")
        .header("mcp-protocol-version", "2026-07-28")
        .header("mcp-method", "tools/call")
        .header("mcp-name", "reply_to_comment_thread")
        .body(body.to_string())
        .send()
        .await
        .unwrap();
    let status = response.status();
    let response_text = response.text().await.unwrap();
    assert!(status.is_success(), "{status}: {response_text}");
    client.cancel().await.unwrap();
    running.stop();
    let comments = worker.join().unwrap();
    assert_eq!(
        comments.thread(&t).unwrap().messages[2].author,
        "agent:claude-code"
    );
    assert_eq!(
        comments.thread(&t).unwrap().messages[1].author,
        "agent:test-agent"
    );
}

/// Renders the comment states offscreen to `.artifacts/comments-ui/` for design review.
#[test]
fn comments_ui_states() {
    use sketch_rs::render::frame;
    use sketch_rs::ui::UiState;
    let Some(g) = common::gpu() else { return };
    let dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(".artifacts/comments-ui");
    std::fs::create_dir_all(&dir).unwrap();
    let mut e = sketch_rs::demo::showcase();
    e.me = Author::person("user:raph", "Raph");
    let ann = Author::person("user:ann", "Ann");
    let agent = Author::agent(Some("claude-code"));
    let page = e.active_page_id().to_string();
    let nodes: Vec<NodeId> = e.doc.root.clone();
    let at = |e: &sketch_rs::editor::Editor, i: usize, dx: f64, dy: f64| {
        let r = e.doc.world_rect(nodes[i % nodes.len()]).unwrap();
        v2(r.x + dx, r.y + dy)
    };
    let me = e.me.clone();
    let p1 = Pin::new(&e.doc, &page, Some(nodes[0]), at(&e, 0, 60.0, 40.0));
    let t1 = e.comments.add_thread(
        &ann,
        &p1,
        Draft::text("The hero headline feels cramped. @agent can you loosen the line height?"),
        &e.doc,
        1_000,
    );
    e.comments
        .reply(
            &t1,
            &agent,
            Draft::text("On it. Line height is now 1.3 @Ann"),
            2_000,
        )
        .unwrap();
    e.comments.react(&t1, None, "👍", &me, None).unwrap();
    let p2 = Pin::new(&e.doc, &page, nodes.get(1).copied(), at(&e, 1, 30.0, 30.0));
    e.comments.add_thread(
        &me,
        &p2,
        Draft::text("Use the brand blue here"),
        &e.doc,
        3_000,
    );
    let p3 = Pin::new(&e.doc, &page, nodes.get(2).copied(), at(&e, 2, 20.0, 80.0));
    e.comments.add_thread(
        &agent,
        &p3,
        Draft::text("I left the old copy below in case you want it back."),
        &e.doc,
        4_000,
    );
    let mut st = UiState {
        project_name: "Launch plan".into(),
        save_state: "Saved".into(),
        ..Default::default()
    };
    let size = [1440.0, 900.0];
    let shot = |name: &str,
                e: &mut sketch_rs::editor::Editor,
                st: &mut UiState,
                ptr: Option<egui::Pos2>| {
        let img = frame::capture_hovering(g, e, st, size, 2.0, ptr, |ed| ed.zoom_to_fit());
        img.save(dir.join(format!("{name}.png"))).unwrap();
    };
    shot("1-pins", &mut e, &mut st, None);
    assert_eq!(st.comments.drawn.len(), 3, "every open thread has a pin");
    let pin1 = st
        .comments
        .drawn
        .iter()
        .find(|(id, _)| *id == t1)
        .unwrap()
        .1;
    shot(
        "2-hover",
        &mut e,
        &mut st,
        Some(pin1 + egui::vec2(10.0, -12.0)),
    );
    st.comments.open = Some(t1.clone());
    shot("3-thread", &mut e, &mut st, None);
    st.comments.open = None;
    shot("4-closed", &mut e, &mut st, None);
    assert_eq!(
        st.comments.drawn.len(),
        3,
        "pins stay after the thread closes"
    );
    // Resolving keeps the pin too, so the discussion can be found again.
    let me = e.me.clone();
    e.comments
        .set_status(&t1, Status::Resolved, &me, 5_000)
        .unwrap();
    shot("4b-resolved", &mut e, &mut st, None);
    assert_eq!(
        st.comments.drawn.len(),
        3,
        "resolved pins stay on the canvas"
    );
    st.comments.hidden = true;
    shot("4c-hidden", &mut e, &mut st, None);
    assert!(st.comments.drawn.is_empty(), "Shift+C hides them");
    st.comments.hidden = false;
    e.comments
        .set_status(&t1, Status::Open, &me, 6_000)
        .unwrap();
    let spot = at(&e, 0, 300.0, 120.0);
    st.comments.start(&mut e, None, spot);
    st.comments.new.as_mut().unwrap().composer.text = "Tighten this up @".into();
    shot("5-new", &mut e, &mut st, None);
    st.comments.close(&e);
    st.show_comments = true;
    shot("6-panel", &mut e, &mut st, None);
    // Held back for the agent: the Send bar and the "Not sent" card.
    let m = e.comments.thread(&t1).unwrap().messages[0].id.clone();
    e.comments.set_pending(&t1, &m, true);
    shot("7-held", &mut e, &mut st, None);
    e.comments.set_pending(&t1, &m, false);
}

/// With the Comment tool a click pins a comment and a drag moves the canvas.
#[test]
fn comment_tool_clicks_to_comment_and_drags_to_pan() {
    use sketch_rs::editor::Tool;
    use sketch_rs::editor::commands::UiEvent;
    let mut e = common::editor(800.0, 600.0);
    e.tool = Tool::Comment;
    common::ev_drag(&mut e, v2(200.0, 200.0), v2(200.0, 200.0), 0.0, true);
    assert!(
        matches!(e.ui_events.as_slice(), [UiEvent::Comment { .. }]),
        "{:?}",
        e.ui_events
    );
    e.ui_events.clear();
    let pan = e.camera.pan;
    common::ev_drag(&mut e, v2(200.0, 200.0), v2(320.0, 260.0), 1_000.0, true);
    assert!(e.ui_events.is_empty(), "a drag starts no comment");
    let moved = e.camera.pan - pan;
    assert!(
        (moved.x + 120.0).abs() < 1e-6 && (moved.y + 60.0).abs() < 1e-6,
        "the canvas follows the pointer: {moved:?}"
    );
    assert_eq!(e.tool, Tool::Comment, "still commenting");
}
