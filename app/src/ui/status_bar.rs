//! The one-line summary along the bottom: where you are in the folder, what the
//! current crop will produce, and what the last action did.

use crate::app::Sort4Print;

pub fn show(app: &mut Sort4Print, ui: &mut egui::Ui) {
    egui::Panel::bottom("status").show(ui, |ui| {
        ui.horizontal(|ui| {
            if app.entries.is_empty() {
                ui.label("No folder open.");
            } else {
                ui.label(format!("{} / {}", app.current + 1, app.entries.len()));
                ui.separator();
                if let Some(entry) = app.current_entry() {
                    ui.label(entry.file_name());
                }
            }

            if let Some(entry) = app.current_entry() {
                if let Some(crop) = entry.crop {
                    let (_, _, w, h) = crop.to_pixel_rect();
                    ui.separator();
                    ui.label(format!("Crop {w}×{h} px"));
                }
            }

            if let Some(run) = &app.export_run {
                ui.separator();
                if run.running {
                    ui.add(egui::ProgressBar::new(run.done as f32 / run.total.max(1) as f32)
                        .desired_width(140.0)
                        .text(format!("{}/{}", run.done, run.total)));
                } else if !run.failures.is_empty() {
                    ui.colored_label(
                        egui::Color32::from_rgb(229, 115, 115),
                        format!("{} failed", run.failures.len()),
                    );
                }
            }

            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.label(&app.status);
            });
        });
    });
}
