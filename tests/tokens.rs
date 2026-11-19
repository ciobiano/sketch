//! Design tokens: the MCP tools, `var()` in styles, layers following token changes, and
//! saving tokens with the project.

use serde_json::{Value, json};
use sketch_rs::mcp::{Out, exec};
use sketch_rs::model::{Color, NodeId, Paint};
use sketch_rs::project::{Manifest, Store};

fn store() -> Store {
    Store::open(std::env::temp_dir().join(format!("sketch-rs-tokens-{:016x}", fastrand::u64(..))))
        .unwrap()
}

fn host() -> exec::SessionHost {
    let mut h = exec::SessionHost::new(store());
    h.session.create(&mut h.ed, "Tokens test").unwrap();
    h
}

fn call(h: &mut exec::SessionHost, name: &str, args: Value) -> Value {
    match exec::call(h, name, &args) {
        Ok(outs) => match outs.into_iter().next() {
            Some(Out::Text(t)) => serde_json::from_str(&t).unwrap_or(Value::String(t)),
            _ => Value::Null,
        },
        Err(e) => panic!("{name} failed: {e}"),
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

fn fill(h: &exec::SessionHost, id: NodeId) -> Color {
    match h.ed.doc.get(id).unwrap().style.fills.last() {
        Some(Paint::Solid { color }) => *color,
        other => panic!("no solid fill: {other:?}"),
    }
}

fn hex(s: &str) -> Color {
    sketch_rs::mcp::css::color(s).unwrap()
}

/// Brand palette used by most tests.
fn palette(h: &mut exec::SessionHost) {
    let r = call(
        h,
        "create_tokens",
        json!({"tokens": [
            {"type": "color", "name": "--blue-500", "value": "#3b82f6"},
            {"type": "color", "name": "--primary", "value": "var(--blue-500)", "description": "Buttons and links"},
            {"type": "spacing", "name": "--space-2", "value": 8},
            {"type": "radius", "name": "--radius-md", "value": "6px"},
        ]}),
    );
    assert!(
        r["results"]
            .as_array()
            .unwrap()
            .iter()
            .all(|e| e["result"] == "created"),
        "{r}"
    );
}

#[test]
fn create_and_list_tokens() {
    let mut h = host();
    palette(&mut h);

    // Bad entries fail on their own; good ones in the same call still land.
    let r = call(
        &mut h,
        "create_tokens",
        json!({"tokens": [
            {"type": "spacing", "name": "--bad", "value": "#fff"},
            {"type": "color", "name": "no-dashes", "value": "#fff"},
            {"type": "color", "name": "--loop", "value": "var(--loop)"},
            {"type": "color", "name": "--gray-100", "value": "#f3f4f6"},
        ]}),
    );
    let results: Vec<&str> = r["results"]
        .as_array()
        .unwrap()
        .iter()
        .map(|e| e["result"].as_str().unwrap())
        .collect();
    assert_eq!(results, ["error", "error", "error", "created"], "{r}");
    assert!(
        r["results"][2]["message"]
            .as_str()
            .unwrap()
            .contains("cycle")
    );

    let all = call(&mut h, "get_tokens", json!({}));
    assert_eq!(all["count"], 5, "{all}");
    let primary = &all["tokens"][1];
    assert_eq!(primary["name"], "--primary");
    assert_eq!(primary["value"], "var(--blue-500)");
    assert_eq!(primary["resolvedValue"], "#3b82f6");
    assert_eq!(primary["description"], "Buttons and links");
    assert_eq!(
        all["tokens"][2]["value"], "8px",
        "numbers become px for sizes"
    );

    let colors = call(&mut h, "get_tokens", json!({"types": ["color"]}));
    assert_eq!(colors["count"], 3);
    let five = call(&mut h, "get_tokens", json!({"namePattern": "*-500"}));
    assert_eq!(five["count"], 1);

    let css = call(&mut h, "get_tokens", json!({"format": "css"}));
    let css = css.as_str().unwrap();
    assert!(css.starts_with(":root {"), "{css}");
    assert!(css.contains("--space-2: 8px;"));
    let tw = call(&mut h, "get_tokens", json!({"format": "tailwind"}));
    let tw = tw.as_str().unwrap();
    assert!(
        tw.contains("--color-primary: var(--color-blue-500);"),
        "{tw}"
    );
    assert!(tw.contains("--spacing-2: 8px;"), "{tw}");

    let info = call(&mut h, "get_basic_info", json!({}));
    assert_eq!(info["tokens"].as_array().unwrap().len(), 5);
}

#[test]
fn layers_follow_token_changes() {
    let mut h = host();
    palette(&mut h);
    let ab = call(
        &mut h,
        "create_artboard",
        json!({"name": "Card", "styles": {"width": "320px", "height": "200px", "padding": "var(--space-2)", "backgroundColor": "var(--primary)"}}),
    );
    let ab_id = num(&ab["id"]);
    let r = call(
        &mut h,
        "write_html",
        json!({
            "targetNodeId": ab["id"], "mode": "insert-children",
            "html": r#"<div layer-name="Button" style="width:120px; height:40px; background: var(--primary); border-radius: var(--radius-md)"></div>
                       <div layer-name="Plain" style="width:120px; height:40px; background: var(--primary)"></div>
                       <div layer-name="Fallback" style="width:40px; height:40px; background: var(--accent, #22c55e)"></div>"#,
        }),
    );
    let ids: Vec<NodeId> = r["createdNodeIds"]
        .as_array()
        .unwrap()
        .iter()
        .map(num)
        .collect();
    let (button, plain, fallback) = (ids[0], ids[1], ids[2]);

    // Resolved on write, aliases followed, fallbacks used for unknown tokens.
    assert_eq!(fill(&h, ab_id), hex("#3b82f6"));
    assert_eq!(fill(&h, button), hex("#3b82f6"));
    assert_eq!(fill(&h, fallback), hex("#22c55e"));
    assert_eq!(
        h.ed.doc.get(ab_id).unwrap().layout.unwrap().padding,
        [8.0; 4]
    );
    assert_eq!(h.ed.doc.get(button).unwrap().style.corner_radius, 6.0);
    assert_eq!(h.ed.doc.get(button).unwrap().rect.x, 8.0);

    // Reads show the token reference.
    let cs = call(
        &mut h,
        "get_computed_styles",
        json!({"nodeIds": [ab["id"]]}),
    );
    let styles = cs.as_object().unwrap().values().next().unwrap();
    assert_eq!(styles["backgroundColor"], "var(--primary)", "{cs}");

    // A layer set by hand stops following the token.
    call(
        &mut h,
        "update_styles",
        json!({"updates": [{"nodeIds": [r["createdNodeIds"][1]], "styles": {"backgroundColor": "#000000"}}]}),
    );
    assert!(h.ed.doc.get(plain).unwrap().tokens.is_empty());

    // Changing a palette color reaches layers through the alias; spacing re-lays out.
    let r = call(
        &mut h,
        "set_tokens",
        json!({"tokens": [
            {"name": "--blue-500", "value": "#ef4444"},
            {"name": "--space-2", "value": "16px"},
        ]}),
    );
    assert_eq!(r["layersUpdated"], 2, "{r}");
    assert_eq!(fill(&h, ab_id), hex("#ef4444"));
    assert_eq!(fill(&h, button), hex("#ef4444"));
    assert_eq!(fill(&h, plain), hex("#000000"));
    assert_eq!(
        h.ed.doc.get(ab_id).unwrap().layout.unwrap().padding,
        [16.0; 4]
    );
    assert_eq!(h.ed.doc.get(button).unwrap().rect.x, 16.0);
    assert_eq!(h.ed.history.undo_labels().last(), Some("Update tokens"));

    // Rename: layers keep the link under the new name.
    call(
        &mut h,
        "set_tokens",
        json!({"tokens": [{"name": "--primary", "newName": "--brand"}]}),
    );
    assert_eq!(
        h.ed.doc.get(button).unwrap().tokens["background"].raw,
        "var(--brand)"
    );
    call(
        &mut h,
        "set_tokens",
        json!({"tokens": [{"name": "--brand", "value": "#111827"}]}),
    );
    assert_eq!(fill(&h, button), hex("#111827"));

    // A fallback is replaced once the token exists.
    call(
        &mut h,
        "create_tokens",
        json!({"tokens": [{"type": "color", "name": "--accent", "value": "#a855f7"}]}),
    );
    assert_eq!(fill(&h, fallback), hex("#a855f7"));

    // Deleting a token keeps the last value and drops the link.
    call(
        &mut h,
        "set_tokens",
        json!({"tokens": [{"name": "--brand", "delete": true}]}),
    );
    assert_eq!(fill(&h, button), hex("#111827"));
    assert!(
        !h.ed
            .doc
            .get(button)
            .unwrap()
            .tokens
            .contains_key("background")
    );
    h.ed.doc.validate().unwrap();
}

#[test]
fn a_property_edited_outside_the_tools_stops_following() {
    let mut h = host();
    palette(&mut h);
    let ab = call(
        &mut h,
        "create_artboard",
        json!({"name": "A", "styles": {"width": "100px", "height": "100px", "backgroundColor": "var(--primary)"}}),
    );
    let id = num(&ab["id"]);
    // As the inspector would: change the fill directly.
    h.ed.edit("Fill", |tx| {
        tx.node_mut(id).unwrap().style.fills = vec![Paint::solid(hex("#facc15"))];
    });
    call(
        &mut h,
        "set_tokens",
        json!({"tokens": [{"name": "--blue-500", "value": "#ef4444"}]}),
    );
    assert_eq!(fill(&h, id), hex("#facc15"));
    assert!(h.ed.doc.get(id).unwrap().tokens.is_empty());
}

#[test]
fn unknown_tokens_are_reported_as_ignored_styles() {
    let mut h = host();
    let ab = call(
        &mut h,
        "create_artboard",
        json!({"name": "A", "styles": {"width": "100px", "height": "100px"}}),
    );
    let r = call(
        &mut h,
        "update_styles",
        json!({"updates": [{"nodeIds": [ab["id"]], "styles": {"backgroundColor": "var(--nope)"}}]}),
    );
    let ignored = r["ignoredStyles"].to_string();
    assert!(ignored.contains("unknown token `--nope`"), "{r}");
    let errs = call(
        &mut h,
        "set_tokens",
        json!({"tokens": [{"name": "--nope", "value": "#fff"}]}),
    );
    assert_eq!(errs["results"][0]["result"], "error");
}

#[test]
fn tokens_and_bindings_are_saved_with_the_project() {
    let mut h = host();
    palette(&mut h);
    let ab = call(
        &mut h,
        "create_artboard",
        json!({"name": "A", "styles": {"width": "100px", "height": "100px", "backgroundColor": "var(--primary)"}}),
    );
    let id = num(&ab["id"]);
    call(&mut h, "finish_working_on_nodes", json!({}));
    let dir = h.session.project.as_ref().unwrap().dir.clone();

    let mut h2 = exec::SessionHost::new(h.session.store.clone());
    h2.session.open(&mut h2.ed, &dir).unwrap();
    assert_eq!(h2.ed.tokens.iter().count(), 4);
    assert_eq!(
        h2.ed.doc.get(id).unwrap().tokens["backgroundColor"].raw,
        "var(--primary)"
    );
    call(
        &mut h2,
        "set_tokens",
        json!({"tokens": [{"name": "--blue-500", "value": "#10b981"}]}),
    );
    assert_eq!(fill(&h2, id), hex("#10b981"));

    // Manifests written before tokens existed still load.
    let old = r#"{"format":"sketch-rs-project","version":1,"id":"x","name":"Old",
        "created":0,"modified":0,"pages":[{"id":"p","name":"Page 1"}]}"#;
    assert!(Manifest::from_json(old).unwrap().tokens.is_empty());
}
