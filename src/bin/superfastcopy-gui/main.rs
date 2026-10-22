#![windows_subsystem = "windows"]

//! Super Fast Copy window.
//!
//! Launch modes (the installer's Explorer menu entries use these):
//! * `superfastcopy-gui`                   – main window
//! * `superfastcopy-gui <paths...>`        – sources prefilled, asks for a destination, then starts
//! * `superfastcopy-gui --copy <paths...>` – no window: puts the paths on the clipboard ("Copy with SFC")
//! * `superfastcopy-gui --paste <folder>`  – compact window copying the clipboard's files into <folder>
//! * `--to <folder>` with the main window   – destination prefilled

mod settings;
mod theme;
mod widgets;

use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use eframe::egui::{self, pos2, vec2, Align, Align2, Color32, CursorIcon, FontId, Layout, Rect, RichText, Sense, Ui};
use superfastcopy::{fmt, shell, Existing, Job, Options, Snapshot};

use settings::Settings;
use theme::*;
use widgets::{self as w, elide_start};

const LOGO: &[u8] = include_bytes!("../../../assets/icon-128.rgba");
/// Speed samples kept for the graph (4 per second).
const HISTORY: usize = 160;
const CARD_H: f32 = 272.0;
/// How far back the time estimate looks when measuring progress speed.
const ETA_WINDOW: Duration = Duration::from_secs(6);

/// Explorer passes drive roots as `"D:\"`, which Windows argument parsing
/// turns into `D:"` (the backslash escapes the quote).
fn clean_arg(a: std::ffi::OsString) -> PathBuf {
    match a.to_str() {
        Some(s) if s.ends_with('"') => {
            let s = s.trim_end_matches('"');
            PathBuf::from(if s.ends_with(':') { format!("{s}\\") } else { s.to_string() })
        }
        _ => PathBuf::from(a),
    }
}

fn main() -> eframe::Result {
    let mut args: Vec<PathBuf> = std::env::args_os().skip(1).map(clean_arg).collect();
    if args.first().is_some_and(|a| a.as_os_str() == "--copy") {
        args.remove(0);
        let _ = shell::add_to_copy_list(&args);
        return Ok(());
    }

    let app = App::from_args(args);
    let (size, min) = match app.mode {
        Mode::Main => ([900.0, 730.0], [780.0, 680.0]),
        Mode::Paste => ([640.0, 700.0], [580.0, 660.0]),
    };
    let icon = egui::IconData { rgba: LOGO.to_vec(), width: 128, height: 128 };
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("Super Fast Copy")
            .with_inner_size(size)
            .with_min_inner_size(min)
            .with_drag_and_drop(true)
            .with_icon(Arc::new(icon)),
        ..Default::default()
    };
    eframe::run_native(
        "Super Fast Copy",
        options,
        Box::new(|cc| {
            theme::install(&cc.egui_ctx);
            Ok(Box::new(app))
        }),
    )
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Mode {
    Main,
    Paste,
}

struct Source {
    path: PathBuf,
    is_dir: bool,
}

struct Run {
    job: Arc<Job>,
    worker: Option<JoinHandle<Result<(), String>>>,
    sources: Vec<PathBuf>,
    dest: PathBuf,
    started: Instant,
    /// Elapsed time and outcome, once the worker has exited.
    finished: Option<(Duration, Result<(), String>)>,
    last: (Instant, u64, u64),
    speed: VecDeque<f32>,
    files_rate: VecDeque<f32>,
    errors: Vec<String>,
    /// Highest progress reached once the totals are known; the ring never
    /// goes below it.
    target: f32,
    /// What the ring shows: eased towards `target` every frame.
    shown: f32,
    /// (time, files done, bytes done) over the last `ETA_WINDOW`.
    progress_hist: VecDeque<(Instant, u64, u64)>,
    /// Smoothed seconds remaining.
    eta: Option<f64>,
}

impl Run {
    fn start(sources: Vec<PathBuf>, dest: PathBuf, opts: Options) -> Self {
        let job = Arc::new(Job::new());
        let worker = {
            let (job, sources, dest) = (job.clone(), sources.clone(), dest.clone());
            std::thread::spawn(move || superfastcopy::run(&job, &sources, &dest, &opts))
        };
        let now = Instant::now();
        Run {
            job,
            worker: Some(worker),
            sources,
            dest,
            started: now,
            finished: None,
            last: (now, 0, 0),
            speed: VecDeque::new(),
            files_rate: VecDeque::new(),
            errors: Vec::new(),
            target: 0.0,
            shown: 0.0,
            progress_hist: VecDeque::new(),
            eta: None,
        }
    }

    fn running(&self) -> bool {
        self.finished.is_none()
    }

    fn succeeded(&self) -> bool {
        matches!(self.finished, Some((_, Ok(()))))
    }

    fn elapsed(&self) -> Duration {
        self.finished.as_ref().map_or_else(|| self.started.elapsed(), |f| f.0)
    }

    fn poll(&mut self) {
        if self.finished.is_none() && self.worker.as_ref().is_some_and(|w| w.is_finished()) {
            let result = self.worker.take().unwrap().join().unwrap_or_else(|_| Err("internal error".into()));
            self.finished = Some((self.started.elapsed(), result));
        }
    }

    fn sample(&mut self, s: &Snapshot) {
        let now = Instant::now();
        let dt = now - self.last.0;
        if self.running() && dt >= Duration::from_millis(250) {
            let push = |q: &mut VecDeque<f32>, v: f64| {
                q.push_back(v as f32);
                while q.len() > HISTORY {
                    q.pop_front();
                }
            };
            push(&mut self.speed, fmt::rate(s.bytes_copied.saturating_sub(self.last.1), dt));
            push(&mut self.files_rate, fmt::rate(s.files_copied.saturating_sub(self.last.2), dt));
            self.last = (now, s.bytes_copied, s.files_copied);
            if s.scan_done {
                self.update_estimate(now, dt, s);
            }
        }
        // Ease the ring towards the target so it moves smoothly.
        self.shown += (self.target - self.shown) * 0.2;
        if (self.target - self.shown).abs() < 0.001 {
            self.shown = self.target;
        }
    }

    /// Time left from the speed over the last few seconds. Files and data
    /// are estimated separately and the later one wins: big files finishing
    /// early mustn't make thousands of small files look almost done, and a
    /// single huge file has no file rate at all. Each new estimate is
    /// blended with the previous one (counted down by the time that passed),
    /// so the number ticks down steadily instead of jumping around.
    fn update_estimate(&mut self, now: Instant, dt: Duration, s: &Snapshot) {
        self.target = self.target.max(s.fraction());
        self.progress_hist.push_back((now, s.files_done(), s.bytes_done));
        while self.progress_hist.len() > 2 && now - self.progress_hist[0].0 > ETA_WINDOW {
            self.progress_hist.pop_front();
        }
        let (t0, f0, b0) = self.progress_hist[0];
        let span = (now - t0).as_secs_f64();
        if span < 1.0 {
            return;
        }
        let time_left = |left: u64, done_in_window: u64| {
            let rate = done_in_window as f64 / span;
            (left > 0 && rate > 0.0).then(|| left as f64 / rate)
        };
        let by_files = time_left(s.files_found.saturating_sub(s.files_done()), s.files_done().saturating_sub(f0));
        let by_bytes = time_left(s.bytes_found.saturating_sub(s.bytes_done), s.bytes_done.saturating_sub(b0));
        let estimate = match (by_files, by_bytes) {
            (Some(f), Some(b)) => Some(f.max(b)),
            (f, b) => f.or(b),
        };
        if let Some(estimate) = estimate {
            self.eta = Some(match self.eta {
                None => estimate,
                Some(prev) => (prev - dt.as_secs_f64()).max(0.0) * 0.8 + estimate * 0.2,
            });
        } else if let Some(prev) = self.eta {
            // Stalled (e.g. waiting on a slow disk): keep counting down.
            self.eta = Some((prev - dt.as_secs_f64()).max(0.0));
        }
    }

    /// Bytes/s and files/s over the last second.
    fn current_speed(&self) -> (f64, f64) {
        let avg = |q: &VecDeque<f32>| {
            let n = q.len().min(4);
            if n == 0 {
                0.0
            } else {
                q.iter().rev().take(n).sum::<f32>() as f64 / n as f64
            }
        };
        (avg(&self.speed), avg(&self.files_rate))
    }
}

impl Drop for Run {
    fn drop(&mut self) {
        self.job.cancel();
        if let Some(w) = self.worker.take() {
            let _ = w.join();
        }
    }
}

enum Action {
    Open,
    Again,
    New,
    Close,
    KeepOpen,
}

struct App {
    mode: Mode,
    sources: Vec<Source>,
    dest: String,
    settings: Settings,
    saved: Settings,
    run: Option<Run>,
    notice: Option<String>,
    /// The pasted files were cut (Ctrl+X); SFC only copies, so say so.
    cut_note: bool,
    pick_and_start: bool,
    start_now: bool,
    frames: u32,
    title: String,
    logo: Option<egui::TextureHandle>,
    styled_titlebar: bool,
    space: Option<(String, Option<(u64, u64)>)>,
    close_at: Option<Instant>,
    keep_open: bool,
    show_errors: bool,
}

impl App {
    fn from_args(args: Vec<PathBuf>) -> Self {
        let settings = Settings::load();
        let mut app = App {
            mode: Mode::Main,
            sources: Vec::new(),
            dest: String::new(),
            saved: settings.clone(),
            settings,
            run: None,
            notice: None,
            cut_note: false,
            pick_and_start: false,
            start_now: false,
            frames: 0,
            title: String::new(),
            logo: None,
            styled_titlebar: false,
            space: None,
            close_at: None,
            keep_open: false,
            show_errors: false,
        };
        let mut args = args.into_iter();
        while let Some(a) = args.next() {
            if a.as_os_str() == "--paste" {
                app.mode = Mode::Paste;
                if let Some(d) = args.next() {
                    app.dest = d.display().to_string();
                }
                let clip = shell::read_clipboard();
                app.cut_note = clip.cut;
                app.add_sources(clip.paths);
                app.start_now = !app.sources.is_empty() && !app.dest.is_empty();
            } else if a.as_os_str() == "--to" {
                if let Some(d) = args.next() {
                    app.dest = d.display().to_string();
                }
            } else {
                app.add_sources([a]);
            }
        }
        app.pick_and_start = app.mode == Mode::Main && !app.sources.is_empty() && app.dest.is_empty();
        app
    }

    fn add_sources(&mut self, paths: impl IntoIterator<Item = PathBuf>) {
        for path in paths {
            if !self.sources.iter().any(|s| s.path == path) {
                let is_dir = path.is_dir();
                self.sources.push(Source { path, is_dir });
            }
        }
    }

    fn running(&self) -> bool {
        self.run.as_ref().is_some_and(Run::running)
    }

    fn start(&mut self) {
        self.notice = None;
        if self.sources.is_empty() {
            self.notice = Some("Add at least one file or folder to copy.".into());
            return;
        }
        let dest = self.dest.trim().to_string();
        if dest.is_empty() {
            self.notice = Some("Choose where to copy to.".into());
            return;
        }
        let s = &self.settings;
        let opts = Options {
            threads: if s.auto_threads { 0 } else { s.threads },
            existing: s.existing,
            contents_only: s.contents_only,
            native: s.native,
            ..Default::default()
        };
        if self.mode == Mode::Main {
            self.settings.remember(&dest);
        }
        self.close_at = None;
        self.keep_open = false;
        self.show_errors = false;
        // Drop (and so join) any previous run before starting the next.
        self.run = None;
        self.run = Some(Run::start(self.sources.iter().map(|s| s.path.clone()).collect(), PathBuf::from(dest), opts));
    }

    fn pick_folders(&mut self) {
        if let Some(dirs) = rfd::FileDialog::new().set_title("Choose folders to copy").pick_folders() {
            self.add_sources(dirs);
        }
    }

    fn pick_files(&mut self) {
        if let Some(files) = rfd::FileDialog::new().set_title("Choose files to copy").pick_files() {
            self.add_sources(files);
        }
    }

    fn paste_sources(&mut self) {
        let clip = shell::read_clipboard();
        if clip.paths.is_empty() {
            self.notice = Some("The clipboard has no files. Copy some in Explorer first (Ctrl+C).".into());
        } else {
            self.notice = None;
            self.add_sources(clip.paths);
        }
    }

    fn dest_space(&mut self) -> Option<(u64, u64)> {
        let d = self.dest.trim().to_string();
        if d.is_empty() {
            return None;
        }
        if self.space.as_ref().map(|s| &s.0) != Some(&d) {
            let space = shell::disk_space(Path::new(&d));
            self.space = Some((d, space));
        }
        self.space.as_ref().and_then(|s| s.1)
    }

    fn status(&self) -> (String, Color32) {
        match &self.run {
            None if self.mode == Mode::Paste => ("Nothing to paste".into(), WARN),
            None => ("Ready".into(), MUTED),
            Some(r) if r.running() && r.job.is_cancelled() => ("Cancelling".into(), WARN),
            Some(r) if r.running() => ("Copying".into(), ACCENT2),
            Some(r) => {
                let errors = r.job.snapshot().errors;
                match &r.finished {
                    Some((_, Err(_))) => ("Failed".into(), BAD),
                    _ if r.job.is_cancelled() => ("Cancelled".into(), WARN),
                    _ if errors > 0 => (format!("{} errors", fmt::count(errors)), WARN),
                    _ => ("Done".into(), GOOD),
                }
            }
        }
    }

    fn auto_close(&mut self, ctx: &egui::Context) {
        if self.mode != Mode::Paste || self.keep_open || !self.settings.auto_close {
            return;
        }
        let Some(run) = &self.run else { return };
        if run.running() || !run.succeeded() || run.job.is_cancelled() || run.job.snapshot().errors > 0 {
            return;
        }
        let at = *self.close_at.get_or_insert_with(|| Instant::now() + Duration::from_millis(2500));
        if Instant::now() >= at {
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }
    }

    // ---- layout ---------------------------------------------------------

    fn header(&self, ui: &mut Ui) {
        ui.horizontal(|ui| {
            if let Some(t) = &self.logo {
                ui.add(egui::Image::new((t.id(), vec2(42.0, 42.0))));
            }
            ui.add_space(4.0);
            ui.vertical(|ui| {
                ui.spacing_mut().item_spacing.y = 0.0;
                ui.add_space(2.0);
                ui.label(RichText::new("Super Fast Copy").family(semibold()).size(21.0).color(TEXT));
                let sub = match self.mode {
                    Mode::Main => "Ultra-fast file copying for Windows",
                    Mode::Paste => "Paste with SFC",
                };
                ui.label(RichText::new(sub).size(12.5).color(MUTED));
            });
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                let (text, color) = self.status();
                w::pill(ui, &text, color);
            });
        });
    }

    fn setup_view(&mut self, ui: &mut Ui) {
        let full = ui.available_width();
        let gap = 44.0;
        let col = ((full - gap) / 2.0).floor();
        ui.horizontal_top(|ui| {
            ui.spacing_mut().item_spacing.x = 0.0;
            w::card(ui, vec2(col, CARD_H), |ui| self.from_card(ui));
            let (r, _) = ui.allocate_exact_size(vec2(gap, CARD_H), Sense::hover());
            let c = r.center();
            w::gradient_rect(ui.painter(), Rect::from_center_size(c, vec2(34.0, 34.0)), 17.0, ACCENT, ACCENT2);
            ui.painter().text(c, Align2::CENTER_CENTER, icon::FORWARD, FontId::new(14.0, icons()), Color32::WHITE);
            w::card(ui, vec2(col, CARD_H), |ui| self.to_card(ui));
        });
        ui.add_space(14.0);
        w::card(ui, vec2(full, 0.0), |ui| self.options_card(ui));
        ui.add_space(18.0);
        let ready = !self.sources.is_empty() && !self.dest.trim().is_empty();
        if w::primary_button(ui, icon::BOLT, "Start copying", vec2(full, 56.0), ready).clicked() {
            self.start();
        }
        if let Some(n) = &self.notice {
            ui.add_space(2.0);
            ui.horizontal(|ui| {
                ui.label(w::icon_label(icon::WARNING, 13.0, WARN));
                ui.label(RichText::new(n).color(WARN));
            });
        }
    }

    fn from_card(&mut self, ui: &mut Ui) {
        ui.horizontal(|ui| {
            w::section_label(ui, "COPY FROM");
            if !self.sources.is_empty() {
                ui.add_space(6.0);
                w::badge(ui, &self.sources.len().to_string());
            }
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                ui.spacing_mut().item_spacing.x = 2.0;
                if !self.sources.is_empty() && w::icon_button(ui, icon::DELETE, "Clear the list").clicked() {
                    self.sources.clear();
                }
                if w::icon_button(ui, icon::PASTE, "Add files from the clipboard (copied with Ctrl+C)").clicked() {
                    self.paste_sources();
                }
                if w::icon_button(ui, icon::FILE, "Add files…").clicked() {
                    self.pick_files();
                }
                if w::icon_button(ui, icon::NEW_FOLDER, "Add folders…").clicked() {
                    self.pick_folders();
                }
            });
        });
        ui.add_space(4.0);
        let list_h = CARD_H - 36.0 - 42.0;

        if self.sources.is_empty() {
            let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), list_h), Sense::click());
            let resp = resp.on_hover_cursor(CursorIcon::PointingHand);
            let p = ui.painter();
            if resp.hovered() {
                p.rect_filled(rect, 12.0, ACCENT.gamma_multiply(0.06));
            }
            w::dashed_rect(p, rect, 12.0, stroke(1.5, if resp.hovered() { ACCENT } else { BORDER }));
            let c = rect.center();
            p.circle_filled(c - vec2(0.0, 28.0), 26.0, ACCENT.gamma_multiply(0.16));
            p.text(c - vec2(0.0, 28.0), Align2::CENTER_CENTER, icon::DROP, FontId::new(22.0, icons()), ACCENT2);
            p.text(c + vec2(0.0, 14.0), Align2::CENTER_CENTER, "Drop files and folders here", FontId::new(15.0, semibold()), TEXT);
            p.text(c + vec2(0.0, 36.0), Align2::CENTER_CENTER, "or click to choose a folder", FontId::proportional(12.5), MUTED);
            if resp.clicked() {
                self.pick_folders();
            }
            return;
        }

        let mut remove = None;
        egui::ScrollArea::vertical().max_height(list_h).auto_shrink([false, false]).show(ui, |ui| {
            ui.spacing_mut().item_spacing.y = 2.0;
            for (i, s) in self.sources.iter().enumerate() {
                if source_row(ui, s) {
                    remove = Some(i);
                }
            }
        });
        if let Some(i) = remove {
            self.sources.remove(i);
        }
    }

    fn to_card(&mut self, ui: &mut Ui) {
        w::section_label(ui, "COPY TO");
        ui.add_space(4.0);
        let dest = self.dest.trim().to_string();
        ui.horizontal(|ui| {
            let (r, _) = ui.allocate_exact_size(vec2(46.0, 46.0), Sense::hover());
            w::gradient_rect(ui.painter(), r, 13.0, ACCENT, ACCENT2);
            let glyph = if dest.is_empty() { icon::FOLDER } else { icon::OPEN };
            ui.painter().text(r.center(), Align2::CENTER_CENTER, glyph, FontId::new(20.0, icons()), Color32::WHITE);
            ui.add_space(4.0);
            ui.vertical(|ui| {
                ui.spacing_mut().item_spacing.y = 1.0;
                ui.add_space(3.0);
                let (name, sub) = if dest.is_empty() {
                    ("No folder chosen".to_string(), "Type a path or browse".to_string())
                } else {
                    let name = Path::new(&dest).file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or(dest.clone());
                    (name, elide_start(&dest, 40))
                };
                ui.label(RichText::new(name).family(semibold()).size(15.5).color(if dest.is_empty() { MUTED } else { TEXT }));
                ui.label(RichText::new(sub).size(12.0).color(MUTED));
            });
        });
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            let edit_w = ui.available_width() - 112.0;
            ui.add(egui::TextEdit::singleline(&mut self.dest).hint_text(r"e.g. D:\Backup").desired_width(edit_w).margin(vec2(10.0, 8.0)));
            if w::ghost_button(ui, icon::OPEN, "Browse").clicked() {
                if let Some(d) = rfd::FileDialog::new().set_title("Copy to…").pick_folder() {
                    self.dest = d.display().to_string();
                }
            }
        });
        if let Some((free, total)) = self.dest_space() {
            ui.add_space(2.0);
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 6.0;
                ui.label(w::icon_label(icon::DRIVE, 12.0, MUTED));
                ui.label(RichText::new(format!("{} free of {}", fmt::bytes(free), fmt::bytes(total))).size(12.0).color(MUTED));
            });
            let used = 1.0 - free as f32 / total.max(1) as f32;
            let (a, b) = if used > 0.9 { (WARN, BAD) } else { (ACCENT, ACCENT2) };
            w::bar(ui, used, 6.0, a, b);
        }
        let recent: Vec<String> =
            self.settings.recent.iter().filter(|r| !r.eq_ignore_ascii_case(&dest)).take(2).cloned().collect();
        if !recent.is_empty() {
            ui.add_space(4.0);
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing = vec2(6.0, 6.0);
                ui.label(w::icon_label(icon::RECENT, 12.0, MUTED)).on_hover_text("Recent destinations");
                for r in recent {
                    let chip = egui::Button::new(RichText::new(elide_start(&r, 22)).size(12.5)).rounding(14.0).min_size(vec2(0.0, 28.0));
                    if ui.add(chip).on_hover_text(&r).on_hover_cursor(CursorIcon::PointingHand).clicked() {
                        self.dest = r;
                    }
                }
            });
        }
    }

    fn options_card(&mut self, ui: &mut Ui) {
        w::section_label(ui, "OPTIONS");
        ui.horizontal(|ui| {
            ui.vertical(|ui| {
                ui.spacing_mut().item_spacing.y = 1.0;
                ui.label(RichText::new("When a file already exists").color(TEXT));
                ui.label(RichText::new("\"Skip unchanged\" makes re-syncing a backup almost instant").size(12.0).color(MUTED));
            });
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                w::segmented(
                    ui,
                    "existing",
                    &mut self.settings.existing,
                    &[(Existing::Overwrite, "Replace"), (Existing::SkipSame, "Skip unchanged"), (Existing::SkipExisting, "Skip all")],
                );
            });
        });
        ui.add_space(2.0);
        let (line, _) = ui.allocate_exact_size(vec2(ui.available_width(), 1.0), Sense::hover());
        ui.painter().rect_filled(line, 0.0, BORDER);
        ui.add_space(4.0);

        let col = (ui.available_width() - 20.0) / 2.0;
        let s = &mut self.settings;
        ui.horizontal_top(|ui| {
            ui.vertical(|ui| {
                ui.set_width(col);
                ui.horizontal(|ui| {
                    w::toggle_row(ui, &mut s.auto_threads, "Automatic threads", "Tuned for SSD, hard disk or network");
                    if !s.auto_threads {
                        ui.add(egui::DragValue::new(&mut s.threads).range(1..=256).suffix(" threads"));
                    }
                });
                w::toggle_row(ui, &mut s.contents_only, "Copy folder contents only", "Don't create a folder named after the source");
            });
            ui.add_space(20.0);
            ui.vertical(|ui| {
                ui.set_width(col);
                w::toggle_row(ui, &mut s.native, "Compatibility mode", "Keeps extra NTFS streams; slower for small files");
                w::toggle_row(ui, &mut s.auto_close, "Close paste window when done", "For \"Paste with SFC\" in Explorer");
            });
        });
    }

    fn run_view(&mut self, ui: &mut Ui) {
        let full = ui.available_width();
        let compact = self.mode == Mode::Paste;
        w::card(ui, vec2(full, 0.0), |ui| self.route(ui, compact));
        ui.add_space(12.0);
        w::card(ui, vec2(full, 0.0), |ui| self.progress(ui, compact));
        ui.add_space(16.0);
        self.actions(ui);
        self.errors(ui);
    }

    fn route(&self, ui: &mut Ui, compact: bool) {
        let Some(run) = &self.run else { return };
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 8.0;
            let first = run.sources.first().and_then(|p| p.file_name()).map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            let what = match run.sources.len() {
                1 => first,
                n => format!("{first} + {} more", n - 1),
            };
            let all: Vec<String> = run.sources.iter().map(|p| p.display().to_string()).collect();
            ui.label(w::icon_label(icon::COPY, 15.0, ACCENT2));
            ui.label(RichText::new(elide_start(&what, if compact { 22 } else { 40 })).family(semibold()).color(TEXT))
                .on_hover_text(all.join("\n"));
            ui.label(w::icon_label(icon::FORWARD, 11.0, MUTED));
            ui.label(w::icon_label(icon::FOLDER, 15.0, ACCENT2));
            let dest = run.dest.display().to_string();
            ui.label(RichText::new(elide_start(&dest, if compact { 26 } else { 46 })).family(semibold()).color(TEXT)).on_hover_text(&dest);
        });
        if let Some(plan) = run.job.plan() {
            ui.label(RichText::new(format!("Using {plan}")).size(12.0).color(MUTED));
        }
        if self.cut_note {
            ui.label(RichText::new("The files were cut, but Super Fast Copy only copies: the originals stay where they are.").size(12.0).color(WARN));
        }
    }

    fn progress(&self, ui: &mut Ui, compact: bool) {
        let Some(run) = &self.run else { return };
        let s = run.job.snapshot();
        let running = run.running();
        let elapsed = run.elapsed();
        let failed = matches!(run.finished, Some((_, Err(_))));
        let cancelled = run.job.is_cancelled();
        let frac = if running { run.shown } else { 1.0 };
        let (bps, fps) = if running {
            run.current_speed()
        } else {
            (fmt::rate(s.bytes_copied, elapsed), fmt::rate(s.files_copied, elapsed))
        };

        let ring_d = if compact { 132.0 } else { 176.0 };
        ui.horizontal_top(|ui| {
            let (rect, _) = ui.allocate_exact_size(vec2(ring_d, ring_d), Sense::hover());
            let p = ui.painter();
            let c = rect.center();
            let width = if compact { 10.0 } else { 13.0 };
            let r = ring_d / 2.0 - width;
            let (a, b) = if running {
                (ACCENT, ACCENT2)
            } else if failed {
                (BAD, BAD)
            } else if cancelled || s.errors > 0 {
                (WARN, lerp_warn())
            } else {
                (GOOD, w::lerp(GOOD, ACCENT2, 0.6))
            };
            // Until the scan has found everything the total is still growing,
            // so a percentage would jump around: spin and count instead.
            let scanning = running && !s.scan_done;
            let spin = scanning.then(|| ui.input(|i| i.time));
            w::ring(p, c, r, width, frac, spin, a, b);
            let sub_y = c + vec2(0.0, if compact { 18.0 } else { 22.0 });
            if scanning {
                let size = if compact { 20.0 } else { 24.0 };
                p.text(c - vec2(0.0, 8.0), Align2::CENTER_CENTER, fmt::count(s.files_found), FontId::new(size, semibold()), TEXT);
                p.text(sub_y, Align2::CENTER_CENTER, "files found", FontId::proportional(12.5), MUTED);
            } else if running {
                let big = FontId::new(if compact { 28.0 } else { 36.0 }, semibold());
                p.text(c - vec2(0.0, 8.0), Align2::CENTER_CENTER, format!("{:.0}%", frac * 100.0), big, TEXT);
                let sub = if cancelled { "cancelling" } else { "complete" };
                p.text(sub_y, Align2::CENTER_CENTER, sub, FontId::proportional(12.5), MUTED);
            } else {
                let (glyph, color, label) = if failed {
                    (icon::ERROR, BAD, "Failed")
                } else if cancelled {
                    (icon::STOP, WARN, "Cancelled")
                } else if s.errors > 0 {
                    (icon::WARNING, WARN, "Finished")
                } else {
                    (icon::CHECK, GOOD, "Done")
                };
                p.circle_filled(c, r - width, color.gamma_multiply(0.10));
                p.text(c - vec2(0.0, 9.0), Align2::CENTER_CENTER, glyph, FontId::new(if compact { 26.0 } else { 34.0 }, icons()), color);
                p.text(c + vec2(0.0, if compact { 20.0 } else { 26.0 }), Align2::CENTER_CENTER, label, FontId::new(13.5, semibold()), color);
            }

            ui.add_space(if compact { 16.0 } else { 26.0 });
            ui.vertical(|ui| {
                ui.spacing_mut().item_spacing.y = 4.0;
                let headline = match &run.finished {
                    Some((_, Err(e))) => format!("Could not start: {e}"),
                    _ if running && cancelled => "Stopping…".into(),
                    _ if running && s.scan_done => "Copying files".into(),
                    _ if running => "Scanning and copying".into(),
                    _ if cancelled => "Copy cancelled".into(),
                    _ if s.errors > 0 => format!("Finished with {} errors", fmt::count(s.errors)),
                    _ => format!("Copied {} files in {}", fmt::count(s.files_copied), fmt::duration(elapsed)),
                };
                ui.label(RichText::new(headline).size(13.5).color(if failed { BAD } else { MUTED }));
                ui.horizontal(|ui| {
                    ui.label(
                        RichText::new(format!("{}/s", fmt::bytes(bps as u64)))
                            .family(semibold())
                            .size(if compact { 24.0 } else { 30.0 })
                            .color(TEXT),
                    );
                    ui.vertical(|ui| {
                        ui.add_space(if compact { 8.0 } else { 12.0 });
                        let avg = if running { "" } else { "  average" };
                        ui.label(RichText::new(format!("{} files/s{avg}", fmt::count(fps as u64))).size(13.5).color(MUTED));
                    });
                });
                // Small files: per-file overhead dominates, so files/s is the
                // meaningful curve; MB/s would just show the few big files.
                let small_files = s.files_found > 0 && s.bytes_found / s.files_found < (1 << 20);
                let (series, label) = if small_files { (&run.files_rate, "files/s") } else { (&run.speed, "MB/s") };
                w::sparkline(ui, series, HISTORY, if compact { 46.0 } else { 66.0 }, label);
            });
        });

        ui.add_space(12.0);
        let remaining = if !running {
            "—".into()
        } else if !s.scan_done {
            "after scan".into()
        } else {
            run.eta.map_or_else(|| "estimating…".into(), eta_text)
        };
        let tiles = [
            (
                icon::FILE,
                "FILES",
                format!("{} / {}", fmt::count(s.files_done()), fmt::count(s.files_found)),
                format!("{} skipped · {} failed", fmt::count(s.files_skipped), fmt::count(s.files_failed)),
            ),
            (
                icon::DRIVE,
                "DATA",
                format!("{} / {}", fmt::bytes(s.bytes_done), fmt::bytes(s.bytes_found)),
                format!("{} folders", fmt::count(s.dirs_found)),
            ),
            (icon::STOPWATCH, "ELAPSED", fmt::duration(elapsed), String::new()),
            (icon::RECENT, "REMAINING", remaining, String::new()),
        ];
        let per_row = if compact { 2 } else { 4 };
        let gap = 10.0;
        let tile_w = ((ui.available_width() - gap * (per_row - 1) as f32) / per_row as f32).floor();
        for row in tiles.chunks(per_row) {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = gap;
                for (ic, label, value, sub) in row {
                    w::stat(ui, tile_w, *ic, label, value, sub);
                }
            });
        }
        if s.links_skipped > 0 {
            ui.label(RichText::new(format!("{} folder links/junctions were not followed", s.links_skipped)).size(12.0).color(MUTED));
        }
    }

    fn actions(&mut self, ui: &mut Ui) {
        let Some(run) = &self.run else { return };
        let mut action = None;
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 10.0;
            if run.running() {
                let stop = egui::Button::new(w::icon_text(icon::STOP, "Cancel", 14.0, BAD)).min_size(vec2(140.0, 42.0)).rounding(10.0);
                if ui.add_enabled(!run.job.is_cancelled(), stop).clicked() {
                    run.job.cancel();
                }
                return;
            }
            if w::primary_button(ui, icon::OPEN, "Open folder", vec2(180.0, 42.0), true).clicked() {
                action = Some(Action::Open);
            }
            match self.mode {
                Mode::Main => {
                    if w::ghost_button(ui, icon::REFRESH, "Run again").clicked() {
                        action = Some(Action::Again);
                    }
                    if w::ghost_button(ui, icon::ADD, "New copy").clicked() {
                        action = Some(Action::New);
                    }
                }
                Mode::Paste => {
                    if w::ghost_button(ui, icon::CLOSE, "Close").clicked() {
                        action = Some(Action::Close);
                    }
                    if let Some(at) = self.close_at {
                        let left = at.saturating_duration_since(Instant::now()).as_secs_f32().ceil();
                        ui.label(RichText::new(format!("Closing in {left:.0} s")).size(12.5).color(MUTED));
                        if ui.link("Keep open").clicked() {
                            action = Some(Action::KeepOpen);
                        }
                    }
                }
            }
        });
        match action {
            Some(Action::Open) => {
                let _ = std::process::Command::new("explorer").arg(&run.dest).spawn();
            }
            Some(Action::Again) => self.start(),
            Some(Action::New) => self.run = None,
            Some(Action::Close) => ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close),
            Some(Action::KeepOpen) => {
                self.keep_open = true;
                self.close_at = None;
            }
            None => {}
        }
    }

    fn errors(&mut self, ui: &mut Ui) {
        let App { run, show_errors, .. } = self;
        let Some(run) = run else { return };
        let s = run.job.snapshot();
        if s.errors == 0 {
            return;
        }
        if run.errors.len() as u64 != s.errors.min(500) {
            run.errors = run.job.errors(500);
        }
        ui.add_space(14.0);
        w::card(ui, vec2(ui.available_width(), 0.0), |ui| {
            ui.horizontal(|ui| {
                ui.label(w::icon_label(icon::ERROR, 15.0, BAD));
                ui.label(RichText::new(format!("{} errors", fmt::count(s.errors))).family(semibold()).color(TEXT));
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if ui.button(if *show_errors { "Hide" } else { "Show" }).clicked() {
                        *show_errors = !*show_errors;
                    }
                    if w::icon_button(ui, icon::COPY, "Copy the error list").clicked() {
                        let text = run.errors.join("\r\n");
                        ui.output_mut(|o| o.copied_text = text);
                    }
                });
            });
            if *show_errors {
                egui::ScrollArea::vertical().max_height(150.0).auto_shrink([false, true]).show(ui, |ui| {
                    for e in &run.errors {
                        ui.label(RichText::new(e).size(12.0).color(MUTED));
                    }
                });
            }
        });
    }

    fn empty_paste_view(&mut self, ui: &mut Ui) {
        let full = ui.available_width();
        let mut close = false;
        w::card(ui, vec2(full, 0.0), |ui| {
            ui.vertical_centered(|ui| {
                ui.add_space(12.0);
                ui.label(w::icon_label(icon::PASTE, 36.0, ACCENT2));
                ui.add_space(4.0);
                ui.label(RichText::new("Nothing to paste").family(semibold()).size(18.0).color(TEXT));
                ui.label(
                    RichText::new("Right-click files or folders and choose \"Copy with SFC\" (or copy them with Ctrl+C), then use \"Paste with SFC\" again.")
                        .color(MUTED),
                );
                ui.add_space(10.0);
                close = w::ghost_button(ui, icon::CLOSE, "Close").clicked();
                ui.add_space(6.0);
            });
        });
        if close {
            ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close);
        }
    }

    fn drop_overlay(&self, ctx: &egui::Context) {
        if self.mode != Mode::Main || !ctx.input(|i| !i.raw.hovered_files.is_empty()) {
            return;
        }
        let rect = ctx.screen_rect();
        let p = ctx.layer_painter(egui::LayerId::new(egui::Order::Foreground, egui::Id::new("drop-overlay")));
        p.rect_filled(rect, 0.0, BG.gamma_multiply(0.9));
        w::dashed_rect(&p, rect.shrink(18.0), 18.0, stroke(2.0, ACCENT2));
        let c = rect.center();
        p.circle_filled(c - vec2(0.0, 30.0), 38.0, ACCENT.gamma_multiply(0.2));
        p.text(c - vec2(0.0, 30.0), Align2::CENTER_CENTER, icon::DROP, FontId::new(30.0, icons()), ACCENT2);
        p.text(c + vec2(0.0, 28.0), Align2::CENTER_CENTER, "Drop to add to the copy list", FontId::new(18.0, semibold()), TEXT);
    }
}

/// Time left, rounded the way people read a countdown.
fn eta_text(secs: f64) -> String {
    if secs < 1.0 {
        "almost done".into()
    } else if secs < 60.0 {
        format!("{:.0} s", secs.ceil())
    } else {
        fmt::duration(Duration::from_secs_f64(secs))
    }
}

fn lerp_warn() -> Color32 {
    w::lerp(WARN, BAD, 0.4)
}

/// One row in the source list; returns true when its remove button was clicked.
fn source_row(ui: &mut Ui, s: &Source) -> bool {
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 44.0), Sense::hover());
    let p = ui.painter().clone();
    let hovered = ui.rect_contains_pointer(rect);
    if hovered {
        p.rect_filled(rect, 10.0, CARD_HI);
    }
    let ic = Rect::from_min_size(rect.min + vec2(6.0, 7.0), vec2(30.0, 30.0));
    let (tint, glyph) = if s.is_dir { (ACCENT, icon::FOLDER) } else { (ACCENT2, icon::FILE) };
    p.rect_filled(ic, 8.0, tint.gamma_multiply(0.18));
    p.text(ic.center(), Align2::CENTER_CENTER, glyph, FontId::new(15.0, icons()), w::lerp(tint, Color32::WHITE, 0.3));

    let name = s.path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| s.path.display().to_string());
    let parent = s.path.parent().map(|p| p.display().to_string()).unwrap_or_default();
    let text_x = ic.right() + 10.0;
    let clip = Rect::from_min_max(pos2(text_x, rect.top()), pos2(rect.right() - 38.0, rect.bottom()));
    let tp = p.with_clip_rect(clip);
    tp.text(pos2(text_x, rect.top() + 5.0), Align2::LEFT_TOP, name, FontId::new(14.0, semibold()), TEXT);
    let max_chars = (clip.width() / 6.5) as usize;
    tp.text(pos2(text_x, rect.top() + 24.0), Align2::LEFT_TOP, elide_start(&parent, max_chars), FontId::proportional(11.5), MUTED);

    if !hovered {
        return false;
    }
    let x = Rect::from_center_size(pos2(rect.right() - 20.0, rect.center().y), vec2(28.0, 28.0));
    let resp = ui.interact(x, ui.id().with(("remove", &s.path)), Sense::click()).on_hover_cursor(CursorIcon::PointingHand);
    if resp.hovered() {
        p.rect_filled(x, 8.0, BAD.gamma_multiply(0.18));
    }
    p.text(x.center(), Align2::CENTER_CENTER, icon::CLOSE, FontId::new(11.0, icons()), if resp.hovered() { BAD } else { MUTED });
    resp.clicked()
}

fn style_titlebar(frame: &eframe::Frame) {
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};
    let Ok(handle) = frame.window_handle() else { return };
    if let RawWindowHandle::Win32(h) = handle.as_raw() {
        shell::style_titlebar(h.hwnd.get() as *mut std::ffi::c_void, [BG.r(), BG.g(), BG.b()], [TEXT.r(), TEXT.g(), TEXT.b()]);
    }
}

impl eframe::App for App {
    fn update(&mut self, ctx: &egui::Context, frame: &mut eframe::Frame) {
        self.frames += 1;
        if !self.styled_titlebar {
            self.styled_titlebar = true;
            style_titlebar(frame);
        }
        if self.logo.is_none() {
            let image = egui::ColorImage::from_rgba_unmultiplied([128, 128], LOGO);
            self.logo = Some(ctx.load_texture("logo", image, egui::TextureOptions::LINEAR));
        }
        // Wait for the window to be on screen before showing a modal dialog.
        if self.pick_and_start && self.frames > 2 {
            self.pick_and_start = false;
            if let Some(d) = rfd::FileDialog::new().set_title("Copy to…").pick_folder() {
                self.dest = d.display().to_string();
                self.start_now = true;
            }
        }
        if self.start_now {
            self.start_now = false;
            self.start();
        }

        let dropped: Vec<PathBuf> = ctx.input(|i| i.raw.dropped_files.iter().filter_map(|f| f.path.clone()).collect());
        if !dropped.is_empty() && self.mode == Mode::Main && !self.running() {
            self.run = None;
            self.add_sources(dropped);
        }
        if let Some(run) = &mut self.run {
            run.poll();
            let s = run.job.snapshot();
            run.sample(&s);
        }
        self.auto_close(ctx);
        if self.settings != self.saved {
            self.settings.save();
            self.saved = self.settings.clone();
        }

        let panel = egui::Frame::none().fill(BG).inner_margin(egui::Margin::symmetric(26.0, 20.0));
        egui::CentralPanel::default().frame(panel).show(ctx, |ui| {
            self.header(ui);
            ui.add_space(16.0);
            if self.run.is_some() {
                self.run_view(ui);
            } else if self.mode == Mode::Paste {
                self.empty_paste_view(ui);
            } else {
                self.setup_view(ui);
            }
        });
        if !self.running() {
            self.drop_overlay(ctx);
        }

        let title = match &self.run {
            Some(r) if r.running() && !r.job.snapshot().scan_done => "Scanning · Super Fast Copy".into(),
            Some(r) if r.running() => format!("{:.0}% · Super Fast Copy", r.shown * 100.0),
            _ => "Super Fast Copy".into(),
        };
        if title != self.title {
            ctx.send_viewport_cmd(egui::ViewportCommand::Title(title.clone()));
            self.title = title;
        }
        if self.running() || self.close_at.is_some() {
            ctx.request_repaint_after(Duration::from_millis(33));
        }
    }
}
