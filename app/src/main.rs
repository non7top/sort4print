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

/// Rendering backends in the order they are tried.
///
/// wgpu goes first because on Windows it means Direct3D 12, which works on
/// anything that runs a current Windows. OpenGL is second: it is the lighter
/// path where it works, but glutin cannot get a context at all on a machine
/// with only the basic display driver or over remote desktop, which is exactly
/// where the program looked like it was doing nothing at all.
const BACKENDS: &[(&str, eframe::Renderer)] = &[
    ("wgpu (Direct3D 12 / Vulkan / Metal)", eframe::Renderer::Wgpu),
    ("glow (OpenGL)", eframe::Renderer::Glow),
];

fn main() {
    diagnostics::install_panic_hook();
    diagnostics::start_run();

    let mut failures: Vec<String> = Vec::new();

    for (name, renderer) in BACKENDS {
        diagnostics::log(&format!("opening the window with {name}"));

        let options = eframe::NativeOptions {
            renderer: *renderer,
            viewport: egui::ViewportBuilder::default()
                .with_title("sort4print")
                .with_inner_size([1360.0, 860.0])
                .with_min_inner_size([980.0, 620.0]),
            ..Default::default()
        };

        // The creator is FnOnce, so each attempt needs its own.
        let backend = *name;
        let result = eframe::run_native(
            "sort4print",
            options,
            Box::new(move |cc| {
                diagnostics::log(&format!("graphics ready on {backend}; building the application"));
                let app = Sort4Print::new(cc);
                diagnostics::log("application built");
                Ok(Box::new(app) as Box<dyn eframe::App>)
            }),
        );

        match result {
            Ok(()) => {
                diagnostics::log(&format!("closed normally ({name})"));
                return;
            }
            Err(error) => {
                diagnostics::log(&format!("{name} could not start: {error}"));
                failures.push(format!("{name}: {error}"));
            }
        }
    }

    diagnostics::fatal(
        "sort4print could not open its window",
        &format!(
            "No rendering backend would start.\n\n{}",
            failures.join("\n")
        ),
    );
}
