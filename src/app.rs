use crate::drives::{self, DriveInfo};
use crate::fmt::{bytes, count};
use crate::scan::{self, Progress, Tree};
use eframe::egui::{self, Align, Color32, Layout, RichText, Sense, Vec2};
use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::atomic::Ordering::Relaxed;
use std::sync::{mpsc, Arc};
use std::time::{Duration, Instant};

const ROW_H: f32 = 20.0;
const W_BAR: f32 = 70.0;
const W_SIZE: f32 = 78.0;
const W_PCT: f32 = 50.0;
const W_FILES: f32 = 80.0;
const INDENT: f32 = 20.0;

/// One visible line of the folder tree.
#[derive(Clone, Copy)]
struct Row {
    idx: u32,
    depth: u16,
    /// Last child of its parent (draws └ instead of ├).
    last: bool,
    /// Bit k set = draw a continuing │ guide in indent column k.
    guides: u64,
}

const GUIDE_COLORS: [Color32; 6] = [
    Color32::from_rgb(90, 150, 230),
    Color32::from_rgb(110, 190, 110),
    Color32::from_rgb(230, 170, 70),
    Color32::from_rgb(180, 120, 220),
    Color32::from_rgb(70, 190, 190),
    Color32::from_rgb(230, 110, 150),
];

#[derive(PartialEq, Clone, Copy)]
enum Tab {
    Tree,
    Files,
    Folders,
}

#[derive(PartialEq, Clone, Copy)]
enum DelMode {
    Recycle,
    Permanent,
}

struct Running {
    progress: Arc<Progress>,
    rx: mpsc::Receiver<Tree>,
    started: Instant,
    path: String,
}

type DelResult = Vec<(u32, PathBuf, Result<(), String>)>;

enum Action {
    Toggle(u32),
    Check(u32, bool),
    Reveal(u32),
    ShowInExplorer(u32),
    OpenFolder(u32),
    CopyPath(u32),
    ScanHere(u32),
}

pub struct App {
    drives: Vec<DriveInfo>,
    path_input: String,
    scan: Option<Running>,
    tree: Option<Tree>,
    tab: Tab,
    expanded: HashSet<u32>,
    visible: Vec<Row>,
    visible_dirty: bool,
    files_view: Vec<u32>,
    dirs_view: Vec<u32>,
    lists_dirty: bool,
    checked: HashSet<u32>,
    scroll_to: Option<u32>,
    confirm: Option<DelMode>,
    deleting: Option<mpsc::Receiver<DelResult>>,
    message: String,
    shot_frames: u32,
}

impl App {
    pub fn new(_cc: &eframe::CreationContext<'_>, start: Option<String>) -> Self {
        let drives = drives::list();
        let mut app = App {
            path_input: drives.first().map(|d| d.root.clone()).unwrap_or_default(),
            drives,
            scan: None,
            tree: None,
            tab: Tab::Tree,
            expanded: HashSet::new(),
            visible: Vec::new(),
            visible_dirty: true,
            files_view: Vec::new(),
            dirs_view: Vec::new(),
            lists_dirty: true,
            checked: HashSet::new(),
            scroll_to: None,
            confirm: None,
            deleting: None,
            message: String::new(),
            shot_frames: 0,
        };
        if let Some(p) = start {
            app.start_scan(p);
        }
        app
    }

    fn start_scan(&mut self, path: String) {
        if self.scan.is_some() || self.deleting.is_some() {
            return;
        }
        let mut p = path.trim().trim_matches('"').to_string();
        if p.len() == 2 && p.ends_with(':') {
            p.push('\\');
        }
        let pb = PathBuf::from(&p);
        if !pb.is_dir() {
            self.message = format!("Not a folder: {p}");
            return;
        }
        self.path_input = p.clone();
        self.tree = None;
        self.checked.clear();
        self.expanded.clear();
        self.visible.clear();
        self.message.clear();
        let progress = Arc::new(Progress::default());
        let (tx, rx) = mpsc::channel();
        let pr = progress.clone();
        std::thread::spawn(move || {
            let _ = tx.send(scan::run(pb, &pr));
        });
        self.scan = Some(Running { progress, rx, started: Instant::now(), path: p });
    }

    fn poll(&mut self, ctx: &egui::Context) {
        if let Some(run) = &self.scan {
            match run.rx.try_recv() {
                Ok(tree) => {
                    self.message = format!(
                        "{}Scanned {} files in {:.1}s{}",
                        if tree.cancelled { "Cancelled — partial results. " } else { "" },
                        count(tree.nodes[0].files),
                        tree.elapsed.as_secs_f64(),
                        if tree.errors > 0 {
                            format!(" · {} folders not accessible (run as administrator to include them)", count(tree.errors))
                        } else {
                            String::new()
                        }
                    );
                    // Open the root and follow the biggest folder down a few levels.
                    let mut cur = 0u32;
                    self.expanded.insert(cur);
                    for _ in 0..3 {
                        let Some(big) = tree.children(cur).find(|&c| tree.node(c).is_dir) else { break };
                        if tree.node(big).child_len == 0 {
                            break;
                        }
                        self.expanded.insert(big);
                        cur = big;
                    }
                    self.tree = Some(tree);
                    self.visible_dirty = true;
                    self.lists_dirty = true;
                    self.scan = None;
                }
                Err(mpsc::TryRecvError::Empty) => ctx.request_repaint_after(Duration::from_millis(100)),
                Err(mpsc::TryRecvError::Disconnected) => {
                    self.message = "Scan failed".into();
                    self.scan = None;
                }
            }
        }
        if let Some(rx) = &self.deleting {
            match rx.try_recv() {
                Ok(results) => {
                    self.deleting = None;
                    self.apply_delete_results(results);
                }
                Err(mpsc::TryRecvError::Empty) => ctx.request_repaint_after(Duration::from_millis(100)),
                Err(mpsc::TryRecvError::Disconnected) => {
                    self.deleting = None;
                    self.message = "Delete failed".into();
                }
            }
        }
    }

    fn apply_delete_results(&mut self, results: DelResult) {
        let Some(tree) = &mut self.tree else { return };
        let mut freed = 0u64;
        let mut ok = 0usize;
        let mut errs = Vec::new();
        for (i, path, r) in results {
            match r {
                Ok(()) => {
                    freed += tree.node(i).size;
                    tree.remove(i);
                    self.checked.remove(&i);
                    ok += 1;
                }
                Err(e) => errs.push(format!("{}: {e}", path.display())),
            }
        }
        self.checked.retain(|&i| tree.alive(i));
        self.message = format!("Deleted {ok} item(s), freed {}.", bytes(freed));
        if !errs.is_empty() {
            self.message += &format!(" {} failed — {}", errs.len(), errs[0]);
        }
        self.visible_dirty = true;
        self.lists_dirty = true;
        self.drives = drives::list();
    }

    /// Checked items that still exist, minus those whose ancestor is also checked.
    fn selection(&self) -> Vec<u32> {
        let Some(t) = &self.tree else { return Vec::new() };
        let mut v: Vec<u32> = self
            .checked
            .iter()
            .copied()
            .filter(|&i| t.alive(i) && !t.ancestors(i).any(|a| self.checked.contains(&a)))
            .collect();
        v.sort_unstable_by_key(|&i| std::cmp::Reverse(t.node(i).size));
        v
    }

    fn start_delete(&mut self, mode: DelMode) {
        let Some(t) = &self.tree else { return };
        let items: Vec<(u32, PathBuf, bool)> =
            self.selection().into_iter().map(|i| (i, t.path_of(i), t.node(i).is_dir)).collect();
        if items.is_empty() {
            return;
        }
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            let res = items
                .into_iter()
                .map(|(i, p, is_dir)| {
                    let r = match mode {
                        DelMode::Recycle => trash::delete(&p).map_err(|e| e.to_string()),
                        DelMode::Permanent => delete_permanent(&p, is_dir).map_err(|e| e.to_string()),
                    };
                    (i, p, r)
                })
                .collect();
            let _ = tx.send(res);
        });
        self.deleting = Some(rx);
        self.message = "Deleting…".into();
    }

    fn rebuild_visible(&mut self) {
        self.visible.clear();
        self.visible_dirty = false;
        let Some(t) = &self.tree else { return };
        let mut stack = vec![Row { idx: 0, depth: 0, last: true, guides: 0 }];
        while let Some(r) = stack.pop() {
            self.visible.push(r);
            if self.expanded.contains(&r.idx) {
                let mut ch: Vec<u32> = t.children(r.idx).collect();
                ch.sort_unstable_by_key(|&c| std::cmp::Reverse(t.node(c).size));
                // Children keep drawing this row's │ line if this row has siblings below it.
                let guides = if r.depth >= 1 && !r.last && r.depth <= 64 {
                    r.guides | 1u64 << (r.depth - 1)
                } else {
                    r.guides
                };
                let n = ch.len();
                for (k, c) in ch.into_iter().enumerate().rev() {
                    stack.push(Row { idx: c, depth: r.depth + 1, last: k + 1 == n, guides });
                }
            }
        }
    }

    fn rebuild_lists(&mut self) {
        self.lists_dirty = false;
        let Some(t) = &self.tree else { return };
        self.files_view = t.top_files.iter().copied().filter(|&i| t.alive(i)).collect();
        self.dirs_view = t.top_dirs.iter().copied().filter(|&i| t.alive(i)).collect();
        self.dirs_view.sort_unstable_by_key(|&i| std::cmp::Reverse(t.node(i).size));
    }

    fn apply(&mut self, actions: Vec<Action>, ctx: &egui::Context) {
        for a in actions {
            match a {
                Action::Toggle(i) => {
                    if !self.expanded.remove(&i) {
                        self.expanded.insert(i);
                    }
                    self.visible_dirty = true;
                }
                Action::Check(i, on) => {
                    if on {
                        self.checked.insert(i);
                    } else {
                        self.checked.remove(&i);
                    }
                }
                Action::Reveal(i) => {
                    if let Some(t) = &self.tree {
                        self.expanded.extend(t.ancestors(i));
                    }
                    self.tab = Tab::Tree;
                    self.scroll_to = Some(i);
                    self.visible_dirty = true;
                }
                Action::ShowInExplorer(i) => {
                    if let Some(t) = &self.tree {
                        let p = t.path_of(i);
                        let _ = std::process::Command::new("explorer")
                            .arg(format!("/select,{}", p.display()))
                            .spawn();
                    }
                }
                Action::OpenFolder(i) => {
                    if let Some(t) = &self.tree {
                        let _ = std::process::Command::new("explorer").arg(t.path_of(i)).spawn();
                    }
                }
                Action::CopyPath(i) => {
                    if let Some(t) = &self.tree {
                        ctx.copy_text(t.path_of(i).display().to_string());
                    }
                }
                Action::ScanHere(i) => {
                    if let Some(t) = &self.tree {
                        let p = t.path_of(i).display().to_string();
                        self.start_scan(p);
                    }
                }
            }
        }
    }

    fn top_panel(&mut self, ui: &mut egui::Ui) {
        let mut start: Option<String> = None;
        let busy = self.scan.is_some() || self.deleting.is_some();
        ui.add_space(4.0);
        ui.horizontal_wrapped(|ui| {
            ui.label(RichText::new("Drives:").strong());
            for d in &self.drives {
                let used = d.total.saturating_sub(d.free);
                let frac = if d.total > 0 { used as f32 / d.total as f32 } else { 0.0 };
                let txt = format!("{}  {} free / {}", d.root.trim_end_matches('\\'), bytes(d.free), bytes(d.total));
                let btn = ui
                    .add_enabled(!busy, egui::Button::new(txt).min_size(Vec2::new(0.0, 26.0)))
                    .on_hover_text(format!("{} drive · {:.0}% used · click to scan", d.kind, frac * 100.0));
                let r = btn.rect.shrink2(Vec2::new(4.0, 2.0));
                let mut bar = egui::Rect::from_min_max(egui::pos2(r.left(), r.bottom() - 3.0), r.right_bottom());
                ui.painter().rect_filled(bar, 1.0, ui.visuals().extreme_bg_color);
                bar.set_width(bar.width() * frac);
                ui.painter().rect_filled(bar, 1.0, usage_color(frac));
                if btn.clicked() {
                    start = Some(d.root.clone());
                }
            }
            if ui.add_enabled(!busy, egui::Button::new("Refresh")).clicked() {
                self.drives = drives::list();
            }
        });
        ui.add_space(2.0);
        ui.horizontal(|ui| {
            ui.label("Folder:");
            let te = ui.add_enabled(
                !busy,
                egui::TextEdit::singleline(&mut self.path_input).desired_width(420.0),
            );
            if te.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                start = Some(self.path_input.clone());
            }
            if ui.add_enabled(!busy, egui::Button::new("Browse…")).clicked() {
                if let Some(p) = rfd::FileDialog::new().pick_folder() {
                    start = Some(p.display().to_string());
                }
            }
            if let Some(run) = &self.scan {
                if ui.button("Cancel").clicked() {
                    run.progress.cancel.store(true, Relaxed);
                }
            } else if ui.add_enabled(!busy, egui::Button::new(RichText::new("Scan").strong())).clicked() {
                start = Some(self.path_input.clone());
            }
        });
        ui.add_space(2.0);
        if let Some(run) = &self.scan {
            let p = &run.progress;
            ui.horizontal(|ui| {
                ui.spinner();
                ui.label(format!(
                    "Scanning {} …  {} files · {} folders · {} · {:.1}s",
                    run.path,
                    count(p.files.load(Relaxed)),
                    count(p.dirs.load(Relaxed)),
                    bytes(p.bytes.load(Relaxed)),
                    run.started.elapsed().as_secs_f64()
                ));
            });
        } else if !self.message.is_empty() {
            ui.label(RichText::new(&self.message).weak());
        }
        ui.add_space(4.0);
        if let Some(p) = start {
            self.start_scan(p);
        }
    }

    fn bottom_panel(&mut self, ui: &mut egui::Ui) {
        let sel = self.selection();
        let Some(t) = &self.tree else { return };
        let total: u64 = sel.iter().map(|&i| t.node(i).size).sum();
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            ui.label(format!("{} selected · {}", sel.len(), bytes(total)));
            let can = !sel.is_empty() && self.deleting.is_none() && self.scan.is_none();
            if ui.add_enabled(can, egui::Button::new("Move to Recycle Bin")).clicked() {
                self.confirm = Some(DelMode::Recycle);
            }
            let perm = if can {
                egui::Button::new(RichText::new("Delete permanently").color(Color32::WHITE))
                    .fill(Color32::from_rgb(170, 40, 40))
            } else {
                egui::Button::new("Delete permanently")
            };
            if ui.add_enabled(can, perm).clicked() {
                self.confirm = Some(DelMode::Permanent);
            }
            if ui.add_enabled(!self.checked.is_empty(), egui::Button::new("Clear selection")).clicked() {
                self.checked.clear();
            }
            if self.deleting.is_some() {
                ui.spinner();
                ui.label("Deleting…");
            }
        });
        ui.add_space(4.0);
    }

    fn confirm_window(&mut self, ctx: &egui::Context) {
        let Some(mode) = self.confirm else { return };
        let sel = self.selection();
        let Some(t) = &self.tree else { return };
        let total: u64 = sel.iter().map(|&i| t.node(i).size).sum();
        let mut decision: Option<bool> = None;
        let title = match mode {
            DelMode::Recycle => "Move to Recycle Bin?",
            DelMode::Permanent => "Delete permanently?",
        };
        egui::Window::new(title)
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .show(ctx, |ui| {
                ui.label(format!("{} item(s), {} total:", sel.len(), bytes(total)));
                ui.add_space(4.0);
                let mut system = false;
                for &i in sel.iter().take(12) {
                    let p = t.path_of(i).display().to_string();
                    system |= is_system_path(&p);
                    ui.label(RichText::new(format!("{:>10}  {}", bytes(t.node(i).size), p)).monospace());
                }
                if sel.len() > 12 {
                    ui.label(format!("… and {} more", sel.len() - 12));
                }
                system |= sel.iter().skip(12).any(|&i| is_system_path(&t.path_of(i).display().to_string()));
                ui.add_space(6.0);
                if system {
                    ui.colored_label(
                        Color32::from_rgb(230, 120, 30),
                        "⚠ Selection includes Windows / Program Files / system items. Deleting these can break your system.",
                    );
                }
                if mode == DelMode::Permanent {
                    ui.colored_label(Color32::from_rgb(220, 60, 60), "This cannot be undone.");
                } else {
                    ui.label(RichText::new("Large folders can take a while to move to the Recycle Bin.").weak());
                }
                ui.add_space(6.0);
                ui.horizontal(|ui| {
                    let label = if mode == DelMode::Permanent { "Delete" } else { "Recycle" };
                    if ui.button(RichText::new(label).strong()).clicked() {
                        decision = Some(true);
                    }
                    if ui.button("Cancel").clicked() || ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                        decision = Some(false);
                    }
                });
            });
        match decision {
            Some(true) => {
                self.confirm = None;
                self.start_delete(mode);
            }
            Some(false) => self.confirm = None,
            None => {}
        }
    }

    fn central(&mut self, ui: &mut egui::Ui, actions: &mut Vec<Action>) {
        if self.tree.is_none() {
            ui.centered_and_justified(|ui| {
                ui.label(
                    RichText::new(if self.scan.is_some() {
                        "Scanning…"
                    } else {
                        "Pick a drive above (or a folder) to see what is using the space."
                    })
                    .size(16.0)
                    .weak(),
                );
            });
            return;
        }
        if self.visible_dirty {
            self.rebuild_visible();
        }
        if self.lists_dirty {
            self.rebuild_lists();
        }
        ui.horizontal(|ui| {
            ui.selectable_value(&mut self.tab, Tab::Tree, "Folder tree");
            ui.selectable_value(&mut self.tab, Tab::Files, format!("Largest files ({})", self.files_view.len()));
            ui.selectable_value(&mut self.tab, Tab::Folders, format!("Largest folders ({})", self.dirs_view.len()));
            ui.separator();
            ui.label(
                RichText::new("Double-click a folder to expand · right-click for options · tick boxes to select for deletion")
                    .weak()
                    .small(),
            );
        });
        ui.separator();
        let fixed_cols = ui.spacing().interact_size.y
            + W_BAR
            + W_SIZE
            + W_PCT
            + W_FILES
            + ui.spacing().item_spacing.x * 6.0
            + ui.spacing().scroll.bar_width
            + 8.0;
        let name_w = (ui.available_width() - fixed_cols).max(220.0);
        header(ui, if self.tab == Tab::Tree { "Name" } else { "Path" }, name_w);

        let t = self.tree.as_ref().unwrap();
        let spacing = ui.spacing().item_spacing.y;
        let root_size = t.node(0).size;
        match self.tab {
            Tab::Tree => {
                let mut sa = egui::ScrollArea::vertical().auto_shrink([false, false]).id_salt("tree");
                if let Some(target) = self.scroll_to.take() {
                    if let Some(pos) = self.visible.iter().position(|r| r.idx == target) {
                        sa = sa.vertical_scroll_offset((pos as f32 * (ROW_H + spacing) - 120.0).max(0.0));
                    }
                }
                let visible = &self.visible;
                let expanded = &self.expanded;
                let checked = &self.checked;
                sa.show_rows(ui, ROW_H, visible.len(), |ui, range| {
                    let first = range.start;
                    for (k, r) in visible[range].iter().enumerate() {
                        let n = t.node(r.idx);
                        let denom = if n.parent == scan::NONE { n.size } else { t.node(n.parent).size };
                        let row = RowCtx {
                            n: first + k,
                            tree_row: Some(*r),
                            expanded: expanded.contains(&r.idx),
                            checked: checked.contains(&r.idx),
                            denom,
                            name_w,
                        };
                        draw_row(ui, t, r.idx, row, actions);
                    }
                });
            }
            Tab::Files | Tab::Folders => {
                let list = if self.tab == Tab::Files { &self.files_view } else { &self.dirs_view };
                let checked = &self.checked;
                egui::ScrollArea::vertical()
                    .auto_shrink([false, false])
                    .id_salt(if self.tab == Tab::Files { "files" } else { "dirs" })
                    .show_rows(ui, ROW_H, list.len(), |ui, range| {
                        let first = range.start;
                        for (k, &i) in list[range].iter().enumerate() {
                            let row = RowCtx {
                                n: first + k,
                                tree_row: None,
                                expanded: false,
                                checked: checked.contains(&i),
                                denom: root_size,
                                name_w,
                            };
                            draw_row(ui, t, i, row, actions);
                        }
                    });
            }
        }
    }
}

impl eframe::App for App {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.poll(ctx);
        egui::TopBottomPanel::top("top").show(ctx, |ui| self.top_panel(ui));
        if self.tree.is_some() {
            egui::TopBottomPanel::bottom("bottom").show(ctx, |ui| self.bottom_panel(ui));
        }
        let mut actions = Vec::new();
        egui::CentralPanel::default().show(ctx, |ui| self.central(ui, &mut actions));
        self.confirm_window(ctx);
        self.apply(actions, ctx);
        self.self_screenshot(ctx);
    }
}

impl App {
    /// Debug aid: with SPACESCOUT_SHOT=<file.bmp>, save a screenshot once the scan
    /// is shown, then exit. Lets the UI be checked without touching the desktop.
    fn self_screenshot(&mut self, ctx: &egui::Context) {
        let Ok(path) = std::env::var("SPACESCOUT_SHOT") else { return };
        let img = ctx.input(|i| {
            i.raw.events.iter().find_map(|e| match e {
                egui::Event::Screenshot { image, .. } => Some(image.clone()),
                _ => None,
            })
        });
        if let Some(img) = img {
            let _ = save_bmp(&path, &img);
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        } else if self.tree.is_some() && self.scan.is_none() {
            self.shot_frames += 1;
            if self.shot_frames == 5 {
                ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(Default::default()));
            }
            ctx.request_repaint();
        }
    }
}

fn save_bmp(path: &str, img: &egui::ColorImage) -> std::io::Result<()> {
    let [w, h] = img.size;
    let data_len = (w * h * 4) as u32;
    let mut out = Vec::with_capacity(54 + data_len as usize);
    out.extend_from_slice(b"BM");
    out.extend_from_slice(&(54 + data_len).to_le_bytes());
    out.extend_from_slice(&[0; 4]);
    out.extend_from_slice(&54u32.to_le_bytes());
    out.extend_from_slice(&40u32.to_le_bytes());
    out.extend_from_slice(&(w as i32).to_le_bytes());
    out.extend_from_slice(&(-(h as i32)).to_le_bytes()); // top-down
    out.extend_from_slice(&1u16.to_le_bytes());
    out.extend_from_slice(&32u16.to_le_bytes());
    out.extend_from_slice(&[0; 24]);
    for p in &img.pixels {
        out.extend_from_slice(&[p.b(), p.g(), p.r(), 255]);
    }
    std::fs::write(path, out)
}

fn delete_permanent(p: &std::path::Path, is_dir: bool) -> std::io::Result<()> {
    if is_dir {
        std::fs::remove_dir_all(p)
    } else {
        match std::fs::remove_file(p) {
            Err(e) if e.kind() == std::io::ErrorKind::PermissionDenied => {
                // Clear read-only attribute and retry.
                let mut perm = std::fs::metadata(p)?.permissions();
                #[allow(clippy::permissions_set_readonly_false)]
                perm.set_readonly(false);
                std::fs::set_permissions(p, perm)?;
                std::fs::remove_file(p)
            }
            r => r,
        }
    }
}

fn is_system_path(p: &str) -> bool {
    let l = p.to_ascii_lowercase();
    let rest = l.get(2..).unwrap_or("");
    rest.starts_with("\\windows")
        || rest.starts_with("\\program files")
        || rest.starts_with("\\programdata")
        || rest.starts_with("\\$")
        || rest.starts_with("\\system volume information")
        || rest == "\\pagefile.sys"
        || rest == "\\hiberfil.sys"
        || rest == "\\swapfile.sys"
}

fn usage_color(frac: f32) -> Color32 {
    if frac > 0.9 {
        Color32::from_rgb(220, 60, 60)
    } else if frac > 0.75 {
        Color32::from_rgb(230, 150, 40)
    } else {
        Color32::from_rgb(70, 140, 220)
    }
}

fn header(ui: &mut egui::Ui, name: &str, name_w: f32) {
    ui.horizontal(|ui| {
        ui.add_space(ui.spacing().interact_size.y + ui.spacing().item_spacing.x);
        fixed(ui, name_w, RichText::new(name).strong(), false);
        fixed(ui, W_BAR, RichText::new("Share").strong(), false);
        fixed(ui, W_SIZE, RichText::new("Size").strong(), true);
        fixed(ui, W_PCT, RichText::new("% parent").strong(), true);
        fixed(ui, W_FILES, RichText::new("Files").strong(), true);
    });
}

struct RowCtx {
    /// Row number (for zebra striping).
    n: usize,
    /// Some = folder tree row, None = flat list row (shows full path).
    tree_row: Option<Row>,
    expanded: bool,
    checked: bool,
    denom: u64,
    name_w: f32,
}

fn draw_row(ui: &mut egui::Ui, t: &Tree, i: u32, row: RowCtx, actions: &mut Vec<Action>) {
    let n = t.node(i);
    let checked = row.checked;
    let is_tree = row.tree_row.is_some();
    let spacing_y = ui.spacing().item_spacing.y;

    // Row background (zebra stripe / hover / selected), painted before the content.
    let row_rect = egui::Rect::from_min_size(ui.cursor().min, Vec2::new(ui.available_width(), ROW_H))
        .expand2(Vec2::new(0.0, spacing_y * 0.5));
    if ui.rect_contains_pointer(row_rect) {
        ui.painter().rect_filled(row_rect, 2.0, ui.visuals().widgets.hovered.weak_bg_fill);
    } else if row.n % 2 == 1 {
        ui.painter().rect_filled(row_rect, 0.0, ui.visuals().faint_bg_color);
    }
    if checked {
        ui.painter().rect_filled(row_rect, 2.0, Color32::from_rgba_unmultiplied(230, 110, 60, 30));
    }

    ui.horizontal(|ui| {
        ui.set_height(ROW_H);
        let mut c = checked;
        if i == 0 {
            ui.add_enabled(false, egui::Checkbox::without_text(&mut c));
        } else if ui.add(egui::Checkbox::without_text(&mut c)).changed() {
            actions.push(Action::Check(i, c));
        }

        // ---- Name cell: tree guides + arrow + icon + name ----
        let cell = ui.allocate_ui_with_layout(
            Vec2::new(row.name_w, ROW_H),
            Layout::left_to_right(Align::Center),
            |ui| {
                ui.set_min_width(row.name_w);
                ui.set_max_width(row.name_w);
                ui.spacing_mut().item_spacing.x = 0.0;
                if let Some(r) = row.tree_row {
                    let left = ui.cursor().left();
                    let row_top = ui.max_rect().top();
                    let top = row_top - spacing_y * 0.5 - 1.0;
                    let bottom = row_top + ROW_H + spacing_y * 0.5 + 1.0;
                    let mid = row_top + ROW_H * 0.5;
                    let painter = ui.painter().clone();
                    let col_x = |k: u16| (left + k as f32 * INDENT + INDENT * 0.5).round() + 0.5;
                    let stroke = |k: u16| {
                        egui::Stroke::new(1.0, GUIDE_COLORS[k as usize % GUIDE_COLORS.len()].gamma_multiply(0.8))
                    };
                    // Continuing │ lines of ancestors that still have siblings below.
                    for k in 0..r.depth.saturating_sub(1).min(64) {
                        if r.guides & (1u64 << k) != 0 {
                            painter.line_segment([egui::pos2(col_x(k), top), egui::pos2(col_x(k), bottom)], stroke(k));
                        }
                    }
                    // Own connector: ├─ or └─
                    if r.depth >= 1 {
                        let k = r.depth - 1;
                        let x = col_x(k);
                        let y_end = if r.last { mid } else { bottom };
                        painter.line_segment([egui::pos2(x, top), egui::pos2(x, y_end)], stroke(k));
                        let x_end = left + r.depth as f32 * INDENT + if n.is_dir && n.child_len > 0 { 4.0 } else { 14.0 };
                        painter.line_segment([egui::pos2(x, mid), egui::pos2(x_end, mid)], stroke(k));
                    }
                    // Stub from an open folder down to its children.
                    if row.expanded && n.child_len > 0 {
                        let x = col_x(r.depth);
                        painter.line_segment([egui::pos2(x, mid + 6.0), egui::pos2(x, bottom)], stroke(r.depth));
                    }
                    ui.add_space(r.depth as f32 * INDENT);
                    if n.is_dir && n.child_len > 0 {
                        if arrow(ui, row.expanded).clicked() {
                            actions.push(Action::Toggle(i));
                        }
                    } else {
                        ui.add_space(INDENT);
                    }
                }
                ui.add_space(3.0);
                let icon = match (n.is_dir, row.expanded) {
                    (true, true) => "📂",
                    (true, false) => "📁",
                    _ => "📄",
                };
                let text = if is_tree { t.name(i).to_string() } else { t.path_of(i).display().to_string() };
                let mut rt = RichText::new(format!("{icon} {text}"));
                if checked {
                    rt = rt.color(Color32::from_rgb(240, 130, 80));
                } else if n.is_dir {
                    rt = rt.strong();
                }
                ui.add(egui::Label::new(rt).sense(Sense::click()).truncate())
            },
        );
        let resp = cell.inner;

        let frac = if row.denom > 0 { n.size as f64 / row.denom as f64 } else { 0.0 };
        size_bar(ui, frac as f32);
        fixed(ui, W_SIZE, RichText::new(bytes(n.size)).monospace(), true);
        fixed(ui, W_PCT, RichText::new(format!("{:.1}%", frac * 100.0)).monospace().weak(), true);
        fixed(
            ui,
            W_FILES,
            RichText::new(if n.is_dir { count(n.files) } else { String::new() }).monospace().weak(),
            true,
        );

        if resp.double_clicked() {
            if is_tree {
                if n.is_dir {
                    actions.push(Action::Toggle(i));
                }
            } else {
                actions.push(Action::Reveal(i));
            }
        }
        resp.context_menu(|ui| {
            if ui.button("Show in Explorer").clicked() {
                actions.push(Action::ShowInExplorer(i));
                ui.close_menu();
            }
            if n.is_dir && ui.button("Open folder").clicked() {
                actions.push(Action::OpenFolder(i));
                ui.close_menu();
            }
            if ui.button("Copy path").clicked() {
                actions.push(Action::CopyPath(i));
                ui.close_menu();
            }
            if !is_tree && ui.button("Reveal in tree").clicked() {
                actions.push(Action::Reveal(i));
                ui.close_menu();
            }
            if n.is_dir && i != 0 && ui.button("Scan only this folder").clicked() {
                actions.push(Action::ScanHere(i));
                ui.close_menu();
            }
            if i != 0 {
                ui.separator();
                let label = if checked { "Unselect" } else { "Select for deletion" };
                if ui.button(label).clicked() {
                    actions.push(Action::Check(i, !checked));
                    ui.close_menu();
                }
            }
        });
    });
}

fn fixed(ui: &mut egui::Ui, w: f32, text: RichText, right: bool) {
    let layout = if right { Layout::right_to_left(Align::Center) } else { Layout::left_to_right(Align::Center) };
    ui.allocate_ui_with_layout(Vec2::new(w, ROW_H), layout, |ui| {
        ui.set_min_width(w);
        ui.label(text);
    });
}

fn size_bar(ui: &mut egui::Ui, frac: f32) {
    let (rect, _) = ui.allocate_exact_size(Vec2::new(W_BAR, 10.0), Sense::hover());
    let p = ui.painter();
    p.rect_filled(rect, 2.0, ui.visuals().extreme_bg_color);
    let mut f = rect;
    f.set_width(rect.width() * frac.clamp(0.0, 1.0));
    p.rect_filled(f, 2.0, usage_color(frac * 0.8 + 0.1).gamma_multiply(0.9));
}

fn arrow(ui: &mut egui::Ui, open: bool) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(INDENT, ROW_H), Sense::click());
    let c = rect.center();
    let s = 4.0;
    let color = if resp.hovered() { ui.visuals().strong_text_color() } else { ui.visuals().text_color() };
    let pts = if open {
        vec![c + Vec2::new(-s, -s * 0.5), c + Vec2::new(s, -s * 0.5), c + Vec2::new(0.0, s * 0.6)]
    } else {
        vec![c + Vec2::new(-s * 0.5, -s), c + Vec2::new(s * 0.6, 0.0), c + Vec2::new(-s * 0.5, s)]
    };
    ui.painter().add(egui::Shape::convex_polygon(pts, color, egui::Stroke::NONE));
    resp
}

#[cfg(test)]
mod tests {
    #[test]
    fn permanent_delete_handles_readonly() {
        let base = std::env::temp_dir().join(format!("spacescout_del_{}", std::process::id()));
        std::fs::create_dir_all(base.join("sub")).unwrap();
        let f = base.join("ro.txt");
        std::fs::write(&f, b"x").unwrap();
        let mut perm = std::fs::metadata(&f).unwrap().permissions();
        perm.set_readonly(true);
        std::fs::set_permissions(&f, perm).unwrap();
        super::delete_permanent(&f, false).unwrap();
        assert!(!f.exists());
        super::delete_permanent(&base, true).unwrap();
        assert!(!base.exists());
    }

    #[test]
    fn system_paths() {
        assert!(super::is_system_path(r"C:\Windows\System32"));
        assert!(super::is_system_path(r"C:\pagefile.sys"));
        assert!(!super::is_system_path(r"C:\Users\me\Downloads"));
    }
}
