//! The right-hand panel: what applies to the picture in front of you at the
//! top, and the settings that apply to everything below it.

use sort4print_core::config::{Color, Corner};
use sort4print_core::datefmt::{PhotoDate, PRESETS, TOKEN_HINT};
use sort4print_core::geo::CityDb;

use crate::app::{Sort4Print, SettingsTab};
use crate::ui::editor::caption_swatch;

pub fn show(app: &mut Sort4Print, ui: &mut egui::Ui) {
    egui::Panel::right("settings")
        .resizable(true)
        .default_size(340.0)
        .size_range(280.0..=520.0)
        .show(ui, |ui| {
            egui::ScrollArea::vertical()
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    this_picture(app, ui);
                    ui.separator();
                    tabs(app, ui);
                    ui.separator();
                    match app.settings_tab {
                        SettingsTab::Caption => caption_tab(app, ui),
                        SettingsTab::Date => date_tab(app, ui),
                        SettingsTab::Output => output_tab(app, ui),
                        SettingsTab::Performance => performance_tab(app, ui),
                        SettingsTab::About => about_tab(app, ui),
                    }
                });
        });
}

fn tabs(app: &mut Sort4Print, ui: &mut egui::Ui) {
    ui.horizontal_wrapped(|ui| {
        for (tab, label) in [
            (SettingsTab::Caption, "Caption & font"),
            (SettingsTab::Date, "Date"),
            (SettingsTab::Output, "Output"),
            (SettingsTab::Performance, "Speed"),
            (SettingsTab::About, "About"),
        ] {
            ui.selectable_value(&mut app.settings_tab, tab, label);
        }
    });
}

// ---- current picture -----------------------------------------------------

fn this_picture(app: &mut Sort4Print, ui: &mut egui::Ui) {
    ui.add_space(6.0);
    ui.heading("This picture");

    let Some(index) = (!app.entries.is_empty()).then_some(app.current) else {
        ui.weak("Nothing open.");
        return;
    };

    let path = app.entries[index].path.clone();
    let image = app.prefetch.preview(&path);

    ui.label(app.entries[index].file_name());

    let (date_text, gps, geocoded) = match &image {
        Some(loaded) => {
            let locale = app.config.locales.get_or_default(&app.config.date_locale);
            let date = loaded
                .preview
                .meta
                .date
                .map(|d| sort4print_core::datefmt::format_date(&app.config.date_pattern, d, locale))
                .unwrap_or_else(|| "no date in EXIF".to_string());
            (date, loaded.preview.meta.gps, loaded.place.clone())
        }
        None => ("…".to_string(), None, None),
    };

    ui.horizontal(|ui| {
        ui.label("Date:");
        ui.strong(date_text);
    });

    match (&geocoded, gps) {
        (Some(place), _) => {
            ui.horizontal_wrapped(|ui| {
                ui.label("Nearest:");
                ui.strong(format!("{}, {}", place.city, place.country));
                ui.weak(format!("{:.0} km", place.distance_km));
            });
        }
        (None, Some(_)) => {
            ui.weak("GPS present but no city matched.");
        }
        (None, None) => {
            ui.weak("No GPS in this photo — type the place below.");
        }
    }

    // Overrides. Empty means "use the geocoded value".
    let mut changed = false;
    let mut city = app.entries[index].city_override.clone().unwrap_or_default();
    let mut country = app.entries[index]
        .country_override
        .clone()
        .unwrap_or_default();

    let city_hint = geocoded
        .as_ref()
        .map(|p| p.city.clone())
        .unwrap_or_else(|| "City".into());
    let country_hint = geocoded
        .as_ref()
        .map(|p| p.country.clone())
        .unwrap_or_else(|| "Country".into());

    ui.horizontal(|ui| {
        ui.label("City");
        if ui
            .add(
                egui::TextEdit::singleline(&mut city)
                    .hint_text(city_hint)
                    .desired_width(f32::INFINITY),
            )
            .changed()
        {
            app.entries[index].city_override = (!city.trim().is_empty()).then(|| city.clone());
            changed = true;
        }
    });
    ui.horizontal(|ui| {
        ui.label("Country");
        if ui
            .add(
                egui::TextEdit::singleline(&mut country)
                    .hint_text(country_hint)
                    .desired_width(f32::INFINITY),
            )
            .changed()
        {
            app.entries[index].country_override =
                (!country.trim().is_empty()).then(|| country.clone());
            changed = true;
        }
    });

    // The name of the spot itself, which no database can supply.
    let mut description = app.entries[index].description.clone().unwrap_or_default();
    ui.horizontal(|ui| {
        ui.label("Place");
        if ui
            .add(
                egui::TextEdit::singleline(&mut description)
                    .hint_text("Chinatown, the old harbour, …")
                    .desired_width(f32::INFINITY),
            )
            .on_hover_text("Use {description} in the caption text to print this")
            .changed()
        {
            app.entries[index].description =
                (!description.trim().is_empty()).then(|| description.trim().to_string());
            changed = true;
        }
    });
    if !app.config.caption.template.contains("{description}")
        && app.entries[index].description.is_some()
    {
        ui.colored_label(
            egui::Color32::from_rgb(230, 180, 90),
            "The caption text has no {description} in it, so this will not print.",
        );
    }

    nearby_places(app, ui, index, gps);

    ui.horizontal(|ui| {
        if ui.small_button("Clear").clicked() {
            app.entries[index].city_override = None;
            app.entries[index].country_override = None;
            app.entries[index].description = None;
            changed = true;
        }
        if ui
            .small_button("Apply to all")
            .on_hover_text("Give every photo in the folder this city and country")
            .clicked()
        {
            let (c, k) = (
                app.entries[index].city_override.clone(),
                app.entries[index].country_override.clone(),
            );
            for entry in &mut app.entries {
                entry.city_override = c.clone();
                entry.country_override = k.clone();
            }
            // Descriptions are deliberately left alone: "Chinatown" is about
            // one picture, not about the folder.
            app.notes_changed_everywhere();
        }
    });

    if changed {
        app.note_changed(index);
    }

    let caption = app.caption_for(index, image.as_deref());
    ui.horizontal_wrapped(|ui| {
        ui.label("Caption:");
        if caption.is_empty() {
            ui.weak("(nothing to print)");
        } else {
            ui.strong(caption);
        }
    });
}

/// A short list of alternatives near the GPS fix, plus a free-text search for
/// photos that have no fix at all.
fn nearby_places(app: &mut Sort4Print, ui: &mut egui::Ui, index: usize, gps: Option<(f64, f64)>) {
    egui::CollapsingHeader::new("Pick a different place")
        .default_open(false)
        .show(ui, |ui| {
            if let Some((lat, lon)) = gps {
                for place in CityDb::embedded().nearest_n(lat, lon, 8) {
                    let label = format!("{}, {} · {:.0} km", place.city, place.country, place.distance_km);
                    if ui.selectable_label(false, label).clicked() {
                        app.entries[index].city_override = Some(place.city.clone());
                        app.entries[index].country_override = Some(place.country.clone());
                    }
                }
                ui.separator();
            }

            ui.horizontal(|ui| {
                ui.label("Search");
                ui.add(
                    egui::TextEdit::singleline(&mut app.place_search)
                        .hint_text("city name")
                        .desired_width(f32::INFINITY),
                );
            });
            if app.place_search.trim().len() >= 2 {
                for place in CityDb::embedded().search(&app.place_search, 10) {
                    let label = format!("{}, {}", place.city, place.country);
                    if ui.selectable_label(false, label).clicked() {
                        app.entries[index].city_override = Some(place.city.clone());
                        app.entries[index].country_override = Some(place.country.clone());
                    }
                }
            }
        });
}

// ---- caption and font ----------------------------------------------------

fn caption_tab(app: &mut Sort4Print, ui: &mut egui::Ui) {
    let mut changed = false;

    changed |= ui
        .checkbox(&mut app.config.caption.enabled, "Print the caption")
        .changed();

    ui.add_space(4.0);
    ui.label("Text");
    changed |= ui
        .add(
            egui::TextEdit::multiline(&mut app.config.caption.template)
                .desired_rows(2)
                .desired_width(f32::INFINITY)
                .font(egui::TextStyle::Monospace),
        )
        .changed();
    ui.label(
        egui::RichText::new("{city} {country} {place} {description} {date} {filename}")
            .small()
            .weak(),
    );

    ui.add_space(6.0);
    ui.horizontal(|ui| {
        ui.label("Corner");
        egui::ComboBox::from_id_salt("caption-corner")
            .selected_text(app.config.caption.corner.label())
            .show_ui(ui, |ui| {
                for corner in Corner::ALL {
                    changed |= ui
                        .selectable_value(&mut app.config.caption.corner, corner, corner.label())
                        .changed();
                }
            });
        changed |= ui
            .checkbox(&mut app.config.caption.uppercase, "UPPERCASE")
            .changed();
    });

    ui.add_space(8.0);
    changed |= font_picker(app, ui);

    ui.add_space(8.0);
    ui.label("Size");
    changed |= ui
        .add(
            egui::Slider::new(&mut app.config.caption.size_pct, 0.5..=12.0)
                .suffix(" %")
                .text("of the short side"),
        )
        .changed();
    changed |= ui
        .add(
            egui::Slider::new(&mut app.config.caption.outline_pct, 0.0..=40.0)
                .suffix(" %")
                .text("outline, of size"),
        )
        .changed();
    changed |= ui
        .add(
            egui::Slider::new(&mut app.config.caption.margin_pct, 0.0..=15.0)
                .suffix(" %")
                .text("margin"),
        )
        .changed();

    ui.add_space(6.0);
    ui.horizontal(|ui| {
        ui.label("Fill");
        changed |= color_edit(ui, &mut app.config.caption.fill);
        ui.label("Outline");
        changed |= color_edit(ui, &mut app.config.caption.outline);
    });

    ui.add_space(10.0);
    ui.label("Preview");
    let sample = sample_caption(app);
    caption_swatch(app, ui, &sample);

    if changed {
        app.config_dirty = true;
        app.caption_overlay = None;
    }
}

/// OpenOffice-style family list: a filter box, every installed family, and the
/// styles that family actually ships. Names are drawn in the interface font
/// rather than each in its own face — loading four hundred fonts to paint a
/// list would cost more than the rest of the program put together — so the
/// swatch underneath is what shows you the real thing.
fn font_picker(app: &mut Sort4Print, ui: &mut egui::Ui) -> bool {
    let mut changed = false;

    ui.label("Font");
    if app.font_catalog.is_empty() {
        ui.weak("No fonts were found on this system.");
        return false;
    }

    ui.horizontal(|ui| {
        ui.add(
            egui::TextEdit::singleline(&mut app.font_filter)
                .hint_text("filter…")
                .desired_width(120.0),
        );
        ui.strong(app.config.caption.font_family.clone());
    });

    let catalog = app.font_catalog.clone();
    let filter = app.font_filter.to_lowercase();
    let families: Vec<&str> = catalog
        .families()
        .into_iter()
        .filter(|f| filter.is_empty() || f.to_lowercase().contains(&filter))
        .collect();

    egui::ScrollArea::vertical()
        .id_salt("font-list")
        .max_height(150.0)
        .show(ui, |ui| {
            for family in families {
                let selected = app.config.caption.font_family.eq_ignore_ascii_case(family);
                if ui.selectable_label(selected, family).clicked() && !selected {
                    app.config.caption.font_family = family.to_string();
                    // Keep the current style if this family has it, otherwise
                    // fall back to whatever it does offer.
                    let styles = catalog.styles_for(family);
                    if !styles
                        .iter()
                        .any(|s| s.eq_ignore_ascii_case(&app.config.caption.font_style))
                    {
                        app.config.caption.font_style = styles
                            .first()
                            .map(|s| s.to_string())
                            .unwrap_or_else(|| "Regular".into());
                    }
                    changed = true;
                }
            }
        });

    ui.horizontal(|ui| {
        ui.label("Style");
        egui::ComboBox::from_id_salt("font-style")
            .selected_text(app.config.caption.font_style.clone())
            .show_ui(ui, |ui| {
                for style in catalog.styles_for(&app.config.caption.font_family) {
                    let selected = app.config.caption.font_style.eq_ignore_ascii_case(style);
                    if ui.selectable_label(selected, style).clicked() && !selected {
                        app.config.caption.font_style = style.to_string();
                        changed = true;
                    }
                }
            });
    });

    if catalog
        .find(
            &app.config.caption.font_family,
            &app.config.caption.font_style,
        )
        .is_none()
    {
        ui.colored_label(
            egui::Color32::from_rgb(230, 180, 90),
            "That exact cut is not installed; the closest one is used.",
        );
    }

    changed
}

fn sample_caption(app: &Sort4Print) -> String {
    let text = app.caption_for(app.current, None);
    if !text.trim().is_empty() {
        return text;
    }
    // Nothing to show from the current photo: use a representative stand-in so
    // the preview is never blank.
    let locale = app.config.locales.get_or_default(&app.config.date_locale);
    let date = sort4print_core::datefmt::format_date(
        &app.config.date_pattern,
        PhotoDate {
            year: 2025,
            month: 10,
            day: 5,
            hour: 12,
            minute: 0,
        },
        locale,
    );
    let sample = sort4print_core::export::build_caption(
        &app.config.caption.template,
        &sort4print_core::export::CaptionFields {
            city: "Lisbon".into(),
            country: "Portugal".into(),
            description: "Alfama".into(),
            date,
            filename: "IMG_0042".into(),
        },
    );
    if app.config.caption.uppercase {
        sample.to_uppercase()
    } else {
        sample
    }
}

// ---- date ----------------------------------------------------------------

fn date_tab(app: &mut Sort4Print, ui: &mut egui::Ui) {
    let mut changed = false;

    ui.horizontal(|ui| {
        ui.label("Language");
        let current = app
            .config
            .locales
            .get_or_default(&app.config.date_locale)
            .label
            .clone();
        egui::ComboBox::from_id_salt("date-locale")
            .selected_text(current)
            .show_ui(ui, |ui| {
                let ids: Vec<(String, String)> = app
                    .config
                    .locales
                    .iter()
                    .map(|l| (l.id.clone(), l.label.clone()))
                    .collect();
                for (id, label) in ids {
                    let selected = app.config.date_locale == id;
                    if ui.selectable_label(selected, label).clicked() && !selected {
                        app.config.date_locale = id;
                        changed = true;
                    }
                }
            });
    });
    ui.label(
        egui::RichText::new(
            "English and Russian are built in. Add any other language with a \
             [locale.xx] section in the ini file — see About for where it lives.",
        )
        .small()
        .weak(),
    );

    ui.add_space(8.0);
    ui.label("Format");
    let locale = app
        .config
        .locales
        .get_or_default(&app.config.date_locale)
        .clone();
    let sample = PhotoDate {
        year: 2025,
        month: 10,
        day: 5,
        hour: 9,
        minute: 7,
    };

    for (_, pattern) in PRESETS {
        let rendered = sort4print_core::datefmt::format_date(pattern, sample, &locale);
        let selected = app.config.date_pattern == *pattern;
        if ui.selectable_label(selected, rendered).clicked() && !selected {
            app.config.date_pattern = pattern.to_string();
            changed = true;
        }
    }

    ui.add_space(6.0);
    ui.label("Custom pattern");
    changed |= ui
        .add(
            egui::TextEdit::singleline(&mut app.config.date_pattern)
                .desired_width(f32::INFINITY)
                .font(egui::TextStyle::Monospace),
        )
        .changed();

    ui.horizontal_wrapped(|ui| {
        ui.label("Now reads:");
        ui.strong(sort4print_core::datefmt::format_date(
            &app.config.date_pattern,
            sample,
            &locale,
        ));
    });
    ui.label(egui::RichText::new(TOKEN_HINT).small().weak());

    if changed {
        app.config_dirty = true;
        app.caption_overlay = None;
        app.caption_swatch = None;
    }
}

// ---- output --------------------------------------------------------------

fn output_tab(app: &mut Sort4Print, ui: &mut egui::Ui) {
    let mut changed = false;

    ui.label("Destination");
    ui.horizontal_wrapped(|ui| {
        match &app.config.output_dir {
            Some(dir) => ui.strong(dir.display().to_string()),
            None => ui.weak("not chosen"),
        };
    });
    if ui.button("Choose…").clicked() {
        if let Some(dir) = rfd::FileDialog::new().pick_folder() {
            app.config.output_dir = Some(dir);
            changed = true;
        }
    }

    ui.add_space(8.0);
    ui.label("JPEG quality");
    let mut quality = app.config.jpeg_quality as u32;
    if ui
        .add(egui::Slider::new(&mut quality, 60..=100))
        .on_hover_text("100 keeps the crop as close to the original as JPEG allows")
        .changed()
    {
        app.config.jpeg_quality = quality as u8;
        changed = true;
    }

    changed |= ui
        .checkbox(
            &mut app.config.preserve_exif,
            "Keep the camera's EXIF in the export",
        )
        .changed();

    ui.add_space(8.0);
    ui.horizontal(|ui| {
        ui.label("Border colour");
        changed |= color_edit(ui, &mut app.config.background);
    });
    ui.label(
        egui::RichText::new(
            "Used where the crop window reaches past the photo, which is how a \
             printed border is made.",
        )
        .small()
        .weak(),
    );

    ui.add_space(8.0);
    ui.label(
        egui::RichText::new(
            "The crop is taken from the original at full resolution and never \
             resampled — the proportions are already exact.",
        )
        .small()
        .weak(),
    );

    if changed {
        app.config_dirty = true;
    }
}

// ---- performance ---------------------------------------------------------

fn performance_tab(app: &mut Sort4Print, ui: &mut egui::Ui) {
    let mut changed = false;

    ui.label("Background loading");
    ui.label(
        egui::RichText::new(
            "Photos around the one you are looking at are decoded, rotated, \
             measured and geocoded on worker threads, so stepping through the \
             folder does not wait for anything.",
        )
        .small()
        .weak(),
    );

    changed |= ui
        .add(egui::Slider::new(&mut app.config.prefetch.ahead, 0..=24).text("ahead"))
        .changed();
    changed |= ui
        .add(egui::Slider::new(&mut app.config.prefetch.behind, 0..=24).text("behind"))
        .changed();
    changed |= ui
        .add(
            egui::Slider::new(&mut app.config.prefetch.workers, 0..=16)
                .text("worker threads (0 = automatic)"),
        )
        .changed();

    let mut cache = app.config.prefetch.cache;
    if ui
        .add(egui::Slider::new(&mut cache, 2..=120).text("images kept in memory"))
        .changed()
    {
        app.config.prefetch.cache = cache;
        app.prefetch.set_cache_limit(cache);
        changed = true;
    }

    ui.add_space(8.0);
    let mut preview_px = app.config.preview_max_px;
    if ui
        .add(egui::Slider::new(&mut preview_px, 800..=4000).text("preview size, px"))
        .on_hover_text("Only affects what is on screen; exports always use the original")
        .changed()
    {
        app.config.preview_max_px = preview_px;
        app.clear_image_caches();
        changed = true;
    }

    ui.add_space(8.0);
    ui.weak(format!("{} previews cached", app.prefetch.cached_count()));

    if changed {
        app.config_dirty = true;
    }
}

// ---- about ---------------------------------------------------------------

fn about_tab(app: &mut Sort4Print, ui: &mut egui::Ui) {
    ui.heading(format!("sort4print {}", sort4print_core::VERSION));
    ui.add_space(6.0);
    ui.label("Keyboard");
    ui.label(
        egui::RichText::new(
            "←  →   previous / next\n\
             Space   pick or unpick this photo\n\
             Enter   export everything picked\n\
             drag    move the crop window\n\
             handles resize it, proportions locked\n\
             scroll  zoom the crop window",
        )
        .small(),
    );

    ui.add_space(10.0);
    ui.label("What is remembered, and where");

    let config_path = app.config_path.clone();
    file_line(ui, "Settings", &config_path);

    let notes_path = app
        .config
        .source_dir
        .as_ref()
        .map(|dir| sort4print_core::sidecar::Sidecar::path_for(dir));
    match &notes_path {
        Some(path) => file_line(ui, "Notes on these photos", path),
        None => {
            ui.horizontal(|ui| {
                ui.label("Notes on these photos");
                ui.weak("no folder open");
            });
        }
    }

    file_line(
        ui,
        "Log",
        std::path::Path::new(&crate::diagnostics::log_path_display()),
    );

    if ui
        .button("Save now")
        .on_hover_text("Both files are written as you go; this forces it")
        .clicked()
    {
        app.notes_changed_everywhere();
        app.save_config();
        app.save_notes();
    }

    ui.add_space(10.0);
    ui.label("Formats");
    ui.label(
        egui::RichText::new(
            "JPEG, PNG, TIFF, WebP and BMP are read. HEIC, which iPhones shoot \
             by default, is not: decoding it needs a large C library that would \
             end this program's single-file, no-install property. Set the phone \
             to \"Most Compatible\", or convert first.",
        )
        .small()
        .weak(),
    );

    ui.add_space(10.0);
    ui.label(egui::RichText::new(sort4print_core::ATTRIBUTION).small().weak());
}

/// One line of "here is the file, and here is whether it is actually there".
/// Worth showing plainly: a settings file that is silently not being written
/// looks exactly like one that is, until the next time you open the program.
fn file_line(ui: &mut egui::Ui, label: &str, path: &std::path::Path) {
    ui.horizontal_wrapped(|ui| {
        ui.label(label);
        match std::fs::metadata(path) {
            Ok(meta) => ui.colored_label(
                crate::ui::OK_GREEN,
                format!("✔ {} bytes", meta.len()),
            ),
            Err(_) => ui.colored_label(
                egui::Color32::from_rgb(230, 180, 90),
                "not written yet",
            ),
        };
    });
    ui.label(
        egui::RichText::new(path.display().to_string())
            .small()
            .monospace()
            .weak(),
    );
}

fn color_edit(ui: &mut egui::Ui, color: &mut Color) -> bool {
    let mut rgba = egui::Color32::from_rgba_unmultiplied(color.r, color.g, color.b, color.a);
    let changed = egui::color_picker::color_edit_button_srgba(
        ui,
        &mut rgba,
        egui::color_picker::Alpha::OnlyBlend,
    )
    .changed();
    if changed {
        *color = Color {
            r: rgba.r(),
            g: rgba.g(),
            b: rgba.b(),
            a: rgba.a(),
        };
    }
    changed
}
