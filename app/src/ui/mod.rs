//! The panels. Each module owns one region of the window and takes the whole
//! application state, because nearly every control affects something outside
//! its own panel — picking a print size reflows every crop, ticking a picture
//! changes what the navigation buttons do.

pub mod editor;
pub mod filmstrip;
pub mod settings;
pub mod status_bar;
pub mod theme;
pub mod toolbar;

use egui::Color32;

/// The crop window. Amber reads against both palettes and against a
/// photograph, which the interface accent would not.
pub const ACCENT: Color32 = theme::CROP_AMBER;
pub const OK_GREEN: Color32 = theme::PICKED_GREEN;

/// Ground behind a picked photo in the main view. A hint, never a wash: no
/// green may fall on the photograph, since a tint misrepresents the thing being
/// judged.
pub const PICKED_GROUND: Color32 = theme::CANVAS_PICKED;

pub fn folder_label(path: &Option<std::path::PathBuf>, empty: &str) -> String {
    match path {
        Some(p) => {
            let text = p.display().to_string();
            // Long Windows paths would push everything else off the toolbar.
            if text.chars().count() > 46 {
                let tail: String = text.chars().rev().take(43).collect::<Vec<_>>().into_iter().rev().collect();
                format!("…{tail}")
            } else {
                text
            }
        }
        None => empty.to_string(),
    }
}
