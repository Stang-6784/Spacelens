//! Squarified treemap layout, baked into a single GPU mesh for fast redraws.

use crate::theme;
use crate::tree::Tree;
use eframe::egui::epaint::{Mesh, Vertex, WHITE_UV};
use eframe::egui::{pos2, vec2, Color32, Pos2, Rect};
use std::sync::Arc;

const MAX_ITEMS: usize = 250_000;
const MAX_LABELS: usize = 700;
const HEADER: f32 = 18.0;
const PAD: f32 = 2.0;

pub struct Item {
    pub rect: Rect,
    pub node: u32,
}

pub struct Label {
    pub rect: Rect,
    pub node: u32,
    pub header: bool,
}

#[derive(Default)]
pub struct Layout {
    pub items: Vec<Item>,
    pub labels: Vec<Label>,
    pub mesh: Arc<Mesh>,
}

impl Layout {
    /// Deepest node under the point. Items are stored parents-first, so search from the end.
    pub fn hit(&self, p: Pos2) -> Option<u32> {
        self.items.iter().rev().find(|it| it.rect.contains(p)).map(|it| it.node)
    }

    pub fn rect_of(&self, node: u32) -> Option<Rect> {
        self.items.iter().find(|it| it.node == node).map(|it| it.rect)
    }
}

struct Builder<'a> {
    tree: &'a Tree,
    ppp: f32,
    px: f32,
    out: Layout,
    mesh: Mesh,
}

pub fn build(tree: &Tree, root: u32, area: Rect, pixels_per_point: f32) -> Layout {
    let mut b = Builder { tree, ppp: pixels_per_point, px: 1.0 / pixels_per_point, out: Layout::default(), mesh: Mesh::default() };
    if tree.node(root).size > 0 {
        if tree.node(root).is_dir {
            b.children(root, area, 0);
        } else {
            b.place(root, area, 0);
        }
    }
    b.out.mesh = Arc::new(b.mesh);
    b.out
}

impl Builder<'_> {
    fn snap(&self, v: f32) -> f32 {
        (v * self.ppp).round() / self.ppp
    }

    fn quad(&mut self, r: Rect, top: Color32, bottom: Color32) {
        let i = self.mesh.vertices.len() as u32;
        let v = |pos: Pos2, color: Color32| Vertex { pos, uv: WHITE_UV, color };
        self.mesh.vertices.extend_from_slice(&[
            v(r.left_top(), top),
            v(r.right_top(), top),
            v(r.right_bottom(), bottom),
            v(r.left_bottom(), bottom),
        ]);
        self.mesh.add_triangle(i, i + 1, i + 2);
        self.mesh.add_triangle(i, i + 2, i + 3);
    }

    /// Lays out a folder's children in `r` with the squarified algorithm (Bruls et al.).
    fn children(&mut self, dir: u32, r: Rect, depth: u16) {
        let tree = self.tree;
        let kids = &tree.node(dir).children;
        // Children are sorted by size descending; zero-size ones are skipped.
        let n = kids.iter().take_while(|&&k| tree.node(k).size > 0).count();
        let mut remaining: f64 = kids[..n].iter().map(|&k| tree.node(k).size as f64).sum();
        if remaining <= 0.0 {
            return;
        }
        let (mut x, mut y, mut w, mut h) = (r.min.x as f64, r.min.y as f64, r.width() as f64, r.height() as f64);
        let mut i = 0;
        while i < n && w >= 1.0 && h >= 1.0 && remaining > 0.0 {
            if self.out.items.len() >= MAX_ITEMS {
                return;
            }
            let scale = w * h / remaining;
            let side = w.min(h);
            let first = tree.node(kids[i]).size as f64 * scale;
            let mut j = i;
            let mut row = 0.0;
            let mut worst = f64::MAX;
            while j < n {
                let a = tree.node(kids[j]).size as f64 * scale;
                let s = row + a;
                let ratio = (side * side * first / (s * s)).max(s * s / (side * side * a));
                if j > i && ratio > worst {
                    break;
                }
                worst = ratio;
                row = s;
                j += 1;
            }
            let thick = row / side;
            let mut row_bytes = 0.0;
            if w >= h {
                let mut yy = y;
                for &k in &kids[i..j] {
                    let size = tree.node(k).size as f64;
                    let hk = size * scale / thick;
                    self.place(k, Rect::from_min_size(pos2(x as f32, yy as f32), vec2(thick as f32, hk as f32)), depth);
                    yy += hk;
                    row_bytes += size;
                }
                x += thick;
                w -= thick;
            } else {
                let mut xx = x;
                for &k in &kids[i..j] {
                    let size = tree.node(k).size as f64;
                    let wk = size * scale / thick;
                    self.place(k, Rect::from_min_size(pos2(xx as f32, y as f32), vec2(wk as f32, thick as f32)), depth);
                    xx += wk;
                    row_bytes += size;
                }
                y += thick;
                h -= thick;
            }
            remaining -= row_bytes;
            i = j;
        }
    }

    fn place(&mut self, node: u32, r: Rect, depth: u16) {
        let (x0, y0, x1, y1) = (self.snap(r.min.x), self.snap(r.min.y), self.snap(r.max.x), self.snap(r.max.y));
        let px = self.px;
        if x1 - x0 < px || y1 - y0 < px {
            return;
        }
        let rect = Rect::from_min_max(pos2(x0, y0), pos2(x1, y1));
        self.out.items.push(Item { rect, node });

        // 1px gap on the right/bottom separates neighbours (only when there's room for it).
        let gx = if x1 - x0 > 2.0 * px { px } else { 0.0 };
        let gy = if y1 - y0 > 2.0 * px { px } else { 0.0 };
        let inner = Rect::from_min_max(rect.min, pos2(x1 - gx, y1 - gy));
        let n = self.tree.node(node);

        if !n.is_dir {
            let c = theme::category_color(n.cat);
            self.quad(inner, theme::shade(c, 1.08), theme::shade(c, 0.78));
            if inner.width() >= 64.0 && inner.height() >= 22.0 && self.out.labels.len() < MAX_LABELS {
                self.out.labels.push(Label { rect: inner, node, header: false });
            }
            return;
        }

        let frame = theme::folder_frame(depth);
        self.quad(inner, frame, frame);
        if inner.width() < 4.0 || inner.height() < 4.0 {
            return;
        }
        let header = if inner.width() > 80.0 && inner.height() > 46.0 { HEADER } else { 0.0 };
        if header > 0.0 && self.out.labels.len() < MAX_LABELS {
            self.out.labels.push(Label { rect: Rect::from_min_size(inner.min, vec2(inner.width(), header)), node, header: true });
        }
        let top = if header > 0.0 { header } else { PAD };
        let content = Rect::from_min_max(pos2(inner.min.x + PAD, inner.min.y + top), pos2(inner.max.x - PAD, inner.max.y - PAD));
        if content.width() >= 2.0 && content.height() >= 2.0 {
            self.children(node, content, depth + 1);
        }
    }
}
