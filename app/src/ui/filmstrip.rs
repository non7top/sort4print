//! The list of photos down the left: tick what to print, click to open.
//!
//! Thumbnails are only requested for rows that are actually on screen and at
//! the lowest priority in the queue, so opening a folder of two thousand
//! pictures does not turn into two thousand decodes.

use crate::app::Sort4Print;
use crate::ui::{ACCENT, OK_GREEN};

const ROW_HEIGHT: f32 = 62.0;

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
            // the rows outside the viewport, so there would otherwise be no
            // widget to scroll to when the jump is a long one.
            let mut scroll = egui::ScrollArea::vertical().auto_shrink([false, false]);
            if std::mem::take(&mut app.scroll_to_current) {
                let centred = app.current as f32 * ROW_HEIGHT - (viewport - ROW_HEIGHT) / 2.0;
                let furthest = (total as f32 * ROW_HEIGHT - viewport).max(0.0);
                scroll = scroll.vertical_scroll_offset(centred.clamp(0.0, furthest));
            }

            scroll.show_rows(ui, ROW_HEIGHT, total, |ui, range| {
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

    let background = if is_current {
        ui.visuals().selection.bg_fill
    } else {
        egui::Color32::TRANSPARENT
    };

    let response = egui::Frame::new()
        .fill(background)
        .inner_margin(egui::Margin::symmetric(4, 3))
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                let mut ticked = selected;
                if ui.checkbox(&mut ticked, "").changed() {
                    app.entries[index].selected = ticked;
                    // Without this the tick is lost on the next restart: the
                    // notes file only writes what it has been told changed.
                    app.note_changed(index);
                }

                let cached = app.prefetch.thumb(&path);
                let thumb = cached.map(|image| app.thumb_texture_for(ui.ctx(), &path, &image));

                match thumb {
                    Some(texture) => {
                        ui.add(
                            egui::Image::new(&texture)
                                .fit_to_exact_size(egui::vec2(72.0, ROW_HEIGHT - 12.0))
                                .corner_radius(2.0),
                        );
                    }
                    None => {
                        let (rect, _) = ui.allocate_exact_size(
                            egui::vec2(72.0, ROW_HEIGHT - 12.0),
                            egui::Sense::hover(),
                        );
                        ui.painter()
                            .rect_filled(rect, 2.0, ui.visuals().extreme_bg_color);
                    }
                }

                ui.vertical(|ui| {
                    ui.add_space(2.0);
                    let label = egui::RichText::new(name).small();
                    ui.label(if selected { label.color(ACCENT) } else { label });
                    if exported {
                        ui.colored_label(OK_GREEN, egui::RichText::new("written").small());
                    }
                    if let Some(error) = app.prefetch.error(&path) {
                        ui.colored_label(
                            egui::Color32::from_rgb(229, 115, 115),
                            egui::RichText::new(error).small(),
                        );
                    }
                });
            })
        })
        .response;

    if response.interact(egui::Sense::click()).clicked() {
        app.go_to(index);
    }
}
