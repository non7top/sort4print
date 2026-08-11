//! Persistent settings, stored as an ini file next to the executable.
//!
//! Portable-first: a `sort4print.ini` sitting beside the exe wins, so the tool
//! can live on a stick. If that directory is not writable (Program Files, a
//! read-only share) the per-user location is used instead.
//!
//! Everything the GUI can set is representable in the file, and everything in
//! the file is reachable from the GUI, with one deliberate exception: extra
//! `[locale.*]` sections are file-only, since defining month-name tables is a
//! job for a text editor rather than a dialog.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

use crate::datefmt::{Locale, Locales};
use crate::ini::Ini;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Corner {
    TopLeft,
    TopRight,
    BottomLeft,
    BottomRight,
}

impl Corner {
    pub const ALL: [Corner; 4] = [
        Corner::TopLeft,
        Corner::TopRight,
        Corner::BottomLeft,
        Corner::BottomRight,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            Corner::TopLeft => "top-left",
            Corner::TopRight => "top-right",
            Corner::BottomLeft => "bottom-left",
            Corner::BottomRight => "bottom-right",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Corner::TopLeft => "Top left",
            Corner::TopRight => "Top right",
            Corner::BottomLeft => "Bottom left",
            Corner::BottomRight => "Bottom right",
        }
    }

    pub fn parse(s: &str) -> Option<Corner> {
        match s.trim().to_ascii_lowercase().replace('_', "-").as_str() {
            "top-left" | "tl" => Some(Corner::TopLeft),
            "top-right" | "tr" => Some(Corner::TopRight),
            "bottom-left" | "bl" => Some(Corner::BottomLeft),
            "bottom-right" | "br" => Some(Corner::BottomRight),
            _ => None,
        }
    }

    pub fn is_right(self) -> bool {
        matches!(self, Corner::TopRight | Corner::BottomRight)
    }

    pub fn is_bottom(self) -> bool {
        matches!(self, Corner::BottomLeft | Corner::BottomRight)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Color {
    pub r: u8,
    pub g: u8,
    pub b: u8,
    pub a: u8,
}

impl Color {
    pub const BLACK: Color = Color {
        r: 0,
        g: 0,
        b: 0,
        a: 255,
    };
    pub const WHITE: Color = Color {
        r: 255,
        g: 255,
        b: 255,
        a: 255,
    };

    pub fn parse(s: &str) -> Option<Color> {
        let h = s.trim().trim_start_matches('#');
        let byte = |i: usize| u8::from_str_radix(&h[i..i + 2], 16).ok();
        match h.len() {
            6 => Some(Color {
                r: byte(0)?,
                g: byte(2)?,
                b: byte(4)?,
                a: 255,
            }),
            8 => Some(Color {
                r: byte(0)?,
                g: byte(2)?,
                b: byte(4)?,
                a: byte(6)?,
            }),
            _ => None,
        }
    }

    pub fn to_hex(self) -> String {
        if self.a == 255 {
            format!("#{:02X}{:02X}{:02X}", self.r, self.g, self.b)
        } else {
            format!("#{:02X}{:02X}{:02X}{:02X}", self.r, self.g, self.b, self.a)
        }
    }
}

/// Print proportions, e.g. 10x15. Stored as the two numbers rather than a
/// single float so the ini stays readable and `10:15` survives a round trip.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct AspectRatio {
    pub w: f64,
    pub h: f64,
}

impl AspectRatio {
    pub const fn new(w: f64, h: f64) -> AspectRatio {
        AspectRatio { w, h }
    }

    /// Width divided by height, always as a landscape-or-square value >= 1 when
    /// `landscape` is true and its reciprocal otherwise.
    pub fn value(self, landscape: bool) -> f64 {
        let long = self.w.max(self.h);
        let short = self.w.min(self.h);
        if short <= 0.0 {
            return 1.0;
        }
        if landscape {
            long / short
        } else {
            short / long
        }
    }

    pub fn parse(s: &str) -> Option<AspectRatio> {
        let s = s.trim();
        let (a, b) = s.split_once([':', 'x', 'X', '/'])?;
        let w: f64 = a.trim().parse().ok()?;
        let h: f64 = b.trim().parse().ok()?;
        (w > 0.0 && h > 0.0).then_some(AspectRatio { w, h })
    }

    pub fn to_config_string(self) -> String {
        format!("{}:{}", trim_num(self.w), trim_num(self.h))
    }

    pub fn label(self) -> String {
        format!("{}×{}", trim_num(self.w), trim_num(self.h))
    }
}

/// Ratios offered in the toolbar. Anything else can be typed as a custom value.
pub const RATIO_PRESETS: &[AspectRatio] = &[
    AspectRatio::new(10.0, 15.0),
    AspectRatio::new(13.0, 18.0),
    AspectRatio::new(15.0, 21.0),
    AspectRatio::new(20.0, 30.0),
    AspectRatio::new(9.0, 13.0),
    AspectRatio::new(1.0, 1.0),
    AspectRatio::new(3.0, 4.0),
    AspectRatio::new(2.0, 3.0),
    AspectRatio::new(9.0, 16.0),
];

fn trim_num(v: f64) -> String {
    let s = format!("{v:.4}");
    let s = s.trim_end_matches('0').trim_end_matches('.');
    s.to_string()
}

/// Whether the arrow keys / Next button walk the whole folder or only the
/// pictures already ticked for printing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NavMode {
    All,
    Selected,
}

impl NavMode {
    pub fn as_str(self) -> &'static str {
        match self {
            NavMode::All => "all",
            NavMode::Selected => "selected",
        }
    }

    pub fn parse(s: &str) -> Option<NavMode> {
        match s.trim().to_ascii_lowercase().as_str() {
            "all" => Some(NavMode::All),
            "selected" | "picked" => Some(NavMode::Selected),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct CaptionConfig {
    pub enabled: bool,
    /// `{city}`, `{country}`, `{date}`, `{place}` (city + country) and
    /// `{filename}` are substituted; anything else is literal.
    pub template: String,
    pub corner: Corner,
    pub font_family: String,
    pub font_style: String,
    /// Cap height as a percentage of the cropped image's short side, so the
    /// caption keeps its relative size no matter the source resolution.
    pub size_pct: f32,
    pub fill: Color,
    pub outline: Color,
    /// Outline thickness as a percentage of the font size.
    pub outline_pct: f32,
    /// Distance from the image edges, percentage of the short side.
    pub margin_pct: f32,
    pub uppercase: bool,
}

impl Default for CaptionConfig {
    fn default() -> Self {
        CaptionConfig {
            enabled: true,
            template: "{city}, {country}, {date}".to_string(),
            corner: Corner::BottomRight,
            // Calibri ships with Windows and is the closest thing to the
            // requested "Colibri Black"; there is no Black weight, so Bold is
            // the heaviest real cut. Any installed font can be picked instead.
            font_family: "Calibri".to_string(),
            font_style: "Bold".to_string(),
            size_pct: 3.2,
            fill: Color::BLACK,
            outline: Color::WHITE,
            outline_pct: 14.0,
            margin_pct: 2.5,
            uppercase: false,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct PrefetchConfig {
    /// How many images past the current one to decode ahead of time.
    pub ahead: usize,
    /// How many behind, so stepping back is instant too.
    pub behind: usize,
    /// 0 means "pick from the CPU count".
    pub workers: usize,
    /// Decoded images held in memory at once.
    pub cache: usize,
}

impl Default for PrefetchConfig {
    fn default() -> Self {
        PrefetchConfig {
            // Enough that holding the Next key does not outrun the decoders.
            // A preview is a few megabytes, so twenty of them is a rounding
            // error next to the photos themselves.
            ahead: 10,
            behind: 4,
            workers: 0,
            cache: 28,
        }
    }
}

/// The on-disk cache of already-decoded previews and thumbnails.
#[derive(Debug, Clone, PartialEq)]
pub struct CacheConfig {
    pub enabled: bool,
    /// Empty means the standard per-user cache location.
    pub directory: Option<PathBuf>,
    pub budget_mb: u64,
}

impl Default for CacheConfig {
    fn default() -> Self {
        CacheConfig {
            enabled: true,
            directory: None,
            // A view-sized preview is a fraction of the megabytes its original
            // takes, and a thumbnail a rounding error, so this holds a folder of
            // ten thousand or more outright. Generous on purpose: everything in
            // here is rebuildable, and running out is the only failure mode that
            // costs the user time.
            budget_mb: 4096,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Config {
    pub source_dir: Option<PathBuf>,
    pub output_dir: Option<PathBuf>,
    pub nav: NavMode,
    pub ratio: AspectRatio,
    /// Keep the ratio's long side along the image's long side, so a portrait
    /// photo gets a portrait 10×15 without touching the setting.
    pub ratio_follows_image: bool,
    pub jpeg_quality: u8,
    /// Copy the source EXIF block into the export (with Orientation reset to
    /// 1, because the pixels are already rotated).
    pub preserve_exif: bool,
    /// Fills whatever part of the crop window lies outside the photo, which is
    /// how a deliberate print border gets made.
    pub background: Color,
    pub caption: CaptionConfig,
    pub date_locale: String,
    pub date_pattern: String,
    pub locales: Locales,
    pub prefetch: PrefetchConfig,
    /// Long side of the preview the editor works on. The export always re-reads
    /// the original at full resolution.
    ///
    /// Deliberately a fixed setting rather than the window's current size: the
    /// cache is keyed on the photo, not on the window, and following the window
    /// would throw the whole cache away every time it was resized.
    pub preview_max_px: u32,
    pub cache: CacheConfig,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            source_dir: None,
            output_dir: None,
            nav: NavMode::All,
            ratio: AspectRatio::new(10.0, 15.0),
            ratio_follows_image: true,
            jpeg_quality: 100,
            preserve_exif: true,
            background: Color::WHITE,
            caption: CaptionConfig::default(),
            date_locale: "en".to_string(),
            date_pattern: "{MMM} '{yy}".to_string(),
            locales: Locales::default(),
            prefetch: PrefetchConfig::default(),
            // Comfortably sharper than any editor viewport, while keeping the
            // per-step texture upload small enough not to be felt.
            preview_max_px: 1800,
            cache: CacheConfig::default(),
        }
    }
}

impl Config {
    /// Where settings are read from and written to. Beside the exe if that
    /// directory takes a write, otherwise the per-user config location.
    pub fn default_path() -> PathBuf {
        if let Some(dir) = std::env::current_exe()
            .ok()
            .and_then(|p| p.parent().map(Path::to_path_buf))
        {
            let candidate = dir.join("sort4print.ini");
            if candidate.exists() || dir_is_writable(&dir) {
                return candidate;
            }
        }
        user_config_dir().join("sort4print.ini")
    }

    /// Missing file is not an error: it just means "all defaults".
    pub fn load(path: &Path) -> Result<Config> {
        match std::fs::read_to_string(path) {
            Ok(text) => Ok(Config::from_ini(&Ini::parse(&text))),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Config::default()),
            Err(e) => Err(e).with_context(|| format!("reading {}", path.display())),
        }
    }

    pub fn save(&self, path: &Path) -> Result<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).ok();
        }
        std::fs::write(path, self.to_ini_string())
            .with_context(|| format!("writing {}", path.display()))
    }

    pub fn from_ini(ini: &Ini) -> Config {
        let d = Config::default();
        let dc = &d.caption;
        let dp = &d.prefetch;

        let mut locales = Locales::default();
        for section in ini.section_names() {
            let Some(id) = section.strip_prefix("locale.") else {
                continue;
            };
            if let Some(locale) = parse_locale_section(ini, section, id) {
                locales.insert(locale);
            }
        }

        Config {
            source_dir: ini.get("paths", "source_dir").filter(|s| !s.is_empty()).map(PathBuf::from),
            output_dir: ini.get("paths", "output_dir").filter(|s| !s.is_empty()).map(PathBuf::from),
            nav: ini
                .get("view", "navigate")
                .and_then(NavMode::parse)
                .unwrap_or(d.nav),
            ratio: ini
                .get("crop", "ratio")
                .and_then(AspectRatio::parse)
                .unwrap_or(d.ratio),
            ratio_follows_image: ini.get_bool("crop", "follow_image_orientation", d.ratio_follows_image),
            jpeg_quality: ini
                .get_parsed::<u8>("output", "jpeg_quality")
                .map(|q| q.clamp(1, 100))
                .unwrap_or(d.jpeg_quality),
            preserve_exif: ini.get_bool("output", "preserve_exif", d.preserve_exif),
            background: ini
                .get("output", "background")
                .and_then(Color::parse)
                .unwrap_or(d.background),
            caption: CaptionConfig {
                enabled: ini.get_bool("caption", "enabled", dc.enabled),
                template: ini.get_or("caption", "template", &dc.template).to_string(),
                corner: ini
                    .get("caption", "corner")
                    .and_then(Corner::parse)
                    .unwrap_or(dc.corner),
                font_family: ini
                    .get_or("caption", "font_family", &dc.font_family)
                    .to_string(),
                font_style: ini.get_or("caption", "font_style", &dc.font_style).to_string(),
                size_pct: ini
                    .get_parsed::<f32>("caption", "size_pct")
                    .map(|v| v.clamp(0.3, 30.0))
                    .unwrap_or(dc.size_pct),
                fill: ini
                    .get("caption", "color")
                    .and_then(Color::parse)
                    .unwrap_or(dc.fill),
                outline: ini
                    .get("caption", "outline_color")
                    .and_then(Color::parse)
                    .unwrap_or(dc.outline),
                outline_pct: ini
                    .get_parsed::<f32>("caption", "outline_pct")
                    .map(|v| v.clamp(0.0, 100.0))
                    .unwrap_or(dc.outline_pct),
                margin_pct: ini
                    .get_parsed::<f32>("caption", "margin_pct")
                    .map(|v| v.clamp(0.0, 40.0))
                    .unwrap_or(dc.margin_pct),
                uppercase: ini.get_bool("caption", "uppercase", dc.uppercase),
            },
            date_locale: ini.get_or("date", "locale", &d.date_locale).to_string(),
            date_pattern: ini.get_or("date", "format", &d.date_pattern).to_string(),
            locales,
            prefetch: PrefetchConfig {
                ahead: ini
                    .get_parsed::<usize>("prefetch", "ahead")
                    .map(|v| v.min(64))
                    .unwrap_or(dp.ahead),
                behind: ini
                    .get_parsed::<usize>("prefetch", "behind")
                    .map(|v| v.min(64))
                    .unwrap_or(dp.behind),
                workers: ini
                    .get_parsed::<usize>("prefetch", "workers")
                    .map(|v| v.min(64))
                    .unwrap_or(dp.workers),
                cache: ini
                    .get_parsed::<usize>("prefetch", "cache")
                    .map(|v| v.clamp(2, 256))
                    .unwrap_or(dp.cache),
            },
            preview_max_px: ini
                .get_parsed::<u32>("view", "preview_max_px")
                .map(|v| v.clamp(600, 8000))
                .unwrap_or(d.preview_max_px),
            cache: CacheConfig {
                enabled: ini.get_bool("cache", "enabled", d.cache.enabled),
                directory: ini
                    .get("cache", "directory")
                    .filter(|s| !s.trim().is_empty())
                    .map(PathBuf::from),
                budget_mb: ini
                    .get_parsed::<u64>("cache", "budget_mb")
                    .map(|v| v.clamp(64, 200_000))
                    .unwrap_or(d.cache.budget_mb),
            },
        }
    }

    /// Where decoded previews are kept, and how much room they get.
    pub fn disk_cache(&self) -> Option<crate::cache::DiskCache> {
        if !self.cache.enabled {
            return None;
        }
        let root = self
            .cache
            .directory
            .clone()
            .unwrap_or_else(crate::cache::DiskCache::default_root);
        Some(crate::cache::DiskCache::new(
            root,
            self.cache.budget_mb.saturating_mul(1024 * 1024),
        ))
    }

    pub fn to_ini(&self) -> Ini {
        let mut ini = Ini::new();
        let path_str = |p: &Option<PathBuf>| {
            p.as_ref()
                .map(|p| p.display().to_string())
                .unwrap_or_default()
        };

        ini.set("paths", "source_dir", &path_str(&self.source_dir));
        ini.set("paths", "output_dir", &path_str(&self.output_dir));

        ini.set("view", "navigate", self.nav.as_str());
        ini.set("view", "preview_max_px", &self.preview_max_px.to_string());

        ini.set("crop", "ratio", &self.ratio.to_config_string());
        ini.set(
            "crop",
            "follow_image_orientation",
            bool_str(self.ratio_follows_image),
        );

        ini.set("output", "jpeg_quality", &self.jpeg_quality.to_string());
        ini.set("output", "preserve_exif", bool_str(self.preserve_exif));
        ini.set("output", "background", &self.background.to_hex());

        let c = &self.caption;
        ini.set("caption", "enabled", bool_str(c.enabled));
        ini.set("caption", "template", &c.template);
        ini.set("caption", "corner", c.corner.as_str());
        ini.set("caption", "font_family", &c.font_family);
        ini.set("caption", "font_style", &c.font_style);
        ini.set("caption", "size_pct", &trim_num(c.size_pct as f64));
        ini.set("caption", "color", &c.fill.to_hex());
        ini.set("caption", "outline_color", &c.outline.to_hex());
        ini.set("caption", "outline_pct", &trim_num(c.outline_pct as f64));
        ini.set("caption", "margin_pct", &trim_num(c.margin_pct as f64));
        ini.set("caption", "uppercase", bool_str(c.uppercase));

        ini.set("date", "locale", &self.date_locale);
        ini.set("date", "format", &self.date_pattern);

        let p = &self.prefetch;
        ini.set("prefetch", "ahead", &p.ahead.to_string());
        ini.set("prefetch", "behind", &p.behind.to_string());
        ini.set("prefetch", "workers", &p.workers.to_string());
        ini.set("prefetch", "cache", &p.cache.to_string());

        ini.set("cache", "enabled", bool_str(self.cache.enabled));
        ini.set(
            "cache",
            "directory",
            &self
                .cache
                .directory
                .as_ref()
                .map(|p| p.display().to_string())
                .unwrap_or_default(),
        );
        ini.set("cache", "budget_mb", &self.cache.budget_mb.to_string());

        // Only user-defined locales are written back; re-emitting the built-ins
        // would bloat the file and freeze them against future corrections.
        let builtin: Vec<String> = crate::datefmt::builtin_locales()
            .into_iter()
            .map(|l| l.id)
            .collect();
        for locale in self.locales.iter() {
            if builtin.contains(&locale.id) {
                continue;
            }
            let section = format!("locale.{}", locale.id);
            ini.set(&section, "label", &locale.label);
            ini.set(&section, "months_short", &locale.months_short.join(","));
            ini.set(&section, "months_long", &locale.months_long.join(","));
            ini.set(&section, "months_long_of", &locale.months_long_of.join(","));
        }

        ini
    }

    pub fn to_ini_string(&self) -> String {
        let header = "\
# sort4print settings.
#
# Edited by the application, and safe to edit by hand while it is closed.
# Comments you add here are not preserved when the application saves.
";
        format!("{header}\n{}", self.to_ini().to_string_with_comments(&comment_for))
    }
}

fn bool_str(v: bool) -> &'static str {
    if v {
        "true"
    } else {
        "false"
    }
}

/// The explanatory comments written above each key, so the ini is documentation
/// for itself.
fn comment_for(section: &str, key: &str) -> Option<String> {
    let text = match (section, key) {
        ("view", "navigate") => {
            "all = Next/Previous walk every picture in the folder\n\
             selected = they walk only the ones ticked for printing"
        }
        ("view", "preview_max_px") => {
            "Long side of the on-screen preview. Exports always re-read the\n\
             original at full resolution regardless of this."
        }
        ("crop", "ratio") => "Print proportions, e.g. 10:15, 13:18, 1:1.",
        ("crop", "follow_image_orientation") => {
            "true = a portrait photo gets a portrait crop of the same ratio."
        }
        ("output", "jpeg_quality") => "1-100. 100 keeps the crop as close to the original as JPEG allows.",
        ("output", "background") => {
            "Fills the part of the crop window that falls outside the photo.\n\
             The window is allowed to be bigger than the picture, which is how\n\
             you get a printed border."
        }
        ("output", "preserve_exif") => {
            "Copy the camera's EXIF block into the exported file, with the\n\
             orientation tag reset since the pixels are already upright."
        }
        ("caption", "template") => {
            "Placeholders: {city} {country} {place} {description} {date} {filename}\n\
             {place} is city and country joined, and collapses cleanly when\n\
             one of them is unknown. {description} is the per-photo note you\n\
             type in the panel, e.g. Chinatown."
        }
        ("caption", "size_pct") => "Font size as a percentage of the cropped image's short side.",
        ("caption", "outline_pct") => "Outline thickness as a percentage of the font size.",
        ("caption", "margin_pct") => "Gap to the image edge, percentage of the short side.",
        ("caption", "font_family") => {
            "Any font installed on the system. Windows ships Calibri; there is\n\
             no Black weight, so Bold is the heaviest genuine cut."
        }
        ("date", "locale") => {
            "Month-name language. 'en' and 'ru' are built in; add your own with\n\
             a [locale.xx] section (see below) and name it here."
        }
        ("date", "format") => {
            "Tokens: {yyyy} {yy} {MMMM} {MMMMo} {MMM} {MM} {M} {dd} {d} {HH} {mm}\n\
             {MMMMo} is the form used after a day number, which matters in\n\
             languages that decline: '5 октября' vs 'Октябрь 2025'.\n\
             Example: {MMM} '{yy}  ->  Oct '25"
        }
        ("prefetch", "ahead") => {
            "Images decoded in the background ahead of the current one, so\n\
             stepping forward is instant. Costs memory, not responsiveness."
        }
        ("prefetch", "workers") => "0 picks a sensible number from the CPU count.",
        ("prefetch", "cache") => "How many decoded previews stay in memory.",
        ("cache", "enabled") => {
            "Keep decoded previews and thumbnails on disk, so a folder visited\n\
             a second time opens without decoding anything again. Everything in\n\
             there can be rebuilt from the photos; deleting it costs only time."
        }
        ("cache", "directory") => "Empty means the standard per-user cache location.",
        ("cache", "budget_mb") => {
            "Upper limit on the cache. Least recently used entries go first."
        }
        _ => return None,
    };
    Some(text.to_string())
}

fn parse_locale_section(ini: &Ini, section: &str, id: &str) -> Option<Locale> {
    let split = |key: &str| -> Option<[String; 12]> {
        let raw = ini.get(section, key)?;
        let parts: Vec<String> = raw.split(',').map(|s| s.trim().to_string()).collect();
        <[String; 12]>::try_from(parts).ok()
    };
    let short = split("months_short")?;
    // A locale that only declares short names still works: reuse them.
    let long = split("months_long").unwrap_or_else(|| short.clone());
    let long_of = split("months_long_of").unwrap_or_else(|| long.clone());
    Some(Locale {
        id: id.to_string(),
        label: ini.get_or(section, "label", id).to_string(),
        months_short: short,
        months_long: long,
        months_long_of: long_of,
    })
}

fn dir_is_writable(dir: &Path) -> bool {
    let probe = dir.join(".sort4print-write-test");
    match std::fs::write(&probe, b"") {
        Ok(()) => {
            let _ = std::fs::remove_file(&probe);
            true
        }
        Err(_) => false,
    }
}

fn user_config_dir() -> PathBuf {
    if let Ok(appdata) = std::env::var("APPDATA") {
        if !appdata.is_empty() {
            return PathBuf::from(appdata).join("sort4print");
        }
    }
    if let Ok(xdg) = std::env::var("XDG_CONFIG_HOME") {
        if !xdg.is_empty() {
            return PathBuf::from(xdg).join("sort4print");
        }
    }
    if let Ok(home) = std::env::var("HOME") {
        if !home.is_empty() {
            return PathBuf::from(home).join(".config").join("sort4print");
        }
    }
    PathBuf::from(".")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_match_the_brief() {
        let c = Config::default();
        assert_eq!(c.ratio, AspectRatio::new(10.0, 15.0));
        assert_eq!(c.jpeg_quality, 100);
        assert_eq!(c.date_pattern, "{MMM} '{yy}");
        assert_eq!(c.caption.fill, Color::BLACK);
        assert_eq!(c.caption.outline, Color::WHITE);
    }

    #[test]
    fn config_round_trips_through_the_ini() {
        let mut c = Config::default();
        c.output_dir = Some(PathBuf::from("/tmp/print"));
        c.nav = NavMode::Selected;
        c.ratio = AspectRatio::new(13.0, 18.0);
        c.date_locale = "ru".into();
        c.caption.template = "{place} — {date}".into();
        c.caption.corner = Corner::TopLeft;
        c.caption.size_pct = 4.5;
        c.caption.outline = Color {
            r: 1,
            g: 2,
            b: 3,
            a: 128,
        };
        c.prefetch.ahead = 9;

        let text = c.to_ini_string();
        let back = Config::from_ini(&Ini::parse(&text));
        assert_eq!(back, c);
    }

    #[test]
    fn user_locales_survive_a_round_trip_and_builtins_are_not_duplicated() {
        let mut c = Config::default();
        let de = Locale {
            id: "de".into(),
            label: "Deutsch".into(),
            months_short: [
                "Jan", "Feb", "Mär", "Apr", "Mai", "Jun", "Jul", "Aug", "Sep", "Okt", "Nov", "Dez",
            ]
            .map(str::to_string),
            months_long: [
                "Januar",
                "Februar",
                "März",
                "April",
                "Mai",
                "Juni",
                "Juli",
                "August",
                "September",
                "Oktober",
                "November",
                "Dezember",
            ]
            .map(str::to_string),
            months_long_of: [
                "Januar",
                "Februar",
                "März",
                "April",
                "Mai",
                "Juni",
                "Juli",
                "August",
                "September",
                "Oktober",
                "November",
                "Dezember",
            ]
            .map(str::to_string),
        };
        c.locales.insert(de.clone());

        let text = c.to_ini_string();
        assert!(!text.contains("[locale.en]"));
        assert!(text.contains("[locale.de]"));

        let back = Config::from_ini(&Ini::parse(&text));
        assert_eq!(back.locales.get("de"), Some(&de));
        assert!(back.locales.get("ru").is_some(), "built-ins still present");
    }

    #[test]
    fn a_locale_may_declare_only_short_names() {
        let ini = Ini::parse(
            "[locale.xx]\nmonths_short = a,b,c,d,e,f,g,h,i,j,k,l\n",
        );
        let c = Config::from_ini(&ini);
        let xx = c.locales.get("xx").unwrap();
        assert_eq!(xx.months_long[0], "a");
        assert_eq!(xx.months_long_of[11], "l");
    }

    #[test]
    fn a_malformed_locale_section_is_skipped_not_fatal() {
        let ini = Ini::parse("[locale.bad]\nmonths_short = only,three,values\n");
        let c = Config::from_ini(&ini);
        assert!(c.locales.get("bad").is_none());
        assert!(c.locales.get("en").is_some());
    }

    #[test]
    fn out_of_range_values_are_clamped_rather_than_rejected() {
        let ini = Ini::parse(
            "[output]\njpeg_quality = 250\n[caption]\nsize_pct = 900\nmargin_pct = -4\n",
        );
        let c = Config::from_ini(&ini);
        assert_eq!(c.jpeg_quality, 100);
        assert_eq!(c.caption.size_pct, 30.0);
        assert_eq!(c.caption.margin_pct, 0.0);
    }

    #[test]
    fn colors_parse_both_lengths() {
        assert_eq!(Color::parse("#FFFFFF"), Some(Color::WHITE));
        assert_eq!(
            Color::parse("00000080"),
            Some(Color {
                r: 0,
                g: 0,
                b: 0,
                a: 128
            })
        );
        assert_eq!(Color::parse("#GGG"), None);
    }

    #[test]
    fn ratio_parses_the_spellings_people_use() {
        assert_eq!(AspectRatio::parse("10:15"), Some(AspectRatio::new(10.0, 15.0)));
        assert_eq!(AspectRatio::parse("10x15"), Some(AspectRatio::new(10.0, 15.0)));
        assert_eq!(AspectRatio::parse(" 4 / 3 "), Some(AspectRatio::new(4.0, 3.0)));
        assert_eq!(AspectRatio::parse("10:0"), None);
        assert_eq!(AspectRatio::parse("nonsense"), None);
    }

    #[test]
    fn ratio_orients_itself() {
        let r = AspectRatio::new(10.0, 15.0);
        assert!((r.value(true) - 1.5).abs() < 1e-9);
        assert!((r.value(false) - 2.0 / 3.0).abs() < 1e-9);
    }

    #[test]
    fn an_empty_file_yields_defaults() {
        assert_eq!(Config::from_ini(&Ini::parse("")), Config::default());
    }
}
