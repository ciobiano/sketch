//! Hand-drawn workspace glyphs on a common 24-point grid.

use super::Icon;
use crate::io::svg_import;
use crate::render::{draw, tess::Tess};
use egui::{Color32, Mesh, Painter, Rect, epaint::WHITE_UV, pos2};
use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};

macro_rules! glyph {
    ($name:literal) => {
        include_str!(concat!("../../assets/icons/editing/", $name, ".svg"))
    };
}

pub(super) fn paint(painter: &Painter, rect: Rect, icon: Icon, color: Color32) -> bool {
    let svg = match icon {
        Icon::Dashboard => glyph!("dashboard"),
        Icon::Page => glyph!("page"),
        Icon::Pen => glyph!("pen"),
        Icon::Text => glyph!("text"),
        Icon::Comment => glyph!("comment"),
        Icon::Search => glyph!("search"),
        Icon::Folder => glyph!("folder"),
        Icon::Play => glyph!("play"),
        Icon::More => glyph!("more"),
        Icon::Image => glyph!("image"),
        Icon::Lock => glyph!("lock"),
        Icon::LockOpen => glyph!("lock-open"),
        Icon::Send => glyph!("send"),
        Icon::Hand => glyph!("hand"),
        Icon::Frame => glyph!("frame"),
        Icon::Rect => glyph!("rectangle"),
        Icon::Group => glyph!("group"),
        Icon::Sidebar => glyph!("sidebar"),
        Icon::SidebarRight => glyph!("sidebar-right"),
        _ => return false,
    };
    static CACHE: OnceLock<Mutex<HashMap<Icon, Mesh>>> = OnceLock::new();
    let mut cache = CACHE.get_or_init(Mutex::default).lock().unwrap();
    let mut mesh = cache.entry(icon).or_insert_with(|| tessellate(svg)).clone();
    drop(cache);
    let side = rect.width().min(rect.height());
    let origin = rect.center() - egui::Vec2::splat(side * 0.5);
    for vertex in &mut mesh.vertices {
        vertex.pos = origin + vertex.pos.to_vec2() * (side / 24.0);
        vertex.color = color;
    }
    painter.add(egui::Shape::mesh(mesh));
    true
}

fn tessellate(svg: &str) -> Mesh {
    let mut tess = Tess::new(4.0);
    let mut mesh = Mesh::default();
    for node in svg_import::parse(svg).expect("valid bundled icon SVG") {
        let ops = draw::node_draw(&node, node.rect, 1.0).ops;
        for piece in tess.ops(&ops) {
            let base = mesh.vertices.len() as u32;
            mesh.vertices.extend(
                piece
                    .mesh
                    .vertices
                    .iter()
                    .map(|vertex| egui::epaint::Vertex {
                        pos: pos2(vertex.pos[0], vertex.pos[1]),
                        uv: WHITE_UV,
                        color: Color32::WHITE,
                    }),
            );
            mesh.indices
                .extend(piece.mesh.indices.iter().map(|i| i + base));
        }
    }
    mesh
}
