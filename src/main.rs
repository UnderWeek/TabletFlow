slint::include_modules!();

use slint::ComponentHandle;
use std::process::Command;

fn daemon_is_running() -> bool {
    #[cfg(target_os = "windows")]
    {
        return Command::new("tasklist")
            .args(["/FI", "IMAGENAME eq OpenTabletDriver.Daemon.exe"])
            .output()
            .map(|output| {
                String::from_utf8_lossy(&output.stdout).contains("OpenTabletDriver.Daemon")
            })
            .unwrap_or(false);
    }

    #[cfg(not(target_os = "windows"))]
    {
        Command::new("pgrep")
            .args(["-f", "OpenTabletDriver.Daemon"])
            .status()
            .map(|status| status.success())
            .unwrap_or(false)
    }
}

fn backend_state() -> &'static str {
    if daemon_is_running() {
        "daemon-running"
    } else {
        "daemon-not-running"
    }
}

fn start_daemon() -> bool {
    #[cfg(target_os = "macos")]
    {
        if Command::new("open")
            .args(["-a", "OpenTabletDriver"])
            .status()
            .map(|status| status.success())
            .unwrap_or(false)
        {
            return true;
        }
    }

    #[cfg(target_os = "windows")]
    let executable = "OpenTabletDriver.Daemon.exe";
    #[cfg(not(target_os = "windows"))]
    let executable = "OpenTabletDriver.Daemon";

    Command::new(executable).spawn().is_ok()
}

fn main() -> Result<(), slint::PlatformError> {
    let ui = MainWindow::new()?;
    ui.set_backend_state(backend_state().into());

    let weak_ui = ui.as_weak();
    ui.on_navigate(move |page| {
        if let Some(ui) = weak_ui.upgrade() {
            ui.set_active_page(page);
        }
    });

    let weak_ui = ui.as_weak();
    ui.on_retry_backend(move || {
        if let Some(ui) = weak_ui.upgrade() {
            ui.set_backend_state(backend_state().into());
        }
    });

    let weak_ui = ui.as_weak();
    ui.on_start_daemon(move || {
        if let Some(ui) = weak_ui.upgrade() {
            ui.set_backend_state(if start_daemon() {
                "daemon-starting".into()
            } else {
                "daemon-not-running".into()
            });
        }
    });

    ui.run()
}
