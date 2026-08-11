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
use sort4print_core::cropbox::{Constraints, CropBox, Handle};
use sort4print_core::export;
use sort4print_core::fonts::FontCatalog;
use sort4print_core::loader;
use sort4print_core::sidecar::{PhotoState, Sidecar};

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
    /// What you call this particular spot — "Chinatown". Stays with the photo,
    /// including across restarts via the folder's notes file.
    pub description: Option<String>,
    /// True once the crop has actually been moved. Merely opening a photo gives
    /// it the default centred window, and that is not worth recording: it is
    /// recomputed identically next time, and writing one for every photo walked
    /// past is what made the notes file grow with the folder rather than with
    /// the work done.
    pub crop_adjusted: bool,
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
            description: None,
            crop_adjusted: false,
            exported: false,
        }
    }

    fn to_state(&self) -> PhotoState {
        PhotoState {
            selected: self.selected,
            crop: if self.crop_adjusted { self.crop } else { None },
            city: self.city_override.clone(),
            country: self.country_override.clone(),
            description: self.description.clone(),
        }
    }

    fn apply_state(&mut self, state: &PhotoState) {
        self.selected = state.selected;
        self.crop = state.crop;
        // A crop that came back from the notes was deliberate by definition.
        self.crop_adjusted = state.crop.is_some();
        self.city_override = state.city.clone();
        self.country_override = state.country.clone();
        self.description = state.description.clone();
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

/// A frame's worth of keyboard state, read once so the input lock is taken
/// once rather than a dozen times.
struct Keys {
    alt: bool,
    shift: bool,
    right: bool,
    left: bool,
    up: bool,
    down: bool,
    page_down: bool,
    page_up: bool,
    d: bool,
    a: bool,
    space: bool,
    enter: bool,
}

pub struct DragState {
    pub handle: Handle,
    /// Where the pointer went down, in original-image pixels.
    pub start_pointer: (f64, f64),
    /// The window as it was when the drag began.
    ///
    /// A move is computed from these two every frame, rather than by adding
    /// this frame's pointer delta to the window's current position. That
    /// distinction is the whole difference between magnetism and glue: applying
    /// deltas to an already-snapped box means each small movement is undone by
    /// the snap that follows it, and the window can never leave an edge unless
    /// the pointer jumps past the snap radius within a single frame.
    pub start_box: CropBox,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SettingsTab {
    Caption,
    Date,
    Output,
    Performance,
    About,
}

/// A pass over the whole folder that fills the disk cache.
///
/// Deliberately a slow feed rather than one enormous queue: jobs are handed out
/// only as earlier ones finish, so the queue stays short, the photo actually
/// being looked at is never stuck behind the scan, and nothing has to be dropped
/// to keep the backlog bounded.
pub struct ScanAll {
    pub next: usize,
    pub total: usize,
}

impl ScanAll {
    pub fn fraction(&self) -> f32 {
        if self.total == 0 {
            1.0
        } else {
            self.next as f32 / self.total as f32
        }
    }
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
    description: Option<String>,
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
    /// Upload order, so the oldest texture can be dropped when the cache fills.
    texture_order: std::collections::VecDeque<PathBuf>,
    /// Thumbnails are tiny and there are never many on screen, so these are
    /// simply kept.
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
    /// View magnification, 1.0 being "everything fits". Ctrl+scroll changes it;
    /// it is about looking closely, and has nothing to do with the crop.
    pub view_zoom: f32,
    /// View offset in screen pixels, for panning once zoomed in.
    pub view_pan: egui::Vec2,
    /// Set when something other than a click moved the selection, so the
    /// filmstrip knows to bring the new row into view.
    pub scroll_to_current: bool,
    pub show_settings: bool,
    pub settings_tab: SettingsTab,
    pub status: String,
    pub export_run: Option<ExportRun>,
    /// Free-text city search box in the location panel.
    pub place_search: String,
    pub disk_cache: Option<sort4print_core::cache::DiskCache>,
    /// Long side of the editor area in real screen pixels, measured each frame.
    /// A laptop panel and an external monitor want different preview sizes, and
    /// this is how that gets noticed rather than guessed.
    pub editor_long_px: u32,
    /// A run through the whole folder filling the cache, so browsing afterwards
    /// waits for nothing. Holds the next index to queue.
    pub scan_all: Option<ScanAll>,
    /// Per-photo choices for the open folder, mirrored to a notes file there.
    notes: Sidecar,
    notes_dirty: bool,
    /// Set when the folder holds notes that could not be read. Saving is then
    /// refused for that folder, so an unreadable file is never replaced by an
    /// empty one.
    notes_blocked: bool,
    /// When the notes were last written, so a busy session does not rewrite a
    /// large file on every frame.
    last_notes_write: Option<std::time::Instant>,
}

impl Sort4Print {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Sort4Print {
        // Each of these touches the filesystem and could be the thing that goes
        // wrong on a machine that is not this one, so each announces itself.
        let config_path = Config::default_path();
        let existed = config_path.exists();
        let config = match Config::load(&config_path) {
            Ok(config) => config,
            Err(e) => {
                // Falling back to defaults here is what made a settings file
                // that could not be read look exactly like one that was never
                // written, so say which it was.
                crate::diagnostics::log(&format!(
                    "could not read {}: {e:#} — starting from defaults",
                    config_path.display()
                ));
                Config::default()
            }
        };
        crate::diagnostics::log(&format!(
            "settings: {} ({}), last folder {}",
            config_path.display(),
            if existed { "read" } else { "none yet" },
            config
                .source_dir
                .as_ref()
                .map(|p| p.display().to_string())
                .unwrap_or_else(|| "not remembered".into())
        ));

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

        let disk_cache = config.disk_cache();
        crate::diagnostics::log(&match &disk_cache {
            Some(cache) => format!(
                "image cache: {} ({} MB budget)",
                cache.root().display(),
                cache.budget_bytes() / (1024 * 1024)
            ),
            None => "image cache: off".to_string(),
        });

        let prefetch = Prefetcher::new(
            config.prefetch.workers,
            config.prefetch.cache,
            disk_cache.clone(),
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
            texture_order: std::collections::VecDeque::new(),
            thumb_textures: HashMap::new(),
            caption_swatch: None,
            caption_overlay: None,
            ratio_input: config_ratio_text,
            font_filter: String::new(),
            drag: None,
            view_zoom: 1.0,
            view_pan: egui::Vec2::ZERO,
            scroll_to_current: false,
            notes: Sidecar::default(),
            notes_dirty: false,
            notes_blocked: false,
            last_notes_write: None,
            show_settings: true,
            settings_tab: SettingsTab::Caption,
            status: String::new(),
            export_run: None,
            place_search: String::new(),
            disk_cache,
            editor_long_px: 0,
            scan_all: None,
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
        // Whatever was decided about the folder being left behind.
        self.save_notes_now();

        match loader::scan_folder(dir) {
            Ok(files) => {
                let report = Sidecar::load_detailed(dir);
                let notes = report.sidecar;
                let restored = notes.len();
                crate::diagnostics::log(&format!(
                    "notes: read {} photo(s) from {}{}",
                    restored,
                    report
                        .source
                        .as_ref()
                        .map(|p| p.display().to_string())
                        .unwrap_or_else(|| "nothing (no notes yet)".into()),
                    if report.incomplete {
                        " — no end marker, so written by an older version or cut short"
                    } else {
                        ""
                    }
                ));

                // A notes file we cannot make sense of must not be written
                // over: whatever it holds is unknown, and guessing cost a
                // folder's worth of decisions once already.
                self.notes_blocked = report.unreadable_file_present;
                if self.notes_blocked {
                    let path = Sidecar::path_for(dir);
                    crate::diagnostics::log(&format!(
                        "refusing to write notes: {} exists but could not be read",
                        path.display()
                    ));
                }

                // Notes are matched to photos by file name. Counting the
                // matches separately from the entries read is what would tell
                // a file that never arrived apart from one whose names do not
                // line up with what is actually in the folder.
                let mut matched = 0usize;
                self.entries = files
                    .into_iter()
                    .map(|path| {
                        let mut entry = Entry::new(path);
                        if let Some(state) = notes.get(&entry.file_name()) {
                            entry.apply_state(state);
                            matched += 1;
                        }
                        entry
                    })
                    .collect();
                if restored > 0 || matched > 0 {
                    crate::diagnostics::log(&format!(
                        "{matched} of {restored} note(s) matched a photo in the folder"
                    ));
                }

                // Back to the photo that was on screen last. If that file has
                // since gone, its old position puts you near where you were
                // rather than back at the start of the folder.
                self.current = notes
                    .last_file
                    .as_deref()
                    .and_then(|wanted| {
                        self.entries.iter().position(|e| e.file_name() == wanted)
                    })
                    .or_else(|| {
                        notes
                            .last_index
                            .map(|i| i.min(self.entries.len().saturating_sub(1)))
                    })
                    .unwrap_or(0);
                if self.current > 0 {
                    self.scroll_to_current = true;
                    crate::diagnostics::log(&format!(
                        "resuming at {} of {}",
                        self.current + 1,
                        self.entries.len()
                    ));
                }

                self.notes = notes;
                self.notes_dirty = false;
                self.reset_view();
                self.clear_image_caches();
                self.config.source_dir = Some(dir.to_path_buf());
                self.config_dirty = true;
                self.status = format!(
                    "{} — {} picture{}{}",
                    dir.display(),
                    self.entries.len(),
                    if self.entries.len() == 1 { "" } else { "s" },
                    if restored > 0 {
                        format!(", {restored} with notes from last time")
                    } else {
                        String::new()
                    }
                );
            }
            Err(e) => self.status = format!("Could not open folder: {e:#}"),
        }
    }

    /// Records that the per-photo state changed, so it gets written out.
    /// Cheap to call from anywhere a control is touched.
    pub fn note_changed(&mut self, index: usize) {
        let Some(entry) = self.entries.get(index) else {
            return;
        };
        let name = entry.file_name();
        let state = entry.to_state();
        self.notes.set(&name, state);
        self.notes_dirty = true;
    }

    /// Re-reads every entry into the notes. Used after bulk operations, where
    /// tracking each change individually would be more code than it is worth.
    pub fn notes_changed_everywhere(&mut self) {
        for index in 0..self.entries.len() {
            self.note_changed(index);
        }
    }

    /// Writes the notes if they have changed and enough time has passed.
    ///
    /// The throttle exists because a folder of eleven thousand photos makes the
    /// notes big enough that writing them on every frame of a crop drag would
    /// be real work. Anything that must not be lost calls `save_notes_now`.
    pub fn save_notes(&mut self) {
        const MIN_INTERVAL: std::time::Duration = std::time::Duration::from_secs(2);
        if !self.notes_dirty {
            return;
        }
        if let Some(last) = self.last_notes_write {
            if last.elapsed() < MIN_INTERVAL {
                return;
            }
        }
        self.save_notes_now();
    }

    pub fn save_notes_now(&mut self) {
        if !self.notes_dirty {
            return;
        }
        if self.notes_blocked {
            self.status =
                "Existing notes in this folder could not be read; not writing over them.".into();
            return;
        }
        let Some(dir) = self.config.source_dir.clone() else {
            return;
        };

        // Where you were is part of the notes, and is only ever needed at the
        // moment of writing them.
        self.notes.last_file = self.current_entry().map(Entry::file_name);
        self.notes.last_index = Some(self.current);
        self.last_notes_write = Some(std::time::Instant::now());
        let path = sort4print_core::sidecar::Sidecar::path_for(&dir);
        match self.notes.save(&dir) {
            Ok(()) => {
                self.notes_dirty = false;
                crate::diagnostics::log(&format!(
                    "notes written: {} ({} photos)",
                    path.display(),
                    self.notes.len()
                ));
            }
            Err(e) => {
                crate::diagnostics::log(&format!(
                    "could not write {}: {e:#}",
                    path.display()
                ));
                self.status = format!("Could not save notes: {e:#}");
            }
        }
    }

    pub fn reset_view(&mut self) {
        self.view_zoom = 1.0;
        self.view_pan = egui::Vec2::ZERO;
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
        self.note_changed(self.current);
    }

    pub fn current_is_selected(&self) -> bool {
        self.current_entry().map(|e| e.selected).unwrap_or(false)
    }

    /// Steps through the folder, honouring the All / Only selected switch.
    /// Falls back to plain stepping when nothing is selected, so the switch can
    /// never strand you on a picture with no way forward.
    pub fn step(&mut self, delta: isize) {
        if self.entries.is_empty() {
            return;
        }
        let len = self.entries.len();
        let nav = self.config.nav;
        // A walk that would visit nothing strands you with no way forward, so in
        // that case it degrades to plain stepping rather than doing nothing.
        let restrict = nav != NavMode::All
            && self.entries.iter().any(|e| nav.accepts(e.selected));

        let mut index = self.current as isize;
        for _ in 0..len {
            index += delta;
            if index < 0 {
                index = len as isize - 1;
            } else if index >= len as isize {
                index = 0;
            }
            if !restrict || nav.accepts(self.entries[index as usize].selected) {
                self.current = index as usize;
                // A close-up of one photo says nothing about the next.
                self.reset_view();
                // The list has to follow, or the picture being edited scrolls
                // out of sight after a few steps.
                self.scroll_to_current = true;
                // Where you got to is worth keeping, but only worth writing at
                // the throttled rate — hence dirty rather than an immediate save.
                self.notes_dirty = true;
                return;
            }
        }
    }

    pub fn go_to(&mut self, index: usize) {
        if index < self.entries.len() && index != self.current {
            self.current = index;
            self.reset_view();
        }
    }

    // ---- images ----------------------------------------------------------

    /// Queues decodes around the current position: the picture in view first,
    /// then outwards, so a forward walk is always already loaded.
    fn schedule_prefetch(&mut self) {
        if self.entries.is_empty() {
            return;
        }
        let max_px = self.preview_target_px();
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

    /// Starts a pass over the whole folder, filling the cache so that browsing
    /// afterwards waits for nothing.
    pub fn start_scan_all(&mut self) {
        if self.entries.is_empty() {
            return;
        }
        self.scan_all = Some(ScanAll {
            next: 0,
            total: self.entries.len(),
        });
        self.status = format!("Reading all {} pictures…", self.entries.len());
        crate::diagnostics::log(&format!("scan of {} pictures started", self.entries.len()));
    }

    pub fn stop_scan_all(&mut self) {
        if self.scan_all.take().is_some() {
            self.status = "Stopped reading.".into();
        }
    }

    /// Hands out a few more of the scan's photos when the workers have room.
    fn pump_scan_all(&mut self) {
        // Small enough that the queue never builds up, large enough to keep
        // every worker busy.
        const IN_FLIGHT: usize = 24;

        let Some(scan) = self.scan_all.as_ref() else {
            return;
        };
        let (mut next, total) = (scan.next, scan.total);
        let max_px = self.preview_target_px();

        while self.prefetch.queued() < IN_FLIGHT && next < total {
            let Some(entry) = self.entries.get(next) else {
                break;
            };
            let path = entry.path.clone();
            next += 1;
            // Far behind anything the user is waiting on.
            self.prefetch.request(&path, JobKind::Thumb, 220, 50_000);
            self.prefetch
                .request(&path, JobKind::Preview, max_px, 60_000);
        }

        let finished = next >= total && self.prefetch.queued() == 0;
        if let Some(scan) = self.scan_all.as_mut() {
            scan.next = next;
        }
        if finished {
            self.scan_all = None;
            let freed = self.disk_cache.as_ref().map(|c| c.prune()).unwrap_or(0);
            self.status = match self.disk_cache.as_ref() {
                Some(cache) => format!(
                    "Read all {total} pictures — cache now {} MB{}",
                    cache.total_bytes() / (1024 * 1024),
                    if freed > 0 {
                        format!(", {} MB dropped to stay in budget", freed / (1024 * 1024))
                    } else {
                        String::new()
                    }
                ),
                None => format!("Read all {total} pictures (cache is off)"),
            };
            crate::diagnostics::log(&self.status.clone());
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
        self.texture_order.push_back(path.to_path_buf());

        // Evict one at a time, oldest first. Emptying the whole map when it
        // filled up meant every visible photo had to be re-uploaded at once,
        // which is a stall you can feel; dropping the least recent costs
        // nothing noticeable.
        let limit = self.config.prefetch.cache.max(4);
        while self.texture_order.len() > limit {
            if let Some(oldest) = self.texture_order.pop_front() {
                if oldest != path {
                    self.textures.remove(&oldest);
                }
            }
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

    /// The size previews are decoded and cached at.
    ///
    /// Follows the editor area, rounded up to one of a handful of buckets and
    /// capped by the setting. Bucketing is what keeps this from making a new
    /// cache entry for every width the window has ever had, while still giving a
    /// laptop panel and a large monitor each a size that suits them.
    pub fn preview_target_px(&self) -> u32 {
        let cap = self.config.preview_max_px;
        if self.editor_long_px == 0 {
            return cap;
        }
        sort4print_core::cache::DiskCache::bucket_for(self.editor_long_px, cap)
    }

    /// Drops decoded images and their textures together. Anything that changes
    /// what a preview should look like has to clear both, or the old textures
    /// stay on screen at the wrong size.
    pub fn clear_image_caches(&mut self) {
        self.prefetch.clear();
        self.textures.clear();
        self.texture_order.clear();
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

    /// The photo's edges and how strongly the window sticks to them. The snap
    /// distance is given in image pixels but chosen in screen pixels, so it
    /// feels the same however far the view is zoomed in.
    pub fn constraints_for(&self, image_w: u32, image_h: u32, screen_per_image: f64) -> Constraints {
        const SNAP_SCREEN_PX: f64 = 9.0;
        let snap = if screen_per_image > 0.0 {
            SNAP_SCREEN_PX / screen_per_image
        } else {
            0.0
        };
        Constraints::new(image_w as f64, image_h as f64, snap)
    }

    pub fn set_crop(&mut self, index: usize, crop: CropBox) {
        let changed = match self.entries.get_mut(index) {
            Some(entry) if entry.crop != Some(crop) => {
                entry.crop = Some(crop);
                // This came from a gesture, so it is worth remembering.
                entry.crop_adjusted = true;
                true
            }
            _ => false,
        };
        if changed {
            self.note_changed(index);
        }
    }

    pub fn reset_crop(&mut self, index: usize) {
        if let Some(entry) = self.entries.get_mut(index) {
            entry.crop = None;
            entry.crop_adjusted = false;
        }
        self.note_changed(index);
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
            entry.description.as_deref(),
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
                description: e.description.clone(),
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
                        description,
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
                            description.as_deref(),
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

        // The notes describe exactly what is being exported; get them on disk
        // before a long job starts rather than after it, and rebuild them from
        // the entries so nothing missed by the incremental path is lost.
        self.notes_changed_everywhere();
        self.save_notes_now();

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
        let keys = ctx.input(|i| Keys {
            alt: i.modifiers.alt,
            shift: i.modifiers.shift,
            right: i.key_pressed(egui::Key::ArrowRight),
            left: i.key_pressed(egui::Key::ArrowLeft),
            up: i.key_pressed(egui::Key::ArrowUp),
            down: i.key_pressed(egui::Key::ArrowDown),
            page_down: i.key_pressed(egui::Key::PageDown),
            page_up: i.key_pressed(egui::Key::PageUp),
            d: i.key_pressed(egui::Key::D),
            a: i.key_pressed(egui::Key::A),
            space: i.key_pressed(egui::Key::Space),
            enter: i.key_pressed(egui::Key::Enter),
        });

        // Alt turns the arrows into a nudge of the crop window rather than a
        // step through the folder — the two gestures are close enough in intent
        // that sharing the keys reads naturally, and far enough apart that they
        // must not be confused.
        if keys.alt && (keys.left || keys.right || keys.up || keys.down) {
            let step = if keys.shift { 40.0 } else { 8.0 };
            let dx = f64::from(i32::from(keys.right) - i32::from(keys.left)) * step;
            let dy = f64::from(i32::from(keys.down) - i32::from(keys.up)) * step;
            self.nudge_crop(dx, dy);
            return;
        }

        if keys.right || keys.page_down || keys.d {
            self.step(1);
        }
        if keys.left || keys.page_up || keys.a {
            self.step(-1);
        }
        if keys.space {
            self.toggle_current();
        }
        if keys.enter && self.can_export() {
            self.start_export();
        }
    }

    /// Moves the crop window by whole image pixels, with the same sticking and
    /// bounds a drag would get.
    pub fn nudge_crop(&mut self, dx: f64, dy: f64) {
        let index = self.current;
        let Some(entry) = self.entries.get(index) else {
            return;
        };
        // Only a photo already measured can be nudged; anything else has no
        // crop yet and no dimensions to constrain one with.
        let path = entry.path.clone();
        let Some(image) = self.prefetch.preview(&path) else {
            return;
        };
        let (w, h) = (image.preview.full_w, image.preview.full_h);
        let crop = self.crop_for(index, w, h);
        // Nudges are in image pixels, so the snap distance is too.
        let constraints = Constraints::new(w as f64, h as f64, 6.0);
        self.set_crop(index, crop.apply_move(dx, dy, constraints));
    }

    pub fn save_config(&mut self) {
        match self.config.save(&self.config_path) {
            Ok(()) => crate::diagnostics::log(&format!(
                "settings written: {}",
                self.config_path.display()
            )),
            Err(e) => {
                crate::diagnostics::log(&format!(
                    "could not write {}: {e:#}",
                    self.config_path.display()
                ));
                self.status = format!("Could not save settings: {e:#}");
            }
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
            // Something is on screen, so start-up is over: from here a panic is
            // a real fault and should be put in front of the user rather than
            // treated as a backend that needs retrying.
            crate::diagnostics::mark_running();
        });

        let ctx = ui.ctx().clone();

        self.prefetch.poll();
        self.poll_export();
        self.handle_keys(&ctx);
        self.schedule_prefetch();
        self.pump_scan_all();

        // Panels claim their edges in this order; the editor takes whatever is
        // left, so it has to come last.
        crate::ui::toolbar::show(self, ui);
        crate::ui::status_bar::show(self, ui);
        crate::ui::filmstrip::show(self, ui);
        if self.show_settings {
            crate::ui::settings::show(self, ui);
        }
        crate::ui::editor::show(self, ui);

        // Both files are written once a gesture has finished rather than on
        // every frame of one, so a crop drag or a slider sweep is a single
        // write and not a hundred. Waiting for a clean exit is not enough:
        // settings should survive the program being killed too.
        let holding_something = self.drag.is_some() || ctx.input(|i| i.pointer.any_down());
        if !holding_something {
            self.save_notes();
            if self.config_dirty {
                self.save_config();
            }
        }

        // Work in flight repaints so its progress moves; otherwise the window
        // sleeps until something happens.
        let busy = self.export_run.as_ref().map(|r| r.running).unwrap_or(false)
            || self.scan_all.is_some();
        if busy {
            ctx.request_repaint_after(std::time::Duration::from_millis(150));
        }
    }

    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        crate::diagnostics::log("closing; writing settings and notes");
        // Rebuild the notes from what is actually on screen rather than
        // trusting that every control remembered to report its change. The
        // incremental path is the fast one; this is the one that has to be
        // right, and it only runs once.
        self.notes_changed_everywhere();
        self.config_dirty = true;
        self.save_config();
        self.save_notes_now();
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
