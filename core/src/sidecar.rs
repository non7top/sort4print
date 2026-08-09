//! Remembering what you decided about each photo.
//!
//! Ticks, crop windows, place overrides and descriptions are written to
//! `sort4print-notes.ini` in the folder being sorted, so closing the program
//! half way through a holiday's worth of pictures does not throw the work away.
//! It lives with the photos rather than with the program because it is *about*
//! those photos — copy the folder to another machine and the notes travel.
//!
//! The name is deliberately not `sort4print.ini`: that is the settings file,
//! and someone will eventually keep the exe in the same folder as their photos.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

use crate::cropbox::CropBox;
use crate::ini::Ini;

pub const FILE_NAME: &str = "sort4print-notes.ini";

#[derive(Debug, Clone, Default, PartialEq)]
pub struct PhotoState {
    pub selected: bool,
    /// In original-image pixels, upright.
    pub crop: Option<CropBox>,
    pub city: Option<String>,
    pub country: Option<String>,
    /// The name of the spot, e.g. "Chinatown".
    pub description: Option<String>,
}

impl PhotoState {
    /// Nothing worth writing down.
    pub fn is_empty(&self) -> bool {
        !self.selected
            && self.crop.is_none()
            && self.city.is_none()
            && self.country.is_none()
            && self.description.is_none()
    }
}

#[derive(Debug, Clone, Default)]
pub struct Sidecar {
    entries: BTreeMap<String, PhotoState>,
}

impl Sidecar {
    pub fn path_for(folder: &Path) -> PathBuf {
        folder.join(FILE_NAME)
    }

    /// A missing or unreadable file simply means "no notes yet".
    pub fn load(folder: &Path) -> Sidecar {
        let path = Sidecar::path_for(folder);
        let Ok(text) = std::fs::read_to_string(path) else {
            return Sidecar::default();
        };
        Sidecar::from_ini(&Ini::parse(&text))
    }

    pub fn from_ini(ini: &Ini) -> Sidecar {
        let mut entries = BTreeMap::new();
        for section in ini.section_names() {
            if section.is_empty() {
                continue;
            }
            let state = PhotoState {
                selected: ini.get_bool(section, "selected", false),
                crop: ini.get(section, "crop").and_then(parse_crop),
                city: non_empty(ini.get(section, "city")),
                country: non_empty(ini.get(section, "country")),
                description: non_empty(ini.get(section, "description")),
            };
            if !state.is_empty() {
                entries.insert(section.to_string(), state);
            }
        }
        Sidecar { entries }
    }

    pub fn to_ini(&self) -> Ini {
        let mut ini = Ini::new();
        for (name, state) in &self.entries {
            if state.is_empty() {
                continue;
            }
            if state.selected {
                ini.set(name, "selected", "true");
            }
            if let Some(crop) = state.crop {
                ini.set(name, "crop", &format_crop(crop));
            }
            for (key, value) in [
                ("city", &state.city),
                ("country", &state.country),
                ("description", &state.description),
            ] {
                if let Some(value) = value {
                    ini.set(name, key, value);
                }
            }
        }
        ini
    }

    /// Writes the notes, or deletes the file when there is nothing left to say.
    pub fn save(&self, folder: &Path) -> Result<()> {
        let path = Sidecar::path_for(folder);
        if self.entries.values().all(PhotoState::is_empty) {
            if path.exists() {
                std::fs::remove_file(&path)
                    .with_context(|| format!("removing {}", path.display()))?;
            }
            return Ok(());
        }

        let header = "\
# Notes made by sort4print about the photos in this folder: which are picked,
# how each is cropped, and what you called the place.
#
# Sections are file names. Deleting this file discards those choices and
# nothing else; the photos are untouched.
";
        let body = format!("{header}\n{}", self.to_ini());
        std::fs::write(&path, body).with_context(|| format!("writing {}", path.display()))
    }

    pub fn get(&self, file_name: &str) -> Option<&PhotoState> {
        self.entries.get(file_name)
    }

    pub fn set(&mut self, file_name: &str, state: PhotoState) {
        if state.is_empty() {
            self.entries.remove(file_name);
        } else {
            self.entries.insert(file_name.to_string(), state);
        }
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

fn non_empty(value: Option<&str>) -> Option<String> {
    value
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .map(str::to_string)
}

fn format_crop(c: CropBox) -> String {
    // One decimal is well below a pixel of framing and keeps the file readable.
    format!("{:.1},{:.1},{:.1},{:.1}", c.x, c.y, c.w, c.h)
}

fn parse_crop(text: &str) -> Option<CropBox> {
    let parts: Vec<f64> = text
        .split(',')
        .map(|p| p.trim().parse::<f64>())
        .collect::<Result<Vec<_>, _>>()
        .ok()?;
    let [x, y, w, h] = parts.as_slice() else {
        return None;
    };
    (w.is_finite() && h.is_finite() && *w > 0.0 && *h > 0.0 && x.is_finite() && y.is_finite())
        .then_some(CropBox::new(*x, *y, *w, *h))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Sidecar {
        let mut s = Sidecar::default();
        s.set(
            "IMG_0001.JPG",
            PhotoState {
                selected: true,
                crop: Some(CropBox::new(100.0, 200.5, 2000.0, 3000.0)),
                city: Some("San Francisco".into()),
                country: Some("United States".into()),
                description: Some("Chinatown".into()),
            },
        );
        s.set(
            "IMG_0002.JPG",
            PhotoState {
                description: Some("the back garden".into()),
                ..Default::default()
            },
        );
        s
    }

    #[test]
    fn notes_survive_a_round_trip() {
        let original = sample();
        let text = original.to_ini().to_string();
        let back = Sidecar::from_ini(&Ini::parse(&text));

        assert_eq!(back.len(), 2);
        let first = back.get("IMG_0001.JPG").unwrap();
        assert!(first.selected);
        assert_eq!(first.description.as_deref(), Some("Chinatown"));
        assert_eq!(first.city.as_deref(), Some("San Francisco"));
        let crop = first.crop.unwrap();
        assert!((crop.x - 100.0).abs() < 0.05 && (crop.y - 200.5).abs() < 0.05);
        assert!((crop.w - 2000.0).abs() < 0.05 && (crop.h - 3000.0).abs() < 0.05);

        let second = back.get("IMG_0002.JPG").unwrap();
        assert!(!second.selected);
        assert!(second.crop.is_none());
        assert_eq!(second.description.as_deref(), Some("the back garden"));
    }

    #[test]
    fn a_photo_with_nothing_said_about_it_is_not_written() {
        let mut s = Sidecar::default();
        s.set("IMG_0003.JPG", PhotoState::default());
        assert!(s.is_empty());
        assert_eq!(s.to_ini().to_string(), "");
    }

    #[test]
    fn setting_an_empty_state_forgets_a_previous_one() {
        let mut s = sample();
        s.set("IMG_0002.JPG", PhotoState::default());
        assert!(s.get("IMG_0002.JPG").is_none());
        assert_eq!(s.len(), 1);
    }

    #[test]
    fn descriptions_may_contain_the_punctuation_people_type() {
        let mut s = Sidecar::default();
        s.set(
            "a.jpg",
            PhotoState {
                description: Some("Chinatown, 3rd & Main; #2".into()),
                ..Default::default()
            },
        );
        let back = Sidecar::from_ini(&Ini::parse(&s.to_ini().to_string()));
        assert_eq!(
            back.get("a.jpg").unwrap().description.as_deref(),
            Some("Chinatown, 3rd & Main; #2")
        );
    }

    #[test]
    fn a_missing_file_is_simply_no_notes() {
        let s = Sidecar::load(Path::new("/nonexistent/folder"));
        assert!(s.is_empty());
    }

    #[test]
    fn a_corrupt_crop_is_ignored_rather_than_fatal() {
        let ini = Ini::parse("[a.jpg]\ncrop = nonsense\ndescription = kept\n");
        let s = Sidecar::from_ini(&ini);
        let state = s.get("a.jpg").unwrap();
        assert!(state.crop.is_none());
        assert_eq!(state.description.as_deref(), Some("kept"));

        // Wrong arity, and zero-sized boxes, are refused too.
        assert!(parse_crop("1,2,3").is_none());
        assert!(parse_crop("1,2,0,4").is_none());
    }

    #[test]
    fn saving_and_loading_from_disk() {
        let dir = std::env::temp_dir().join(format!("sort4print-sidecar-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();

        sample().save(&dir).unwrap();
        assert!(Sidecar::path_for(&dir).exists());
        let back = Sidecar::load(&dir);
        assert_eq!(back.len(), 2);

        // Clearing everything removes the file rather than leaving a stub.
        Sidecar::default().save(&dir).unwrap();
        assert!(!Sidecar::path_for(&dir).exists());

        std::fs::remove_dir_all(&dir).ok();
    }
}
