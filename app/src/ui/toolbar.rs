//! The top bar: folders, print size, how Next/Previous behave, and Export.

use sort4print_core::config::{NavMode, RATIO_PRESETS};

use crate::app::Sort4Print;
use crate::ui::{folder_label, ACCENT};

pub fn show(app: &mut Sort4Print, ui: &mut egui::Ui) {
    egui::Panel::top("toolbar").show(ui, |ui| {
        ui.add_space(4.0);
        ui.horizontal_wrapped(|ui| {
            source_folder(app, ui);
            output_folder(app, ui);
            ui.separator();
            print_size(app, ui);
            ui.separator();
            navigation(app, ui);
            ui.separator();
            export(app, ui);

            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.toggle_value(&mut app.show_settings, "⚙ Settings");
            });
        });
        ui.add_space(4.0);
    });
}

fn source_folder(app: &mut Sort4Print, ui: &mut egui::Ui) {
    let label = folder_label(&app.config.source_dir, "Choose photos…");
    if ui
        .button(format!("📂 {label}"))
        .on_hover_text("The folder of photos to go through")
        .clicked()
    {
        let mut dialog = rfd::FileDialog::new();
        if let Some(current) = &app.config.source_dir {
            dialog = dialog.set_directory(current);
        }
        if let Some(dir) = dialog.pick_folder() {
            app.open_folder(&dir);
        }
    }
}

fn output_folder(app: &mut Sort4Print, ui: &mut egui::Ui) {
    let label = folder_label(&app.config.output_dir, "Choose destination…");
    if ui
        .button(format!("💾 {label}"))
        .on_hover_text("Where the cropped prints are written")
        .clicked()
    {
        let mut dialog = rfd::FileDialog::new();
        if let Some(current) = &app.config.output_dir {
            dialog = dialog.set_directory(current);
        }
        if let Some(dir) = dialog.pick_folder() {
            app.config.output_dir = Some(dir);
            app.config_dirty = true;
        }
    }
}

fn print_size(app: &mut Sort4Print, ui: &mut egui::Ui) {
    ui.label("Print");
    let mut changed = false;

    egui::ComboBox::from_id_salt("print-size")
        .selected_text(app.config.ratio.label())
        .width(96.0)
        .show_ui(ui, |ui| {
            for preset in RATIO_PRESETS {
                let selected = app.config.ratio == *preset;
                if ui.selectable_label(selected, preset.label()).clicked() && !selected {
                    app.config.ratio = *preset;
                    changed = true;
                }
            }
        });

    let response = ui.add(
        egui::TextEdit::singleline(&mut app.ratio_input)
            .desired_width(64.0)
            .hint_text("10:15"),
    );
    if response.changed() {
        if let Some(ratio) = sort4print_core::config::AspectRatio::parse(&app.ratio_input) {
            app.config.ratio = ratio;
            changed = true;
        }
    }
    response.on_hover_text("Any proportion, written as 10:15 or 10x15");

    if ui
        .checkbox(&mut app.config.ratio_follows_image, "↕ follow photo")
        .on_hover_text("Give a portrait photo a portrait crop of the same proportions")
        .changed()
    {
        changed = true;
    }

    if changed {
        app.reflow_crops();
        app.config_dirty = true;
    }
}

fn navigation(app: &mut Sort4Print, ui: &mut egui::Ui) {
    ui.label("Walk");

    let total = app.entries.len();
    let selected = app.selected_count();
    let unselected = total.saturating_sub(selected);
    let mut nav = app.config.nav;

    for mode in NavMode::ALL {
        let count = match mode {
            NavMode::All => total,
            NavMode::Selected => selected,
            NavMode::Unselected => unselected,
        };
        let hint = match mode {
            NavMode::All => "Next/Previous steps through every photo in the folder",
            NavMode::Selected => "Next/Previous visits only the photos you ticked",
            NavMode::Unselected => {
                "Next/Previous visits only what you have not ticked — the second \
                 pass, once the obvious ones are done"
            }
        };
        if ui
            .selectable_value(&mut nav, mode, format!("{} ({count})", mode.label()))
            .on_hover_text(hint)
            .clicked()
            && app.config.nav != nav
        {
            app.config.nav = nav;
            app.config_dirty = true;
        }
    }

    if selected > 0 {
        ui.colored_label(ACCENT, format!("✔ {selected}"));
    }
}

fn export(app: &mut Sort4Print, ui: &mut egui::Ui) {
    let running = app.export_run.as_ref().map(|r| r.running).unwrap_or(false);
    let count = app.selected_count();

    let button = egui::Button::new(if running {
        "Exporting…".to_string()
    } else {
        format!("⬇ Export {count}")
    });

    let enabled = app.can_export();
    let response = ui.add_enabled(enabled, button);
    if response.clicked() {
        app.start_export();
    }

    if !enabled && !running {
        let reason = if app.config.output_dir.is_none() {
            "Choose a destination folder first"
        } else if count == 0 {
            "Tick some photos first (Space)"
        } else {
            "Busy"
        };
        response.on_hover_text(reason);
    }
}
