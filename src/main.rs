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
use std::sync::{mpsc, Arc};

fn main() -> Result<(), slint::PlatformError> {
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
    ui_bridge::initialize_permissions(&ui, platform);

    if let Err(error) = settings.save(platform) {
        platform.log(&format!("failed to save or migrate settings: {error}"));
    }
    if let Err(error) =
        platform.configure_autostart(settings.start_with_system, settings.start_minimized)
    {
        platform.log(&format!("failed to configure autostart: {error}"));
    }

    let displays = display::enumerate_displays();
    ui_bridge::set_monitor_options(&ui, &displays);

    let (backend_commands, command_receiver) = mpsc::channel();
    let backend_events = backend_commands.clone();
    let sink: Arc<dyn BackendSink> = Arc::new(ui_bridge::UiSink::new(ui.as_weak()));
    let backend_displays = displays.clone();
    let backend_thread = std::thread::spawn(move || {
        core::backend::run(
            platform,
            sink,
            command_receiver,
            backend_events,
            backend_displays,
        );
    });

    let session = ui_bridge::wire(&ui, platform, backend_commands.clone(), displays)?;

    ui.show()?;
    if start_minimized {
        ui.window().set_minimized(true);
    }
    let event_loop_result = slint::run_event_loop();

    let _ = backend_commands.send(BackendCommand::Shutdown);
    let _ = backend_thread.join();
    platform.stop_daemon();
    drop(session);

    event_loop_result
}
