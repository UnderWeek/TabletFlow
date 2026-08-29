slint::include_modules!();

use serde_json::{json, Value};
use slint::{CloseRequestResponse, ComponentHandle};
use std::fs;
use std::io::{self, Read, Write};
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Mutex, OnceLock};
use std::thread;
use std::time::Duration;

#[cfg(unix)]
use std::os::unix::net::UnixStream;

static EMBEDDED_DAEMON: OnceLock<Mutex<Option<Child>>> = OnceLock::new();
static BACKEND_RETRY: AtomicBool = AtomicBool::new(false);

const DAEMON_PIPE_NAME: &str = "OpenTabletDriver.Daemon";

enum DaemonStream {
    #[cfg(unix)]
    Unix(UnixStream),
    #[cfg(windows)]
    Windows(std::fs::File),
}

impl Read for DaemonStream {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        match self {
            #[cfg(unix)]
            Self::Unix(stream) => stream.read(buffer),
            #[cfg(windows)]
            Self::Windows(stream) => stream.read(buffer),
        }
    }
}

impl Write for DaemonStream {
    fn write(&mut self, buffer: &[u8]) -> io::Result<usize> {
        match self {
            #[cfg(unix)]
            Self::Unix(stream) => stream.write(buffer),
            #[cfg(windows)]
            Self::Windows(stream) => stream.write(buffer),
        }
    }

    fn flush(&mut self) -> io::Result<()> {
        match self {
            #[cfg(unix)]
            Self::Unix(stream) => stream.flush(),
            #[cfg(windows)]
            Self::Windows(stream) => stream.flush(),
        }
    }
}

struct DaemonClient {
    stream: DaemonStream,
    next_id: u64,
}

impl DaemonClient {
    fn connect() -> io::Result<Self> {
        #[cfg(unix)]
        let stream = {
            let stream = UnixStream::connect(
                std::env::temp_dir().join(format!("CoreFxPipe_{DAEMON_PIPE_NAME}")),
            )?;
            stream.set_read_timeout(Some(Duration::from_secs(45)))?;
            stream.set_write_timeout(Some(Duration::from_secs(15)))?;
            DaemonStream::Unix(stream)
        };

        #[cfg(windows)]
        let stream = DaemonStream::Windows(
            std::fs::OpenOptions::new()
                .read(true)
                .write(true)
                .open(format!(r"\\.\pipe\{DAEMON_PIPE_NAME}"))?,
        );

        Ok(Self { stream, next_id: 1 })
    }

    fn call(&mut self, method: &str, params: Value) -> io::Result<Value> {
        let id = self.next_id;
        self.next_id += 1;
        let body = serde_json::to_vec(&json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": method,
            "params": params
        }))
        .map_err(io::Error::other)?;
        let header = format!("Content-Length: {}\r\n\r\n", body.len());
        self.stream.write_all(header.as_bytes())?;
        self.stream.write_all(&body)?;
        self.stream.flush()?;

        loop {
            let response = read_rpc_message(&mut self.stream)?;
            if response.get("id") == Some(&json!(id)) {
                if let Some(error) = response.get("error") {
                    return Err(io::Error::other(error.to_string()));
                }
                return response
                    .get("result")
                    .cloned()
                    .ok_or_else(|| io::Error::other("RPC response has no result"));
            }
        }
    }
}

fn read_rpc_message(stream: &mut DaemonStream) -> io::Result<Value> {
    let mut headers = Vec::new();
    let mut delimiter = Vec::new();
    loop {
        let mut byte = [0u8; 1];
        stream.read_exact(&mut byte)?;
        headers.push(byte[0]);
        delimiter.push(byte[0]);
        if delimiter.ends_with(b"\r\n\r\n") || delimiter.ends_with(b"\n\n") {
            break;
        }
        if delimiter.len() > 4 {
            delimiter.remove(0);
        }
        if headers.len() > 8192 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "RPC headers are too large",
            ));
        }
    }

    let headers = String::from_utf8_lossy(&headers);
    let content_length = headers
        .lines()
        .find_map(|line| {
            let (name, value) = line.split_once(':')?;
            name.eq_ignore_ascii_case("content-length")
                .then(|| value.trim().parse::<usize>().ok())
                .flatten()
        })
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "RPC content length missing"))?;

    let mut body = vec![0u8; content_length];
    stream.read_exact(&mut body)?;
    serde_json::from_slice(&body).map_err(io::Error::other)
}

#[derive(Default)]
struct BackendSnapshot {
    state: &'static str,
    device_name: String,
    preview_width: f32,
    preview_height: f32,
    area_width: String,
    area_height: String,
    area_x: String,
    area_y: String,
    area_rotation: String,
    pen_data_available: bool,
}

enum BackendCommand {
    Detect,
    ApplyArea {
        tablet_name: String,
        width: String,
        height: String,
        x: String,
        y: String,
        rotation: String,
    },
}

fn json_member<'a>(value: &'a Value, name: &str) -> Option<&'a Value> {
    value
        .get(name)
        .or_else(|| value.get(name[..1].to_ascii_lowercase() + &name[1..]))
}

fn json_member_mut<'a>(value: &'a mut Value, name: &str) -> Option<&'a mut Value> {
    let lower_name = name[..1].to_ascii_lowercase() + &name[1..];
    if value.get(name).is_some() {
        value.get_mut(name)
    } else {
        value.get_mut(&lower_name)
    }
}

fn json_string(value: &Value, name: &str) -> Option<String> {
    json_member(value, name)
        .and_then(Value::as_str)
        .map(ToOwned::to_owned)
}

fn json_number_string(value: &Value, name: &str) -> String {
    let Some(number) = json_member(value, name).and_then(Value::as_f64) else {
        return String::new();
    };
    if number.fract() == 0.0 {
        format!("{number:.0}")
    } else {
        format!("{number:.2}")
    }
}

fn tablet_from_result(result: &Value) -> Option<(String, Value)> {
    let tablet = result.as_array()?.first()?;
    let properties = json_member(tablet, "Properties")?;
    let name = json_string(properties, "Name")?;
    Some((name, tablet.clone()))
}

fn settings_for_tablet(settings: &Value, tablet_name: &str) -> Option<Value> {
    let profiles = json_member(settings, "Profiles")?.as_array()?;
    profiles.iter().find_map(|profile| {
        (json_string(profile, "Tablet").as_deref() == Some(tablet_name)).then(|| profile.clone())
    })
}

fn query_backend(client: &mut DaemonClient, detect: bool) -> io::Result<BackendSnapshot> {
    let tablets = if detect {
        client.call("DetectTablets", json!([]))?
    } else {
        client.call("GetTablets", json!([]))?
    };
    let Some((device_name, _tablet)) = tablet_from_result(&tablets) else {
        return Ok(BackendSnapshot {
            state: "no-tablet",
            ..Default::default()
        });
    };

    let mut snapshot = BackendSnapshot {
        state: "ready",
        device_name: device_name.clone(),
        preview_width: 152.0,
        preview_height: 95.0,
        ..Default::default()
    };
    if let Ok(settings) = client.call("GetSettings", json!([])) {
        if let Some(profile) = settings_for_tablet(&settings, &device_name) {
            if let Some(absolute) = json_member(&profile, "AbsoluteModeSettings") {
                if let Some(display_area) = json_member(absolute, "Display") {
                    let width = json_member(display_area, "Width").and_then(Value::as_f64);
                    let height = json_member(display_area, "Height").and_then(Value::as_f64);
                    if let (Some(width), Some(height)) = (width, height) {
                        let aspect = width / height;
                        if aspect.is_finite() && aspect > 0.1 && aspect < 10.0 {
                            snapshot.preview_width = (width / 10.0) as f32;
                            snapshot.preview_height = (height / 10.0) as f32;
                        }
                    }
                }
                if let Some(tablet_area) = json_member(absolute, "Tablet") {
                    snapshot.area_width = json_number_string(tablet_area, "Width");
                    snapshot.area_height = json_number_string(tablet_area, "Height");
                    snapshot.area_x = json_number_string(tablet_area, "X");
                    snapshot.area_y = json_number_string(tablet_area, "Y");
                    snapshot.area_rotation = json_number_string(tablet_area, "Rotation");
                }
            }
            snapshot.pen_data_available = true;
        }
    }
    Ok(snapshot)
}

fn apply_area(client: &mut DaemonClient, command: BackendCommand) -> io::Result<()> {
    let BackendCommand::ApplyArea {
        tablet_name,
        width,
        height,
        x,
        y,
        rotation,
    } = command
    else {
        return Ok(());
    };

    let mut settings = client.call("GetSettings", json!([]))?;
    let profiles = json_member_mut(&mut settings, "Profiles")
        .and_then(Value::as_array_mut)
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "Profiles missing"))?;
    let profile = profiles
        .iter_mut()
        .find(|profile| json_string(profile, "Tablet").as_deref() == Some(&tablet_name))
        .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "Tablet profile missing"))?;
    let area = json_member_mut(profile, "AbsoluteModeSettings")
        .and_then(|absolute| json_member_mut(absolute, "Tablet"))
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "Tablet area missing"))?;

    for (name, value) in [
        ("Width", width),
        ("Height", height),
        ("X", x),
        ("Y", y),
        ("Rotation", rotation),
    ] {
        let number = value.parse::<f64>().map_err(|_| {
            io::Error::new(
                io::ErrorKind::InvalidInput,
                format!("Invalid area value for {name}"),
            )
        })?;
        if !number.is_finite() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!("Invalid area value for {name}"),
            ));
        }
        let Some(field) = json_member_mut(area, name) else {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("Area field {name} missing"),
            ));
        };
        *field = json!(number);
    }

    let _ = client.call("SetSettings", json!([settings]))?;
    Ok(())
}

fn publish_backend_snapshot(ui: &MainWindow, snapshot: BackendSnapshot) {
    ui.set_backend_state(snapshot.state.into());
    ui.set_device_name(snapshot.device_name.into());
    ui.set_area_preview_width(snapshot.preview_width);
    ui.set_area_preview_height(snapshot.preview_height);
    ui.set_area_width(snapshot.area_width.into());
    ui.set_area_height(snapshot.area_height.into());
    ui.set_area_x(snapshot.area_x.into());
    ui.set_area_y(snapshot.area_y.into());
    ui.set_area_rotation(snapshot.area_rotation.into());
    ui.set_pen_data_available(snapshot.pen_data_available);
}

fn start_backend_worker(ui: slint::Weak<MainWindow>) -> Sender<BackendCommand> {
    let (command_sender, command_receiver) = mpsc::channel();
    thread::spawn(move || {
        backend_worker(ui, command_receiver);
    });
    command_sender
}

fn backend_worker(ui: slint::Weak<MainWindow>, command_receiver: Receiver<BackendCommand>) {
    let mut client = None;
    let mut detect_requested = false;
    let mut last_ipc_error = String::new();
    #[cfg(debug_assertions)]
    let mut last_backend_status = String::new();

    loop {
        if !daemon_is_running() {
            client = None;
            detect_requested = false;
            last_ipc_error.clear();
            let weak_ui = ui.clone();
            let _ = slint::invoke_from_event_loop(move || {
                if let Some(ui) = weak_ui.upgrade() {
                    ui.set_backend_state("daemon-not-running".into());
                    ui.set_device_name("".into());
                    ui.set_pen_data_available(false);
                }
            });
            while command_receiver.try_recv().is_ok() {}
            thread::sleep(Duration::from_millis(1200));
            continue;
        }

        while let Ok(command) = command_receiver.try_recv() {
            match command {
                BackendCommand::Detect => detect_requested = true,
                command @ BackendCommand::ApplyArea { .. } => {
                    if let Some(connection) = client.as_mut() {
                        if apply_area(connection, command).is_err() {
                            client = None;
                        }
                    }
                }
            }
        }

        if client.is_none() {
            match DaemonClient::connect() {
                Ok(connection) => {
                    client = Some(connection);
                    last_ipc_error.clear();
                }
                Err(error) => {
                    let message = error.to_string();
                    if message != last_ipc_error {
                        eprintln!("OpenTabletDriver IPC connection failed: {message}");
                        last_ipc_error = message;
                    }
                    let weak_ui = ui.clone();
                    let _ = slint::invoke_from_event_loop(move || {
                        if let Some(ui) = weak_ui.upgrade() {
                            ui.set_backend_state("daemon-running".into());
                        }
                    });
                    thread::sleep(Duration::from_millis(1200));
                    continue;
                }
            }
        }

        let detect =
            std::mem::take(&mut detect_requested) || BACKEND_RETRY.swap(false, Ordering::Relaxed);
        let result = client
            .as_mut()
            .map(|connection| query_backend(connection, detect));

        if let Some(Ok(snapshot)) = result {
            last_ipc_error.clear();
            #[cfg(debug_assertions)]
            {
                let status = format!("{}:{}", snapshot.state, snapshot.device_name);
                if status != last_backend_status {
                    eprintln!(
                        "OpenTabletDriver backend: state={}, device={}",
                        snapshot.state,
                        if snapshot.device_name.is_empty() {
                            "none"
                        } else {
                            &snapshot.device_name
                        }
                    );
                    last_backend_status = status;
                }
            }
            let weak_ui = ui.clone();
            let _ = slint::invoke_from_event_loop(move || {
                if let Some(ui) = weak_ui.upgrade() {
                    publish_backend_snapshot(&ui, snapshot);
                }
            });
        } else {
            if let Some(Err(error)) = result {
                let message = error.to_string();
                if message != last_ipc_error {
                    eprintln!("OpenTabletDriver IPC request failed: {message}");
                    last_ipc_error = message;
                }
            }
            client = None;
            let weak_ui = ui.clone();
            let _ = slint::invoke_from_event_loop(move || {
                if let Some(ui) = weak_ui.upgrade() {
                    ui.set_backend_state("daemon-running".into());
                    ui.set_device_name("".into());
                    ui.set_pen_data_available(false);
                }
            });
        }

        thread::sleep(Duration::from_millis(1200));
    }
}

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
                .stdout(Stdio::null())
                .stderr(Stdio::null())
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
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
        let _ = Command::new("launchctl")
            .args([
                "bootstrap",
                &format!("gui/{}", current_uid()),
                path.to_string_lossy().as_ref(),
            ])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
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

#[cfg(target_os = "macos")]
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

    #[cfg(debug_assertions)]
    {
        let runtime_id = match (std::env::consts::OS, std::env::consts::ARCH) {
            ("macos", "aarch64") => Some("osx-arm64"),
            ("macos", "x86_64") => Some("osx-x64"),
            ("windows", "aarch64") => Some("win-arm64"),
            ("windows", "x86_64") => Some("win-x64"),
            ("windows", "x86") => Some("win-x86"),
            ("linux", "aarch64") => Some("linux-arm64"),
            ("linux", "x86_64") => Some("linux-x64"),
            ("linux", "x86") => Some("linux-x86"),
            _ => None,
        };
        if let Some(runtime_id) = runtime_id {
            let directory = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("target/otd")
                .join(runtime_id);
            candidates.push(directory.join("OpenTabletDriver.Daemon"));
            candidates.push(directory.join("OpenTabletDriver.Daemon.exe"));
            candidates.push(directory.join("OpenTabletDriver.Daemon.dll"));
        }
    }

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
            .output()
            .map(|output| output.status.success())
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

    let backend_commands = start_backend_worker(ui.as_weak());

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
    let retry_commands = backend_commands.clone();
    ui.on_retry_backend(move || {
        BACKEND_RETRY.store(true, Ordering::Relaxed);
        let _ = retry_commands.send(BackendCommand::Detect);
        if let Some(ui) = weak_ui.upgrade() {
            ui.set_backend_state(backend_state().into());
        }
    });

    let weak_ui = ui.as_weak();
    let start_commands = backend_commands.clone();
    ui.on_start_daemon(move || {
        BACKEND_RETRY.store(true, Ordering::Relaxed);
        let _ = start_commands.send(BackendCommand::Detect);
        if let Some(ui) = weak_ui.upgrade() {
            ui.set_backend_state(if start_daemon() {
                "daemon-starting".into()
            } else {
                "daemon-not-running".into()
            });
        }
    });

    let apply_commands = backend_commands.clone();
    ui.on_apply_area(move |tablet, width, height, x, y, rotation| {
        let _ = apply_commands.send(BackendCommand::ApplyArea {
            tablet_name: tablet.to_string(),
            width: width.to_string(),
            height: height.to_string(),
            x: x.to_string(),
            y: y.to_string(),
            rotation: rotation.to_string(),
        });
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
