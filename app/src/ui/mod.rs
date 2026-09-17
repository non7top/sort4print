//! The panels. Each module owns one region of the window and takes the whole
//! application state, because nearly every control affects something outside
//! its own panel — picking a print size reflows every crop, ticking a picture
//! changes what the navigation buttons do.

pub mod editor;
pub mod filmstrip;
pub mod settings;
pub mod status_bar;
pub mod toolbar;

use egui::Color32;

/// Accent used for the crop window and the selected state.
pub const ACCENT: Color32 = Color32::from_rgb(255, 176, 32);
pub const OK_GREEN: Color32 = Color32::from_rgb(102, 187, 106);

/// Ground behind a picked photo, in the list.
///
/// Deliberately a hint rather than a wash. The signal has to be readable at a
/// glance without the eye being dragged to it, and nothing green may fall on the
/// photograph itself — a tint over the picture misrepresents the thing you are
/// judging.
pub const PICKED_GROUND: Color32 = Color32::from_rgb(20, 46, 28);

/// The same, for the editor's canvas surround, where the photo may not fill the
/// whole area. Brighter than `PICKED_GROUND`: the surround is the one place this
/// signal has room to be seen clearly rather than as a thin edge, so it can
/// afford more saturation than a hint.
pub const CANVAS_SURROUND_PICKED: Color32 = Color32::from_rgb(19, 73, 37);

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
