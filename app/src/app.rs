//! Application state and the per-frame loop.
//!
//! The panels themselves live in `ui`; this module owns the data they act on
//! and the rules that tie it together — which picture is current, what its crop
//! is, what the caption says, and what has been exported.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{channel, Receiver};
use std::sync::Arc;

use ab_glyph::FontVec;
use sort4print_core::config::{Config, NavMode};
use sort4print_core::cropbox::{CropBox, Handle};
use sort4print_core::export;
use sort4print_core::fonts::FontCatalog;
use sort4print_core::loader;

use crate::prefetch::{JobKind, LoadedImage, Prefetcher};

/// Per-picture state. Crop rectangles are stored in original-image pixels, so
/// they survive the preview being evicted from the cache and re-decoded.
pub struct Entry {
    pub path: PathBuf,
    pub selected: bool,
    pub crop: Option<CropBox>,
    /// Manual replacements for the geocoded values, when the nearest big town
    /// is not the name you want on the print.
    pub city_override: Option<String>,
    pub country_override: Option<String>,
    pub exported: bool,
}

impl Entry {
    fn new(path: PathBuf) -> Entry {
        Entry {
            path,
            selected: false,
            crop: None,
            city_override: None,
            country_override: None,
            exported: false,
        }
    }

    pub fn file_name(&self) -> String {
        self.path
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_default()
    }

    pub fn stem(&self) -> String {
        self.path
            .file_stem()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_default()
    }
}

pub struct DragState {
    pub handle: Handle,
    /// Pointer position at the last frame, in original-image pixels.
    pub last: (f64, f64),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SettingsTab {
    Caption,
    Date,
    Output,
    Performance,
    About,
}

pub enum ExportMsg {
    Ok { index: usize },
    Failed { index: usize, error: String },
    Finished,
}

/// One picture handed to the export thread. Carries everything the thread
/// needs so it never has to reach back into the UI state.
struct ExportJob {
    index: usize,
    path: PathBuf,
    stem: String,
    crop: Option<CropBox>,
    city_override: Option<String>,
    country_override: Option<String>,
}

pub struct ExportRun {
    pub rx: Receiver<ExportMsg>,
    pub total: usize,
    pub done: usize,
    pub failures: Vec<(String, String)>,
    pub running: bool,
}

pub struct Sort4Print {
    pub config: Config,
    pub config_path: PathBuf,
    pub config_dirty: bool,

    pub entries: Vec<Entry>,
    pub current: usize,

    pub font_catalog: Arc<FontCatalog>,
    caption_font: Option<Arc<FontVec>>,
    caption_font_key: (String, String),

    pub prefetch: Prefetcher,
    textures: HashMap<PathBuf, egui::TextureHandle>,
    thumb_textures: HashMap<PathBuf, egui::TextureHandle>,
    /// Rendered caption swatch for the settings panel, keyed by the settings it
    /// was rendered from so it is only rebuilt when something changes.
    pub caption_swatch: Option<(u64, egui::TextureHandle)>,
    /// The same idea for the caption drawn over the crop in the editor.
    pub caption_overlay: Option<(u64, egui::TextureHandle)>,
    /// Free-text print proportions box in the toolbar.
    pub ratio_input: String,
    /// Filter box above the font family list.
    pub font_filter: String,

    pub drag: Option<DragState>,
    pub show_settings: bool,
    pub settings_tab: SettingsTab,
    pub status: String,
    pub export_run: Option<ExportRun>,
    /// Free-text city search box in the location panel.
    pub place_search: String,
}

impl Sort4Print {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Sort4Print {
        // Each of these touches the filesystem and could be the thing that goes
        // wrong on a machine that is not this one, so each announces itself.
        let config_path = Config::default_path();
        crate::diagnostics::log(&format!("settings: {}", config_path.display()));
        let config = Config::load(&config_path).unwrap_or_default();

        // Reading every installed font is the slowest thing at startup — a
        // Windows font folder can be a couple of hundred megabytes — so the
        // time it took is worth knowing if the window ever seems to hang.
        let started = std::time::Instant::now();
        let catalog = FontCatalog::scan();
        crate::diagnostics::log(&format!(
            "fonts: {} faces in {} families, {} ms",
            catalog.len(),
            catalog.families().len(),
            started.elapsed().as_millis()
        ));

        let prefetch = Prefetcher::new(
            config.prefetch.workers,
            config.prefetch.cache,
            cc.egui_ctx.clone(),
        );
        let config_ratio_text = config.ratio.to_config_string();

        let mut app = Sort4Print {
            entries: Vec::new(),
            current: 0,
            font_catalog: Arc::new(catalog),
            caption_font: None,
            caption_font_key: (String::new(), String::new()),
            prefetch,
            textures: HashMap::new(),
            thumb_textures: HashMap::new(),
            caption_swatch: None,
            caption_overlay: None,
            ratio_input: config_ratio_text,
            font_filter: String::new(),
            drag: None,
            show_settings: true,
            settings_tab: SettingsTab::Caption,
            status: String::new(),
            export_run: None,
            place_search: String::new(),
            config_path,
            config_dirty: false,
            config,
        };

        if let Some(dir) = app.config.source_dir.clone() {
            if dir.is_dir() {
                app.open_folder(&dir);
            }
        }
        app
    }

    // ---- folder and selection -------------------------------------------

    pub fn open_folder(&mut self, dir: &Path) {
        match loader::scan_folder(dir) {
            Ok(files) => {
                self.entries = files.into_iter().map(Entry::new).collect();
                self.current = 0;
                self.clear_image_caches();
                self.config.source_dir = Some(dir.to_path_buf());
                self.config_dirty = true;
                self.status = format!(
                    "{} — {} picture{}",
                    dir.display(),
                    self.entries.len(),
                    if self.entries.len() == 1 { "" } else { "s" }
                );
            }
            Err(e) => self.status = format!("Could not open folder: {e:#}"),
        }
    }

    pub fn selected_count(&self) -> usize {
        self.entries.iter().filter(|e| e.selected).count()
    }

    pub fn exported_count(&self) -> usize {
        self.entries.iter().filter(|e| e.exported).count()
    }

    pub fn current_entry(&self) -> Option<&Entry> {
        self.entries.get(self.current)
    }

    pub fn toggle_current(&mut self) {
        if let Some(entry) = self.entries.get_mut(self.current) {
            entry.selected = !entry.selected;
        }
    }

    /// Steps through the folder, honouring the All / Only selected switch.
    /// Falls back to plain stepping when nothing is selected, so the switch can
    /// never strand you on a picture with no way forward.
    pub fn step(&mut self, delta: isize) {
        if self.entries.is_empty() {
            return;
        }
        let len = self.entries.len();
        let restrict = self.config.nav == NavMode::Selected && self.selected_count() > 0;

        let mut index = self.current as isize;
        for _ in 0..len {
            index += delta;
            if index < 0 {
                index = len as isize - 1;
            } else if index >= len as isize {
                index = 0;
            }
            if !restrict || self.entries[index as usize].selected {
                self.current = index as usize;
                return;
            }
        }
    }

    pub fn go_to(&mut self, index: usize) {
        if index < self.entries.len() {
            self.current = index;
        }
    }

    // ---- images ----------------------------------------------------------

    /// Queues decodes around the current position: the picture in view first,
    /// then outwards, so a forward walk is always already loaded.
    fn schedule_prefetch(&mut self) {
        if self.entries.is_empty() {
            return;
        }
        let max_px = self.config.preview_max_px;
        let ahead = self.config.prefetch.ahead as isize;
        let behind = self.config.prefetch.behind as isize;
        let len = self.entries.len() as isize;
        let current = self.current as isize;

        for offset in -behind..=ahead {
            let index = current + offset;
            if index < 0 || index >= len {
                continue;
            }
            // Priority is distance from the current picture; forward steps are
            // marginally cheaper than backward ones because that is the
            // direction people move through a folder.
            let priority = if offset >= 0 {
                offset as u32 * 2
            } else {
                (-offset) as u32 * 2 + 1
            };
            let path = self.entries[index as usize].path.clone();
            self.prefetch
                .request(&path, JobKind::Preview, max_px, priority);
        }
    }

    pub fn request_thumb(&mut self, index: usize) {
        let Some(entry) = self.entries.get(index) else {
            return;
        };
        let path = entry.path.clone();
        // Well below every preview job so thumbnails never delay the picture
        // actually being looked at.
        self.prefetch.request(&path, JobKind::Thumb, 220, 10_000);
    }

    /// Uploads (once) and returns the GPU texture for a preview.
    pub fn texture_for(
        &mut self,
        ctx: &egui::Context,
        path: &Path,
        image: &LoadedImage,
    ) -> egui::TextureHandle {
        if let Some(existing) = self.textures.get(path) {
            return existing.clone();
        }
        let handle = upload(ctx, &path.to_string_lossy(), &image.preview.rgba);
        self.textures.insert(path.to_path_buf(), handle.clone());
        // Textures are cheap to rebuild and expensive to hoard; keep roughly as
        // many as the decode cache holds.
        if self.textures.len() > self.config.prefetch.cache * 2 {
            self.textures.clear();
        }
        handle
    }

    pub fn thumb_texture_for(
        &mut self,
        ctx: &egui::Context,
        path: &Path,
        image: &LoadedImage,
    ) -> egui::TextureHandle {
        if let Some(existing) = self.thumb_textures.get(path) {
            return existing.clone();
        }
        let handle = upload(ctx, &format!("thumb:{}", path.to_string_lossy()), &image.preview.rgba);
        self.thumb_textures.insert(path.to_path_buf(), handle.clone());
        handle
    }

    /// Drops decoded images and their textures together. Anything that changes
    /// what a preview should look like has to clear both, or the old textures
    /// stay on screen at the wrong size.
    pub fn clear_image_caches(&mut self) {
        self.prefetch.clear();
        self.textures.clear();
        self.thumb_textures.clear();
        self.caption_overlay = None;
    }

    // ---- crop ------------------------------------------------------------

    /// Width divided by height for the current settings, given the shape of the
    /// image the crop is going on.
    pub fn ratio_for(&self, image_w: u32, image_h: u32) -> f64 {
        if self.config.ratio_follows_image {
            self.config.ratio.value(image_w >= image_h)
        } else {
            self.config.ratio.w / self.config.ratio.h
        }
    }

    /// The crop for a picture, created in-fit and centred the first time it is
    /// needed.
    pub fn crop_for(&mut self, index: usize, full_w: u32, full_h: u32) -> CropBox {
        let ratio = self.ratio_for(full_w, full_h);
        let entry = &mut self.entries[index];
        *entry.crop.get_or_insert_with(|| {
            CropBox::fit_centered(full_w as f64, full_h as f64, ratio)
        })
    }

    pub fn set_crop(&mut self, index: usize, crop: CropBox) {
        if let Some(entry) = self.entries.get_mut(index) {
            entry.crop = Some(crop);
        }
    }

    pub fn reset_crop(&mut self, index: usize) {
        if let Some(entry) = self.entries.get_mut(index) {
            entry.crop = None;
        }
    }

    /// Re-proportions every crop already made. Called when the print size
    /// changes so existing framing is kept rather than thrown away.
    pub fn reflow_crops(&mut self) {
        let follows = self.config.ratio_follows_image;
        let ratio = self.config.ratio;
        for entry in &mut self.entries {
            let Some(crop) = entry.crop else { continue };
            let r = if follows {
                ratio.value(crop.w >= crop.h)
            } else {
                ratio.w / ratio.h
            };
            entry.crop = Some(crop.with_ratio(r));
        }
    }

    // ---- caption ---------------------------------------------------------

    /// Loads the configured font, reloading it when the setting changes.
    /// `None` means no font could be found at all, in which case captions are
    /// silently skipped rather than blocking the export.
    pub fn caption_font(&mut self) -> Option<Arc<FontVec>> {
        let key = (
            self.config.caption.font_family.clone(),
            self.config.caption.font_style.clone(),
        );
        if self.caption_font.is_some() && self.caption_font_key == key {
            return self.caption_font.clone();
        }

        let face = self
            .font_catalog
            .best_match(&key.0, &key.1)
            .cloned();
        self.caption_font_key = key;
        self.caption_font = face.and_then(|f| {
            let data = f.read().ok()?;
            FontVec::try_from_vec_and_index(data, f.index).ok().map(Arc::new)
        });
        self.caption_font.clone()
    }

    pub fn caption_for(&self, index: usize, image: Option<&LoadedImage>) -> String {
        let Some(entry) = self.entries.get(index) else {
            return String::new();
        };
        let meta = image
            .map(|i| i.preview.meta.clone())
            .unwrap_or_default();
        export::caption_text(
            &self.config,
            &meta,
            image.and_then(|i| i.place.as_ref()),
            entry.city_override.as_deref(),
            entry.country_override.as_deref(),
            &entry.stem(),
        )
    }

    // ---- export ----------------------------------------------------------

    pub fn can_export(&self) -> bool {
        self.config.output_dir.is_some()
            && self.selected_count() > 0
            && self.export_run.as_ref().map(|r| !r.running).unwrap_or(true)
    }

    /// Exports every ticked picture on a background thread. Full-resolution
    /// decoding of a folder's worth of photos takes long enough that doing it
    /// on the UI thread would look like a hang.
    pub fn start_export(&mut self) {
        let Some(output_dir) = self.config.output_dir.clone() else {
            self.status = "Choose an output folder first.".into();
            return;
        };

        // Every selected picture needs a crop; ones never opened get the
        // default in-fit centred window, which needs the real image size.
        let jobs: Vec<ExportJob> = self
            .entries
            .iter()
            .enumerate()
            .filter(|(_, e)| e.selected)
            .map(|(i, e)| ExportJob {
                index: i,
                path: e.path.clone(),
                stem: e.stem(),
                crop: e.crop,
                city_override: e.city_override.clone(),
                country_override: e.country_override.clone(),
            })
            .collect();

        if jobs.is_empty() {
            self.status = "Nothing is selected.".into();
            return;
        }

        let config = self.config.clone();
        let font = self.caption_font();
        let ratio = self.config.ratio;
        let follows = self.config.ratio_follows_image;

        let (tx, rx) = channel();
        let total = jobs.len();

        std::thread::Builder::new()
            .name("sort4print-export".into())
            .spawn(move || {
                for job in jobs {
                    let ExportJob {
                        index,
                        path,
                        stem,
                        crop,
                        city_override,
                        country_override,
                    } = job;
                    let result = (|| -> anyhow::Result<()> {
                        let crop = match crop {
                            Some(c) => c,
                            None => {
                                // Nobody framed this one: read the real size and
                                // use the default in-fit centred window.
                                let (w, h) = image_size(&path)?;
                                let r = if follows {
                                    ratio.value(w >= h)
                                } else {
                                    ratio.w / ratio.h
                                };
                                CropBox::fit_centered(w as f64, h as f64, r)
                            }
                        };
                        // The caption is rebuilt here rather than on the UI
                        // thread so it is correct for pictures that were never
                        // opened and so never went through the preview cache.
                        let meta = sort4print_core::exif_data::read_meta(&path);
                        let place = meta
                            .gps
                            .and_then(|(lat, lon)| sort4print_core::geo::CityDb::embedded().nearest(lat, lon));
                        let caption = export::caption_text(
                            &config,
                            &meta,
                            place.as_ref(),
                            city_override.as_deref(),
                            country_override.as_deref(),
                            &stem,
                        );
                        export::export(
                            &path,
                            &output_dir,
                            &crop,
                            &config,
                            &caption,
                            font.as_deref(),
                        )?;
                        Ok(())
                    })();

                    let message = match result {
                        Ok(()) => ExportMsg::Ok { index },
                        Err(e) => ExportMsg::Failed {
                            index,
                            error: format!("{e:#}"),
                        },
                    };
                    if tx.send(message).is_err() {
                        return;
                    }
                }
                let _ = tx.send(ExportMsg::Finished);
            })
            .ok();

        self.export_run = Some(ExportRun {
            rx,
            total,
            done: 0,
            failures: Vec::new(),
            running: true,
        });
        self.status = format!("Exporting {total} picture{}…", if total == 1 { "" } else { "s" });
    }

    fn poll_export(&mut self) {
        let Some(run) = self.export_run.as_mut() else {
            return;
        };
        let mut finished = false;
        let mut completed: Vec<usize> = Vec::new();
        let mut failures: Vec<(usize, String)> = Vec::new();

        while let Ok(message) = run.rx.try_recv() {
            match message {
                ExportMsg::Ok { index } => {
                    run.done += 1;
                    completed.push(index);
                }
                ExportMsg::Failed { index, error } => {
                    run.done += 1;
                    failures.push((index, error));
                }
                ExportMsg::Finished => finished = true,
            }
        }

        for index in completed {
            if let Some(entry) = self.entries.get_mut(index) {
                entry.exported = true;
            }
        }
        for (index, error) in failures {
            let name = self
                .entries
                .get(index)
                .map(Entry::file_name)
                .unwrap_or_default();
            if let Some(run) = self.export_run.as_mut() {
                run.failures.push((name, error));
            }
        }

        if finished {
            if let Some(run) = self.export_run.as_mut() {
                run.running = false;
                let failed = run.failures.len();
                self.status = if failed == 0 {
                    format!("Exported {} picture(s).", run.done)
                } else {
                    format!("Exported {} picture(s), {failed} failed.", run.done - failed)
                };
            }
        }
    }

    // ---- frame -----------------------------------------------------------

    fn handle_keys(&mut self, ctx: &egui::Context) {
        // A text field has focus: leave the letters and arrows to it.
        if ctx.egui_wants_keyboard_input() {
            return;
        }
        let (next, prev, toggle, export_now) = ctx.input(|i| {
            (
                i.key_pressed(egui::Key::ArrowRight)
                    || i.key_pressed(egui::Key::PageDown)
                    || i.key_pressed(egui::Key::D),
                i.key_pressed(egui::Key::ArrowLeft)
                    || i.key_pressed(egui::Key::PageUp)
                    || i.key_pressed(egui::Key::A),
                i.key_pressed(egui::Key::Space),
                i.key_pressed(egui::Key::Enter),
            )
        });

        if next {
            self.step(1);
        }
        if prev {
            self.step(-1);
        }
        if toggle {
            self.toggle_current();
        }
        if export_now && self.can_export() {
            self.start_export();
        }
    }

    pub fn save_config(&mut self) {
        if let Err(e) = self.config.save(&self.config_path) {
            self.status = format!("Could not save settings: {e:#}");
        }
        self.config_dirty = false;
    }
}

impl eframe::App for Sort4Print {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        // Distinguishes "never got as far as painting" from "painted, but you
        // are looking at an empty window".
        static FIRST_FRAME: std::sync::Once = std::sync::Once::new();
        FIRST_FRAME.call_once(|| {
            crate::diagnostics::log(&format!(
                "first frame, available area {:?}",
                ui.available_size()
            ));
        });

        let ctx = ui.ctx().clone();

        self.prefetch.poll();
        self.poll_export();
        self.handle_keys(&ctx);
        self.schedule_prefetch();

        // Panels claim their edges in this order; the editor takes whatever is
        // left, so it has to come last.
        crate::ui::toolbar::show(self, ui);
        crate::ui::status_bar::show(self, ui);
        crate::ui::filmstrip::show(self, ui);
        if self.show_settings {
            crate::ui::settings::show(self, ui);
        }
        crate::ui::editor::show(self, ui);

        // An export in flight repaints so its progress moves; otherwise the
        // window sleeps until something happens.
        if self.export_run.as_ref().map(|r| r.running).unwrap_or(false) {
            ctx.request_repaint_after(std::time::Duration::from_millis(150));
        }
    }

    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        self.save_config();
    }
}

fn upload(ctx: &egui::Context, name: &str, rgba: &image::RgbaImage) -> egui::TextureHandle {
    let size = [rgba.width() as usize, rgba.height() as usize];
    let image = egui::ColorImage::from_rgba_unmultiplied(size, rgba.as_raw());
    ctx.load_texture(name, image, egui::TextureOptions::LINEAR)
}

/// Reads just the header to learn an image's upright dimensions, without
/// decoding it. Used for pictures exported without ever being opened.
fn image_size(path: &Path) -> anyhow::Result<(u32, u32)> {
    let meta = sort4print_core::exif_data::read_meta(path);
    let reader = image::ImageReader::open(path)?.with_guessed_format()?;
    let (w, h) = reader.into_dimensions()?;
    Ok(if meta.orientation.swaps_axes() {
        (h, w)
    } else {
        (w, h)
    })
}
