//! Design tokens: named values stored with a project and used as CSS custom properties.
//!
//! A token is `--name: value` with a type (color, spacing, radius, ...) and an optional
//! description, like Paper's design tokens (behaviour notes in `analysis/paper-mcp.md`). A
//! value can alias another token with `var(--other)`. Styles written through the MCP tools
//! may use `var(--name)` or `var(--name, fallback)`; the layer then remembers the original
//! text per property ([`TokenRef`]) so it follows later changes to the token.
//!
//! The tokens live in the project manifest. While an MCP call runs, the editor's tokens are
//! the active [`Scope`], which `css::apply` uses to resolve `var()`.

use crate::mcp::css;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::cell::RefCell;

/// The kinds of token, named as in CSS-in-JS theme files.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum TokenType {
    Color,
    Spacing,
    Radius,
    FontFamily,
    FontSize,
    FontWeight,
    LineHeight,
    LetterSpacing,
    Opacity,
    Breakpoint,
    Container,
}

impl TokenType {
    pub const ALL: [TokenType; 11] = [
        TokenType::Color,
        TokenType::Spacing,
        TokenType::Radius,
        TokenType::FontFamily,
        TokenType::FontSize,
        TokenType::FontWeight,
        TokenType::LineHeight,
        TokenType::LetterSpacing,
        TokenType::Opacity,
        TokenType::Breakpoint,
        TokenType::Container,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            TokenType::Color => "color",
            TokenType::Spacing => "spacing",
            TokenType::Radius => "radius",
            TokenType::FontFamily => "fontFamily",
            TokenType::FontSize => "fontSize",
            TokenType::FontWeight => "fontWeight",
            TokenType::LineHeight => "lineHeight",
            TokenType::LetterSpacing => "letterSpacing",
            TokenType::Opacity => "opacity",
            TokenType::Breakpoint => "breakpoint",
            TokenType::Container => "container",
        }
    }

    pub fn parse(s: &str) -> Option<TokenType> {
        TokenType::ALL.into_iter().find(|t| t.as_str() == s.trim())
    }

    /// Heading in the Tokens panel.
    pub fn label(self) -> &'static str {
        match self {
            TokenType::Color => "Colors",
            TokenType::Spacing => "Spacing",
            TokenType::Radius => "Radius",
            TokenType::FontFamily => "Font families",
            TokenType::FontSize => "Font sizes",
            TokenType::FontWeight => "Font weights",
            TokenType::LineHeight => "Line heights",
            TokenType::LetterSpacing => "Letter spacing",
            TokenType::Opacity => "Opacity",
            TokenType::Breakpoint => "Breakpoints",
            TokenType::Container => "Containers",
        }
    }

    /// A starting value for a token added in the Tokens panel.
    pub fn default_value(self) -> &'static str {
        match self {
            TokenType::Color => "#3b82f6",
            TokenType::Spacing => "8px",
            TokenType::Radius => "4px",
            TokenType::FontFamily => "Inter",
            TokenType::FontSize => "16px",
            TokenType::FontWeight => "400",
            TokenType::LineHeight => "1.5",
            TokenType::LetterSpacing => "0px",
            TokenType::Opacity => "0.5",
            TokenType::Breakpoint => "768px",
            TokenType::Container => "1024px",
        }
    }

    /// Tailwind v4 theme namespace, and name prefixes that already mean the same thing.
    fn tailwind(self) -> (&'static str, &'static [&'static str]) {
        match self {
            TokenType::Color => ("color", &["color-", "colors-"]),
            TokenType::Spacing => ("spacing", &["spacing-", "space-"]),
            TokenType::Radius => ("radius", &["radius-", "rounded-"]),
            TokenType::FontFamily => ("font", &["font-family-", "font-"]),
            TokenType::FontSize => ("text", &["text-", "font-size-", "size-"]),
            TokenType::FontWeight => ("font-weight", &["font-weight-", "weight-"]),
            TokenType::LineHeight => ("leading", &["leading-", "line-height-"]),
            TokenType::LetterSpacing => ("tracking", &["tracking-", "letter-spacing-"]),
            TokenType::Opacity => ("opacity", &["opacity-"]),
            TokenType::Breakpoint => ("breakpoint", &["breakpoint-", "bp-"]),
            TokenType::Container => ("container", &["container-"]),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Token {
    /// CSS custom property name, with the leading `--`.
    pub name: String,
    #[serde(rename = "type")]
    pub kind: TokenType,
    /// CSS value text: `#3b82f6`, `8px`, `600`, or an alias such as `var(--blue-500)`.
    pub value: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub description: String,
}

/// How a layer property uses tokens: the text as written, and the value it resolved to.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TokenRef {
    /// The CSS value containing `var(...)`, e.g. `var(--space-2) var(--space-4)`.
    pub raw: String,
    /// What `raw` resolved to when it was last applied.
    pub applied: String,
}

pub const MAX_DESCRIPTION: usize = 1024;
/// Alias chains longer than this are reported as errors (they are almost surely cycles).
const MAX_DEPTH: usize = 32;

/// `--` followed by letters, digits, `_` or `-`.
pub fn valid_name(s: &str) -> bool {
    s.len() > 2
        && s.starts_with("--")
        && s[2..]
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
}

/// The value contains a `var(` reference.
pub fn has_var(s: &str) -> bool {
    s.to_ascii_lowercase().contains("var(")
}

/// Every `var(...)` in `s` as (start, end, name, fallback), outermost only.
fn vars(s: &str) -> Vec<(usize, usize, String, Option<String>)> {
    let lower = s.to_ascii_lowercase();
    let mut out = Vec::new();
    let mut from = 0;
    while let Some(i) = lower[from..].find("var(").map(|i| i + from) {
        // `var(` must not be the tail of a longer identifier (`somevar(`).
        let prev = lower[..i].chars().next_back();
        if prev.is_some_and(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_') {
            from = i + 4;
            continue;
        }
        let open = i + 3;
        let mut depth = 0i32;
        let mut end = None;
        for (j, c) in s[open..].char_indices() {
            match c {
                '(' => depth += 1,
                ')' => {
                    depth -= 1;
                    if depth == 0 {
                        end = Some(open + j);
                        break;
                    }
                }
                _ => {}
            }
        }
        let Some(end) = end else { break };
        let inner = &s[open + 1..end];
        let (name, fallback) = match top_comma(inner) {
            Some(k) => (&inner[..k], Some(inner[k + 1..].trim().to_string())),
            None => (inner, None),
        };
        out.push((i, end + 1, name.trim().to_string(), fallback));
        from = end + 1;
    }
    out
}

/// Position of the first comma outside parentheses.
fn top_comma(s: &str) -> Option<usize> {
    let mut depth = 0i32;
    for (i, c) in s.char_indices() {
        match c {
            '(' => depth += 1,
            ')' => depth -= 1,
            ',' if depth == 0 => return Some(i),
            _ => {}
        }
    }
    None
}

/// Token names referenced anywhere in `s`, including inside fallbacks.
pub fn references(s: &str) -> Vec<String> {
    let mut out = Vec::new();
    for (_, _, name, fb) in vars(s) {
        out.push(name);
        if let Some(fb) = fb {
            out.extend(references(&fb));
        }
    }
    out
}

/// `s` with every `var(--old ...)` reference renamed to `new`.
pub fn rename_refs(s: &str, old: &str, new: &str) -> String {
    let mut out = String::new();
    let mut last = 0;
    for (start, end, name, fb) in vars(s) {
        out.push_str(&s[last..start]);
        let name = if name == old { new.to_string() } else { name };
        match fb {
            Some(fb) => out.push_str(&format!("var({name}, {})", rename_refs(&fb, old, new))),
            None => out.push_str(&format!("var({name})")),
        }
        last = end;
    }
    out.push_str(&s[last..]);
    out
}

/// A token value given as JSON (`"8px"`, `8`, `0.5`) as CSS text for its type.
pub fn value_from_json(kind: TokenType, v: &Value) -> Result<String, String> {
    match v {
        Value::String(s) if !s.trim().is_empty() => Ok(s.trim().to_string()),
        Value::Number(n) => {
            let x = n.as_f64().unwrap_or(0.0);
            Ok(match kind {
                TokenType::Spacing
                | TokenType::Radius
                | TokenType::FontSize
                | TokenType::Breakpoint
                | TokenType::Container => format!("{}px", fmt_num(x)),
                _ => fmt_num(x),
            })
        }
        _ => Err("`value` must be a non-empty string or a number".into()),
    }
}

fn fmt_num(x: f64) -> String {
    if x.fract() == 0.0 && x.abs() < 1e15 {
        format!("{}", x as i64)
    } else {
        format!("{x}")
    }
}

/// Check that a resolved value makes sense for the token's type.
fn check_type(kind: TokenType, v: &str) -> Result<(), String> {
    let v = v.trim();
    let ok = match kind {
        TokenType::Color => css::color(v).is_some(),
        TokenType::Spacing
        | TokenType::Radius
        | TokenType::FontSize
        | TokenType::Breakpoint
        | TokenType::Container
        | TokenType::LetterSpacing => css::length(v, 16.0).is_some(),
        TokenType::FontWeight => {
            matches!(v, "normal" | "bold" | "lighter" | "bolder")
                || v.parse::<f64>().is_ok_and(|w| (1.0..=1000.0).contains(&w))
        }
        TokenType::LineHeight => {
            v == "normal" || v.parse::<f64>().is_ok() || css::length(v, 16.0).is_some()
        }
        TokenType::Opacity => match css::percent(v) {
            Some(p) => (0.0..=1.0).contains(&p),
            None => v.parse::<f64>().is_ok_and(|o| (0.0..=1.0).contains(&o)),
        },
        TokenType::FontFamily => !v.is_empty(),
    };
    if ok {
        Ok(())
    } else {
        Err(format!("`{v}` is not a valid {} value", kind.as_str()))
    }
}

/// The tokens of one project, in the order they were defined.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Tokens(pub Vec<Token>);

impl Tokens {
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    pub fn iter(&self) -> impl Iterator<Item = &Token> {
        self.0.iter()
    }

    /// The token named `name`. Like CSS, the last definition wins when a name repeats.
    pub fn get(&self, name: &str) -> Option<&Token> {
        self.0.iter().rev().find(|t| t.name == name)
    }

    /// `value` with every `var()` replaced by what it stands for.
    pub fn substitute(&self, value: &str) -> Result<String, String> {
        self.subst(value, &mut Vec::new())
    }

    fn subst(&self, value: &str, stack: &mut Vec<String>) -> Result<String, String> {
        let found = vars(value);
        if found.is_empty() {
            return Ok(value.to_string());
        }
        let mut out = String::new();
        let mut last = 0;
        for (start, end, name, fallback) in found {
            out.push_str(&value[last..start]);
            out.push_str(&self.lookup(&name, fallback.as_deref(), stack)?);
            last = end;
        }
        out.push_str(&value[last..]);
        Ok(out)
    }

    fn lookup(
        &self,
        name: &str,
        fallback: Option<&str>,
        stack: &mut Vec<String>,
    ) -> Result<String, String> {
        if stack.iter().any(|s| s == name) {
            stack.push(name.to_string());
            return Err(format!("token alias cycle: {}", stack.join(" → ")));
        }
        if stack.len() >= MAX_DEPTH {
            return Err(format!("token aliases nest deeper than {MAX_DEPTH} levels"));
        }
        match self.get(name) {
            Some(t) => {
                stack.push(name.to_string());
                let r = self.subst(&t.value, stack);
                stack.pop();
                r
            }
            None => match fallback {
                Some(fb) => self.subst(fb, stack),
                None => Err(format!("unknown token `{name}`")),
            },
        }
    }

    /// The fully resolved value of a token.
    pub fn resolve(&self, name: &str) -> Result<String, String> {
        self.lookup(name, None, &mut Vec::new())
    }

    /// Check a value for a token of `kind` named `name` (aliases must resolve, without
    /// cycles, to a value of the right type).
    pub fn check(&self, name: &str, kind: TokenType, value: &str) -> Result<(), String> {
        let mut stack = vec![name.to_string()];
        let resolved = self.subst(value, &mut stack)?;
        check_type(kind, &resolved)
    }

    /// Add a token at the end.
    pub fn create(
        &mut self,
        kind: TokenType,
        name: &str,
        value: &str,
        description: &str,
    ) -> Result<(), String> {
        if !valid_name(name) {
            return Err(format!(
                "`{name}` is not a valid token name (use --letters-digits_or-dashes)"
            ));
        }
        if description.chars().count() > MAX_DESCRIPTION {
            return Err(format!(
                "description is longer than {MAX_DESCRIPTION} characters"
            ));
        }
        self.check(name, kind, value)?;
        self.0.push(Token {
            name: name.to_string(),
            kind,
            value: value.trim().to_string(),
            description: description.trim().to_string(),
        });
        Ok(())
    }

    /// Change a token's value (checked against its type).
    pub fn set_value(&mut self, name: &str, value: &str) -> Result<(), String> {
        let kind = self
            .get(name)
            .ok_or(format!("unknown token `{name}`"))?
            .kind;
        self.check(name, kind, value)?;
        for t in self.0.iter_mut().filter(|t| t.name == name) {
            t.value = value.trim().to_string();
        }
        Ok(())
    }

    pub fn set_description(&mut self, name: &str, description: &str) -> Result<(), String> {
        if description.chars().count() > MAX_DESCRIPTION {
            return Err(format!(
                "description is longer than {MAX_DESCRIPTION} characters"
            ));
        }
        let mut found = false;
        for t in self.0.iter_mut().filter(|t| t.name == name) {
            t.description = description.trim().to_string();
            found = true;
        }
        if found {
            Ok(())
        } else {
            Err(format!("unknown token `{name}`"))
        }
    }

    /// Rename a token and every alias that points at it.
    pub fn rename(&mut self, old: &str, new: &str) -> Result<(), String> {
        if !valid_name(new) {
            return Err(format!(
                "`{new}` is not a valid token name (use --letters-digits_or-dashes)"
            ));
        }
        if self.get(old).is_none() {
            return Err(format!("unknown token `{old}`"));
        }
        if old != new && self.get(new).is_some() {
            return Err(format!("a token named `{new}` already exists"));
        }
        for t in &mut self.0 {
            if t.name == old {
                t.name = new.to_string();
            }
            if has_var(&t.value) {
                t.value = rename_refs(&t.value, old, new);
            }
        }
        Ok(())
    }

    /// Delete every definition of `name`. Aliases to it keep their text (and stop resolving
    /// unless they have a fallback).
    pub fn remove(&mut self, name: &str) -> bool {
        let before = self.0.len();
        self.0.retain(|t| t.name != name);
        self.0.len() != before
    }

    /// Tokens matching a type filter and a name glob, in definition order.
    pub fn filter<'a>(
        &'a self,
        types: &[TokenType],
        pattern: Option<&str>,
    ) -> impl Iterator<Item = &'a Token> + 'a {
        let types = types.to_vec();
        let pattern = pattern.map(str::to_string);
        self.0.iter().filter(move |t| {
            (types.is_empty() || types.contains(&t.kind))
                && pattern
                    .as_deref()
                    .is_none_or(|p| crate::mcp::exec::glob(p, &t.name))
        })
    }
}

/// `:root { --name: value; }`, with descriptions as comments.
pub fn to_css<'a>(tokens: impl IntoIterator<Item = &'a Token>) -> String {
    let mut out = String::from(":root {\n");
    for t in tokens {
        if !t.description.is_empty() {
            out.push_str(&format!("  /* {} */\n", t.description.replace("*/", "* /")));
        }
        out.push_str(&format!("  {}: {};\n", t.name, t.value));
    }
    out.push_str("}\n");
    out
}

/// The Tailwind v4 name of a token: `--primary` (color) → `--color-primary`.
pub fn tailwind_name(t: &Token) -> String {
    let (ns, prefixes) = t.kind.tailwind();
    let bare = &t.name[2..];
    let rest = prefixes
        .iter()
        .find_map(|p| bare.strip_prefix(p))
        .filter(|r| !r.is_empty())
        .unwrap_or(bare);
    format!("--{ns}-{rest}")
}

/// A Tailwind v4 `@theme { ... }` block. Aliases are renamed to the namespaced names.
pub fn to_tailwind<'a>(all: &Tokens, tokens: impl IntoIterator<Item = &'a Token>) -> String {
    let mut out = String::from("@theme {\n");
    for t in tokens {
        let mut value = t.value.clone();
        for r in references(&t.value) {
            if let Some(target) = all.get(&r) {
                value = rename_refs(&value, &r, &tailwind_name(target));
            }
        }
        if !t.description.is_empty() {
            out.push_str(&format!("  /* {} */\n", t.description.replace("*/", "* /")));
        }
        out.push_str(&format!("  {}: {};\n", tailwind_name(t), value));
    }
    out.push_str("}\n");
    out
}

// ------------------------------------------------------------------------------ scope

thread_local! {
    static ACTIVE: RefCell<Option<Tokens>> = const { RefCell::new(None) };
}

/// While alive, `css::apply` on this thread resolves `var()` against these tokens.
pub struct Scope {
    prev: Option<Tokens>,
}

pub fn scope(tokens: &Tokens) -> Scope {
    let prev = ACTIVE.with(|a| a.replace(Some(tokens.clone())));
    Scope { prev }
}

impl Drop for Scope {
    fn drop(&mut self) {
        let prev = self.prev.take();
        ACTIVE.with(|a| *a.borrow_mut() = prev);
    }
}

/// `short` is a CSS shorthand that sets `long` (both camelCase).
fn covers(short: &str, long: &str) -> bool {
    short != long
        && match short {
            "background" | "padding" | "margin" => long.starts_with(short),
            "border" => long.starts_with("border") && !long.ends_with("Radius"),
            "borderRadius" => long.ends_with("Radius"),
            "gap" => matches!(long, "rowGap" | "columnGap"),
            "font" => long.starts_with("font") || long == "lineHeight",
            "flex" => long.starts_with("flex") && long != "flexDirection" && long != "flexWrap",
            "inset" => matches!(long, "top" | "right" | "bottom" | "left"),
            _ => false,
        }
}

/// Setting `a` also sets (part of) `b`, or the other way round.
fn overlaps(a: &str, b: &str) -> bool {
    a == b || covers(a, b) || covers(b, a)
}

/// Resolve `var()` in `styles` against the active scope, recording or clearing each
/// property's [`TokenRef`] on `refs`. Properties that can't be resolved are left out and
/// reported in `ignored` with the reason.
pub fn bind(
    refs: &mut std::collections::BTreeMap<String, TokenRef>,
    styles: &[(String, String)],
    ignored: &mut Vec<String>,
) -> Vec<(String, String)> {
    ACTIVE.with(|a| {
        let active = a.borrow();
        let mut out = Vec::with_capacity(styles.len());
        for (k, v) in styles {
            // A new value for a property replaces any token link on it, its shorthand or its
            // longhands (`background` / `backgroundColor`, `padding` / `paddingTop`, ...).
            refs.retain(|j, _| !overlaps(j, k));
            if !has_var(v) {
                out.push((k.clone(), v.clone()));
                continue;
            }
            let resolved = match active.as_ref() {
                Some(t) => t.substitute(v),
                None => Err("design tokens are not available here".to_string()),
            };
            match resolved {
                Ok(r) => {
                    refs.insert(
                        k.clone(),
                        TokenRef {
                            raw: v.clone(),
                            applied: r.clone(),
                        },
                    );
                    out.push((k.clone(), r));
                }
                Err(e) => ignored.push(format!("{k} ({e})")),
            }
        }
        out
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn set() -> Tokens {
        let mut t = Tokens::default();
        t.create(TokenType::Color, "--blue-500", "#3b82f6", "")
            .unwrap();
        t.create(TokenType::Color, "--primary", "var(--blue-500)", "Buttons")
            .unwrap();
        t.create(TokenType::Spacing, "--space-2", "8px", "")
            .unwrap();
        t
    }

    #[test]
    fn names_follow_css_custom_property_rules() {
        assert!(valid_name("--color-primary"));
        assert!(valid_name("--a_1"));
        assert!(!valid_name("color"));
        assert!(!valid_name("--"));
        assert!(!valid_name("--a b"));
    }

    #[test]
    fn aliases_and_fallbacks_resolve() {
        let t = set();
        assert_eq!(t.resolve("--primary").unwrap(), "#3b82f6");
        assert_eq!(
            t.substitute("var(--space-2) var(--missing, 4px)").unwrap(),
            "8px 4px"
        );
        assert_eq!(
            t.substitute("1px solid var(--primary)").unwrap(),
            "1px solid #3b82f6"
        );
        assert_eq!(
            t.substitute("var(--missing, var(--space-2))").unwrap(),
            "8px"
        );
        assert!(t.substitute("var(--missing)").is_err());
    }

    #[test]
    fn cycles_are_errors_not_hangs() {
        let mut t = set();
        assert!(
            t.set_value("--blue-500", "var(--primary)")
                .unwrap_err()
                .contains("cycle")
        );
        // Self reference on create.
        assert!(
            t.create(TokenType::Color, "--x", "var(--x)", "")
                .unwrap_err()
                .contains("cycle")
        );
    }

    #[test]
    fn values_are_checked_against_the_type() {
        let mut t = set();
        assert!(t.create(TokenType::Color, "--bad", "8px", "").is_err());
        assert!(t.create(TokenType::Spacing, "--bad", "#fff", "").is_err());
        assert!(t.create(TokenType::Opacity, "--o", "50%", "").is_ok());
        assert!(t.create(TokenType::Opacity, "--o2", "2", "").is_err());
        assert!(t.create(TokenType::FontWeight, "--w", "600", "").is_ok());
        assert_eq!(
            value_from_json(TokenType::Spacing, &serde_json::json!(12)).unwrap(),
            "12px"
        );
        assert_eq!(
            value_from_json(TokenType::FontWeight, &serde_json::json!(600)).unwrap(),
            "600"
        );
    }

    #[test]
    fn rename_rewrites_aliases() {
        let mut t = set();
        t.rename("--blue-500", "--brand").unwrap();
        assert_eq!(t.get("--primary").unwrap().value, "var(--brand)");
        assert_eq!(t.resolve("--primary").unwrap(), "#3b82f6");
        assert!(t.rename("--primary", "--space-2").is_err());
        assert_eq!(
            rename_refs("var(--a, var(--b)) var(--b)", "--b", "--c"),
            "var(--a, var(--c)) var(--c)"
        );
    }

    #[test]
    fn css_and_tailwind_output() {
        let t = set();
        let css = to_css(t.iter());
        assert!(css.contains("  --primary: var(--blue-500);"));
        assert!(css.contains("/* Buttons */"));
        let tw = to_tailwind(&t, t.iter());
        assert!(tw.starts_with("@theme {"));
        assert!(tw.contains("--color-primary: var(--color-blue-500);"));
        assert!(tw.contains("--spacing-2: 8px;"));
    }

    #[test]
    fn shorthands_overlap_their_longhands() {
        assert!(overlaps("background", "backgroundColor"));
        assert!(overlaps("paddingTop", "padding"));
        assert!(overlaps("border", "borderColor"));
        assert!(!overlaps("border", "borderRadius"));
        assert!(overlaps("borderRadius", "borderTopLeftRadius"));
        assert!(!overlaps("fontSize", "fontWeight"));
        assert!(!overlaps("flex", "flexDirection"));
    }

    #[test]
    fn bind_records_and_clears_refs() {
        let t = set();
        let _s = scope(&t);
        let mut refs = Default::default();
        let mut ignored = Vec::new();
        let out = bind(
            &mut refs,
            &[
                ("backgroundColor".into(), "var(--primary)".into()),
                ("padding".into(), "var(--nope)".into()),
            ],
            &mut ignored,
        );
        assert_eq!(out, vec![("backgroundColor".into(), "#3b82f6".into())]);
        assert_eq!(refs["backgroundColor"].raw, "var(--primary)");
        assert_eq!(ignored.len(), 1);
        let out = bind(
            &mut refs,
            &[("backgroundColor".into(), "red".into())],
            &mut ignored,
        );
        assert_eq!(out.len(), 1);
        assert!(refs.is_empty());
    }
}
