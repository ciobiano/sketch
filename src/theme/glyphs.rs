//! Sketch’s editing glyphs and navigation icons from @hugeicons/core-free-icons 4.4.0.

use super::Icon;
use egui::{Color32, ColorImage, Id, Painter, Rect, TextureHandle, TextureOptions, pos2, vec2};
use resvg::tiny_skia::{Pixmap, Transform};

macro_rules! glyph {
    ($name:literal) => {
        include_str!(concat!("../../assets/icons/editing/", $name, ".svg"))
    };
}

macro_rules! hugeicon {
    ($name:literal) => {
        include_str!(concat!("../../assets/icons/hugeicons/", $name, ".svg"))
    };
}

pub(super) fn paint(painter: &Painter, rect: Rect, icon: Icon, color: Color32) -> bool {
    let svg = match icon {
        Icon::Dashboard => hugeicon!("dashboard"),
        Icon::Page => glyph!("page"),
        Icon::Pen => glyph!("pen"),
        Icon::Text => glyph!("text"),
        Icon::Comment => glyph!("comment"),
        Icon::Search => hugeicon!("search"),
        Icon::Folder => hugeicon!("folder"),
        Icon::Play => hugeicon!("play"),
        Icon::More => hugeicon!("more"),
        Icon::Image => glyph!("image"),
        Icon::Lock => glyph!("lock"),
        Icon::LockOpen => glyph!("lock-open"),
        Icon::Send => hugeicon!("send"),
        Icon::Hand => glyph!("hand"),
        Icon::Frame => glyph!("frame"),
        Icon::Rect => glyph!("rectangle"),
        Icon::Group => glyph!("group"),
        Icon::Sidebar => hugeicon!("sidebar"),
        Icon::SidebarRight => hugeicon!("sidebar-right"),
        Icon::Ellipse => glyph!("ellipse"),
        Icon::Line => glyph!("line"),
        Icon::Divider => glyph!("divider"),
        Icon::VectorPath => glyph!("path"),
        Icon::LayoutRow => glyph!("row"),
        Icon::LayoutColumn => glyph!("column"),
        Icon::Stack => glyph!("stack"),
        Icon::Diamond => glyph!("diamond"),
        Icon::Polygon => glyph!("polygon"),
        Icon::Star => glyph!("star"),
        Icon::Arrow => glyph!("arrow"),
        Icon::Connector => glyph!("connector"),
        Icon::Sticky => glyph!("sticky"),
        Icon::Ink => glyph!("ink"),
        _ => return false,
    };
    if !painter.is_visible() || color == Color32::TRANSPARENT {
        return true;
    }
    let ctx = painter.ctx();
    let scale = ctx.pixels_per_point();
    let pixels = (rect.width().min(rect.height()) * scale)
        .round()
        .clamp(1.0, 128.0) as u32;
    let id = Id::new(("workspace-glyph", icon, pixels));
    let cached = ctx.data(|data| data.get_temp::<TextureHandle>(id));
    let texture = cached.unwrap_or_else(|| {
        let texture = ctx.load_texture(
            format!("{icon:?} {pixels}px"),
            rasterize(svg, pixels),
            TextureOptions::LINEAR,
        );
        ctx.data_mut(|data| data.insert_temp(id, texture.clone()));
        texture
    });
    let side = pixels as f32 / scale;
    let origin = rect.center() - vec2(side / 2.0, side / 2.0);
    let origin = pos2(
        (origin.x * scale).round() / scale,
        (origin.y * scale).round() / scale,
    );
    painter.image(
        texture.id(),
        Rect::from_min_size(origin, vec2(side, side)),
        Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)),
        color,
    );
    true
}

fn rasterize(svg: &str, pixels: u32) -> ColorImage {
    let tree = usvg::Tree::from_str(svg, &usvg::Options::default()).expect("valid bundled glyph");
    let scale = pixels as f32 / tree.size().width();
    let mut pixmap = Pixmap::new(pixels, pixels).expect("small square glyph texture");
    resvg::render(
        &tree,
        Transform::from_scale(scale, scale),
        &mut pixmap.as_mut(),
    );
    ColorImage::from_rgba_premultiplied([pixels as usize; 2], pixmap.data())
}
