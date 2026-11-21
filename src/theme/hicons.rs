// Outline vectors from hicon-react 1.0.0. The Hand glyph was drawn for Sketch.

use super::Icon;
use crate::io::svg_import;
use crate::render::{draw, tess::Tess};
use egui::{Color32, Mesh, Painter, Rect, epaint::WHITE_UV, pos2};
use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};

macro_rules! hicon {
    ($name:literal) => {
        include_str!(concat!("../../assets/icons/hicon/", $name, ".svg"))
    };
}

pub(super) fn paint(painter: &Painter, rect: Rect, icon: Icon, color: Color32) -> bool {
    let svg = match icon {
        Icon::Dashboard => hicon!("dashboard"),
        Icon::Page => hicon!("page"),
        Icon::Pen => hicon!("pen"),
        Icon::Text => hicon!("text"),
        Icon::Comment => hicon!("comment"),
        Icon::Search => hicon!("search"),
        Icon::Folder => hicon!("folder"),
        Icon::Play => hicon!("play"),
        Icon::More => hicon!("more"),
        Icon::Image => hicon!("image"),
        Icon::Lock => hicon!("lock"),
        Icon::LockOpen => hicon!("lock-open"),
        Icon::Send => hicon!("send"),
        Icon::Hand => include_str!("../../assets/icons/editing/hand.svg"),
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
