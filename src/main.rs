#![windows_subsystem = "windows"]

mod core;
mod display;
mod platform;
mod ui_bridge;

slint::include_modules!();

use crate::core::backend::BackendSink;
use crate::core::models::BackendCommand;
use crate::core::settings::Settings;
use slint::ComponentHandle;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Sender;
use std::sync::{mpsc, Arc, Mutex, OnceLock};
use std::thread::JoinHandle;

struct BackendHandles {
    commands: Sender<BackendCommand>,
    shutdown: Arc<AtomicBool>,
    thread: Mutex<Option<JoinHandle<()>>>,
}

static BACKEND: OnceLock<BackendHandles> = OnceLock::new();

/// Stops the backend supervisor and any daemon it owns. Safe to call more
/// than once (only the first call does anything) and safe to call from an
/// `atexit` handler, since it only touches a channel send, an atomic store,
/// a thread join and the platform's own process-kill call - no panics, no
/// allocation-heavy paths that would be unsound during process teardown.
fn shutdown_backend(platform: &'static dyn platform::Platform) {
    let Some(handles) = BACKEND.get() else {
        return;
    };
    let Ok(mut slot) = handles.thread.lock() else {
        return;
    };
    let Some(thread) = slot.take() else {
        // Already shut down by the other call site.
        return;
    };
    let _ = handles.commands.send(BackendCommand::Shutdown);
    handles.shutdown.store(true, Ordering::Release);
    let _ = thread.join();
    platform.stop_daemon();
}

#[cfg(unix)]
extern "C" fn atexit_shutdown_backend() {
    // Safety net for macOS/Linux: the standard OS "Quit" gesture (Cmd+Q,
    // Dock > Quit, an AppleScript `quit`, or logout) tears the process down
    // through libc's exit() without slint::run_event_loop() ever returning,
    // so the normal post-loop cleanup in main() is never reached and any
    // daemon TabletFlow owns is orphaned. libc::atexit runs in-process
    // before that teardown completes, so it's reachable here even though
    // the graceful path below is not. Ordering matters: this stops the
    // backend thread (which owns the reconnect/respawn loop) before
    // killing the daemon, otherwise the still-running backend thread
    // notices the daemon is gone and immediately spawns a replacement.
    shutdown_backend(platform::current());
}

fn main() -> Result<(), slint::PlatformError> {
    #[cfg(unix)]
    unsafe {
        libc::atexit(atexit_shutdown_backend);
    }
    let platform = platform::current();
    platform.initialize_process();

    let arguments = std::env::args().collect::<Vec<_>>();
    if arguments.iter().any(|argument| {
        argument == "--driver-self-test" || argument == "--windows-driver-self-test"
    }) {
        return platform.run_driver_self_test().map_err(|error| {
            slint::PlatformError::Other(format!("Driver self-test failed: {error}"))
        });
    }

    let Some(_instance_guard) = platform.acquire_instance_guard() else {
        return Ok(());
    };

    let settings = Settings::load(platform);
    let start_minimized = arguments
        .iter()
        .any(|argument| argument == "--start-minimized")
        || (settings.start_with_system && settings.start_minimized);

    let ui = MainWindow::new()?;
    ui_bridge::apply_settings(&ui, &settings);
    ui.set_default_close_to_tray(platform.default_close_to_tray());
    ui_bridge::initialize_permissions(&ui, platform);

    if let Err(error) = settings.save(platform) {
        platform.log(&format!("failed to save or migrate settings: {error}"));
    }
    if let Err(error) =
        platform.configure_autostart(settings.start_with_system, settings.start_minimized)
    {
        platform.log(&format!("failed to configure autostart: {error}"));
    }

    // TODO(display-topology-changes): `displays` is captured once here and
    // moved into the backend thread for the rest of the process's life. If
    // monitors are connected/disconnected/rearranged afterward, the backend
    // keeps mapping OTD's Display settings against this stale list, and
    // `ui_bridge::set_monitor_options` is never called again, so the picker
    // can also go stale. A full watcher is out of scope here; the smallest
    // fix that stays testable is to re-run `display::enumerate_displays()`
    // and `ui_bridge::set_monitor_options` whenever the user hits
    // Detect/Reload (`BackendCommand::Detect` / `RefreshSettings`), and pass
    // the refreshed list into `backend::run`'s next `query_backend` call
    // instead of the one captured at startup.
    let displays = display::enumerate_displays();
    ui_bridge::set_monitor_options(&ui, &displays);

    let (backend_commands, command_receiver) = mpsc::channel();
    let backend_events = backend_commands.clone();
    let sink: Arc<dyn BackendSink> = Arc::new(ui_bridge::UiSink::new(ui.as_weak()));
    let backend_displays = displays.clone();
    let shutdown = Arc::new(AtomicBool::new(false));
    let backend_shutdown = Arc::clone(&shutdown);
    let backend_thread = std::thread::spawn(move || {
        core::backend::run(
            platform,
            sink,
            command_receiver,
            backend_events,
            backend_displays,
            backend_shutdown,
        );
    });
    let _ = BACKEND.set(BackendHandles {
        commands: backend_commands.clone(),
        shutdown,
        thread: Mutex::new(Some(backend_thread)),
    });

    let session = ui_bridge::wire(&ui, platform, backend_commands.clone(), displays)?;

    ui.show()?;
    if start_minimized {
        ui.window().set_minimized(true);
    }
    let event_loop_result = slint::run_event_loop();

    shutdown_backend(platform);
    drop(session);

    event_loop_result
}
