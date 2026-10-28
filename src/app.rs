//! The Spacelens window: sidebar, summary, folder table and treemap.

use crate::categories::{self, Category};
use crate::kit::{self, Kind};
use crate::scan::{self, Progress, ScanOutput, CANCELLED};
use crate::theme::{self, icon, *};
use crate::tree::{Tree, NONE};
use crate::{drives, fmt, treemap, win};
use eframe::egui::{
    self, epaint::Shadow, pos2, vec2, Align, Align2, Color32, CornerRadius, Id, Key, LayerId,
    Layout, Order, Pos2, Rect, RichText, Sense, Shape, Stroke, StrokeKind, Ui, UiBuilder, Vec2,
    ViewportCommand,
};
use std::collections::HashSet;
use std::ops::Range;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc};
use std::time::{Duration, Instant};

const ROW_H: f32 = 28.0;
const SEARCH_ID: &str = "search";

enum Action {
    Select(u32),
    Toggle(u32),
    Reveal(u32),
    Zoom(u32),
    Open(u32),
    Explorer(u32),
    CopyPath(u32),
    AskDelete(u32),
    Scan(String),
    PickFolder,
    Cancel,
    Elevate,
}

struct Job {
    path: String,
    progress: Arc<Progress>,
    cancel: Arc<AtomicBool>,
    rx: mpsc::Receiver<Result<ScanOutput, String>>,
    started: Instant,
}

struct LastScan {
    path: String,
    secs: f32,
    used_mft: bool,
    note: Option<String>,
}

pub struct App {
    drives: Vec<drives::Drive>,
    is_admin: bool,

    tree: Option<Tree>,
    view: u32,
    expanded: HashSet<u32>,
    rows: Vec<(u32, u16)>,
    selected: Option<u32>,
    scroll_to: Option<usize>,
    visible: Range<usize>,

    search: String,
    applied_search: String,
    search_edited: Option<Instant>,
    results: Option<Vec<u32>>,
    result_total: usize,

    job: Option<Job>,
    last: Option<LastScan>,

    map: treemap::Layout,
    map_key: Option<(Rect, u32, u64, f32)>,
    generation: u64,
    hover: Option<u32>,
    menu_node: Option<u32>,
    split: f32,

    confirm_delete: Option<u32>,
    message: Option<(String, String)>,

    window_styled: bool,
    frames: u64,
    snapshot: Option<PathBuf>,
    snapshot_frames: u32,
}

impl App {
    pub fn new(cc: &eframe::CreationContext<'_>, start: Option<String>, snapshot: Option<PathBuf>) -> Self {
        theme::install(&cc.egui_ctx);
        let mut app = App {
            drives: drives::list(),
            is_admin: win::is_admin(),
            tree: None,
            view: NONE,
            expanded: HashSet::new(),
            rows: Vec::new(),
            selected: None,
            scroll_to: None,
            visible: 0..0,
            search: String::new(),
            applied_search: String::new(),
            search_edited: None,
            results: None,
            result_total: 0,
            job: None,
            last: None,
            map: treemap::Layout::default(),
            map_key: None,
            generation: 0,
            hover: None,
            menu_node: None,
            split: 0.5,
            confirm_delete: None,
            message: None,
            window_styled: false,
            frames: 0,
            snapshot,
            snapshot_frames: 0,
        };
        if let Some(p) = start {
            app.start_scan(p);
        }
        app
    }

    // ---------------------------------------------------------------- scanning

    fn start_scan(&mut self, path: String) {
        if let Some(j) = self.job.take() {
            j.cancel.store(true, Ordering::Relaxed);
        }
        let path = scan::normalize(&path);
        let progress = Arc::new(Progress::default());
        let cancel = Arc::new(AtomicBool::new(false));
        let (tx, rx) = mpsc::channel();
        {
            let (progress, cancel, path, admin) = (progress.clone(), cancel.clone(), path.clone(), self.is_admin);
            let _ = std::thread::Builder::new().name("scan".into()).spawn(move || {
                let _ = tx.send(scan::run(&path, admin, &progress, &cancel));
            });
        }
        // Free the previous tree off the UI thread; big trees take a moment to drop.
        if let Some(old) = self.tree.take() {
            std::thread::spawn(move || drop(old));
        }
        self.map = treemap::Layout::default();
        self.map_key = None;
        self.rows.clear();
        self.results = None;
        self.hover = None;
        self.selected = None;
        self.job = Some(Job { path, progress, cancel, rx, started: Instant::now() });
    }

    fn poll_job(&mut self) {
        let Some(job) = &self.job else { return };
        let msg = match job.rx.try_recv() {
            Ok(m) => m,
            Err(mpsc::TryRecvError::Empty) => return,
            Err(mpsc::TryRecvError::Disconnected) => Err("The scan stopped unexpectedly.".into()),
        };
        let job = self.job.take().unwrap();
        match msg {
            Ok(out) => {
                let root = out.tree.root;
                self.view = root;
                self.expanded.clear();
                self.expanded.insert(root);
                self.selected = Some(root);
                self.search.clear();
                self.applied_search.clear();
                self.results = None;
                self.tree = Some(out.tree);
                self.generation += 1;
                self.last = Some(LastScan {
                    path: job.path,
                    secs: job.started.elapsed().as_secs_f32(),
                    used_mft: out.used_mft,
                    note: out.note,
                });
                self.rebuild_rows();
                self.scroll_to = Some(0);
                self.drives = drives::list();
            }
            Err(e) if e == CANCELLED => {}
            Err(e) => self.message = Some(("Scan failed".into(), e)),
        }
    }

    fn progress_text(job: &Job) -> String {
        let p = &job.progress;
        let (items, total) = (p.items.load(Ordering::Relaxed), p.total.load(Ordering::Relaxed));
        let secs = job.started.elapsed().as_secs_f32();
        let phase = p.phase();
        if total > 0 && phase.starts_with("Reading") {
            format!("{phase}  ·  {:.0}%  ·  {secs:.1}s", items as f64 / total as f64 * 100.0)
        } else {
            format!("{phase}  ·  {} items  ·  {secs:.1}s", fmt::count(items))
        }
    }

    fn progress_fraction(job: &Job) -> Option<f32> {
        let p = &job.progress;
        let total = p.total.load(Ordering::Relaxed);
        (total > 0 && p.phase().starts_with("Reading"))
            .then(|| (p.items.load(Ordering::Relaxed) as f64 / total as f64) as f32)
    }

    // ---------------------------------------------------------------- rows & search

    fn rebuild_rows(&mut self) {
        self.rows.clear();
        let Some(t) = &self.tree else { return };
        if let Some(r) = &self.results {
            self.rows.extend(r.iter().map(|&i| (i, 0)));
            return;
        }
        let mut stack = vec![(t.root, 0u16)];
        while let Some((i, d)) = stack.pop() {
            self.rows.push((i, d));
            if t.node(i).is_dir && self.expanded.contains(&i) {
                for &c in t.node(i).children.iter().rev() {
                    stack.push((c, d + 1));
                }
            }
        }
    }

    fn apply_search(&mut self, ctx: &egui::Context) {
        if self.search == self.applied_search {
            self.search_edited = None;
            return;
        }
        let edited = *self.search_edited.get_or_insert_with(Instant::now);
        if edited.elapsed() < Duration::from_millis(200) {
            ctx.request_repaint_after(Duration::from_millis(210));
            return;
        }
        self.search_edited = None;
        self.applied_search = self.search.clone();
        let q = self.applied_search.trim().to_lowercase();
        self.results = match (&self.tree, q.is_empty()) {
            (Some(t), false) => {
                let mut hits = Vec::new();
                let mut stack = t.node(t.root).children.clone();
                while let Some(i) = stack.pop() {
                    let n = t.node(i);
                    if contains_ci(&n.name, &q) {
                        hits.push(i);
                    }
                    if n.is_dir {
                        stack.extend_from_slice(&n.children);
                    }
                }
                hits.sort_unstable_by(|&a, &b| t.node(b).size.cmp(&t.node(a).size));
                self.result_total = hits.len();
                hits.truncate(5000);
                Some(hits)
            }
            _ => None,
        };
        self.rebuild_rows();
        self.scroll_to = Some(0);
    }

    fn clear_search(&mut self) {
        self.search.clear();
        self.applied_search.clear();
        self.results = None;
    }

    fn ensure_visible(&mut self, i: u32) {
        if let Some(pos) = self.rows.iter().position(|r| r.0 == i) {
            let v = &self.visible;
            if v.is_empty() || pos <= v.start || pos + 1 >= v.end {
                self.scroll_to = Some(pos);
            }
        }
    }

    fn reveal(&mut self, i: u32) {
        let Some(t) = &self.tree else { return };
        let mut p = t.node(i).parent;
        while p != NONE {
            self.expanded.insert(p);
            p = t.node(p).parent;
        }
        if self.results.is_some() {
            self.clear_search();
        }
        self.selected = Some(i);
        self.rebuild_rows();
        self.scroll_to = self.rows.iter().position(|r| r.0 == i);
    }

    fn delete(&mut self, i: u32) {
        let Some(t) = &mut self.tree else { return };
        let path = t.path(i);
        if let Err(e) = trash::delete(&path) {
            self.message = Some(("Couldn't move to the Recycle Bin".into(), e.to_string()));
            return;
        }
        if std::path::Path::new(&path).exists() {
            self.message = Some(("Couldn't delete".into(), format!("{path} is still on disk.")));
            return;
        }
        let parent = t.node(i).parent;
        let view_inside = t.is_under(self.view, i);
        t.remove(i);
        if view_inside {
            self.view = parent;
        }
        if let Some(r) = &mut self.results {
            r.retain(|&x| x != i && !t.is_under(x, i));
        }
        self.expanded.remove(&i);
        self.selected = Some(parent);
        self.generation += 1;
        self.rebuild_rows();
        self.drives = drives::list();
    }

    fn apply(&mut self, a: Action, ctx: &egui::Context) {
        let path_of = |s: &Self, i: u32| s.tree.as_ref().map(|t| t.path(i));
        match a {
            Action::Select(i) => {
                self.selected = Some(i);
                self.ensure_visible(i);
            }
            Action::Toggle(i) => {
                if !self.expanded.remove(&i) {
                    self.expanded.insert(i);
                }
                self.selected = Some(i);
                self.rebuild_rows();
            }
            Action::Reveal(i) => self.reveal(i),
            Action::Zoom(i) => {
                if let Some(t) = &self.tree {
                    if i != NONE && t.node(i).is_dir && t.is_under(i, t.root) {
                        self.view = i;
                    }
                }
            }
            Action::Open(i) => {
                if let Some(p) = path_of(self, i) {
                    win::open(&p);
                }
            }
            Action::Explorer(i) => {
                if let Some(p) = path_of(self, i) {
                    win::reveal(&p);
                }
            }
            Action::CopyPath(i) => {
                if let Some(p) = path_of(self, i) {
                    ctx.copy_text(p);
                }
            }
            Action::AskDelete(i) => {
                if self.tree.as_ref().is_some_and(|t| i != t.root) {
                    self.confirm_delete = Some(i);
                }
            }
            Action::Scan(p) => self.start_scan(p),
            Action::PickFolder => {
                if let Some(dir) = rfd::FileDialog::new().set_title("Choose a folder to scan").pick_folder() {
                    self.start_scan(dir.to_string_lossy().into_owned());
                }
            }
            Action::Cancel => {
                if let Some(j) = self.job.take() {
                    j.cancel.store(true, Ordering::Relaxed);
                }
            }
            Action::Elevate => {
                let path = self.last.as_ref().map(|l| l.path.clone()).or_else(|| self.job.as_ref().map(|j| j.path.clone()));
                if win::relaunch_as_admin(path.as_deref()) {
                    ctx.send_viewport_cmd(ViewportCommand::Close);
                }
            }
        }
    }

    fn current_path(&self) -> Option<&str> {
        self.job.as_ref().map(|j| j.path.as_str()).or(self.last.as_ref().map(|l| l.path.as_str()))
    }

    // ---------------------------------------------------------------- keyboard

    fn keyboard(&mut self, ctx: &egui::Context, acts: &mut Vec<Action>) {
        if ctx.input(|i| i.modifiers.command && i.key_pressed(Key::F)) {
            ctx.memory_mut(|m| m.request_focus(Id::new(SEARCH_ID)));
        }
        if ctx.wants_keyboard_input() || self.confirm_delete.is_some() || self.message.is_some() {
            return;
        }
        let Some(t) = &self.tree else { return };
        if self.rows.is_empty() {
            return;
        }
        let k = |key: Key| ctx.input(|i| i.key_pressed(key));
        let n = self.rows.len();
        let cur = self.selected.and_then(|s| self.rows.iter().position(|r| r.0 == s)).unwrap_or(0);
        let target = if k(Key::ArrowDown) {
            Some((cur + 1).min(n - 1))
        } else if k(Key::ArrowUp) {
            Some(cur.saturating_sub(1))
        } else if k(Key::PageDown) {
            Some((cur + 15).min(n - 1))
        } else if k(Key::PageUp) {
            Some(cur.saturating_sub(15))
        } else if k(Key::Home) {
            Some(0)
        } else if k(Key::End) {
            Some(n - 1)
        } else {
            None
        };
        if let Some(ti) = target {
            acts.push(Action::Select(self.rows[ti].0));
            return;
        }
        let Some(sel) = self.selected else { return };
        let node = t.node(sel);
        let searching = self.results.is_some();
        if k(Key::ArrowRight) && node.is_dir && !searching {
            if !self.expanded.contains(&sel) {
                acts.push(Action::Toggle(sel));
            } else if let Some(&c) = node.children.first() {
                acts.push(Action::Select(c));
            }
        } else if k(Key::ArrowLeft) && !searching {
            if self.expanded.contains(&sel) && sel != t.root {
                acts.push(Action::Toggle(sel));
            } else if node.parent != NONE {
                acts.push(Action::Select(node.parent));
            }
        } else if k(Key::Enter) {
            acts.push(Action::Zoom(if node.is_dir { sel } else { node.parent }));
        } else if k(Key::Delete) && sel != t.root {
            acts.push(Action::AskDelete(sel));
        } else if k(Key::Backspace) && self.view != t.root {
            acts.push(Action::Zoom(t.node(self.view).parent));
        }
    }

    // ---------------------------------------------------------------- top bar

    fn top_bar(&mut self, ctx: &egui::Context, acts: &mut Vec<Action>) {
        egui::TopBottomPanel::top("top")
            .exact_height(62.0)
            .show_separator_line(false)
            .frame(egui::Frame::new().fill(BG))
            .show(ctx, |ui| {
                let r = ui.max_rect();
                let p = ui.painter().clone();

                let logo = Rect::from_min_size(pos2(r.left() + 22.0, r.center().y - 15.0), vec2(30.0, 30.0));
                p.rect_filled(logo, CornerRadius::same(9), ACCENT);
                p.text(logo.center(), Align2::CENTER_CENTER, icon::CHART, theme::icons(15.0), Color32::WHITE);
                p.text(pos2(logo.right() + 11.0, r.center().y), Align2::LEFT_CENTER, "Spacelens", theme::semibold(16.5), TEXT);

                // Search box, centred in the window.
                let sw = (r.width() - 820.0).clamp(240.0, 540.0);
                let sr = Rect::from_center_size(r.center(), vec2(sw, 38.0));
                let focused = ctx.memory(|m| m.has_focus(Id::new(SEARCH_ID)));
                p.rect(sr, CornerRadius::same(11), SURFACE2, Stroke::new(1.0_f32, if focused { ACCENT } else { LINE }), StrokeKind::Inside);
                p.text(pos2(sr.left() + 15.0, sr.center().y), Align2::LEFT_CENTER, icon::SEARCH, theme::icons(13.0), FAINT);
                let text_rect = Rect::from_min_max(pos2(sr.left() + 40.0, sr.top() + 5.0), pos2(sr.right() - 96.0, sr.bottom() - 5.0));
                let edit = egui::TextEdit::singleline(&mut self.search)
                    .id(Id::new(SEARCH_ID))
                    .hint_text(RichText::new("Search files and folders").color(FAINT))
                    .frame(false)
                    .font(theme::regular(14.0))
                    .text_color(TEXT)
                    .vertical_align(Align::Center);
                let resp = ui.put(text_rect, edit);
                if resp.has_focus() && ui.input(|i| i.key_pressed(Key::Escape)) {
                    self.search.clear();
                }
                let right_hint = if self.results.is_some() {
                    format!("{} found", fmt::count(self.result_total as u64))
                } else {
                    "Ctrl+F".into()
                };
                kit::text_right(&p, pos2(sr.right() - 14.0, sr.center().y), &right_hint, theme::regular(12.0), FAINT);

                // Right side: Turbo mode + scan controls.
                let right = Rect::from_min_max(pos2(sr.right() + 16.0, r.top()), pos2(r.right() - 18.0, r.bottom()));
                let mut rui = ui.new_child(UiBuilder::new().max_rect(right).layout(Layout::right_to_left(Align::Center)));
                rui.spacing_mut().item_spacing.x = 10.0;
                if self.job.is_some() {
                    if kit::button(&mut rui, Kind::Ghost, Some(icon::STOP), "Stop", 92.0).clicked() {
                        acts.push(Action::Cancel);
                    }
                } else if let Some(l) = &self.last {
                    if kit::button(&mut rui, Kind::Accent, Some(icon::REFRESH), "Rescan", 104.0).clicked() {
                        acts.push(Action::Scan(l.path.clone()));
                    }
                }
                if self.is_admin {
                    let (b, resp) = rui.allocate_exact_size(vec2(122.0, 30.0), Sense::hover());
                    let bp = rui.painter();
                    bp.rect(b, CornerRadius::same(15), Color32::from_rgb(42, 35, 12), Stroke::new(1.0_f32, Color32::from_rgb(92, 74, 22)), StrokeKind::Inside);
                    bp.text(pos2(b.left() + 14.0, b.center().y), Align2::LEFT_CENTER, icon::BOLT, theme::icons(12.0), AMBER);
                    bp.text(pos2(b.left() + 32.0, b.center().y), Align2::LEFT_CENTER, "Turbo mode", theme::semibold(12.5), AMBER);
                    resp.on_hover_text("Running as administrator: NTFS drives are scanned by reading the Master File Table directly.");
                } else if kit::button(&mut rui, Kind::Ghost, Some(icon::BOLT), "Turbo mode", 0.0)
                    .on_hover_text("Restart as administrator so NTFS drives can be scanned by reading the Master File Table directly. Usually many times faster.")
                    .clicked()
                {
                    acts.push(Action::Elevate);
                }
            });
    }

    // ---------------------------------------------------------------- status bar

    fn status_bar(&mut self, ctx: &egui::Context) {
        egui::TopBottomPanel::bottom("status")
            .exact_height(30.0)
            .show_separator_line(false)
            .frame(egui::Frame::new().fill(BG))
            .show(ctx, |ui| {
                let r = ui.max_rect();
                let p = ui.painter();
                let left = match (self.hover, &self.tree) {
                    (Some(h), Some(t)) => format!("{}   ·   {}", t.path(h), fmt::size(t.node(h).size)),
                    _ => match (&self.last, &self.job) {
                        (_, Some(_)) => "Scanning…".into(),
                        (Some(l), None) => match &l.note {
                            Some(n) => n.clone(),
                            None => format!("Scanned {} in {:.1}s", l.path, l.secs),
                        },
                        _ => "Ready".into(),
                    },
                };
                kit::text_clip(p, pos2(r.left() + 24.0, r.center().y), &left, theme::regular(12.0), FAINT, r.width() - 320.0);
                let right = if self.is_admin { "Administrator  ·  MFT scanning on" } else { "Standard user  ·  folder scanning" };
                kit::text_right(p, pos2(r.right() - 24.0, r.center().y), &format!("{right}   ·   v{}", env!("CARGO_PKG_VERSION")), theme::regular(12.0), FAINT);
            });
    }

    // ---------------------------------------------------------------- sidebar

    fn sidebar(&mut self, ctx: &egui::Context, acts: &mut Vec<Action>) {
        egui::SidePanel::left("side")
            .exact_width(304.0)
            .resizable(false)
            .show_separator_line(false)
            .frame(egui::Frame::new().fill(BG).inner_margin(egui::Margin { left: 12, right: 0, top: 0, bottom: 2 }))
            .show(ctx, |ui| {
                let rect = ui.max_rect();
                kit::card(ui.painter(), rect);
                let inner = rect.shrink2(vec2(14.0, 16.0));
                let mut cui = ui.new_child(UiBuilder::new().max_rect(inner).layout(Layout::top_down(Align::Min)));
                egui::ScrollArea::vertical()
                    .id_salt("sidebar")
                    .auto_shrink([false, false])
                    .show(&mut cui, |ui| self.sidebar_contents(ui, acts));
            });
    }

    fn sidebar_contents(&mut self, ui: &mut Ui, acts: &mut Vec<Action>) {
        ui.spacing_mut().item_spacing = Vec2::ZERO;
        let w = ui.available_width();
        section(ui, w, "LOCATIONS");
        let current = self.current_path().map(|s| s.to_ascii_uppercase());
        for d in &self.drives {
            let (r, resp) = ui.allocate_exact_size(vec2(w, 74.0), Sense::click());
            let active = current.as_deref().is_some_and(|p| p.starts_with(&d.root));
            let hot = resp.hovered();
            let p = ui.painter();
            let fill = if active { ACCENT_SOFT } else if hot { SURFACE3 } else { SURFACE2 };
            let stroke = if active { Stroke::new(1.0_f32, Color32::from_rgb(84, 74, 170)) } else { Stroke::NONE };
            p.rect(r, CornerRadius::same(12), fill, stroke, StrokeKind::Inside);
            let ib = Rect::from_min_size(pos2(r.left() + 12.0, r.top() + 12.0), vec2(36.0, 36.0));
            p.rect_filled(ib, CornerRadius::same(10), Color32::from_rgb(40, 44, 70));
            let glyph = match d.kind {
                drives::Kind::Fixed => icon::DRIVE,
                drives::Kind::Removable => icon::USB,
                drives::Kind::Network => icon::NETWORK,
                drives::Kind::Optical => icon::DISC,
            };
            p.text(ib.center(), Align2::CENTER_CENTER, glyph, theme::icons(16.0), theme::FOLDER);
            kit::text_clip(p, pos2(r.left() + 60.0, r.top() + 22.0), &format!("{} ({}:)", d.label, d.letter), theme::semibold(13.5), TEXT, w - 110.0);
            kit::text_right(p, pos2(r.right() - 12.0, r.top() + 22.0), &d.fs, theme::regular(11.0), FAINT);
            kit::text_clip(p, pos2(r.left() + 60.0, r.top() + 41.0), &format!("{} free of {}", fmt::size(d.free), fmt::size(d.total)), theme::regular(12.0), MUTED, w - 72.0);
            let used = if d.total > 0 { 1.0 - d.free as f32 / d.total as f32 } else { 0.0 };
            let bar = Rect::from_min_max(pos2(r.left() + 12.0, r.bottom() - 15.0), pos2(r.right() - 12.0, r.bottom() - 10.0));
            kit::bar(p, bar, used, if used > 0.9 { DANGER } else { ACCENT });
            if resp.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
                acts.push(Action::Scan(d.root.clone()));
            }
            ui.add_space(8.0);
        }
        if kit::button(ui, Kind::Ghost, Some(icon::FOLDER_OPEN), "Scan a folder…", w).clicked() {
            acts.push(Action::PickFolder);
        }

        ui.add_space(20.0);
        let (line, _) = ui.allocate_exact_size(vec2(w, 1.0), Sense::hover());
        ui.painter().rect_filled(line, CornerRadius::ZERO, LINE);
        ui.add_space(18.0);
        section(ui, w, "FILE TYPES");

        let Some(t) = &self.tree else {
            ui.add(egui::Label::new(RichText::new("Scan a drive to see which kinds of files take up the most space.").color(FAINT).size(12.5)).wrap());
            return;
        };
        let total = t.cats.iter().map(|c| c.size).sum::<u64>().max(1);
        let mut cats: Vec<(Category, u64, u32)> = categories::ALL
            .iter()
            .map(|&c| (c, t.cats[c as usize].size, t.cats[c as usize].count))
            .filter(|c| c.1 > 0)
            .collect();
        cats.sort_by(|a, b| b.1.cmp(&a.1));
        for (c, size, count) in cats {
            let (r, resp) = ui.allocate_exact_size(vec2(w, 42.0), Sense::hover());
            let p = ui.painter();
            let color = theme::category_color(c);
            p.circle_filled(pos2(r.left() + 5.0, r.top() + 12.0), 4.5, color);
            kit::text_clip(p, pos2(r.left() + 18.0, r.top() + 12.0), c.name(), theme::regular(13.0), TEXT, w - 110.0);
            kit::text_right(p, pos2(r.right(), r.top() + 12.0), &fmt::size(size), theme::semibold(13.0), TEXT);
            let bar = Rect::from_min_max(pos2(r.left(), r.top() + 26.0), pos2(r.right(), r.top() + 30.0));
            kit::bar(p, bar, (size as f64 / total as f64) as f32, color);
            resp.on_hover_text(format!("{} files  ·  {} of all files", fmt::count(count as u64), fmt::percent(size as f64 / total as f64)));
        }

        ui.add_space(10.0);
        section(ui, w, "TOP EXTENSIONS");
        for e in t.exts.iter().take(10) {
            let (r, _) = ui.allocate_exact_size(vec2(w, 26.0), Sense::hover());
            let p = ui.painter();
            let cy = r.center().y;
            p.circle_filled(pos2(r.left() + 5.0, cy), 3.5, theme::category_color(e.cat));
            let label = if e.ext.is_empty() { "(none)".to_string() } else { format!(".{}", e.ext) };
            kit::text_clip(p, pos2(r.left() + 18.0, cy), &label, theme::semibold(12.5), TEXT, 92.0);
            kit::text_clip(p, pos2(r.left() + 116.0, cy), &format!("{} files", fmt::count(e.count as u64)), theme::regular(12.0), FAINT, w - 200.0);
            kit::text_right(p, pos2(r.right(), cy), &fmt::size(e.size), theme::regular(12.5), MUTED);
        }
    }

    // ---------------------------------------------------------------- central area

    fn central(&mut self, ctx: &egui::Context, acts: &mut Vec<Action>) {
        egui::CentralPanel::default()
            .frame(egui::Frame::new().fill(BG).inner_margin(egui::Margin { left: 12, right: 12, top: 0, bottom: 2 }))
            .show(ctx, |ui| {
                let full = ui.max_rect();
                let summary = Rect::from_min_size(full.min, vec2(full.width(), 98.0));
                self.summary(ui, summary);
                let body = Rect::from_min_max(pos2(full.left(), summary.bottom() + 12.0), full.max);
                if self.job.is_some() || self.tree.is_none() {
                    self.empty_state(ui, body, acts);
                    return;
                }
                let split_y = (body.top() + body.height() * self.split).round();
                let handle = Rect::from_min_max(pos2(body.left(), split_y - 6.0), pos2(body.right(), split_y + 6.0));
                let hr = ui.interact(handle, Id::new("split"), Sense::drag());
                if hr.dragged() {
                    if let Some(p) = hr.interact_pointer_pos() {
                        self.split = ((p.y - body.top()) / body.height()).clamp(0.2, 0.8);
                    }
                }
                let hot = hr.hovered() || hr.dragged();
                if hot {
                    ctx.set_cursor_icon(egui::CursorIcon::ResizeVertical);
                }
                ui.painter().rect_filled(Rect::from_center_size(handle.center(), vec2(44.0, 4.0)), CornerRadius::same(2), if hot { ACCENT } else { LINE });

                self.tree_panel(ui, Rect::from_min_max(body.min, pos2(body.right(), split_y - 6.0)), acts);
                self.map_panel(ui, Rect::from_min_max(pos2(body.left(), split_y + 6.0), body.max), acts);
            });
    }

    fn summary(&self, ui: &mut Ui, r: Rect) {
        let p = ui.painter();
        kit::card(p, r);
        let left = r.left() + 24.0;

        let mut stats: Vec<(&str, String)> = Vec::new();
        if let (Some(t), None) = (&self.tree, &self.job) {
            let root = t.node(t.root);
            stats.push(("TOTAL SIZE", fmt::size(root.size)));
            stats.push(("ON DISK", fmt::size(root.alloc)));
            stats.push(("FILES", fmt::count(root.files as u64)));
            stats.push(("FOLDERS", fmt::count(root.folders as u64)));
            let root_path = root.name.to_ascii_uppercase();
            if let Some(d) = self.drives.iter().find(|d| root_path.starts_with(&d.root)) {
                stats.push(("FREE ON DRIVE", fmt::size(d.free)));
            }
        }
        let col_w = 120.0;
        let max_cols = ((r.width() - 420.0) / col_w).floor().max(0.0) as usize;
        stats.truncate(max_cols);
        let stats_w = stats.len() as f32 * col_w;
        let mut x = r.right() - 16.0 - stats_w;
        for (label, value) in &stats {
            kit::caption(p, pos2(x, r.top() + 36.0), label);
            p.text(pos2(x, r.top() + 62.0), Align2::LEFT_CENTER, value, theme::semibold(19.0), TEXT);
            x += col_w;
        }

        let title_w = r.width() - stats_w - 64.0;
        let (title, sub) = if let Some(j) = &self.job {
            (format!("Scanning {}", j.path), Self::progress_text(j))
        } else if let (Some(t), Some(l)) = (&self.tree, &self.last) {
            let mut s = format!("Scanned in {:.1}s", l.secs);
            if let Some(n) = &l.note {
                s.push_str("  ·  ");
                s.push_str(n);
            }
            (t.path(t.root), s)
        } else {
            ("Welcome to Spacelens".into(), "Pick a drive or folder to see what is using your space.".into())
        };
        kit::text_clip(p, pos2(left, r.top() + 36.0), &title, theme::semibold(21.0), TEXT, title_w);

        let mut sx = left;
        if let (None, Some(l), Some(_)) = (&self.job, &self.last, &self.tree) {
            let (glyph, label, fg, bg) = if l.used_mft {
                (icon::BOLT, "MFT direct read", AMBER, Color32::from_rgb(42, 35, 12))
            } else {
                (icon::FOLDER, "Folder scan", MUTED, SURFACE3)
            };
            let g = p.layout_no_wrap(label.into(), theme::semibold(11.5), fg);
            let badge = Rect::from_min_size(pos2(sx, r.top() + 52.0), vec2(g.size().x + 34.0, 22.0));
            p.rect_filled(badge, CornerRadius::same(11), bg);
            p.text(pos2(badge.left() + 10.0, badge.center().y), Align2::LEFT_CENTER, glyph, theme::icons(10.5), fg);
            p.galley(pos2(badge.left() + 25.0, badge.center().y - g.size().y / 2.0), g, fg);
            sx = badge.right() + 12.0;
        }
        kit::text_clip(p, pos2(sx, r.top() + 63.0), &sub, theme::regular(12.5), MUTED, title_w - (sx - left));

        if let Some(j) = &self.job {
            let bar = Rect::from_min_size(pos2(left, r.top() + 82.0), vec2(title_w.min(420.0), 4.0));
            match Self::progress_fraction(j) {
                Some(f) => kit::bar(p, bar, f, ACCENT),
                None => kit::busy_bar(p, bar, ui.input(|i| i.time)),
            }
        }
    }

    fn empty_state(&mut self, ui: &mut Ui, r: Rect, acts: &mut Vec<Action>) {
        let p = ui.painter().clone();
        kit::card(&p, r);
        let c = r.center();
        let tile = Rect::from_center_size(pos2(c.x, c.y - 78.0), vec2(80.0, 80.0));
        p.rect_filled(tile, CornerRadius::same(24), ACCENT);
        let glyph = if self.job.is_some() { icon::SEARCH } else { icon::CHART };
        p.text(tile.center(), Align2::CENTER_CENTER, glyph, theme::icons(32.0), Color32::WHITE);

        let (title, body) = match &self.job {
            Some(j) => (format!("Scanning {}", j.path), Self::progress_text(j)),
            None => (
                "See where your space went".to_string(),
                "Choose a drive or folder on the left. Spacelens maps every file into an interactive treemap so the biggest space hogs stand out instantly.".to_string(),
            ),
        };
        p.text(pos2(c.x, c.y + 2.0), Align2::CENTER_CENTER, title, theme::semibold(22.0), TEXT);
        let g = p.layout(body, theme::regular(13.5), MUTED, 470.0);
        let gpos = pos2(c.x - g.size().x / 2.0, c.y + 24.0);
        let below = gpos.y + g.size().y;
        // Centre each wrapped line.
        p.add(egui::epaint::TextShape::new(gpos, g.clone(), MUTED));
        let _ = g;

        let area = Rect::from_center_size(pos2(c.x, below + 44.0), vec2(360.0, 44.0));
        if let Some(j) = &self.job {
            let bar = Rect::from_center_size(pos2(c.x, below + 22.0), vec2(360.0, 5.0));
            match Self::progress_fraction(j) {
                Some(f) => kit::bar(&p, bar, f, ACCENT),
                None => kit::busy_bar(&p, bar, ui.input(|i| i.time)),
            }
            let mut b = ui.new_child(UiBuilder::new().max_rect(area.translate(vec2(0.0, 20.0))).layout(Layout::top_down(Align::Center)));
            if kit::button(&mut b, Kind::Ghost, Some(icon::STOP), "Stop scan", 140.0).clicked() {
                acts.push(Action::Cancel);
            }
        } else if let Some(d) = self.drives.iter().find(|d| d.letter == 'C').or(self.drives.first()) {
            let mut b = ui.new_child(UiBuilder::new().max_rect(area).layout(Layout::top_down(Align::Center)));
            if kit::button(&mut b, Kind::Accent, Some(icon::DRIVE), &format!("Scan {} ({}:)", d.label, d.letter), 200.0).clicked() {
                acts.push(Action::Scan(d.root.clone()));
            }
        }
    }

    // ---------------------------------------------------------------- folder table

    fn tree_panel(&mut self, ui: &mut Ui, r: Rect, acts: &mut Vec<Action>) {
        let p = ui.painter().clone();
        kit::card(&p, r);
        let Some(t) = &self.tree else { return };
        let cols = Cols::new(r.left() + 16.0, r.right() - 18.0);
        let hy = r.top() + 20.0;
        let name_caption = match &self.results {
            Some(_) => format!("SEARCH RESULTS FOR \u{201C}{}\u{201D}", self.applied_search.trim()),
            None => "NAME".into(),
        };
        kit::caption(&p, pos2(cols.name.0 + 2.0, hy), &name_caption);
        let right_caption = |x: Option<f32>, s: &str| {
            if let Some(x) = x {
                p.text(pos2(x, hy), Align2::RIGHT_CENTER, s, theme::semibold(11.0), FAINT);
            }
        };
        p.text(pos2(cols.pct.0, hy), Align2::LEFT_CENTER, if self.results.is_some() { "% OF SCAN" } else { "% OF PARENT" }, theme::semibold(11.0), FAINT);
        right_caption(Some(cols.size), "SIZE");
        right_caption(cols.alloc, "ON DISK");
        right_caption(cols.files, "FILES");
        right_caption(cols.folders, "FOLDERS");
        right_caption(cols.modified, "MODIFIED");
        p.rect_filled(Rect::from_min_max(pos2(r.left() + 1.0, r.top() + 38.0), pos2(r.right() - 1.0, r.top() + 39.0)), CornerRadius::ZERO, LINE);

        let list = Rect::from_min_max(pos2(r.left() + 6.0, r.top() + 42.0), pos2(r.right() - 6.0, r.bottom() - 6.0));
        if list.height() < ROW_H {
            return;
        }
        let mut cui = ui.new_child(UiBuilder::new().max_rect(list).layout(Layout::top_down(Align::Min)));
        cui.set_clip_rect(list);
        cui.spacing_mut().item_spacing = Vec2::ZERO;
        let mut sa = egui::ScrollArea::vertical().id_salt("tree").auto_shrink([false, false]);
        if let Some(i) = self.scroll_to.take() {
            sa = sa.vertical_scroll_offset((i as f32 * ROW_H - list.height() * 0.4).max(0.0));
        }
        let (rows, expanded, selected, searching) = (&self.rows, &self.expanded, self.selected, self.results.is_some());
        let root_size = t.node(t.root).size;
        let mut visible = 0..0;
        sa.show_rows(&mut cui, ROW_H, rows.len(), |ui, range| {
            visible = range.clone();
            for ri in range {
                let (i, depth) = rows[ri];
                row(ui, t, i, depth, selected == Some(i), expanded.contains(&i), searching, root_size, &cols, acts);
            }
        });
        self.visible = visible;
    }

    // ---------------------------------------------------------------- treemap

    fn map_panel(&mut self, ui: &mut Ui, r: Rect, acts: &mut Vec<Action>) {
        let p = ui.painter().clone();
        kit::card(&p, r);
        let Some(t) = &self.tree else { return };
        if self.view == NONE || !t.is_under(self.view, t.root) {
            self.view = t.root;
        }

        let bar = Rect::from_min_size(r.min, vec2(r.width(), 44.0));
        kit::text_right(&p, pos2(bar.right() - 16.0, bar.center().y), "Click to select  ·  Double-click to zoom  ·  Right-click for actions", theme::regular(11.5), FAINT);
        let mut tb = ui.new_child(UiBuilder::new().max_rect(bar.shrink2(vec2(10.0, 7.0))).layout(Layout::left_to_right(Align::Center)));
        tb.spacing_mut().item_spacing.x = 4.0;
        let can_up = self.view != t.root;
        if kit::icon_button(&mut tb, icon::UP, can_up).on_hover_text("Up one level (Backspace)").clicked() && can_up {
            acts.push(Action::Zoom(t.node(self.view).parent));
        }
        tb.add_space(6.0);
        let mut chain = Vec::new();
        let mut n = self.view;
        while n != NONE {
            chain.push(n);
            if n == t.root {
                break;
            }
            n = t.node(n).parent;
        }
        chain.reverse();
        for (k, &c) in chain.iter().enumerate() {
            if k > 0 {
                tb.label(RichText::new(icon::CHEVRON_RIGHT.to_string()).font(theme::icons(8.0)).color(FAINT));
            }
            let last = k + 1 == chain.len();
            let name = if c == t.root { t.node(c).name.trim_end_matches('\\').to_string() } else { t.node(c).name.to_string() };
            let text = RichText::new(name).font(if last { theme::semibold(13.0) } else { theme::regular(13.0) });
            if last {
                tb.label(text.color(TEXT));
            } else {
                let resp = tb.add(egui::Label::new(text.color(MUTED)).sense(Sense::click()));
                if resp.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
                    acts.push(Action::Zoom(c));
                }
            }
        }

        let area = Rect::from_min_max(pos2(r.left() + 10.0, bar.bottom()), pos2(r.right() - 10.0, r.bottom() - 10.0));
        if area.width() < 10.0 || area.height() < 10.0 {
            return;
        }
        let ppp = ui.ctx().pixels_per_point();
        let key = (area, self.view, self.generation, ppp);
        if self.map_key != Some(key) {
            self.map = treemap::build(t, self.view, area, ppp);
            self.map_key = Some(key);
        }

        let painter = ui.painter_at(area);
        painter.rect_filled(area, CornerRadius::same(6), Color32::from_rgb(17, 19, 25));
        painter.add(Shape::mesh(self.map.mesh.clone()));
        for l in &self.map.labels {
            let n = t.node(l.node);
            let w = l.rect.width() - 12.0;
            if l.header {
                kit::text_clip(&painter, pos2(l.rect.left() + 6.0, l.rect.center().y), &format!("{}   {}", n.name, fmt::size(n.size)), theme::regular(11.5), Color32::from_rgb(198, 204, 220), w);
            } else {
                let (x, y) = (l.rect.left() + 7.0, l.rect.top() + 12.0);
                kit::text_clip(&painter, pos2(x + 1.0, y + 1.0), &n.name, theme::semibold(12.0), Color32::from_black_alpha(110), w);
                kit::text_clip(&painter, pos2(x, y), &n.name, theme::semibold(12.0), Color32::WHITE, w);
                if l.rect.height() >= 40.0 {
                    kit::text_clip(&painter, pos2(x, y + 16.0), &fmt::size(n.size), theme::regular(11.5), Color32::from_white_alpha(210), w);
                }
            }
        }

        // Selection outline (nearest visible ancestor if the item itself is too small).
        if let Some(s) = self.selected.filter(|&s| t.is_under(s, self.view)) {
            let mut n = s;
            while n != NONE {
                if let Some(sr) = self.map.rect_of(n) {
                    painter.rect(sr, CornerRadius::ZERO, Color32::from_white_alpha(24), Stroke::new(2.0_f32, Color32::WHITE), StrokeKind::Inside);
                    break;
                }
                if n == self.view {
                    break;
                }
                n = t.node(n).parent;
            }
        }

        let resp = ui.interact(area, Id::new("treemap"), Sense::click());
        let hit = resp.hover_pos().and_then(|pp| self.map.hit(pp));
        self.hover = hit;
        if let Some(h) = hit {
            if let Some(hr) = self.map.rect_of(h) {
                painter.rect_stroke(hr, CornerRadius::ZERO, Stroke::new(1.5_f32, Color32::from_white_alpha(190)), StrokeKind::Inside);
            }
        }
        if resp.clicked() {
            if let Some(h) = hit {
                acts.push(Action::Reveal(h));
            }
        }
        if resp.double_clicked() {
            if let Some(h) = hit {
                let d = if t.node(h).is_dir { h } else { t.node(h).parent };
                acts.push(Action::Zoom(d));
            }
        }
        if resp.secondary_clicked() {
            self.menu_node = hit;
        }
        let menu_node = self.menu_node;
        resp.context_menu(|ui| {
            if let Some(m) = menu_node {
                node_menu(ui, t, m, acts);
            }
        });
        let menu_open = ui.ctx().memory(|m| m.any_popup_open());
        if let (Some(h), Some(pp), false) = (hit, resp.hover_pos(), menu_open) {
            tooltip(ui.ctx(), t, h, self.view, pp);
        }
    }

    // ---------------------------------------------------------------- dialogs

    fn dialogs(&mut self, ctx: &egui::Context) {
        if let (Some(i), Some(t)) = (self.confirm_delete, &self.tree) {
            let n = t.node(i);
            let path = t.path(i);
            let details = if n.is_dir {
                format!("{}  ·  {} files  ·  {} folders", fmt::size(n.size), fmt::count(n.files as u64), fmt::count(n.folders as u64))
            } else {
                fmt::size(n.size)
            };
            let (mut confirm, mut cancel) = (false, false);
            let modal = egui::Modal::new(Id::new("confirm-delete")).frame(dialog_frame()).show(ctx, |ui| {
                ui.set_width(440.0);
                dialog_header(ui, icon::DELETE, DANGER, "Move to the Recycle Bin?");
                ui.add_space(12.0);
                ui.add(egui::Label::new(RichText::new(&path).color(TEXT).size(13.0)).wrap());
                ui.add_space(4.0);
                ui.label(RichText::new(details).color(MUTED).size(12.5));
                ui.add_space(6.0);
                ui.label(RichText::new("You can restore it from the Recycle Bin later.").color(FAINT).size(12.0));
                ui.add_space(20.0);
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    ui.spacing_mut().item_spacing.x = 10.0;
                    confirm = kit::button(ui, Kind::Danger, Some(icon::DELETE), "Move to Recycle Bin", 0.0).clicked();
                    cancel = kit::button(ui, Kind::Ghost, None, "Cancel", 96.0).clicked();
                });
            });
            if confirm {
                self.confirm_delete = None;
                self.delete(i);
            } else if cancel || modal.should_close() {
                self.confirm_delete = None;
            }
        }

        if let Some((title, body)) = &self.message {
            let mut close = false;
            let modal = egui::Modal::new(Id::new("message")).frame(dialog_frame()).show(ctx, |ui| {
                ui.set_width(420.0);
                dialog_header(ui, icon::INFO, Color32::from_rgb(147, 197, 253), title);
                ui.add_space(12.0);
                ui.add(egui::Label::new(RichText::new(body).color(MUTED).size(13.0)).wrap());
                ui.add_space(20.0);
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    close = kit::button(ui, Kind::Accent, None, "OK", 96.0).clicked();
                });
            });
            if close || modal.should_close() {
                self.message = None;
            }
        }
    }

    // ---------------------------------------------------------------- snapshot (for testing)

    fn snapshot_step(&mut self, ctx: &egui::Context) {
        let Some(path) = self.snapshot.clone() else { return };
        if self.job.is_some() {
            return;
        }
        self.snapshot_frames += 1;
        ctx.request_repaint();
        if self.snapshot_frames == 12 {
            ctx.send_viewport_cmd(ViewportCommand::Screenshot(Default::default()));
        }
        let shot = ctx.input(|i| {
            i.raw.events.iter().find_map(|e| match e {
                egui::Event::Screenshot { image, .. } => Some(image.clone()),
                _ => None,
            })
        });
        if let Some(img) = shot {
            let _ = save_png(&path, &img);
            ctx.send_viewport_cmd(ViewportCommand::Close);
        } else if self.snapshot_frames > 900 {
            ctx.send_viewport_cmd(ViewportCommand::Close);
        }
    }
}

impl eframe::App for App {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.frames += 1;
        if !self.window_styled && self.frames > 1 {
            self.window_styled = win::style_own_window() || self.frames > 120;
        }
        self.poll_job();
        self.apply_search(ctx);

        let mut acts = Vec::new();
        self.keyboard(ctx, &mut acts);
        self.top_bar(ctx, &mut acts);
        self.status_bar(ctx);
        self.sidebar(ctx, &mut acts);
        self.central(ctx, &mut acts);
        self.dialogs(ctx);
        for a in acts {
            self.apply(a, ctx);
        }
        if self.job.is_some() {
            ctx.request_repaint_after(Duration::from_millis(80));
        }
        self.snapshot_step(ctx);
    }
}

// -------------------------------------------------------------------- helpers

/// Column layout for the folder table; low-priority columns drop out when space is tight.
struct Cols {
    name: (f32, f32),
    pct: (f32, f32),
    size: f32,
    alloc: Option<f32>,
    files: Option<f32>,
    folders: Option<f32>,
    modified: Option<f32>,
}

impl Cols {
    fn new(left: f32, right: f32) -> Self {
        let w = right - left;
        let mut x = right;
        let mut take = |show: bool, width: f32| -> Option<f32> {
            show.then(|| {
                let r = x;
                x -= width;
                r
            })
        };
        let modified = take(w > 1020.0, 136.0);
        let folders = take(w > 900.0, 76.0);
        let files = take(w > 640.0, 86.0);
        let alloc = take(w > 780.0, 92.0);
        let size = take(true, 94.0).unwrap();
        let pct_r = x - 14.0;
        let pct_w = if w > 700.0 { 170.0 } else { 120.0 };
        Cols { name: (left, pct_r - pct_w - 16.0), pct: (pct_r - pct_w, pct_r), size, alloc, files, folders, modified }
    }
}

#[allow(clippy::too_many_arguments)]
fn row(ui: &mut Ui, t: &Tree, i: u32, depth: u16, selected: bool, expanded: bool, searching: bool, root_size: u64, cols: &Cols, acts: &mut Vec<Action>) {
    let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), ROW_H), Sense::click());
    let n = t.node(i);
    let p = ui.painter();
    let bg = rect.shrink2(vec2(0.0, 1.0));
    if selected {
        p.rect_filled(bg, CornerRadius::same(7), SELECT);
    } else if resp.hovered() {
        p.rect_filled(bg, CornerRadius::same(7), HOVER);
    }
    let cy = rect.center().y;
    let mut x = cols.name.0 + depth as f32 * 18.0;
    let has_kids = !searching && n.is_dir && !n.children.is_empty();
    let chevron = Rect::from_center_size(pos2(x + 8.0, cy), vec2(20.0, 22.0));
    if has_kids {
        p.text(chevron.center(), Align2::CENTER_CENTER, if expanded { icon::CHEVRON_DOWN } else { icon::CHEVRON_RIGHT }, theme::icons(9.0), MUTED);
    }
    x += 22.0;
    let (glyph, gc) = if n.is_dir { (icon::FOLDER, theme::FOLDER) } else { (icon::FILE, theme::category_color(n.cat)) };
    p.text(pos2(x, cy), Align2::LEFT_CENTER, glyph, theme::icons(14.0), gc);
    x += 24.0;
    let font = if n.is_dir { theme::semibold(13.0) } else { theme::regular(13.0) };
    let nr = kit::text_clip(p, pos2(x, cy), &n.name, font, TEXT, cols.name.1 - x);
    if searching && n.parent != NONE && nr.right() + 30.0 < cols.name.1 {
        kit::text_clip(p, pos2(nr.right() + 10.0, cy), &t.path(n.parent), theme::regular(12.0), FAINT, cols.name.1 - nr.right() - 10.0);
    }

    let denom = if searching || n.parent == NONE { root_size } else { t.node(n.parent).size };
    let frac = if denom > 0 { n.size as f64 / denom as f64 } else { 0.0 };
    kit::bar(p, Rect::from_min_max(pos2(cols.pct.0, cy - 3.0), pos2(cols.pct.1 - 54.0, cy + 3.0)), frac as f32, ACCENT);
    kit::text_right(p, pos2(cols.pct.1, cy), &fmt::percent(frac), theme::regular(12.0), MUTED);
    kit::text_right(p, pos2(cols.size, cy), &fmt::size(n.size), theme::semibold(13.0), TEXT);
    let muted = |x: Option<f32>, s: String| {
        if let Some(x) = x {
            kit::text_right(p, pos2(x, cy), &s, theme::regular(12.5), MUTED);
        }
    };
    muted(cols.alloc, fmt::size(n.alloc));
    if n.is_dir {
        muted(cols.files, fmt::count(n.files as u64));
        muted(cols.folders, fmt::count(n.folders as u64));
    }
    muted(cols.modified, win::format_filetime(n.modified));

    let on_chevron = has_kids && resp.interact_pointer_pos().is_some_and(|pp| chevron.contains(pp));
    if resp.clicked() {
        acts.push(if on_chevron { Action::Toggle(i) } else { Action::Select(i) });
    }
    if resp.double_clicked() && !on_chevron {
        if searching {
            acts.push(Action::Reveal(i));
        } else if has_kids {
            acts.push(Action::Toggle(i));
        } else if !n.is_dir {
            acts.push(Action::Open(i));
        }
    }
    if resp.secondary_clicked() {
        acts.push(Action::Select(i));
    }
    resp.context_menu(|ui| node_menu(ui, t, i, acts));
}

fn node_menu(ui: &mut Ui, t: &Tree, i: u32, acts: &mut Vec<Action>) {
    ui.set_min_width(230.0);
    let n = t.node(i);
    ui.add(egui::Label::new(RichText::new(format!("{}  ·  {}", n.name, fmt::size(n.size))).font(theme::semibold(12.5)).color(MUTED)).truncate());
    ui.separator();
    let mut item = |ui: &mut Ui, glyph: char, label: &str, color: Color32, act: Action| {
        if ui.button(RichText::new(format!("{glyph}    {label}")).color(color)).clicked() {
            acts.push(act);
            ui.close_menu();
        }
    };
    item(ui, icon::OPEN, if n.is_dir { "Open folder" } else { "Open" }, TEXT, Action::Open(i));
    item(ui, icon::FOLDER_OPEN, "Show in File Explorer", TEXT, Action::Explorer(i));
    item(ui, icon::COPY, "Copy path", TEXT, Action::CopyPath(i));
    if n.is_dir {
        item(ui, icon::ZOOM, "Zoom treemap here", TEXT, Action::Zoom(i));
    }
    if n.parent != NONE {
        ui.separator();
        item(ui, icon::DELETE, "Move to Recycle Bin…", DANGER_HOVER, Action::AskDelete(i));
    }
}

fn tooltip(ctx: &egui::Context, t: &Tree, h: u32, view: u32, pointer: Pos2) {
    let n = t.node(h);
    let p = ctx.layer_painter(LayerId::new(Order::Tooltip, Id::new("treemap-tip")));
    let view_size = t.node(view).size.max(1);
    let pct = fmt::percent(n.size as f64 / view_size as f64);
    let line2 = if n.is_dir {
        format!("{}  ·  {} of view  ·  {} files", fmt::size(n.size), pct, fmt::count(n.files as u64))
    } else {
        format!("{}  ·  {} of view  ·  {}", fmt::size(n.size), pct, n.cat.name())
    };
    let clip = |text: String, font, color, w: f32| {
        let mut job = egui::text::LayoutJob::simple_singleline(text, font, color);
        job.wrap = egui::text::TextWrapping::truncate_at_width(w);
        p.layout_job(job)
    };
    let g1 = clip(n.name.to_string(), theme::semibold(13.5), TEXT, 400.0);
    let g2 = clip(line2, theme::regular(12.5), MUTED, 420.0);
    let g3 = clip(t.path(h), theme::regular(11.5), FAINT, 420.0);
    let w = (g1.size().x + 16.0).max(g2.size().x).max(g3.size().x) + 30.0;
    let hgt = 14.0 + g1.size().y + 5.0 + g2.size().y + 4.0 + g3.size().y + 14.0;
    let screen = ctx.screen_rect();
    let mut pos = pointer + vec2(18.0, 20.0);
    if pos.x + w > screen.right() - 8.0 {
        pos.x = pointer.x - w - 14.0;
    }
    if pos.y + hgt > screen.bottom() - 8.0 {
        pos.y = pointer.y - hgt - 14.0;
    }
    let rect = Rect::from_min_size(pos, vec2(w, hgt));
    let shadow = Shadow { offset: [0, 8], blur: 26, spread: 0, color: Color32::from_black_alpha(150) };
    p.add(shadow.as_shape(rect, CornerRadius::same(12)));
    p.rect(rect, CornerRadius::same(12), Color32::from_rgb(25, 28, 38), Stroke::new(1.0_f32, Color32::from_rgb(54, 60, 78)), StrokeKind::Inside);
    let color = if n.is_dir { theme::FOLDER } else { theme::category_color(n.cat) };
    let mut y = rect.top() + 14.0;
    p.circle_filled(pos2(rect.left() + 19.0, y + g1.size().y / 2.0), 4.5, color);
    let h1 = g1.size().y;
    p.galley(pos2(rect.left() + 31.0, y), g1, TEXT);
    y += h1 + 5.0;
    let h2 = g2.size().y;
    p.galley(pos2(rect.left() + 15.0, y), g2, MUTED);
    y += h2 + 4.0;
    p.galley(pos2(rect.left() + 15.0, y), g3, FAINT);
}

fn section(ui: &mut Ui, w: f32, text: &str) {
    let (r, _) = ui.allocate_exact_size(vec2(w, 18.0), Sense::hover());
    kit::caption(ui.painter(), pos2(r.left() + 2.0, r.center().y), text);
    ui.add_space(10.0);
}

fn dialog_frame() -> egui::Frame {
    egui::Frame::new()
        .fill(SURFACE2)
        .stroke(Stroke::new(1.0_f32, Color32::from_rgb(48, 54, 70)))
        .corner_radius(CornerRadius::same(16))
        .inner_margin(egui::Margin::same(24))
}

fn dialog_header(ui: &mut Ui, glyph: char, color: Color32, title: &str) {
    ui.horizontal(|ui| {
        let (r, _) = ui.allocate_exact_size(vec2(38.0, 38.0), Sense::hover());
        ui.painter().rect_filled(r, CornerRadius::same(11), color.gamma_multiply(0.18));
        ui.painter().text(r.center(), Align2::CENTER_CENTER, glyph, theme::icons(16.0), color);
        ui.add_space(8.0);
        ui.label(RichText::new(title).font(theme::semibold(17.0)).color(TEXT));
    });
}

/// ASCII case-insensitive substring search (falls back to full lowercasing for non-ASCII).
fn contains_ci(hay: &str, needle_lower: &str) -> bool {
    if !needle_lower.is_ascii() {
        return hay.to_lowercase().contains(needle_lower);
    }
    let (h, n) = (hay.as_bytes(), needle_lower.as_bytes());
    if n.len() > h.len() {
        return false;
    }
    h.windows(n.len()).any(|w| w.iter().zip(n).all(|(a, b)| a.to_ascii_lowercase() == *b))
}

fn save_png(path: &std::path::Path, img: &egui::ColorImage) -> std::io::Result<()> {
    let file = std::io::BufWriter::new(std::fs::File::create(path)?);
    let mut enc = png::Encoder::new(file, img.size[0] as u32, img.size[1] as u32);
    enc.set_color(png::ColorType::Rgba);
    enc.set_depth(png::BitDepth::Eight);
    let mut w = enc.write_header().map_err(std::io::Error::other)?;
    w.write_image_data(img.as_raw()).map_err(std::io::Error::other)?;
    Ok(())
}
