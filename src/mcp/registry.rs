//! Open Sketch apps note their MCP port in a small file, so `sketch-rs-mcp` can find whichever
//! ones are running: the installed app, a dev build on another port, or both at once.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::time::Duration;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Instance {
    pub port: u16,
    pub pid: u32,
    /// The app's name, e.g. "Sketch" or "Sketch Dev".
    pub name: String,
    /// When it started, in Unix milliseconds.
    pub started: u64,
}

/// `$SKETCH_RS_RUNTIME_DIR`, else `<config dir>/sketch-rs/running`.
pub fn dir() -> Option<PathBuf> {
    crate::env_var("RUNTIME_DIR")
        .map(PathBuf::from)
        .or_else(|| dirs::config_dir().map(|d| d.join("sketch-rs").join("running")))
}

/// This app's entry. Dropping it (the app quitting) removes the file.
pub struct Registration(PathBuf);

impl Drop for Registration {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

/// Record that this process serves MCP on `port`.
pub fn register(port: u16) -> Option<Registration> {
    let dir = dir()?;
    std::fs::create_dir_all(&dir).ok()?;
    let me = Instance {
        port,
        pid: std::process::id(),
        name: app_name(),
        started: crate::project::now_ms(),
    };
    let path = dir.join(format!("{}.json", me.pid));
    std::fs::write(&path, serde_json::to_vec(&me).ok()?).ok()?;
    Some(Registration(path))
}

/// The `.app` bundle's name, or "Sketch" outside a bundle (`cargo run`).
fn app_name() -> String {
    std::env::current_exe()
        .ok()
        .and_then(|p| {
            p.ancestors()
                .find(|a| a.extension().is_some_and(|e| e == "app"))
                .and_then(|a| a.file_stem())
                .map(|s| s.to_string_lossy().into_owned())
        })
        .unwrap_or_else(|| "Sketch".into())
}

/// Something accepts connections on this loopback port.
pub fn answers(port: u16) -> bool {
    std::net::TcpStream::connect_timeout(
        &std::net::SocketAddr::from(([127, 0, 0, 1], port)),
        Duration::from_millis(200),
    )
    .is_ok()
}

/// The apps open now, newest first, one per port. Entries whose port no longer answers (an
/// app that crashed) are deleted.
pub fn running() -> Vec<Instance> {
    let Some(entries) = dir().and_then(|d| std::fs::read_dir(d).ok()) else {
        return Vec::new();
    };
    let mut v = Vec::new();
    for e in entries.flatten() {
        let path = e.path();
        if path.extension().is_none_or(|x| x != "json") {
            continue;
        }
        match std::fs::read(&path)
            .ok()
            .and_then(|b| serde_json::from_slice::<Instance>(&b).ok())
        {
            Some(i) if answers(i.port) => v.push(i),
            _ => {
                let _ = std::fs::remove_file(&path);
            }
        }
    }
    v.sort_by_key(|i| std::cmp::Reverse(i.started));
    let mut seen = Vec::new();
    v.retain(|i| {
        let new = !seen.contains(&i.port);
        seen.push(i.port);
        new
    });
    v
}

/// The ports the bridge should use. `$SKETCH_RS_MCP_PORT` alone when it is set; otherwise
/// every open app, newest first, then the default port (apps before 0.1.4 write no entry).
pub fn targets() -> Vec<u16> {
    if crate::env_var("MCP_PORT").is_some() {
        return vec![super::port()];
    }
    let mut v: Vec<u16> = running().into_iter().map(|i| i.port).collect();
    if !v.contains(&super::DEFAULT_PORT) {
        v.push(super::DEFAULT_PORT);
    }
    v
}
