//! The list of photos down the left: tick what to print, click to open.
//!
//! Rows are laid out by hand at an exact height rather than with nested
//! layouts. `ScrollArea::show_rows` only builds the rows that are on screen,
//! and to do that it has to trust the height it was told: a row whose content
//! came out a few pixels shorter left a gap under every one of them, a row with
//! an extra line of text pushed its neighbours out of place, and the scroll
//! offset that should have brought the current photo into view landed somewhere
//! else. Painting into a fixed rectangle makes the promise true by construction.
//!
//! Thumbnails are only requested for rows actually on screen and at the lowest
//! priority in the queue, so opening a folder of eleven thousand pictures does
//! not turn into eleven thousand decodes.

use crate::app::Sort4Print;
use crate::ui::{ACCENT, OK_GREEN};

const ROW_HEIGHT: f32 = 62.0;
const THUMB_WIDTH: f32 = 72.0;
const CHECKBOX_WIDTH: f32 = 22.0;
const PAD: f32 = 5.0;

pub fn show(app: &mut Sort4Print, ui: &mut egui::Ui) {
    egui::Panel::left("filmstrip")
        .resizable(true)
        .default_size(268.0)
        .size_range(180.0..=420.0)
        .show(ui, |ui| {
            header(app, ui);
            ui.separator();

            let total = app.entries.len();
            let viewport = ui.available_height();

            // Rows are a fixed height, so the offset that centres one can be
            // computed outright. That matters because `show_rows` never builds
            // the rows outside the viewport, so on a long jump there would be no
            // widget to scroll to.
            let mut scroll = egui::ScrollArea::vertical().auto_shrink([false, false]);
            if std::mem::take(&mut app.scroll_to_current) {
                let centred = app.current as f32 * ROW_HEIGHT - (viewport - ROW_HEIGHT) / 2.0;
                let furthest = (total as f32 * ROW_HEIGHT - viewport).max(0.0);
                scroll = scroll.vertical_scroll_offset(centred.clamp(0.0, furthest));
            }

            scroll.show_rows(ui, ROW_HEIGHT, total, |ui, range| {
                // Any spacing between rows would be height `show_rows` has not
                // accounted for, which is what put gaps between them.
                ui.spacing_mut().item_spacing.y = 0.0;
                for index in range {
                    row(app, ui, index);
                }
            });
        });
}

fn header(app: &mut Sort4Print, ui: &mut egui::Ui) {
    ui.add_space(4.0);
    ui.horizontal(|ui| {
        let selected = app.selected_count();
        ui.strong(format!("{selected} selected"));
        ui.label(format!("of {}", app.entries.len()));
    });

    ui.horizontal(|ui| {
        let mut bulk_change = false;
        if ui.small_button("All").clicked() {
            for entry in &mut app.entries {
                entry.selected = true;
            }
            bulk_change = true;
        }
        if ui.small_button("None").clicked() {
            for entry in &mut app.entries {
                entry.selected = false;
            }
            bulk_change = true;
        }
        if ui.small_button("Invert").clicked() {
            for entry in &mut app.entries {
                entry.selected = !entry.selected;
            }
            bulk_change = true;
        }
        if bulk_change {
            app.notes_changed_everywhere();
        }
        let exported = app.exported_count();
        if exported > 0 {
            ui.colored_label(OK_GREEN, format!("✔ {exported} written"));
        }
    });

    ui.horizontal(|ui| match &app.scan_all {
        Some(scan) => {
            let fraction = scan.fraction();
            ui.add(
                egui::ProgressBar::new(fraction)
                    .desired_width(150.0)
                    .text(format!("{}/{}", scan.next, scan.total)),
            );
            if ui.small_button("Stop").clicked() {
                app.stop_scan_all();
            }
        }
        None => {
            if ui
                .small_button("Read all")
                .on_hover_text(
                    "Go through the whole folder once, filling the cache, so that \
                     browsing afterwards waits for nothing. Runs in the background \
                     and can be stopped.",
                )
                .clicked()
            {
                app.start_scan_all();
            }
        }
    });
    ui.add_space(4.0);
}

fn row(app: &mut Sort4Print, ui: &mut egui::Ui, index: usize) {
    // Thumbnails are fetched lazily for whatever the scroll area is showing.
    app.request_thumb(index);

    let is_current = index == app.current;
    let (selected, name, exported, path) = {
        let entry = &app.entries[index];
        (
            entry.selected,
            entry.file_name(),
            entry.exported,
            entry.path.clone(),
        )
    };

    let (rect, response) = ui.allocate_exact_size(
        egui::vec2(ui.available_width(), ROW_HEIGHT),
        egui::Sense::click(),
    );

    let cached = app.prefetch.thumb(&path);
    let texture = cached.map(|image| app.thumb_texture_for(ui.ctx(), &path, &image));
    let error = app.prefetch.error(&path).map(str::to_string);

    let painter = ui.painter_at(rect);
    let body = rect.shrink2(egui::vec2(PAD, 3.0));

    if is_current {
        painter.rect_filled(rect.shrink(1.0), 3.0, ui.visuals().selection.bg_fill);
    } else if selected {
        // A picked photo reads as picked in the list too, not only in the view.
        painter.rect_filled(rect.shrink(1.0), 3.0, crate::ui::PICKED_GROUND);
    }

    // Thumbnail, letterboxed into its slot so nothing is stretched.
    let slot = egui::Rect::from_min_size(
        egui::pos2(body.left() + CHECKBOX_WIDTH, body.top()),
        egui::vec2(THUMB_WIDTH, body.height()),
    );
    match &texture {
        Some(texture) => {
            let size = texture.size_vec2();
            let scale = (slot.width() / size.x).min(slot.height() / size.y);
            let drawn = egui::Rect::from_center_size(slot.center(), size * scale);
            painter.image(
                texture.id(),
                drawn,
                egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                egui::Color32::WHITE,
            );
        }
        None => {
            painter.rect_filled(slot, 2.0, ui.visuals().extreme_bg_color);
        }
    }

    // Name, and whatever else is worth saying about this one.
    let text_left = slot.right() + 6.0;
    let mut line_y = body.top() + 2.0;
    let name_colour = if selected {
        ACCENT
    } else {
        ui.visuals().text_color()
    };
    painter.text(
        egui::pos2(text_left, line_y),
        egui::Align2::LEFT_TOP,
        elide(&name, body.right() - text_left, 11.5),
        egui::FontId::proportional(11.5),
        name_colour,
    );
    line_y += 15.0;

    if selected {
        painter.text(
            egui::pos2(text_left, line_y),
            egui::Align2::LEFT_TOP,
            "✔ picked",
            egui::FontId::proportional(10.5),
            OK_GREEN,
        );
        line_y += 13.0;
    }
    if exported {
        painter.text(
            egui::pos2(text_left, line_y),
            egui::Align2::LEFT_TOP,
            "written",
            egui::FontId::proportional(10.5),
            OK_GREEN,
        );
        line_y += 13.0;
    }
    if let Some(error) = error {
        painter.text(
            egui::pos2(text_left, line_y),
            egui::Align2::LEFT_TOP,
            elide(&error, body.right() - text_left, 10.5),
            egui::FontId::proportional(10.5),
            egui::Color32::from_rgb(229, 115, 115),
        );
    }

    // The tick goes in last so it sits above the row's background.
    let check = egui::Rect::from_min_size(
        egui::pos2(body.left(), rect.center().y - 9.0),
        egui::vec2(18.0, 18.0),
    );
    let mut ticked = selected;
    if ui.put(check, egui::Checkbox::new(&mut ticked, "")).changed() {
        app.entries[index].selected = ticked;
        // Without this the tick is lost on the next restart: the notes file
        // only writes what it has been told changed.
        app.note_changed(index);
    }

    if response.clicked() {
        app.go_to(index);
    }
}

/// Cuts a string to what will fit, since the row paints into a fixed width and
/// has no layout to wrap for it.
fn elide(text: &str, width: f32, font_size: f32) -> String {
    // Proportional text averages a little over half the font size per character;
    // erring on the narrow side is what keeps a long name off the edge.
    let fits = (width / (font_size * 0.52)).floor().max(4.0) as usize;
    if text.chars().count() <= fits {
        return text.to_string();
    }
    let kept: String = text.chars().take(fits.saturating_sub(1)).collect();
    format!("{kept}…")
}
