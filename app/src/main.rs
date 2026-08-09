// Release builds are a GUI program: no console window should flash up when the
// exe is double-clicked. Debug builds keep the console so panics are visible.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;
mod prefetch;
mod ui;

use app::Sort4Print;

fn main() -> eframe::Result<()> {
    prefetch::quiet_worker_panics();

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("sort4print")
            .with_inner_size([1360.0, 860.0])
            .with_min_inner_size([980.0, 620.0]),
        ..Default::default()
    };

    eframe::run_native(
        "sort4print",
        options,
        Box::new(|cc| Ok(Box::new(Sort4Print::new(cc)))),
    )
}
