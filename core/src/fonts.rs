//! Finding the fonts installed on the machine.
//!
//! Done by walking the font directories and reading the `name` table directly
//! rather than going through DirectWrite or fontconfig: it is a hundred lines,
//! it behaves identically on the Linux build used for testing and on the
//! Windows build that ships, and it keeps the executable free of a platform
//! text stack it would otherwise only use for one dropdown.
//!
//! Families are grouped by the typographic name (name IDs 16/17) when the font
//! provides one, which is what puts Calibri's Light and Bold cuts under a
//! single "Calibri" entry instead of scattering them.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FontFace {
    pub family: String,
    /// Subfamily as the font names it: Regular, Bold, Light Italic, ...
    pub style: String,
    pub path: PathBuf,
    /// Index within a TrueType collection; 0 for a plain font file.
    pub index: u32,
    /// usWeightClass, 100-900. Used only for ordering the style list.
    pub weight: u16,
    pub italic: bool,
}

impl FontFace {
    pub fn display_name(&self) -> String {
        if self.style.eq_ignore_ascii_case("regular") {
            self.family.clone()
        } else {
            format!("{} {}", self.family, self.style)
        }
    }

    /// The bytes ab_glyph needs. Read on demand: holding every installed font
    /// in memory would cost far more than re-reading the one in use.
    pub fn read(&self) -> Result<Vec<u8>> {
        std::fs::read(&self.path).with_context(|| format!("reading font {}", self.path.display()))
    }
}

#[derive(Debug, Default, Clone)]
pub struct FontCatalog {
    faces: Vec<FontFace>,
}

/// Tried in order when the configured family is not installed, so a settings
/// file written on one machine still renders something sensible on another.
const FALLBACK_FAMILIES: &[&str] = &[
    "Calibri",
    "Segoe UI",
    "Arial",
    "Tahoma",
    "Verdana",
    "Liberation Sans",
    "DejaVu Sans",
    "Noto Sans",
];

impl FontCatalog {
    /// Walks the system font directories. Unreadable or unparseable files are
    /// skipped silently — a broken font should not stop the program starting.
    pub fn scan() -> FontCatalog {
        let mut faces = Vec::new();
        for dir in font_dirs() {
            collect_dir(&dir, 0, &mut faces);
        }

        faces.sort_by(|a, b| {
            a.family
                .to_lowercase()
                .cmp(&b.family.to_lowercase())
                .then(a.weight.cmp(&b.weight))
                .then(a.italic.cmp(&b.italic))
                .then(a.style.cmp(&b.style))
        });
        faces.dedup_by(|a, b| a.family == b.family && a.style == b.style);

        FontCatalog { faces }
    }

    pub fn from_faces(faces: Vec<FontFace>) -> FontCatalog {
        FontCatalog { faces }
    }

    pub fn is_empty(&self) -> bool {
        self.faces.is_empty()
    }

    pub fn len(&self) -> usize {
        self.faces.len()
    }

    pub fn faces(&self) -> &[FontFace] {
        &self.faces
    }

    /// Family names, alphabetical, each appearing once.
    pub fn families(&self) -> Vec<&str> {
        let mut seen = BTreeMap::new();
        for f in &self.faces {
            seen.entry(f.family.to_lowercase())
                .or_insert(f.family.as_str());
        }
        seen.into_values().collect()
    }

    /// Styles offered for a family, lightest first, in the order a font dialog
    /// would list them.
    pub fn styles_for(&self, family: &str) -> Vec<&str> {
        let mut out: Vec<&str> = self
            .faces
            .iter()
            .filter(|f| f.family.eq_ignore_ascii_case(family))
            .map(|f| f.style.as_str())
            .collect();
        out.dedup();
        out
    }

    pub fn find(&self, family: &str, style: &str) -> Option<&FontFace> {
        self.faces
            .iter()
            .find(|f| f.family.eq_ignore_ascii_case(family) && f.style.eq_ignore_ascii_case(style))
    }

    /// The face to actually render with: exact match, else another style of the
    /// same family, else a fallback family, else whatever is installed.
    pub fn best_match(&self, family: &str, style: &str) -> Option<&FontFace> {
        if let Some(face) = self.find(family, style) {
            return Some(face);
        }
        if let Some(face) = self.closest_in_family(family, style) {
            return Some(face);
        }
        for fallback in FALLBACK_FAMILIES {
            if let Some(face) = self
                .find(fallback, style)
                .or_else(|| self.closest_in_family(fallback, style))
            {
                return Some(face);
            }
        }
        self.faces.first()
    }

    /// Nearest weight within a family, preferring a matching slant.
    fn closest_in_family(&self, family: &str, style: &str) -> Option<&FontFace> {
        let want_italic = style.to_lowercase().contains("italic")
            || style.to_lowercase().contains("oblique");
        let want_weight = weight_hint(style);
        self.faces
            .iter()
            .filter(|f| f.family.eq_ignore_ascii_case(family))
            .min_by_key(|f| {
                let slant_penalty = if f.italic == want_italic { 0 } else { 2000 };
                slant_penalty + (f.weight as i32 - want_weight as i32).unsigned_abs()
            })
    }
}

/// Maps a style name to the weight it implies, for fallback ordering.
fn weight_hint(style: &str) -> u16 {
    let s = style.to_lowercase();
    if s.contains("thin") {
        100
    } else if s.contains("extralight") || s.contains("ultralight") {
        200
    } else if s.contains("light") {
        300
    } else if s.contains("medium") {
        500
    } else if s.contains("semibold") || s.contains("demibold") {
        600
    } else if s.contains("extrabold") || s.contains("ultrabold") {
        800
    } else if s.contains("black") || s.contains("heavy") {
        900
    } else if s.contains("bold") {
        700
    } else {
        400
    }
}

fn font_dirs() -> Vec<PathBuf> {
    let mut dirs: Vec<PathBuf> = Vec::new();
    let mut push = |p: PathBuf| {
        if p.is_dir() && !dirs.contains(&p) {
            dirs.push(p);
        }
    };

    if cfg!(windows) {
        let windir = std::env::var("WINDIR").unwrap_or_else(|_| "C:\\Windows".to_string());
        push(PathBuf::from(windir).join("Fonts"));
        if let Ok(local) = std::env::var("LOCALAPPDATA") {
            push(PathBuf::from(local)
                .join("Microsoft")
                .join("Windows")
                .join("Fonts"));
        }
    } else {
        for p in [
            "/usr/share/fonts",
            "/usr/local/share/fonts",
            "/run/host/fonts",
        ] {
            push(PathBuf::from(p));
        }
        if let Ok(home) = std::env::var("HOME") {
            push(PathBuf::from(&home).join(".fonts"));
            push(PathBuf::from(&home).join(".local/share/fonts"));
        }
    }
    dirs
}

/// Font directories nest a couple of levels on Linux; the depth cap stops a
/// symlink loop from turning startup into a filesystem crawl.
fn collect_dir(dir: &Path, depth: u32, out: &mut Vec<FontFace>) {
    const MAX_DEPTH: u32 = 4;
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let Ok(kind) = entry.file_type() else { continue };
        if kind.is_dir() {
            if depth < MAX_DEPTH {
                collect_dir(&path, depth + 1, out);
            }
            continue;
        }
        let ext = path
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or_default()
            .to_lowercase();
        if !matches!(ext.as_str(), "ttf" | "otf" | "ttc" | "otc") {
            continue;
        }
        let Ok(data) = std::fs::read(&path) else {
            continue;
        };
        out.extend(faces_in_file(&data, &path));
    }
}

pub fn faces_in_file(data: &[u8], path: &Path) -> Vec<FontFace> {
    let count = ttf_parser::fonts_in_collection(data).unwrap_or(1).max(1);
    (0..count)
        .filter_map(|index| face_at(data, path, index))
        .collect()
}

fn face_at(data: &[u8], path: &Path, index: u32) -> Option<FontFace> {
    let face = ttf_parser::Face::parse(data, index).ok()?;

    // 16/17 are the typographic names, which group weights under one family;
    // 1/2 are the legacy pair every font has.
    let family = name_string(&face, 16).or_else(|| name_string(&face, 1))?;
    let style = name_string(&face, 17)
        .or_else(|| name_string(&face, 2))
        .unwrap_or_else(|| "Regular".to_string());

    if family.trim().is_empty() {
        return None;
    }

    Some(FontFace {
        family: family.trim().to_string(),
        style: style.trim().to_string(),
        path: path.to_path_buf(),
        index,
        weight: face.weight().to_number(),
        italic: face.is_italic() || face.is_oblique(),
    })
}

fn name_string(face: &ttf_parser::Face, name_id: u16) -> Option<String> {
    let mut fallback = None;
    for name in face.names() {
        if name.name_id != name_id {
            continue;
        }
        // Windows platform, Unicode BMP or full: UTF-16BE.
        if name.platform_id == ttf_parser::PlatformId::Windows {
            if let Some(s) = decode_utf16be(name.name) {
                return Some(s);
            }
        }
        if fallback.is_none() {
            fallback = decode_utf16be(name.name).or_else(|| decode_mac_roman(name.name));
        }
    }
    fallback
}

fn decode_utf16be(bytes: &[u8]) -> Option<String> {
    if bytes.is_empty() || bytes.len() % 2 != 0 {
        return None;
    }
    let units: Vec<u16> = bytes
        .chunks_exact(2)
        .map(|c| u16::from_be_bytes([c[0], c[1]]))
        .collect();
    String::from_utf16(&units).ok().filter(|s| !s.is_empty())
}

/// Enough of Mac Roman for font names, which are overwhelmingly ASCII.
fn decode_mac_roman(bytes: &[u8]) -> Option<String> {
    if bytes.is_empty() || !bytes.iter().all(|b| b.is_ascii() && *b >= 0x20) {
        return None;
    }
    Some(bytes.iter().map(|b| *b as char).collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn face(family: &str, style: &str, weight: u16, italic: bool) -> FontFace {
        FontFace {
            family: family.into(),
            style: style.into(),
            path: PathBuf::from("/dev/null"),
            index: 0,
            weight,
            italic,
        }
    }

    fn catalog() -> FontCatalog {
        FontCatalog::from_faces(vec![
            face("Calibri", "Light", 300, false),
            face("Calibri", "Regular", 400, false),
            face("Calibri", "Bold", 700, false),
            face("Calibri", "Bold Italic", 700, true),
            face("Arial", "Regular", 400, false),
        ])
    }

    #[test]
    fn families_are_unique_and_sorted() {
        assert_eq!(catalog().families(), vec!["Arial", "Calibri"]);
    }

    #[test]
    fn styles_are_listed_lightest_first() {
        let c = catalog();
        assert_eq!(
            c.styles_for("calibri"),
            vec!["Light", "Regular", "Bold", "Bold Italic"]
        );
        assert!(c.styles_for("Nonexistent").is_empty());
    }

    #[test]
    fn exact_match_wins() {
        let c = catalog();
        let f = c.best_match("Calibri", "Bold").unwrap();
        assert_eq!((f.family.as_str(), f.style.as_str()), ("Calibri", "Bold"));
    }

    #[test]
    fn a_missing_style_falls_back_within_the_family_by_weight() {
        let c = catalog();
        // No Black cut exists; Bold is the heaviest and should win.
        let f = c.best_match("Calibri", "Black").unwrap();
        assert_eq!(f.style, "Bold");
        assert!(!f.italic, "slant should be preserved");
    }

    #[test]
    fn a_missing_family_falls_back_to_an_installed_one() {
        let c = catalog();
        let f = c.best_match("Nonexistent Sans", "Bold").unwrap();
        assert_eq!(f.family, "Calibri");
    }

    #[test]
    fn best_match_on_an_empty_catalog_is_none_rather_than_a_panic() {
        assert!(FontCatalog::default().best_match("Calibri", "Bold").is_none());
    }

    #[test]
    fn weight_hints_cover_the_common_style_names() {
        assert_eq!(weight_hint("Regular"), 400);
        assert_eq!(weight_hint("Bold"), 700);
        assert_eq!(weight_hint("Black"), 900);
        assert_eq!(weight_hint("SemiBold Italic"), 600);
        assert_eq!(weight_hint("ExtraLight"), 200);
    }

    #[test]
    fn display_name_omits_a_redundant_regular() {
        assert_eq!(face("Arial", "Regular", 400, false).display_name(), "Arial");
        assert_eq!(
            face("Arial", "Bold", 700, false).display_name(),
            "Arial Bold"
        );
    }

    #[test]
    fn utf16_name_decoding() {
        let bytes: Vec<u8> = "Calibri".encode_utf16().flat_map(u16::to_be_bytes).collect();
        assert_eq!(decode_utf16be(&bytes).as_deref(), Some("Calibri"));
        assert_eq!(decode_utf16be(&[0x41]), None, "odd length is not UTF-16");
        assert_eq!(decode_mac_roman(b"Arial").as_deref(), Some("Arial"));
    }

    /// Not an assertion about any particular machine: the build container has
    /// DejaVu installed, and this catches a scan that silently finds nothing.
    #[test]
    fn scanning_the_host_finds_something() {
        let c = FontCatalog::scan();
        if c.is_empty() {
            eprintln!("no system fonts found; skipping");
            return;
        }
        assert!(!c.families().is_empty());
        let first = c.families()[0].to_string();
        assert!(!c.styles_for(&first).is_empty());
        assert!(c.best_match(&first, "Regular").is_some());
    }
}
