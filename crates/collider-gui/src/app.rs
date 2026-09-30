//! The application shell: panels, navigation, the URL bar and background bookkeeping.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use egui::{
    Align, Align2, CentralPanel, Color32, CursorIcon, Frame, Id, Key, KeyboardShortcut, Layout,
    Margin, Modifiers, Panel, Pos2, Rect, RichText, ScrollArea, Sense, Stroke, StrokeKind, Ui,
    Vec2, ViewportCommand,
};
use egui_phosphor::regular as ph;
use tokio::runtime::Handle;

use crate::format;
use crate::history::{History, Outcome, Record};
use crate::jobs::{self, Ffmpeg, Intent, Job, JobSpec, Phase, Probe};
use crate::settings::Settings;
use crate::theme::*;
use crate::views;
use crate::widgets::{self, Badge, Button, Size};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Page {
    Downloads,
    Watch,
    History,
    Settings,
    NewDownload,
    Job(u64),
}

/// A question the user must answer before something irreversible happens.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Confirm {
    Discard(u64),
    Quit,
}

/// Where settings and history are kept (`None`: not persisted).
#[derive(Debug, Clone, Default)]
pub struct Storage {
    pub settings: Option<PathBuf>,
    pub history: Option<PathBuf>,
}

impl Storage {
    /// `~/.config/collider/gui.toml` and `~/.local/share/collider/history.json`.
    pub fn user() -> Self {
        Self {
            settings: Settings::path(),
            history: History::default_path(),
        }
    }
}

pub struct Volume {
    pub dir: PathBuf,
    pub free: u64,
    pub total: u64,
}

pub struct App {
    pub rt: Handle,
    pub settings_path: Option<PathBuf>,
    pub settings: Settings,
    saved_settings: Settings,
    settings_changed: Option<Instant>,
    pub history: History,
    pub jobs: Vec<Job>,
    next_id: u64,
    pub page: Page,
    pub url: String,
    pub url_error: Option<String>,
    focus_url: bool,
    pub probe: Option<Probe>,
    /// The last successful probe: (url, time taken).
    pub probed: Option<(String, Duration)>,
    pub draft: Option<views::new_download::Draft>,
    /// Downloads filter tab: all, active, completed, failed.
    pub filter: usize,
    pub newest_first: bool,
    pub ffmpeg: Arc<Mutex<Option<Ffmpeg>>>,
    pub volume: Arc<Mutex<Option<Volume>>>,
    volume_checked: Option<Instant>,
    logo: Option<egui::TextureHandle>,
    pub confirm: Option<Confirm>,
    quitting: bool,
}

impl App {
    pub fn new(ctx: &egui::Context, rt: Handle, storage: Storage) -> Self {
        crate::theme::install(ctx);
        let settings = storage
            .settings
            .as_deref()
            .map(Settings::load_from)
            .unwrap_or_default();
        ctx.set_zoom_factor(settings.ui_scale);
        let logo = eframe::icon_data::from_png_bytes(include_bytes!("../assets/icon-64.png"))
            .ok()
            .map(|icon| {
                let image = egui::ColorImage::from_rgba_unmultiplied(
                    [icon.width as usize, icon.height as usize],
                    &icon.rgba,
                );
                ctx.load_texture("logo", image, egui::TextureOptions::LINEAR)
            });
        let app = Self {
            rt,
            saved_settings: settings.clone(),
            settings,
            settings_changed: None,
            settings_path: storage.settings,
            history: storage
                .history
                .as_deref()
                .map(History::load_from)
                .unwrap_or_default(),
            jobs: Vec::new(),
            next_id: 1,
            page: Page::Downloads,
            url: String::new(),
            url_error: None,
            focus_url: true,
            probe: None,
            probed: None,
            draft: None,
            filter: 0,
            newest_first: true,
            ffmpeg: Arc::new(Mutex::new(None)),
            volume: Arc::new(Mutex::new(None)),
            volume_checked: None,
            logo,
            confirm: None,
            quitting: false,
        };
        app.detect_ffmpeg(ctx);
        app
    }

    fn save_settings(&self) {
        if let Some(path) = &self.settings_path {
            if let Err(e) = self.settings.save_to(path) {
                eprintln!("collider: cannot save settings to {}: {e}", path.display());
            }
        }
    }

    pub fn detect_ffmpeg(&self, ctx: &egui::Context) {
        let (slot, ctx) = (self.ffmpeg.clone(), ctx.clone());
        std::thread::spawn(move || {
            let found = jobs::detect_ffmpeg();
            *slot.lock().unwrap_or_else(|e| e.into_inner()) = Some(found);
            ctx.request_repaint();
        });
    }

    pub fn ffmpeg_status(&self) -> Option<Ffmpeg> {
        self.ffmpeg
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }

    fn check_volume(&mut self, ctx: &egui::Context) {
        let due = self
            .volume_checked
            .is_none_or(|t| t.elapsed() > Duration::from_secs(10));
        if !due {
            return;
        }
        self.volume_checked = Some(Instant::now());
        let (dir, slot, ctx) = (
            self.settings.output_dir.clone(),
            self.volume.clone(),
            ctx.clone(),
        );
        std::thread::spawn(move || {
            // The output folder may not exist yet: measure its nearest existing parent.
            let mut probe_dir: &Path = &dir;
            while !probe_dir.exists() {
                match probe_dir.parent() {
                    Some(p) => probe_dir = p,
                    None => return,
                }
            }
            if let Ok(stats) = fs4::statvfs(probe_dir) {
                *slot.lock().unwrap_or_else(|e| e.into_inner()) = Some(Volume {
                    dir,
                    free: stats.available_space(),
                    total: stats.total_space(),
                });
                ctx.request_repaint();
            }
        });
    }

    pub fn job(&self, id: u64) -> Option<&Job> {
        self.jobs.iter().find(|j| j.id == id)
    }

    pub fn job_mut(&mut self, id: u64) -> Option<&mut Job> {
        self.jobs.iter_mut().find(|j| j.id == id)
    }

    /// A job spec from the settings, for `url` saved as `stem` in the output folder.
    pub fn default_spec(&self, url: &str, stem: &str) -> JobSpec {
        let s = &self.settings;
        JobSpec {
            url: url.to_string(),
            output: format::unique_output(&s.output_dir, stem, s.container.ext()),
            quality: s.quality.to_quality(),
            audio_lang: Some(s.audio_lang.trim().to_string()).filter(|l| !l.is_empty()),
            concurrency: s.concurrency,
            max_duration: None,
            wait: None,
            retries: s.retries,
            timeout: s.timeout(),
            headers: s.http_headers(),
        }
    }

    /// Add a job without starting it (demo scenes set its state by hand).
    #[cfg(test)]
    pub fn insert_job(&mut self, name: &str, spec: JobSpec) -> &Job {
        let id = self.next_id;
        self.next_id += 1;
        self.jobs.push(Job::new(id, name.to_string(), spec));
        self.jobs.last().expect("just pushed")
    }

    /// Add a job and start it at once.
    pub fn add_job(&mut self, ctx: &egui::Context, name: String, spec: JobSpec) -> u64 {
        let id = self.next_id;
        self.next_id += 1;
        let mut job = Job::new(id, name, spec);
        job.start(&self.rt, ctx);
        self.jobs.push(job);
        id
    }

    /// Validate the URL bar; on error, show it under the field.
    fn checked_url(&mut self) -> Option<String> {
        let url = self.url.trim().to_string();
        let error = if url.is_empty() {
            Some("Paste a link to an .m3u8 playlist or an .mpd manifest first.")
        } else {
            match url::Url::parse(&url) {
                Ok(u) if matches!(u.scheme(), "http" | "https") => None,
                _ => Some("That is not an http(s) link."),
            }
        };
        self.url_error = error.map(str::to_string);
        if error.is_some() {
            self.focus_url = true;
            None
        } else {
            Some(url)
        }
    }

    pub fn start_probe(&mut self, ctx: &egui::Context) {
        let Some(url) = self.checked_url() else {
            return;
        };
        self.probe = Some(Probe::start(
            &self.rt,
            ctx,
            url.clone(),
            &self.settings.http_headers(),
            self.settings.retries,
            self.settings.timeout(),
        ));
        self.draft = Some(views::new_download::Draft::probing(url, &self.settings));
        self.page = Page::NewDownload;
    }

    /// Add the URL bar's stream with the default settings, without probing first.
    pub fn quick_add(&mut self, ctx: &egui::Context) {
        let Some(url) = self.checked_url() else {
            return;
        };
        let name = format::stream_name(&url);
        let spec = self.default_spec(&url, &name);
        self.add_job(ctx, name, spec);
        self.url.clear();
        self.probed = None;
        self.page = Page::Downloads;
    }

    /// Stop and delete (after confirmation for anything with data).
    pub fn request_discard(&mut self, id: u64) {
        self.confirm = Some(Confirm::Discard(id));
    }

    pub fn discard(&mut self, id: u64) {
        let Some(pos) = self.jobs.iter().position(|j| j.id == id) else {
            return;
        };
        let running = self.jobs[pos].state().phase.is_running();
        if running {
            self.jobs[pos].stop(Intent::Discard);
        } else {
            self.jobs[pos].delete_parts(&self.rt);
        }
        self.jobs.remove(pos);
        if self.page == Page::Job(id) {
            self.page = Page::Downloads;
        }
    }

    pub fn remove(&mut self, id: u64) {
        self.jobs.retain(|j| j.id != id);
        if self.page == Page::Job(id) {
            self.page = Page::Downloads;
        }
    }

    pub fn open_folder(&self, path: &Path) {
        let dir = if path.is_dir() {
            path.to_path_buf()
        } else {
            path.parent().map(Path::to_path_buf).unwrap_or_default()
        };
        if let Err(e) = open::that_detached(&dir) {
            eprintln!("collider: cannot open {}: {e}", dir.display());
        }
    }

    /// Per-frame bookkeeping outside of drawing.
    fn tick(&mut self, ctx: &egui::Context) {
        if let Some(result) = self.probe.as_ref().and_then(Probe::take) {
            let probe = self.probe.take().expect("checked above");
            if let Ok((_, took)) = &result {
                self.probed = Some((probe.url.clone(), *took));
            }
            if let Some(draft) = self.draft.as_mut().filter(|d| d.url == probe.url) {
                draft.set_result(result, &self.settings);
            }
        }

        for job in &mut self.jobs {
            let restart = job.state().phase == Phase::Restarting;
            if restart && !self.quitting {
                job.start(&self.rt, ctx);
            }
        }

        for job in self.jobs.iter_mut().filter(|j| !j.archived) {
            let st = job.state();
            let outcome = match &st.phase {
                Phase::Completed { .. } => Outcome::Completed,
                Phase::Failed { .. } => Outcome::Failed,
                Phase::Unsupported { .. } => Outcome::Unsupported,
                _ => continue,
            };
            let record = Record {
                name: job.name.clone(),
                url: job.spec.url.clone(),
                outcome,
                files: match &st.phase {
                    Phase::Completed { files } => files.clone(),
                    _ => Vec::new(),
                },
                bytes: st.bytes(),
                live: st.live,
                finished: chrono::Local::now().timestamp(),
                detail: match &st.phase {
                    Phase::Failed { message } | Phase::Unsupported { message } => message.clone(),
                    _ => String::new(),
                },
            };
            drop(st);
            job.archived = true;
            self.history.push(record);
        }

        if self.settings != self.saved_settings {
            if self.settings.ui_scale != self.saved_settings.ui_scale {
                ctx.set_zoom_factor(self.settings.ui_scale);
            }
            if self.settings.output_dir != self.saved_settings.output_dir {
                self.volume_checked = None;
            }
            self.saved_settings = self.settings.clone();
            self.settings_changed = Some(Instant::now());
        }
        if let Some(t) = self.settings_changed {
            if t.elapsed() > Duration::from_millis(400) {
                self.settings_changed = None;
                self.save_settings();
            } else {
                ctx.request_repaint_after(Duration::from_millis(450));
            }
        }

        self.check_volume(ctx);

        let running = self.jobs.iter().any(|j| j.state().phase.is_running());
        if ctx.input(|i| i.viewport().close_requested()) && running && !self.quitting {
            ctx.send_viewport_cmd(ViewportCommand::CancelClose);
            self.confirm = Some(Confirm::Quit);
        }
        if self.quitting && !running {
            self.save_settings();
            ctx.send_viewport_cmd(ViewportCommand::Close);
        }
        if running {
            // Clocks, countdowns and rates move even without new events.
            ctx.request_repaint_after(Duration::from_millis(500));
        }
    }

    fn shortcuts(&mut self, ctx: &egui::Context) {
        let focus = KeyboardShortcut::new(Modifiers::COMMAND, Key::L);
        if ctx.input_mut(|i| i.consume_shortcut(&focus)) {
            self.focus_url = true;
        }
        let typing = ctx.memory(|m| m.focused().is_some());
        if !typing
            && self.confirm.is_none()
            && ctx.input(|i| i.key_pressed(Key::Escape))
            && matches!(self.page, Page::NewDownload | Page::Job(_))
        {
            self.page = Page::Downloads;
        }
    }

    pub fn counts(&self) -> Counts {
        let mut c = Counts::default();
        for j in &self.jobs {
            let st = j.state();
            match &st.phase {
                Phase::Waiting { .. } => c.watching += 1,
                Phase::Running | Phase::Stopping | Phase::Starting | Phase::Remuxing if st.live => {
                    c.recording += 1
                }
                Phase::Running | Phase::Stopping | Phase::Starting | Phase::Remuxing => {
                    c.downloading += 1
                }
                _ => {}
            }
            if !st.phase.is_finished() {
                c.open += 1;
            }
            if j.spec.wait.is_some() && !st.phase.is_finished() {
                c.watch_list += 1;
            }
            if st.phase.is_running() {
                c.rate += st.rate.current(Instant::now());
            }
        }
        c
    }

    fn status_bar(&self, ui: &mut Ui) {
        let r = ui.max_rect();
        ui.painter()
            .hline(r.x_range(), r.top() + 0.5, Stroke::new(1.0, LINE));
        let c = self.counts();
        ui.horizontal_centered(|ui| {
            ui.add_space(18.0);
            ui.spacing_mut().item_spacing.x = 6.0;
            ui.label(widgets::icon_text(ph::DOWNLOAD_SIMPLE, 13.0, TX2));
            ui.label(RichText::new("Total").font(small()).color(TX2));
            ui.label(
                RichText::new(format::speed(c.rate))
                    .font(mono(12.0))
                    .color(TX1),
            );
            for (n, what, color) in [
                (c.downloading, "downloading", P300),
                (c.recording, "recording", AC),
                (c.watching, "watching", INFO),
            ] {
                separator(ui);
                dot(ui, color);
                ui.label(
                    RichText::new(format!("{n} {what}"))
                        .font(small())
                        .color(TX2),
                );
            }
            separator(ui);
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                ui.add_space(18.0);
                match self.ffmpeg_status() {
                    None => {
                        ui.label(
                            RichText::new("looking for ffmpeg…")
                                .font(small())
                                .color(TX3),
                        );
                    }
                    Some(Ffmpeg {
                        path: Some(path),
                        version,
                    }) => {
                        ui.label(
                            RichText::new(format::tilde(&path))
                                .font(mono(12.0))
                                .color(TX1),
                        );
                        separator(ui);
                        ui.label(RichText::new("detected").font(small()).color(TX2));
                        ui.label(
                            RichText::new(format!("ffmpeg {}", version.as_deref().unwrap_or("")))
                                .font(small())
                                .color(OK),
                        );
                        ui.label(widgets::icon_text(ph::CHECK_CIRCLE, 14.0, OK));
                    }
                    Some(_) => {
                        ui.label(
                            RichText::new("ffmpeg not found: raw files are saved")
                                .font(small())
                                .color(WARN),
                        );
                        ui.label(widgets::icon_text(ph::WARNING, 14.0, WARN));
                    }
                }
            });
        });
    }

    fn sidebar(&mut self, ui: &mut Ui, rail: bool) {
        let panel = ui.max_rect();
        ui.painter()
            .vline(panel.right() - 0.5, panel.y_range(), Stroke::new(1.0, LINE));
        let c = self.counts();
        let pad = 12.0;
        // Wordmark.
        let top = Rect::from_min_size(panel.min, Vec2::new(panel.width(), 68.0));
        let mark = Rect::from_center_size(
            Pos2::new(
                if rail {
                    top.center().x
                } else {
                    top.left() + 20.0 + 15.0
                },
                top.center().y,
            ),
            Vec2::splat(30.0),
        );
        if let Some(logo) = &self.logo {
            ui.painter().image(
                logo.id(),
                mark,
                Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)),
                Color32::WHITE,
            );
        }
        if !rail {
            ui.painter().text(
                Pos2::new(mark.right() + 11.0, top.center().y),
                Align2::LEFT_CENTER,
                "collider",
                strong(19.0),
                TX1,
            );
            ui.painter().text(
                Pos2::new(top.right() - 20.0, top.center().y + 1.0),
                Align2::RIGHT_CENTER,
                env!("CARGO_PKG_VERSION"),
                mono(11.0),
                TX3,
            );
        }
        let mut y = top.bottom() + 6.0;
        if !rail {
            ui.painter().text(
                Pos2::new(panel.left() + 22.0, y + 6.0),
                Align2::LEFT_CENTER,
                "LIBRARY",
                label(),
                TX3,
            );
            y += 22.0;
        }
        let items: [(Page, &str, &str, Option<usize>); 4] = [
            (
                Page::Downloads,
                ph::DOWNLOAD_SIMPLE,
                "Downloads",
                Some(c.open),
            ),
            (
                Page::Watch,
                ph::BINOCULARS,
                "Watch list",
                Some(c.watch_list),
            ),
            (Page::History, ph::CLOCK_COUNTER_CLOCKWISE, "History", None),
            (Page::Settings, ph::GEAR_SIX, "Settings", None),
        ];
        for (page, icon_str, text, count) in items {
            let item = Rect::from_min_size(
                Pos2::new(panel.left() + pad, y),
                Vec2::new(panel.width() - 2.0 * pad, 38.0),
            );
            let resp = ui
                .interact(item, Id::new(("nav", text)), Sense::click())
                .on_hover_cursor(CursorIcon::PointingHand);
            resp.widget_info(|| {
                egui::WidgetInfo::selected(egui::WidgetType::Button, true, self.page == page, text)
            });
            let resp = if rail { resp.on_hover_text(text) } else { resp };
            let active = self.page == page
                || (page == Page::Downloads
                    && matches!(self.page, Page::NewDownload | Page::Job(_)));
            let p = ui.painter();
            if active {
                p.rect(item, 8, P750, Stroke::new(1.0, LINE2), StrokeKind::Inside);
                p.rect_filled(
                    Rect::from_min_size(
                        Pos2::new(panel.left(), item.top() + 7.0),
                        Vec2::new(3.0, item.height() - 14.0),
                    ),
                    1,
                    AC,
                );
            } else if resp.hovered() {
                p.rect_filled(item, 8, P800);
            }
            let icon_x = if rail {
                item.center().x
            } else {
                item.left() + 20.0
            };
            p.text(
                Pos2::new(icon_x, item.center().y),
                Align2::CENTER_CENTER,
                icon_str,
                icon(18.0),
                if active { AC } else { TX2 },
            );
            let n = count.filter(|n| *n > 0);
            if rail {
                if let Some(n) = n {
                    let b = Pos2::new(icon_x + 11.0, item.top() + 9.0);
                    p.circle_filled(b, 8.0, if active { P650 } else { P700 });
                    p.text(b, Align2::CENTER_CENTER, n.to_string(), mono(10.0), TX1);
                }
            } else {
                p.text(
                    Pos2::new(item.left() + 39.0, item.center().y),
                    Align2::LEFT_CENTER,
                    text,
                    medium(14.0),
                    if active { TX1 } else { TX2 },
                );
                if let Some(n) = n {
                    let pill = Rect::from_center_size(
                        Pos2::new(item.right() - 22.0, item.center().y),
                        Vec2::new(24.0, 20.0),
                    );
                    p.rect_filled(pill, 10, if active { P650 } else { P800 });
                    p.text(
                        pill.center(),
                        Align2::CENTER_CENTER,
                        n.to_string(),
                        mono(11.5),
                        TX2,
                    );
                }
            }
            if resp.clicked() {
                self.page = page;
            }
            y += 42.0;
        }

        // Output volume card.
        if !rail {
            if let Some(v) = &*self.volume.lock().unwrap_or_else(|e| e.into_inner()) {
                let card = Rect::from_min_max(
                    Pos2::new(panel.left() + 12.0, panel.bottom() - 92.0),
                    Pos2::new(panel.right() - 12.0, panel.bottom() - 14.0),
                );
                let p = ui.painter();
                p.rect(card, 10, P850, Stroke::new(1.0, LINE2), StrokeKind::Inside);
                p.text(
                    Pos2::new(card.left() + 13.0, card.top() + 20.0),
                    Align2::LEFT_CENTER,
                    "Output volume",
                    small(),
                    TX2,
                );
                p.text(
                    Pos2::new(card.right() - 13.0, card.top() + 20.0),
                    Align2::RIGHT_CENTER,
                    format!("{} free", format::bytes(v.free)),
                    mono(12.0),
                    TX1,
                );
                let used = 1.0 - v.free as f32 / v.total.max(1) as f32;
                widgets::paint_progress(
                    ui,
                    Rect::from_min_size(
                        Pos2::new(card.left() + 13.0, card.top() + 36.0),
                        Vec2::new(card.width() - 26.0, 4.0),
                    ),
                    &[(used, P400)],
                );
                let path = format::tilde(&v.dir);
                ui.painter().text(
                    Pos2::new(card.left() + 13.0, card.top() + 58.0),
                    Align2::LEFT_CENTER,
                    truncate_middle(&path, 30),
                    mono(11.0),
                    TX3,
                );
            }
        }
    }

    fn top_bar(&mut self, ui: &mut Ui) {
        let r = ui.max_rect();
        let ctx = ui.ctx().clone();
        ui.painter()
            .hline(r.x_range(), r.bottom() + 11.5, Stroke::new(1.0, LINE));
        ui.horizontal_centered(|ui| {
            ui.spacing_mut().item_spacing.x = 12.0;
            let field_w = (ui.available_width() - 222.0).max(240.0);
            let enter = self.url_field(ui, field_w);
            let probing = self.probe.is_some();
            let probe = ui
                .add(
                    Button::secondary(if probing { "Probing…" } else { "Probe" })
                        .icon(ph::MAGNIFYING_GLASS)
                        .size(Size::Large)
                        .min_width(108.0)
                        .enabled(!probing),
                )
                .on_hover_text("Read the stream and choose quality, audio and file name (Enter)");
            let add = ui
                .add(
                    Button::primary("Add")
                        .icon(ph::PLUS)
                        .size(Size::Large)
                        .min_width(90.0),
                )
                .on_hover_text("Download right away with the default settings");
            if (probe.clicked() || enter) && !probing {
                self.start_probe(&ctx);
            }
            if add.clicked() {
                self.quick_add(&ctx);
            }
        });
    }

    /// The URL field; returns true when Enter was pressed in it.
    fn url_field(&mut self, ui: &mut Ui, width: f32) -> bool {
        let (rect, _) = ui.allocate_exact_size(Vec2::new(width, 40.0), Sense::hover());
        let id = Id::new("url-field");
        let focused = ui.memory(|m| m.has_focus(id));
        let error = self.url_error.is_some();
        ui.painter().rect(
            rect,
            8,
            P900,
            if focused {
                Stroke::new(2.0, FOCUS)
            } else if error {
                Stroke::new(1.0, BAD)
            } else {
                Stroke::new(1.0, P400)
            },
            StrokeKind::Inside,
        );
        ui.painter().text(
            Pos2::new(rect.left() + 18.0, rect.center().y),
            Align2::CENTER_CENTER,
            ph::LINK,
            icon(16.0),
            TX3,
        );
        // Trailing hint: the probe result for this URL, or the shortcut.
        let probed = self
            .probed
            .as_ref()
            .filter(|(u, _)| *u == self.url.trim())
            .map(|(_, t)| format!("Probed · {} ms", t.as_millis()));
        let hint_w = match &probed {
            Some(text) => {
                let size = widgets::badge_size(ui, text, Badge::Ok);
                widgets::paint_badge(
                    ui,
                    Pos2::new(rect.right() - 12.0 - size.x, rect.center().y - 10.0),
                    text,
                    Badge::Ok,
                );
                size.x + 20.0
            }
            None => {
                let mut x = rect.right() - 12.0;
                for key in ["L", "Ctrl"] {
                    let g = ui.painter().layout_no_wrap(key.into(), mono(10.5), TX2);
                    let cap = Rect::from_min_max(
                        Pos2::new(x - g.size().x - 10.0, rect.center().y - 10.0),
                        Pos2::new(x, rect.center().y + 10.0),
                    );
                    ui.painter()
                        .rect(cap, 4, P850, Stroke::new(1.0, LINE2), StrokeKind::Inside);
                    ui.painter().galley(
                        Pos2::new(cap.left() + 5.0, cap.center().y - g.size().y / 2.0),
                        g,
                        TX2,
                    );
                    x = cap.left() - 5.0;
                }
                rect.right() - x + 8.0
            }
        };
        let text_rect = Rect::from_min_max(
            Pos2::new(rect.left() + 36.0, rect.top() + 2.0),
            Pos2::new(rect.right() - hint_w, rect.bottom() - 2.0),
        );
        // A child UI, so the text box does not move the parent's cursor (`ui.put` would,
        // and the buttons after the field would then overlap it).
        let mut child = ui.new_child(
            egui::UiBuilder::new()
                .max_rect(text_rect)
                .layout(Layout::left_to_right(Align::Center)),
        );
        let resp = child.add(
            egui::TextEdit::singleline(&mut self.url)
                .id(id)
                .frame(Frame::NONE)
                .hint_text(
                    RichText::new("Paste an HLS or DASH URL (.m3u8 or .mpd)")
                        .color(TX3)
                        .font(body()),
                )
                .font(mono(13.0))
                .text_color(TX1)
                .desired_width(text_rect.width())
                .vertical_align(Align::Center),
        );
        if resp.changed() {
            self.url_error = None;
        }
        if self.focus_url {
            self.focus_url = false;
            resp.request_focus();
        }
        if let Some(err) = &self.url_error {
            let tip = Rect::from_min_size(
                Pos2::new(rect.left(), rect.bottom() + 4.0),
                Vec2::new(rect.width(), 16.0),
            );
            ui.painter()
                .text(tip.left_center(), Align2::LEFT_CENTER, err, small(), BAD);
        }
        resp.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter))
    }

    fn modals(&mut self, ctx: &egui::Context) {
        let Some(confirm) = self.confirm else {
            if self.quitting {
                let n = self
                    .jobs
                    .iter()
                    .filter(|j| j.state().phase.is_running())
                    .count();
                egui::Modal::new(Id::new("quitting"))
                    .frame(modal_frame())
                    .show(ctx, |ui| {
                        ui.set_width(360.0);
                        ui.horizontal(|ui| {
                            widgets::spinner(ui, 18.0);
                            ui.label(
                                RichText::new("Saving and stopping…")
                                    .font(card_title())
                                    .color(TX1),
                            );
                        });
                        ui.label(
                            RichText::new(format!(
                                "Waiting for {n} job(s) to finish their segments in flight."
                            ))
                            .font(small())
                            .color(TX2),
                        );
                    });
            }
            return;
        };
        let mut close = false;
        match confirm {
            Confirm::Discard(id) => {
                let Some(job) = self.job(id) else {
                    self.confirm = None;
                    return;
                };
                let (live, bytes, running) = {
                    let st = job.state();
                    (st.live, st.bytes(), st.phase.is_running())
                };
                let name = job.name.clone();
                let mut discard = false;
                let resp = egui::Modal::new(Id::new("discard"))
                    .frame(modal_frame())
                    .show(ctx, |ui| {
                        ui.set_width(420.0);
                        ui.label(
                            RichText::new(format!("Cancel {name}?"))
                                .font(title())
                                .color(TX1),
                        );
                        ui.add_space(4.0);
                        let what = if live { "recording" } else { "download" };
                        let keep = if live {
                            "To keep it, use Stop and save instead."
                        } else if running {
                            "Pause keeps the segments so it can resume later."
                        } else {
                            "The finished segments are kept until then."
                        };
                        ui.label(
                            RichText::new(format!(
                                "The {what} so far ({}) will be deleted. {keep}",
                                format::bytes(bytes)
                            ))
                            .font(body())
                            .color(TX2),
                        );
                        ui.add_space(14.0);
                        ui.horizontal(|ui| {
                            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                if ui.add(Button::danger("Delete").icon(ph::TRASH)).clicked() {
                                    discard = true;
                                }
                                if ui.add(Button::secondary("Keep")).clicked() {
                                    close = true;
                                }
                            });
                        });
                    });
                if resp.should_close() {
                    close = true;
                }
                if discard {
                    self.discard(id);
                    close = true;
                }
            }
            Confirm::Quit => {
                let running: Vec<(String, bool)> = self
                    .jobs
                    .iter()
                    .filter_map(|j| {
                        let st = j.state();
                        st.phase.is_running().then(|| (j.name.clone(), st.live))
                    })
                    .collect();
                let mut quit = false;
                let resp = egui::Modal::new(Id::new("quit"))
                    .frame(modal_frame())
                    .show(ctx, |ui| {
                        ui.set_width(440.0);
                        ui.label(
                            RichText::new(format!("{} job(s) are still running", running.len()))
                                .font(title())
                                .color(TX1),
                        );
                        ui.add_space(4.0);
                        ui.label(
                            RichText::new(
                                "Quitting saves live recordings and pauses downloads. A paused \
                                 download resumes when you add the same link again.",
                            )
                            .font(body())
                            .color(TX2),
                        );
                        ui.add_space(6.0);
                        for (name, live) in &running {
                            ui.horizontal(|ui| {
                                ui.label(widgets::icon_text(
                                    if *live {
                                        ph::RECORD
                                    } else {
                                        ph::DOWNLOAD_SIMPLE
                                    },
                                    14.0,
                                    if *live { AC } else { TX2 },
                                ));
                                ui.label(RichText::new(name).font(mono(12.5)).color(TX1));
                            });
                        }
                        ui.add_space(14.0);
                        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                            if ui.add(Button::primary("Stop and quit")).clicked() {
                                quit = true;
                            }
                            if ui.add(Button::secondary("Keep running")).clicked() {
                                close = true;
                            }
                        });
                    });
                if resp.should_close() {
                    close = true;
                }
                if quit {
                    for job in &mut self.jobs {
                        let live = job.state().live;
                        job.stop(if live { Intent::Save } else { Intent::Pause });
                    }
                    self.quitting = true;
                    close = true;
                }
            }
        }
        if close {
            self.confirm = None;
        }
    }
}

#[derive(Default)]
pub struct Counts {
    pub open: usize,
    pub downloading: usize,
    pub recording: usize,
    pub watching: usize,
    pub watch_list: usize,
    pub rate: f64,
}

fn modal_frame() -> Frame {
    Frame::new()
        .fill(P800)
        .stroke(Stroke::new(1.0, LINE2))
        .corner_radius(12)
        .shadow(card_shadow())
        .inner_margin(Margin::same(22))
}

fn separator(ui: &mut Ui) {
    let (r, _) = ui.allocate_exact_size(Vec2::new(13.0, 16.0), Sense::hover());
    ui.painter()
        .vline(r.center().x, r.y_range(), Stroke::new(1.0, LINE));
}

fn dot(ui: &mut Ui, color: Color32) {
    let (r, _) = ui.allocate_exact_size(Vec2::new(8.0, 8.0), Sense::hover());
    ui.painter().circle_filled(r.center(), 3.5, color);
}

pub fn truncate_middle(s: &str, max: usize) -> String {
    let n = s.chars().count();
    if n <= max {
        return s.to_string();
    }
    let keep = max.saturating_sub(1);
    let head: String = s.chars().take(keep / 2).collect();
    let tail: String = s.chars().skip(n - (keep - keep / 2)).collect();
    format!("{head}…{tail}")
}

impl eframe::App for App {
    fn ui(&mut self, ui: &mut Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        self.tick(&ctx);
        self.shortcuts(&ctx);
        let rail = ctx.content_rect().width() < 1300.0;

        Panel::bottom("status")
            .exact_size(30.0)
            .resizable(false)
            .show_separator_line(false)
            .frame(Frame::new().fill(P950))
            .show(ui, |ui| self.status_bar(ui));
        Panel::left("nav")
            .exact_size(if rail { 64.0 } else { 232.0 })
            .resizable(false)
            .show_separator_line(false)
            .frame(Frame::new().fill(P900))
            .show(ui, |ui| self.sidebar(ui, rail));
        Panel::top("top")
            .exact_size(64.0)
            .resizable(false)
            .show_separator_line(false)
            .frame(
                Frame::new()
                    .fill(P850)
                    .inner_margin(Margin::symmetric(24, 12)),
            )
            .show(ui, |ui| self.top_bar(ui));
        CentralPanel::default()
            .frame(Frame::new().fill(P850))
            .show(ui, |ui| {
                ScrollArea::vertical()
                    .id_salt(("page", format!("{:?}", self.page)))
                    .auto_shrink(false)
                    .show(ui, |ui| {
                        Frame::new()
                            .inner_margin(Margin {
                                left: 24,
                                right: 24,
                                top: 20,
                                bottom: 24,
                            })
                            .show(ui, |ui| match self.page {
                                Page::Downloads => views::downloads::page(self, ui, false),
                                Page::Watch => views::downloads::page(self, ui, true),
                                Page::History => views::history::page(self, ui),
                                Page::Settings => views::settings::page(self, ui),
                                Page::NewDownload => views::new_download::page(self, ui),
                                Page::Job(id) => views::details::page(self, ui, id),
                            });
                    });
            });
        self.modals(&ctx);
    }

    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        self.save_settings();
    }
}
