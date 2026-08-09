// Release builds are a GUI program: no console window should flash up when the
// exe is double-clicked. Debug builds keep the console so panics are visible.
//
// The cost of that is that nothing printed to stderr in a release build has
// anywhere to go, which is why `diagnostics` exists and is set up first.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;
mod diagnostics;
mod prefetch;
mod ui;

use app::Sort4Print;

fn main() {
    diagnostics::install_panic_hook();
    diagnostics::start_run();

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("sort4print")
            .with_inner_size([1360.0, 860.0])
            .with_min_inner_size([980.0, 620.0]),
        ..Default::default()
    };

    diagnostics::log("creating the window");

    let result = eframe::run_native(
        "sort4print",
        options,
        Box::new(|cc| {
            diagnostics::log("graphics context ready; building the application");
            let app = Sort4Print::new(cc);
            diagnostics::log("application built");
            Ok(Box::new(app))
        }),
    );

    match result {
        Ok(()) => diagnostics::log("closed normally"),
        // Overwhelmingly this is the window or the graphics backend refusing to
        // start, which on Windows means no OpenGL 3.3 — a bare install without
        // GPU drivers, or a remote desktop session.
        Err(error) => diagnostics::fatal(
            "sort4print could not open its window",
            &format!("{error}"),
        ),
    }
}
