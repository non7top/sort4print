//! Making failures visible.
//!
//! Release builds set `windows_subsystem = "windows"`, so there is no console:
//! a panic on the way up, or a windowing/graphics error out of `run_native`,
//! would otherwise end the process with no window and nothing to look at. Every
//! run therefore writes a short log, and anything fatal also puts up a dialog
//! saying what happened and where the log is.
//!
//! The log is deliberately plain and short — a handful of milestones — so it is
//! something a user can read and paste, not a trace to trawl.

use std::io::Write;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::OnceLock;

/// Worker decode panics are already caught and turned into a per-file error, so
/// they must not raise a dialog. They are still logged.
const WORKER_THREAD_PREFIX: &str = "sort4print-decode";

fn log_file() -> &'static Option<PathBuf> {
    static PATH: OnceLock<Option<PathBuf>> = OnceLock::new();
    PATH.get_or_init(|| {
        // Beside the exe when that is writable, matching where the ini goes,
        // so both sit together on a portable install.
        if let Some(dir) = std::env::current_exe()
            .ok()
            .and_then(|p| p.parent().map(|p| p.to_path_buf()))
        {
            let candidate = dir.join("sort4print.log");
            if std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(&candidate)
                .is_ok()
            {
                return Some(candidate);
            }
        }
        let fallback = std::env::temp_dir().join("sort4print.log");
        std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&fallback)
            .ok()
            .map(|_| fallback)
    })
}

pub fn log_path_display() -> String {
    match log_file() {
        Some(path) => path.display().to_string(),
        None => "(nowhere writable)".to_string(),
    }
}

/// Appends one line. Never fails loudly: diagnostics must not become the thing
/// that breaks the program.
pub fn log(message: &str) {
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);

    if let Some(path) = log_file() {
        if let Ok(mut file) = std::fs::OpenOptions::new().create(true).append(true).open(path) {
            let _ = writeln!(file, "[{stamp}] {message}");
        }
    }
    // Debug builds keep a console, so mirror it there.
    #[cfg(debug_assertions)]
    eprintln!("[{stamp}] {message}");
}

/// Starts a fresh section in the log for this run.
pub fn start_run() {
    if let Some(path) = log_file() {
        // Keep the file from growing without bound across many runs.
        if std::fs::metadata(path).map(|m| m.len() > 256 * 1024).unwrap_or(false) {
            let _ = std::fs::write(path, b"");
        }
    }
    log("----");
    log(&format!(
        "sort4print {} starting on {}",
        sort4print_core::VERSION,
        std::env::consts::OS
    ));
    log(&format!("log: {}", log_path_display()));
}

/// Logs, then puts the message in front of the user, because with no console
/// there is nowhere else for it to go.
pub fn fatal(title: &str, detail: &str) {
    log(&format!("FATAL {title}: {detail}"));
    rfd::MessageDialog::new()
        .set_level(rfd::MessageLevel::Error)
        .set_title(title)
        .set_description(format!(
            "{detail}\n\nDetails were written to:\n{}",
            log_path_display()
        ))
        .show();
}

static HOOK_INSTALLED: AtomicBool = AtomicBool::new(false);

/// True until the program has painted something.
///
/// While starting up, a panic is not necessarily fatal: the rendering backends
/// are tried in turn, and one of them blowing up is a signal to try the next,
/// not something to interrupt the user about. Once a frame has been drawn, any
/// panic is a real one and gets a dialog.
static STARTING_UP: AtomicBool = AtomicBool::new(true);

/// Called once the first frame is on screen.
pub fn mark_running() {
    STARTING_UP.store(false, Ordering::SeqCst);
}

/// Routes panics to the log, and to a dialog when they are not the already
/// handled kind from a decode worker.
pub fn install_panic_hook() {
    if HOOK_INSTALLED.swap(true, Ordering::SeqCst) {
        return;
    }
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let thread = std::thread::current();
        let name = thread.name().unwrap_or("unnamed").to_string();
        let location = info
            .location()
            .map(|l| format!("{}:{}", l.file(), l.line()))
            .unwrap_or_else(|| "unknown location".to_string());
        let message = payload(info);

        log(&format!("PANIC on '{name}' at {location}: {message}"));

        if name.starts_with(WORKER_THREAD_PREFIX) {
            // Already reported against the offending file in the UI.
            return;
        }
        if STARTING_UP.load(Ordering::SeqCst) {
            // A rendering backend failing to come up is handled by trying the
            // next one; the user only hears about it if none of them work.
            return;
        }

        #[cfg(debug_assertions)]
        previous(info);
        #[cfg(not(debug_assertions))]
        {
            let _ = &previous;
            rfd::MessageDialog::new()
                .set_level(rfd::MessageLevel::Error)
                .set_title("sort4print stopped unexpectedly")
                .set_description(format!(
                    "{message}\n\nat {location}\n\nDetails were written to:\n{}",
                    log_path_display()
                ))
                .show();
        }
    }));
}

fn payload(info: &std::panic::PanicHookInfo<'_>) -> String {
    let p = info.payload();
    if let Some(s) = p.downcast_ref::<&str>() {
        (*s).to_string()
    } else if let Some(s) = p.downcast_ref::<String>() {
        s.clone()
    } else {
        "unknown panic".to_string()
    }
}
