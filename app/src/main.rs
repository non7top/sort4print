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

/// Rendering backends, in the order they are tried.
///
/// Neither one works everywhere, and on an Optimus laptop the two halves of the
/// same machine disagree: OpenGL cannot get a context on the Intel side, and
/// wgpu's Direct3D path fails on the NVIDIA side with "Invalid surface".
///
/// OpenGL goes first because of *how* it fails rather than how often. When it
/// cannot start it returns an error, tidily, and the next backend can be tried.
/// wgpu panics instead, and while that is caught below, unwinding out of a
/// half-initialised graphics stack is not something to do by choice. Trying the
/// well-behaved failure first means the common cases never reach it.
const BACKENDS: &[(&str, eframe::Renderer)] = &[
    ("glow (OpenGL)", eframe::Renderer::Glow),
    ("wgpu (Direct3D 12 / Vulkan / Metal)", eframe::Renderer::Wgpu),
];

fn main() {
    diagnostics::install_panic_hook();
    diagnostics::start_run();

    let mut failures: Vec<String> = Vec::new();

    for (name, renderer) in BACKENDS {
        diagnostics::log(&format!("opening the window with {name}"));
        match try_backend(name, *renderer) {
            Ok(()) => {
                diagnostics::log(&format!("closed normally ({name})"));
                return;
            }
            Err(reason) => {
                diagnostics::log(&format!("{name} could not start: {reason}"));
                failures.push(format!("{name}: {reason}"));
            }
        }
    }

    diagnostics::fatal(
        "sort4print could not open its window",
        &format!(
            "No rendering backend would start.\n\n{}",
            failures.join("\n\n")
        ),
    );
}

fn try_backend(name: &str, renderer: eframe::Renderer) -> Result<(), String> {
    let options = eframe::NativeOptions {
        renderer,
        viewport: egui::ViewportBuilder::default()
            .with_title("sort4print")
            .with_inner_size([1360.0, 860.0])
            .with_min_inner_size([980.0, 620.0]),
        ..Default::default()
    };

    let backend = name.to_string();

    // A backend that cannot start does not always have the manners to say so
    // with an error: wgpu asserts its way out from inside the driver. Catching
    // that here is what lets the next backend be tried instead of the program
    // simply vanishing.
    let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(move || {
        eframe::run_native(
            "sort4print",
            options,
            Box::new(move |cc| {
                diagnostics::log(&format!("graphics ready on {backend}; building the application"));
                let app = Sort4Print::new(cc);
                diagnostics::log("application built");
                Ok(Box::new(app) as Box<dyn eframe::App>)
            }),
        )
    }));

    match outcome {
        Ok(Ok(())) => Ok(()),
        Ok(Err(error)) => Err(error.to_string()),
        // The panic itself was already logged, with its location, by the hook.
        Err(_) => Err("it panicked during start-up".to_string()),
    }
}
