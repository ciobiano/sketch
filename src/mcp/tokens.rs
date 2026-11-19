//! The design token tools (`get_tokens`, `create_tokens`, `set_tokens`) and the pass that
//! keeps token-bound layers in step with the tokens.
//!
//! Argument shapes follow Paper's token tools (`analysis/paper-mcp.md` §2): tokens are CSS
//! custom properties with a type, a value and an optional description, and a value can alias
//! another token with `var(--other)`.

use super::css;
use super::exec::{Parent, fit_text, relayout};
use super::{Out, ToolResult};
use crate::editor::Editor;
use crate::history::Tx;
use crate::model::NodeId;
use crate::tokens::{self, TokenRef, TokenType, Tokens};
use serde_json::{Map, Value, json};

fn text(v: Value) -> ToolResult {
    Ok(vec![Out::Text(
        serde_json::to_string_pretty(&v).unwrap_or_default(),
    )])
}

fn token_json(all: &Tokens, t: &tokens::Token) -> Value {
    let mut v = json!({"name": t.name, "type": t.kind.as_str(), "value": t.value});
    if tokens::has_var(&t.value)
        && let Ok(r) = all.resolve(&t.name)
    {
        v["resolvedValue"] = json!(r);
    }
    if !t.description.is_empty() {
        v["description"] = json!(t.description);
    }
    v
}

/// A compact token list for `get_basic_info`.
pub fn summary(ed: &Editor) -> Value {
    json!(
        ed.tokens
            .iter()
            .map(|t| json!({"name": t.name, "type": t.kind.as_str(), "value": t.value}))
            .collect::<Vec<_>>()
    )
}

pub fn get_tokens(ed: &Editor, args: &Map<String, Value>) -> ToolResult {
    let mut types = Vec::new();
    if let Some(list) = args.get("types").filter(|v| !v.is_null()) {
        for t in list.as_array().ok_or("`types` must be an array")? {
            let s = t.as_str().unwrap_or_default();
            types.push(TokenType::parse(s).ok_or_else(|| {
                format!(
                    "unknown token type `{s}`; types are {}",
                    TokenType::ALL.map(|t| t.as_str()).join(", ")
                )
            })?);
        }
    }
    let pattern = args
        .get("namePattern")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|p| !p.is_empty());
    let list: Vec<_> = ed.tokens.filter(&types, pattern).collect();
    match args.get("format").and_then(Value::as_str).unwrap_or("json") {
        "json" => text(json!({
            "tokens": list.iter().map(|t| token_json(&ed.tokens, t)).collect::<Vec<_>>(),
            "count": list.len(),
        })),
        "css" => Ok(vec![Out::Text(tokens::to_css(list))]),
        "tailwind" => Ok(vec![Out::Text(tokens::to_tailwind(&ed.tokens, list))]),
        other => Err(format!(
            "unknown format `{other}`; use json, css or tailwind"
        )),
    }
}

fn entries(args: &Map<String, Value>) -> Result<&Vec<Value>, String> {
    let list = args
        .get("tokens")
        .and_then(Value::as_array)
        .ok_or("missing required array argument `tokens`")?;
    if list.is_empty() {
        return Err("`tokens` must have at least one entry".into());
    }
    Ok(list)
}

fn str_field<'a>(e: &'a Map<String, Value>, k: &str) -> Option<&'a str> {
    e.get(k).and_then(Value::as_str).map(str::trim)
}

pub fn create_tokens(ed: &mut Editor, args: &Map<String, Value>) -> ToolResult {
    let mut results = Vec::new();
    let mut created = 0;
    for e in entries(args)? {
        let empty = Map::new();
        let e = e.as_object().unwrap_or(&empty);
        let name = str_field(e, "name").unwrap_or_default().to_string();
        let r = (|| {
            let kind = str_field(e, "type")
                .and_then(TokenType::parse)
                .ok_or("`type` must be one of the token types")?;
            let value = tokens::value_from_json(kind, e.get("value").unwrap_or(&Value::Null))?;
            let desc = str_field(e, "description").unwrap_or_default();
            ed.tokens.create(kind, &name, &value, desc)
        })();
        results.push(match r {
            Ok(()) => {
                created += 1;
                json!({"name": name, "result": "created"})
            }
            Err(m) => json!({"name": name, "result": "error", "message": m}),
        });
    }
    let mut v = json!({"results": results});
    if created > 0 {
        ed.meta_dirty = true;
        // A repeated name overrides the earlier definition, which can change bound layers.
        v["layersUpdated"] = json!(refresh(ed, &[], None));
    }
    text(v)
}

pub fn set_tokens(ed: &mut Editor, args: &Map<String, Value>) -> ToolResult {
    let mut results = Vec::new();
    let mut renames: Vec<(String, String)> = Vec::new();
    let mut changed = false;
    for e in entries(args)? {
        let empty = Map::new();
        let e = e.as_object().unwrap_or(&empty);
        let name = str_field(e, "name").unwrap_or_default().to_string();
        // Apply one entry to a copy, so a failing entry changes nothing.
        let mut next = ed.tokens.clone();
        let r = (|| -> Result<&'static str, String> {
            let kind = next
                .get(&name)
                .ok_or_else(|| format!("unknown token `{name}`"))?
                .kind;
            if e.get("delete").and_then(Value::as_bool) == Some(true) {
                next.remove(&name);
                return Ok("deleted");
            }
            let mut target = name.clone();
            if let Some(new) = str_field(e, "newName").filter(|n| *n != name) {
                next.rename(&name, new)?;
                target = new.to_string();
            }
            if let Some(v) = e.get("value").filter(|v| !v.is_null()) {
                next.set_value(&target, &tokens::value_from_json(kind, v)?)?;
            }
            if let Some(d) = str_field(e, "description") {
                next.set_description(&target, d)?;
            }
            Ok("updated")
        })();
        results.push(match r {
            Ok(result) => {
                if let Some(new) = str_field(e, "newName").filter(|n| *n != name)
                    && result == "updated"
                {
                    renames.push((name.clone(), new.to_string()));
                }
                ed.tokens = next;
                changed = true;
                json!({"name": name, "result": result})
            }
            Err(m) => json!({"name": name, "result": "error", "message": m}),
        });
    }
    let mut v = json!({"results": results});
    if changed {
        ed.meta_dirty = true;
        v["layersUpdated"] = json!(refresh(ed, &renames, None));
    }
    text(v)
}

/// Bring every token-bound layer on every page up to date with `ed.tokens`, after applying
/// `renames` (old, new) to the layers' references. Each page that changes gets one undo step.
/// With `coalesce`, repeated refreshes under the same key (a color picker being dragged) merge
/// into one undo step. Returns the number of layers changed.
pub fn refresh(ed: &mut Editor, renames: &[(String, String)], coalesce: Option<&str>) -> usize {
    let tokens = ed.tokens.clone();
    let mut changed = 0;
    for page in 0..ed.pages.len() {
        let ids: Vec<NodeId> = ed
            .page_doc(page)
            .nodes
            .values()
            .filter(|n| !n.tokens.is_empty())
            .map(|n| n.id)
            .collect();
        if ids.is_empty() {
            continue;
        }
        let page_id = ed.pages[page].id.clone();
        changed += ed.with_page(page, |ed| {
            let work = |tx: &mut Tx| {
                ids.iter()
                    .filter(|id| refresh_node(tx, &page_id, **id, &tokens, renames))
                    .count()
            };
            match coalesce {
                Some(key) => ed.edit_coalescing("Update tokens", key, work),
                None => ed.edit("Update tokens", work),
            }
        });
    }
    changed
}

/// Re-resolve one layer's token references. A property that no longer shows the value its
/// tokens last gave it was changed by hand since; it keeps that value and loses the link.
/// A reference that no longer resolves (deleted token, no fallback) keeps the last value.
fn refresh_node(
    tx: &mut Tx,
    page_id: &str,
    id: NodeId,
    tokens: &Tokens,
    renames: &[(String, String)],
) -> bool {
    let Some(orig) = tx.doc().get(id).cloned() else {
        return false;
    };
    let ctx = Parent::of(tx.doc(), page_id, orig.parent).ctx();
    let mut n = orig.clone();
    let mut text_metrics = false;
    for (k, r) in orig.tokens.iter() {
        let raw = renames.iter().fold(r.raw.clone(), |s, (old, new)| {
            tokens::rename_refs(&s, old, new)
        });
        let Ok(new) = tokens.substitute(&raw) else {
            n.tokens.remove(k);
            continue;
        };
        if new == r.applied {
            if raw != r.raw {
                n.tokens.get_mut(k).unwrap().raw = raw;
            }
            continue;
        }
        let mut probe = n.clone();
        css::apply(&mut probe, &[(k.clone(), r.applied.clone())], &ctx);
        probe.tokens = n.tokens.clone();
        if probe != n {
            n.tokens.remove(k);
            continue;
        }
        text_metrics |= matches!(
            k.as_str(),
            "fontSize" | "fontWeight" | "fontFamily" | "font" | "lineHeight" | "letterSpacing"
        );
        let refs = std::mem::take(&mut n.tokens);
        css::apply(&mut n, &[(k.clone(), new.clone())], &ctx);
        n.tokens = refs;
        n.tokens.insert(k.clone(), TokenRef { raw, applied: new });
    }
    if n == orig {
        return false;
    }
    let (old, new) = (orig.rect, n.rect);
    *tx.node_mut(id).unwrap() = n;
    if (old.w - new.w).abs() > 1e-9 || (old.h - new.h).abs() > 1e-9 {
        crate::layout::resize_children(tx, id, old, new);
    }
    if text_metrics {
        fit_text(tx, id, false);
    }
    relayout(tx, Some(id));
    true
}

/// `get_computed_styles` shows `var(...)` for token-bound properties.
pub fn annotate(n: &crate::model::Node, styles: &mut Map<String, Value>) {
    for (k, r) in &n.tokens {
        if styles.contains_key(k) {
            styles.insert(k.clone(), json!(r.raw));
        }
    }
}
