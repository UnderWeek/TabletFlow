slint::include_modules!();

use slint::{CloseRequestResponse, ComponentHandle, Timer, TimerMode};
use std::fs;
use std::io;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::{Mutex, OnceLock};
use std::time::Duration;

static EMBEDDED_DAEMON: OnceLock<Mutex<Option<Child>>> = OnceLock::new();

#[derive(Clone, Debug)]
struct Settings {
    theme: String,
    accent: String,
    compact_ui: bool,
    reduce_animations: bool,
    start_with_system: bool,
    start_minimized: bool,
    close_to_tray: bool,
    check_updates: bool,
    pause_hidden: bool,
    disable_unfocused_animations: bool,
    polling_interval: String,
    low_power_mode: bool,
    show_diagnostics: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            theme: "System".into(),
            accent: "Blue".into(),
            compact_ui: false,
            reduce_animations: false,
            start_with_system: false,
            start_minimized: false,
            close_to_tray: false,
            check_updates: true,
            pause_hidden: true,
            disable_unfocused_animations: false,
            polling_interval: "Auto".into(),
            low_power_mode: false,
            show_diagnostics: false,
        }
    }
}

fn parse_bool(value: &str, fallback: bool) -> bool {
    match value {
        "true" => true,
        "false" => false,
        _ => fallback,
    }
}

fn settings_path() -> Option<PathBuf> {
    #[cfg(target_os = "macos")]
    {
        return std::env::var_os("HOME")
            .map(PathBuf::from)
            .map(|home| home.join("Library/Application Support/TabletFlow/settings.conf"));
    }

    #[cfg(target_os = "windows")]
    {
        return std::env::var_os("APPDATA")
            .map(PathBuf::from)
            .map(|app_data| app_data.join("TabletFlow/settings.conf"));
    }

    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    {
        if let Some(config_home) = std::env::var_os("XDG_CONFIG_HOME") {
            return Some(PathBuf::from(config_home).join("TabletFlow/settings.conf"));
        }
        std::env::var_os("HOME")
            .map(PathBuf::from)
            .map(|home| home.join(".config/TabletFlow/settings.conf"))
    }
}

fn load_settings() -> Settings {
    let mut settings = Settings::default();
    let Some(path) = settings_path() else {
        return settings;
    };

    let Ok(contents) = fs::read_to_string(path) else {
        return settings;
    };

    for line in contents.lines() {
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        match key {
            "theme" if matches!(value, "System" | "Light" | "Dark") => {
                settings.theme = value.into()
            }
            "accent" if matches!(value, "Blue" | "Amber" | "Mint") => {
                settings.accent = value.into()
            }
            "compact_ui" => settings.compact_ui = parse_bool(value, settings.compact_ui),
            "reduce_animations" => {
                settings.reduce_animations = parse_bool(value, settings.reduce_animations)
            }
            "start_with_system" => {
                settings.start_with_system = parse_bool(value, settings.start_with_system)
            }
            "start_minimized" => {
                settings.start_minimized = parse_bool(value, settings.start_minimized)
            }
            "close_to_tray" => settings.close_to_tray = parse_bool(value, settings.close_to_tray),
            "check_updates" => settings.check_updates = parse_bool(value, settings.check_updates),
            "pause_hidden" => settings.pause_hidden = parse_bool(value, settings.pause_hidden),
            "disable_unfocused_animations" => {
                settings.disable_unfocused_animations =
                    parse_bool(value, settings.disable_unfocused_animations)
            }
            "polling_interval" if matches!(value, "Auto" | "Low" | "High") => {
                settings.polling_interval = value.into()
            }
            "low_power_mode" => {
                settings.low_power_mode = parse_bool(value, settings.low_power_mode)
            }
            "show_diagnostics" => {
                settings.show_diagnostics = parse_bool(value, settings.show_diagnostics)
            }
            _ => {}
        }
    }

    settings
}

fn save_settings(ui: &MainWindow) -> io::Result<()> {
    let Some(path) = settings_path() else {
        return Ok(());
    };
    let Some(parent) = path.parent() else {
        return Ok(());
    };
    fs::create_dir_all(parent)?;

    let contents = format!(
        "theme={}\naccent={}\ncompact_ui={}\nreduce_animations={}\nstart_with_system={}\nstart_minimized={}\nclose_to_tray={}\ncheck_updates={}\npause_hidden={}\ndisable_unfocused_animations={}\npolling_interval={}\nlow_power_mode={}\nshow_diagnostics={}\n",
        ui.get_theme(),
        ui.get_accent(),
        ui.get_compact_ui(),
        ui.get_reduce_animations(),
        ui.get_start_with_system(),
        ui.get_start_minimized(),
        ui.get_close_to_tray(),
        ui.get_check_updates(),
        ui.get_pause_hidden(),
        ui.get_disable_unfocused_animations(),
        ui.get_polling_interval(),
        ui.get_low_power_mode(),
        ui.get_show_diagnostics(),
    );

    let temporary_path = path.with_extension("tmp");
    fs::write(&temporary_path, contents)?;
    fs::rename(temporary_path, path)
}

fn apply_settings(ui: &MainWindow, settings: &Settings) {
    ui.set_theme(settings.theme.clone().into());
    ui.set_accent(settings.accent.clone().into());
    ui.set_compact_ui(settings.compact_ui);
    ui.set_reduce_animations(settings.reduce_animations);
    ui.set_start_with_system(settings.start_with_system);
    ui.set_start_minimized(settings.start_minimized);
    ui.set_close_to_tray(settings.close_to_tray);
    ui.set_check_updates(settings.check_updates);
    ui.set_pause_hidden(settings.pause_hidden);
    ui.set_disable_unfocused_animations(settings.disable_unfocused_animations);
    ui.set_polling_interval(settings.polling_interval.clone().into());
    ui.set_low_power_mode(settings.low_power_mode);
    ui.set_show_diagnostics(settings.show_diagnostics);

    let theme = ui.global::<AppTheme>();
    theme.set_mode(
        if settings.theme == "Light" {
            "light"
        } else if settings.theme == "Dark" {
            "dark"
        } else {
            "system"
        }
        .into(),
    );
    theme.set_accent(settings.accent.clone().into());
    theme.set_compact(settings.compact_ui);
    theme.set_reduce_motion(settings.reduce_animations);
}

#[allow(unreachable_code, unused_variables)]
fn configure_autostart(enabled: bool, start_minimized: bool) -> io::Result<()> {
    let executable = std::env::current_exe()?;
    let executable = executable.to_string_lossy();
    let minimized_argument = if start_minimized {
        " --start-minimized"
    } else {
        ""
    };

    #[cfg(target_os = "macos")]
    {
        let Some(home) = std::env::var_os("HOME") else {
            return Ok(());
        };
        let path = PathBuf::from(home).join("Library/LaunchAgents/com.underweek.tabletflow.plist");
        if !enabled {
            let _ = Command::new("launchctl")
                .args([
                    "bootout",
                    &format!("gui/{}", current_uid()),
                    path.to_string_lossy().as_ref(),
                ])
                .status();
            if path.exists() {
                fs::remove_file(path)?;
            }
            return Ok(());
        }

        let Some(parent) = path.parent() else {
            return Ok(());
        };
        fs::create_dir_all(parent)?;
        let escaped_executable = executable
            .replace('&', "&amp;")
            .replace('<', "&lt;")
            .replace('>', "&gt;");
        let argument = if start_minimized {
            "            <string>--start-minimized</string>\n"
        } else {
            ""
        };
        let plist = format!(
            "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<!DOCTYPE plist PUBLIC \"-//Apple//DTD PLIST 1.0//EN\" \"http://www.apple.com/DTDs/PropertyList-1.0.dtd\">\n<plist version=\"1.0\"><dict>\n<key>Label</key><string>com.underweek.tabletflow</string>\n<key>ProgramArguments</key><array>\n            <string>{escaped_executable}</string>\n{argument}</array>\n<key>RunAtLoad</key><true/>\n</dict></plist>\n"
        );
        fs::write(&path, plist)?;
        let _ = Command::new("launchctl")
            .args([
                "bootout",
                &format!("gui/{}", current_uid()),
                path.to_string_lossy().as_ref(),
            ])
            .status();
        let _ = Command::new("launchctl")
            .args([
                "bootstrap",
                &format!("gui/{}", current_uid()),
                path.to_string_lossy().as_ref(),
            ])
            .status();
        return Ok(());
    }

    #[cfg(target_os = "windows")]
    {
        let Some(app_data) = std::env::var_os("APPDATA") else {
            return Ok(());
        };
        let path = PathBuf::from(app_data)
            .join("Microsoft/Windows/Start Menu/Programs/Startup/TabletFlow.cmd");
        if !enabled {
            if path.exists() {
                fs::remove_file(path)?;
            }
            return Ok(());
        }
        let Some(parent) = path.parent() else {
            return Ok(());
        };
        fs::create_dir_all(parent)?;
        fs::write(
            path,
            format!("@start \"\" \"{}\"{}\n", executable, minimized_argument),
        )?;
        return Ok(());
    }

    #[cfg(all(unix, not(target_os = "macos")))]
    {
        let config_home = std::env::var_os("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".config")));
        let Some(config_home) = config_home else {
            return Ok(());
        };
        let path = config_home.join("autostart/TabletFlow.desktop");
        if !enabled {
            if path.exists() {
                fs::remove_file(path)?;
            }
            return Ok(());
        }
        let Some(parent) = path.parent() else {
            return Ok(());
        };
        fs::create_dir_all(parent)?;
        fs::write(
            path,
            format!(
                "[Desktop Entry]\nType=Application\nName=TabletFlow\nExec=\"{}\"{}\nX-GNOME-Autostart-enabled=true\n",
                executable, minimized_argument
            ),
        )?;
    }

    Ok(())
}

fn set_tray_visible(tray: &TrayIcon, visible: bool) {
    tray.set_shown(visible);
}

#[cfg(unix)]
fn current_uid() -> String {
    Command::new("id")
        .arg("-u")
        .output()
        .ok()
        .and_then(|output| String::from_utf8(output.stdout).ok())
        .map(|uid| uid.trim().to_owned())
        .filter(|uid| !uid.is_empty())
        .unwrap_or_else(|| "0".into())
}

fn daemon_process() -> &'static Mutex<Option<Child>> {
    EMBEDDED_DAEMON.get_or_init(|| Mutex::new(None))
}

fn daemon_candidates() -> Vec<PathBuf> {
    let mut candidates = Vec::new();

    if let Ok(executable) = std::env::current_exe() {
        if let Some(directory) = executable.parent() {
            candidates.push(directory.join("OpenTabletDriver.Daemon"));
            candidates.push(directory.join("OpenTabletDriver.Daemon.exe"));
            candidates.push(directory.join("OpenTabletDriver.Daemon.dll"));
            candidates.push(directory.join("resources/OpenTabletDriver.Daemon"));
            candidates.push(directory.join("resources/OpenTabletDriver.Daemon.exe"));
            candidates.push(directory.join("resources/OpenTabletDriver.Daemon.dll"));

            if let Some(contents) = directory.parent() {
                candidates.push(contents.join("Resources/OpenTabletDriver.Daemon"));
                candidates.push(contents.join("Resources/OpenTabletDriver.Daemon.dll"));
            }
        }
    }

    if let Some(resource_root) = std::env::var_os("TABLETFLOW_RESOURCE_DIR") {
        let resource_root = PathBuf::from(resource_root);
        candidates.push(resource_root.join("OpenTabletDriver.Daemon"));
        candidates.push(resource_root.join("OpenTabletDriver.Daemon.exe"));
        candidates.push(resource_root.join("OpenTabletDriver.Daemon.dll"));
    }

    candidates
}

fn embedded_daemon_path() -> Option<PathBuf> {
    daemon_candidates()
        .into_iter()
        .find(|candidate| candidate.is_file())
}

fn owned_daemon_is_running() -> bool {
    let Ok(mut daemon) = daemon_process().lock() else {
        return false;
    };
    let Some(child) = daemon.as_mut() else {
        return false;
    };

    match child.try_wait() {
        Ok(None) => true,
        Ok(Some(_)) | Err(_) => {
            *daemon = None;
            false
        }
    }
}

fn daemon_is_running() -> bool {
    if owned_daemon_is_running() {
        return true;
    }

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
    if daemon_is_running() {
        return true;
    }

    let Some(path) = embedded_daemon_path() else {
        return false;
    };

    let mut command = if path.extension().is_some_and(|extension| extension == "dll") {
        let mut command = Command::new("dotnet");
        command.arg(&path);
        command
    } else {
        Command::new(&path)
    };
    command
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());

    let Ok(child) = command.spawn() else {
        return false;
    };
    if let Ok(mut daemon) = daemon_process().lock() {
        *daemon = Some(child);
        true
    } else {
        false
    }
}

fn stop_daemon() {
    let Ok(mut daemon) = daemon_process().lock() else {
        return;
    };
    let Some(mut child) = daemon.take() else {
        return;
    };
    let _ = child.kill();
    let _ = child.wait();
}

fn open_github() {
    const URL: &str = "https://github.com/UnderWeek/TabletFlow";

    #[cfg(target_os = "macos")]
    let _ = Command::new("open").arg(URL).spawn();

    #[cfg(target_os = "windows")]
    let _ = Command::new("cmd").args(["/C", "start", "", URL]).spawn();

    #[cfg(all(unix, not(target_os = "macos")))]
    let _ = Command::new("xdg-open").arg(URL).spawn();
}

fn main() -> Result<(), slint::PlatformError> {
    let mut settings = load_settings();
    if std::env::args().any(|argument| argument == "--start-minimized") {
        settings.start_with_system = true;
        settings.start_minimized = true;
    }

    let ui = MainWindow::new()?;
    apply_settings(&ui, &settings);
    let _ = configure_autostart(settings.start_with_system, settings.start_minimized);

    let daemon_started = if daemon_is_running() {
        false
    } else {
        start_daemon()
    };
    ui.set_backend_state(if daemon_started {
        "daemon-starting".into()
    } else {
        backend_state().into()
    });

    let daemon_timer = Timer::default();
    let weak_ui = ui.as_weak();
    daemon_timer.start(
        TimerMode::Repeated,
        Duration::from_millis(1200),
        move || {
            if let Some(ui) = weak_ui.upgrade() {
                ui.set_backend_state(backend_state().into());
            }
        },
    );

    let tray = TrayIcon::new()?;
    set_tray_visible(&tray, settings.close_to_tray);

    let weak_ui = ui.as_weak();
    tray.on_open(move || {
        if let Some(ui) = weak_ui.upgrade() {
            let _ = ui.show();
        }
    });
    tray.on_quit(move || {
        let _ = slint::quit_event_loop();
    });

    let weak_ui = ui.as_weak();
    ui.window().on_close_requested(move || {
        if let Some(ui) = weak_ui.upgrade() {
            if ui.get_close_to_tray() {
                return CloseRequestResponse::HideWindow;
            }
        }
        let _ = slint::quit_event_loop();
        CloseRequestResponse::HideWindow
    });

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

    let weak_ui = ui.as_weak();
    let weak_tray = tray.as_weak();
    ui.on_settings_changed(move || {
        if let Some(ui) = weak_ui.upgrade() {
            let _ = save_settings(&ui);
            let _ = configure_autostart(ui.get_start_with_system(), ui.get_start_minimized());
            if let Some(tray) = weak_tray.upgrade() {
                set_tray_visible(&tray, ui.get_close_to_tray());
            }
        }
    });

    ui.on_open_github(move || {
        open_github();
    });

    ui.show()?;
    if settings.start_with_system && settings.start_minimized {
        ui.window().set_minimized(true);
    }
    let event_loop_result = slint::run_event_loop();
    stop_daemon();
    event_loop_result
}
