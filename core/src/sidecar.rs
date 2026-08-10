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

/// Previous copy, kept so a half-written file is not the only one there is.
pub const BACKUP_SUFFIX: &str = ".bak";
/// Written last. Its absence is how a truncated file is recognised.
const END_MARKER: &str = "# end";

/// Holds where you were, rather than anything about one photo. The name cannot
/// collide with a photo's: every picture the scanner accepts has an image
/// extension, and this has none.
const SESSION_SECTION: &str = "sort4print";

#[derive(Debug, Clone, Default, PartialEq)]
pub struct PhotoState {
    pub selected: bool,
    /// In original-image pixels, upright.
    ///
    /// Only ever set for a crop that was actually moved. Merely looking at a
    /// photo gives it the default centred window, and recording that for every
    /// picture walked past is what made this file grow into six figures for a
    /// folder of a couple of thousand — while also meaning a photo you never
    /// touched came back with remembered framing instead of a fresh fit.
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
    /// The photo that was on screen last, so reopening the folder returns to it.
    pub last_file: Option<String>,
    /// Its position, used when that file is no longer in the folder: you land
    /// roughly where you were rather than back at the beginning.
    pub last_index: Option<usize>,
}

impl Sidecar {
    pub fn path_for(folder: &Path) -> PathBuf {
        folder.join(FILE_NAME)
    }

    pub fn backup_path_for(folder: &Path) -> PathBuf {
        folder.join(format!("{FILE_NAME}{BACKUP_SUFFIX}"))
    }

    /// Reads the notes, falling back to the backup if the main file is missing
    /// or was not finished being written. A missing pair means "no notes yet",
    /// which is not an error.
    pub fn load(folder: &Path) -> Sidecar {
        let main = Sidecar::path_for(folder);
        if let Some(sidecar) = Sidecar::read_complete(&main) {
            return sidecar;
        }
        let backup = Sidecar::backup_path_for(folder);
        Sidecar::read_complete(&backup).unwrap_or_default()
    }

    /// `None` when the file cannot be read, or when it lacks the end marker and
    /// is therefore a write that did not finish.
    fn read_complete(path: &Path) -> Option<Sidecar> {
        let text = std::fs::read_to_string(path).ok()?;
        text.lines()
            .any(|line| line.trim() == END_MARKER)
            .then(|| Sidecar::from_ini(&Ini::parse(&text)))
    }

    pub fn from_ini(ini: &Ini) -> Sidecar {
        let mut entries = BTreeMap::new();
        let mut sidecar = Sidecar::default();
        for section in ini.section_names() {
            if section.is_empty() {
                continue;
            }
            if section == SESSION_SECTION {
                sidecar.last_file = non_empty(ini.get(section, "last_file"));
                sidecar.last_index = ini.get_parsed::<usize>(section, "last_index");
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
        sidecar.entries = entries;
        sidecar
    }

    pub fn to_ini(&self) -> Ini {
        let mut ini = Ini::new();

        if let Some(last) = &self.last_file {
            ini.set(SESSION_SECTION, "last_file", last);
            if let Some(index) = self.last_index {
                ini.set(SESSION_SECTION, "last_index", &index.to_string());
            }
        }

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

    /// Writes the notes, or removes them when there is nothing left to say.
    ///
    /// Never writes over the live file in place. The new copy is written
    /// alongside, flushed to the disk, and only then moved into position, with
    /// the previous copy kept as `.bak`. A crash or a full disk can therefore
    /// leave a stray temporary file, but never a half-written set of notes and
    /// nothing else — and the end marker means a truncated file is recognised
    /// as one rather than read as a shorter set of decisions.
    pub fn save(&self, folder: &Path) -> Result<()> {
        let path = Sidecar::path_for(folder);
        let backup = Sidecar::backup_path_for(folder);

        if self.is_empty() && self.last_file.is_none() {
            for stale in [&path, &backup] {
                if stale.exists() {
                    std::fs::remove_file(stale)
                        .with_context(|| format!("removing {}", stale.display()))?;
                }
            }
            return Ok(());
        }

        let header = "\
# Notes made by sort4print about the photos in this folder: which are picked,
# how each is cropped, and what you called the place. Only choices you actually
# made are in here, so its size follows the work done rather than the number of
# photos in the folder.
#
# Sections are file names. Deleting this file discards those choices and
# nothing else; the photos are untouched.
";
        let body = format!("{header}\n{}\n{END_MARKER}\n", self.to_ini());

        let temporary = folder.join(format!("{FILE_NAME}.writing"));
        write_and_sync(&temporary, body.as_bytes())
            .with_context(|| format!("writing {}", temporary.display()))?;

        // Copying rather than moving the old file means that if the rename
        // below fails, the original is still where the reader looks for it.
        if path.exists() {
            std::fs::copy(&path, &backup)
                .with_context(|| format!("backing up to {}", backup.display()))?;
        }

        std::fs::rename(&temporary, &path)
            .with_context(|| format!("moving the new notes into {}", path.display()))
    }

    /// Roughly what the notes take on disk, for reporting.
    pub fn approximate_bytes(&self) -> usize {
        self.to_ini().to_string().len()
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

/// Writes to a fresh file and gets it onto the disk before returning, so the
/// rename that follows is swapping in something complete.
fn write_and_sync(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    use std::io::Write as _;

    let mut file = std::fs::File::create(path)?;
    file.write_all(bytes)?;
    file.flush()?;
    file.sync_all()
}

fn format_crop(c: CropBox) -> String {
    // Whole pixels: the export rounds to them anyway, and four short integers
    // instead of four decimals is a third off the size of the busiest line.
    format!(
        "{},{},{},{}",
        c.x.round() as i64,
        c.y.round() as i64,
        c.w.round().max(1.0) as i64,
        c.h.round().max(1.0) as i64
    )
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
        // Crops are stored as whole pixels, so a half-pixel in goes to the
        // nearest one out. That is deliberate: the export rounds to pixels
        // anyway, and integers keep the busiest line in the file short.
        let crop = first.crop.unwrap();
        assert!((crop.x - 100.0).abs() <= 0.5, "x = {}", crop.x);
        assert!((crop.y - 201.0).abs() <= 0.5, "y = {}", crop.y);
        assert!((crop.w - 2000.0).abs() <= 0.5 && (crop.h - 3000.0).abs() <= 0.5);

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

    /// Each disk test gets its own directory so they can run concurrently.
    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "sort4print-{name}-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        std::fs::remove_dir_all(&dir).ok();
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn saving_and_loading_from_disk() {
        let dir = scratch("roundtrip");

        sample().save(&dir).unwrap();
        assert!(Sidecar::path_for(&dir).exists());
        let back = Sidecar::load(&dir);
        assert_eq!(back.len(), 2);

        // Clearing everything removes the notes rather than leaving a stub.
        Sidecar::default().save(&dir).unwrap();
        assert!(!Sidecar::path_for(&dir).exists());
        assert!(!Sidecar::backup_path_for(&dir).exists());

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn the_previous_copy_is_kept_and_no_temporary_is_left_behind() {
        let dir = scratch("backup");

        sample().save(&dir).unwrap();
        assert!(!Sidecar::backup_path_for(&dir).exists(), "nothing to back up yet");

        let mut second = sample();
        second.set(
            "IMG_0009.JPG",
            PhotoState {
                selected: true,
                ..Default::default()
            },
        );
        second.save(&dir).unwrap();

        assert!(Sidecar::backup_path_for(&dir).exists(), "previous copy kept");
        assert_eq!(Sidecar::load(&dir).len(), 3);
        assert!(
            !dir.join(format!("{FILE_NAME}.writing")).exists(),
            "the temporary file should have been moved, not left"
        );

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn a_truncated_file_falls_back_to_the_backup() {
        let dir = scratch("truncated");

        // Two good saves, so a backup exists.
        sample().save(&dir).unwrap();
        sample().save(&dir).unwrap();

        // Simulate a write cut short: real content, but no end marker.
        let path = Sidecar::path_for(&dir);
        let full = std::fs::read_to_string(&path).unwrap();
        let cut = &full[..full.len() / 2];
        std::fs::write(&path, cut).unwrap();
        assert!(!cut.contains(END_MARKER));

        let recovered = Sidecar::load(&dir);
        assert_eq!(recovered.len(), 2, "should have come from the backup");
        assert_eq!(
            recovered.get("IMG_0001.JPG").unwrap().description.as_deref(),
            Some("Chinatown")
        );

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn with_neither_file_readable_it_starts_empty_rather_than_failing() {
        let dir = scratch("garbage");
        std::fs::write(Sidecar::path_for(&dir), "not even ini, and no marker").unwrap();
        assert!(Sidecar::load(&dir).is_empty());
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn where_you_were_is_remembered() {
        let dir = scratch("position");
        let mut s = sample();
        s.last_file = Some("IMG_0002.JPG".into());
        s.last_index = Some(137);
        s.save(&dir).unwrap();

        let back = Sidecar::load(&dir);
        assert_eq!(back.last_file.as_deref(), Some("IMG_0002.JPG"));
        assert_eq!(back.last_index, Some(137));
        // And it is not mistaken for a photo.
        assert!(back.get(SESSION_SECTION).is_none());
        assert_eq!(back.len(), 2);

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn a_position_alone_is_still_worth_writing() {
        let dir = scratch("position-only");
        let mut s = Sidecar::default();
        s.last_file = Some("IMG_0100.JPG".into());
        s.last_index = Some(99);
        s.save(&dir).unwrap();

        assert!(Sidecar::path_for(&dir).exists());
        assert_eq!(Sidecar::load(&dir).last_file.as_deref(), Some("IMG_0100.JPG"));

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn crops_are_stored_as_whole_pixels() {
        let mut s = Sidecar::default();
        s.set(
            "a.jpg",
            PhotoState {
                crop: Some(CropBox::new(100.4, 200.6, 2000.2, 3000.7)),
                ..Default::default()
            },
        );
        let text = s.to_ini().to_string();
        assert!(text.contains("crop = 100,201,2000,3001"), "got: {text}");

        let back = Sidecar::from_ini(&Ini::parse(&text));
        let crop = back.get("a.jpg").unwrap().crop.unwrap();
        assert!((crop.w - 2000.0).abs() < 1.0 && (crop.h - 3001.0).abs() < 1.0);
    }

    /// The size complaint that prompted all this: a folder of 11 000 photos
    /// where a realistic number of decisions have been made must not produce a
    /// file measured in hundreds of kilobytes.
    #[test]
    fn the_file_follows_the_work_done_not_the_size_of_the_folder() {
        let mut s = Sidecar::default();
        for i in 0..300 {
            s.set(
                &format!("IMG_2025{i:05}.JPG"),
                PhotoState {
                    selected: true,
                    crop: Some(CropBox::new(120.0, 240.0, 2000.0, 3000.0)),
                    ..Default::default()
                },
            );
        }
        s.last_file = Some("IMG_202500042.JPG".into());
        s.last_index = Some(42);

        let bytes = s.approximate_bytes();
        assert!(
            bytes < 32 * 1024,
            "300 decided photos should be a few tens of KB, got {bytes} bytes"
        );

        // And photos merely walked past contribute nothing at all.
        let before = s.approximate_bytes();
        for i in 0..10_000 {
            s.set(&format!("IMG_9{i:06}.JPG"), PhotoState::default());
        }
        assert_eq!(s.approximate_bytes(), before, "untouched photos must cost nothing");
    }
}
