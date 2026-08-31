#![windows_subsystem = "windows"]

mod backend;
mod display;
mod platform;
mod settings;
#[cfg(unix)]
mod unix_runtime;
#[cfg(target_os = "windows")]
mod windows_runtime;

slint::include_modules!();

use serde_json::{json, Value};
use slint::{CloseRequestResponse, ComponentHandle, ModelRc, SharedString, VecModel};
use std::io::{self, Read, Write};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};
use std::thread;
use std::time::Duration;

use display::{enumerate_displays, selected_display_index, DisplayInfo};
use platform::{
    backend_state, configure_autostart, daemon_ipc_is_available, daemon_is_running,
    macos_permissions, open_github, owned_daemon_is_running, request_macos_permissions,
    run_driver_self_test, runtime_log, start_daemon, stop_daemon,
};
use settings::{apply_settings, load_settings, save_settings};

#[cfg(unix)]
use std::net::Shutdown;
#[cfg(unix)]
use std::os::unix::net::UnixStream;

const BACKEND_RECONNECT_INTERVAL: Duration = Duration::from_millis(1200);

#[cfg(not(target_os = "windows"))]
fn acquire_instance_guard() -> Option<unix_runtime::InstanceGuard> {
    unix_runtime::acquire_instance_guard()
}

#[cfg(target_os = "windows")]
fn acquire_instance_guard() -> Option<windows_runtime::InstanceGuard> {
    windows_runtime::acquire_instance_guard()
}

enum DaemonStream {
    #[cfg(unix)]
    Unix(UnixStream),
    #[cfg(windows)]
    Windows(std::fs::File),
}

/// Owns the notification reader and guarantees that dropping an IPC client
/// also stops its blocking reader thread on every platform.
struct ReaderGuard {
    cancelled: Arc<AtomicBool>,
    thread: Option<thread::JoinHandle<()>>,
    interrupter: Option<DaemonStream>,
}

impl Drop for ReaderGuard {
    fn drop(&mut self) {
        let Some(reader) = self.thread.take() else {
            return;
        };

        self.cancelled.store(true, Ordering::Release);
        #[cfg(target_os = "windows")]
        windows_runtime::cancel_reader(&reader);
        #[cfg(unix)]
        if let Some(DaemonStream::Unix(stream)) = self.interrupter.take() {
            let _ = stream.shutdown(Shutdown::Both);
        }
        let _ = reader.join();
    }
}

impl DaemonStream {
    fn try_clone(&self) -> io::Result<Self> {
        match self {
            #[cfg(unix)]
            Self::Unix(stream) => stream.try_clone().map(Self::Unix),
            #[cfg(windows)]
            Self::Windows(stream) => stream.try_clone().map(Self::Windows),
        }
    }
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
    responses: Receiver<Value>,
    next_id: u64,
    _reader: ReaderGuard,
}

impl DaemonClient {
    fn connect(
        backend_events: Sender<BackendCommand>,
        connection_generation: u64,
    ) -> io::Result<Self> {
        #[cfg(unix)]
        let stream = { DaemonStream::Unix(unix_runtime::connect_pipe()?) };

        #[cfg(windows)]
        let stream = DaemonStream::Windows(windows_runtime::connect_pipe()?);

        let mut reader = stream.try_clone()?;
        let interrupter = reader.try_clone()?;
        let (response_sender, responses) = mpsc::channel();
        let cancelled = Arc::new(AtomicBool::new(false));
        let reader_cancelled = Arc::clone(&cancelled);
        let reader_thread = thread::spawn(move || loop {
            if reader_cancelled.load(Ordering::Acquire) {
                break;
            }

            match read_rpc_message(&mut reader) {
                Ok(message) if message.get("id").is_some() => {
                    if response_sender.send(message).is_err() {
                        break;
                    }
                }
                Ok(message) => {
                    let method = message
                        .get("method")
                        .and_then(Value::as_str)
                        .unwrap_or_default();
                    if method.contains("TabletsChanged")
                        || (cfg!(target_os = "windows") && method.contains("Resynchronize"))
                    {
                        let _ = backend_events.send(BackendCommand::TabletChanged {
                            generation: connection_generation,
                        });
                    }
                }
                Err(error) => {
                    if reader_cancelled.load(Ordering::Acquire) {
                        break;
                    }
                    let _ = backend_events.send(BackendCommand::DriverDisconnected {
                        generation: connection_generation,
                        reason: error.to_string(),
                    });
                    break;
                }
            }
        });

        Ok(Self {
            stream,
            responses,
            next_id: 1,
            _reader: ReaderGuard {
                cancelled,
                thread: Some(reader_thread),
                interrupter: Some(interrupter),
            },
        })
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
            let response = self
                .responses
                .recv_timeout(if cfg!(target_os = "windows") {
                    match method {
                        "DetectTablets" | "SetSettings" => Duration::from_secs(180),
                        _ => Duration::from_secs(30),
                    }
                } else {
                    Duration::from_secs(15)
                })
                .map_err(|error| match error {
                    RecvTimeoutError::Timeout => {
                        io::Error::new(io::ErrorKind::TimedOut, "OpenTabletDriver did not respond")
                    }
                    RecvTimeoutError::Disconnected => io::Error::new(
                        io::ErrorKind::BrokenPipe,
                        "OpenTabletDriver connection closed",
                    ),
                })?;
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

trait DriverRpc {
    fn rpc_call(&mut self, method: &str, params: Value) -> io::Result<Value>;
}

impl DriverRpc for DaemonClient {
    fn rpc_call(&mut self, method: &str, params: Value) -> io::Result<Value> {
        self.call(method, params)
    }
}

/// OTD separates hardware detection from rebuilding the output pipeline. On
/// Windows an explicit DetectTablets must therefore be followed by reapplying
/// the current settings; otherwise the UI can show a tablet while pen input is
/// still disabled. The non-Windows call path passes `false` and is unchanged.
fn query_tablets<C: DriverRpc>(
    client: &mut C,
    detect: bool,
    restore_pipeline_after_detect: bool,
    driver_settings: &mut Option<Value>,
) -> io::Result<Value> {
    if !detect {
        return client.rpc_call("GetTablets", json!([]));
    }

    let tablets = client.rpc_call("DetectTablets", json!([]))?;
    if restore_pipeline_after_detect {
        let settings = client.rpc_call("GetSettings", json!([]))?;
        client.rpc_call("SetSettings", json!([settings]))?;
        // SetSettings may create a default profile for a newly detected tablet.
        // Read it back instead of caching the pre-detection settings object.
        *driver_settings = Some(client.rpc_call("GetSettings", json!([]))?);
    }
    Ok(tablets)
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
    tablet_width: f32,
    tablet_height: f32,
    area_width: String,
    area_height: String,
    area_x: String,
    area_y: String,
    area_rotation: String,
    area_frequency: String,
    monitor_index: i32,
    pen_data_available: bool,
}

#[derive(Clone)]
enum BackendCommand {
    Detect,
    TabletChanged {
        generation: u64,
    },
    DriverDisconnected {
        generation: u64,
        reason: String,
    },
    ApplyArea {
        tablet_name: String,
        width: String,
        height: String,
        x: String,
        y: String,
        rotation: String,
        frequency: String,
        display: Option<DisplayInfo>,
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

fn filter_frequency(profile: &Value) -> String {
    let Some(filters) = json_member(profile, "Filters").and_then(Value::as_array) else {
        return "1000".into();
    };

    let frequency_from = |filter: &Value| {
        json_member(filter, "Settings")
            .and_then(Value::as_array)
            .and_then(|settings| {
                settings.iter().find_map(|setting| {
                    (json_string(setting, "Property").as_deref() == Some("Frequency"))
                        .then(|| json_number_string(setting, "Value"))
                })
            })
            .filter(|value| !value.is_empty())
    };

    filters
        .iter()
        .filter(|filter| {
            json_member(filter, "Enable")
                .and_then(Value::as_bool)
                .unwrap_or(false)
        })
        .find_map(frequency_from)
        .or_else(|| filters.iter().find_map(frequency_from))
        .unwrap_or_else(|| "1000".into())
}

fn query_backend(
    client: &mut DaemonClient,
    detect: bool,
    displays: &[DisplayInfo],
    automatic_mapping_attempted: &mut bool,
    driver_settings: &mut Option<Value>,
) -> io::Result<BackendSnapshot> {
    let tablets = query_tablets(client, detect, cfg!(target_os = "windows"), driver_settings)?;
    let Some((device_name, tablet)) = tablet_from_result(&tablets) else {
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
        tablet_width: 152.0,
        tablet_height: 95.0,
        area_frequency: "1000".into(),
        monitor_index: -1,
        ..Default::default()
    };
    if let Some(digitizer) = json_member(&tablet, "Properties")
        .and_then(|properties| json_member(properties, "Specifications"))
        .and_then(|specifications| json_member(specifications, "Digitizer"))
    {
        let width = json_member(digitizer, "Width").and_then(Value::as_f64);
        let height = json_member(digitizer, "Height").and_then(Value::as_f64);
        if let (Some(width), Some(height)) = (width, height) {
            if width.is_finite() && height.is_finite() && width > 0.1 && height > 0.1 {
                snapshot.tablet_width = width as f32;
                snapshot.tablet_height = height as f32;
            }
        }
    }
    if driver_settings.is_none() {
        *driver_settings = Some(client.call("GetSettings", json!([]))?);
    }
    if let Some(settings) = driver_settings.as_ref() {
        if let Some(profile) = settings_for_tablet(settings, &device_name) {
            snapshot.area_frequency = filter_frequency(&profile);
            if let Some(absolute) = json_member(&profile, "AbsoluteModeSettings") {
                if let Some(display_area) = json_member(absolute, "Display") {
                    let width = json_member(display_area, "Width").and_then(Value::as_f64);
                    let height = json_member(display_area, "Height").and_then(Value::as_f64);
                    let x = json_member(display_area, "X").and_then(Value::as_f64);
                    let y = json_member(display_area, "Y").and_then(Value::as_f64);
                    if let (Some(width), Some(height)) = (width, height) {
                        let aspect = width / height;
                        if aspect.is_finite() && aspect > 0.1 && aspect < 10.0 {
                            snapshot.preview_width = (width / 10.0) as f32;
                            snapshot.preview_height = (height / 10.0) as f32;
                        }
                        if let (Some(x), Some(y)) = (x, y) {
                            snapshot.monitor_index = selected_display_index(
                                displays,
                                width as f32,
                                height as f32,
                                x as f32,
                                y as f32,
                            );
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

    // Automatic display mapping may write settings only once after a tablet
    // configuration becomes available. Event retries must not recreate the
    // driver's output pipeline while the pen is already active.
    if !cfg!(target_os = "windows") && snapshot.pen_data_available && !*automatic_mapping_attempted
    {
        *automatic_mapping_attempted = true;
        if let Some(display) = displays
            .iter()
            .find(|display| display.detected && display.primary)
            .or_else(|| displays.iter().find(|display| display.detected))
            .filter(|_| snapshot.monitor_index < 0)
            .cloned()
        {
            if !snapshot.area_width.is_empty()
                && !snapshot.area_height.is_empty()
                && !snapshot.area_x.is_empty()
                && !snapshot.area_y.is_empty()
                && !snapshot.area_rotation.is_empty()
            {
                let settings = driver_settings.as_mut().ok_or_else(|| {
                    io::Error::new(io::ErrorKind::InvalidData, "Driver settings missing")
                })?;
                apply_area(
                    client,
                    settings,
                    BackendCommand::ApplyArea {
                        tablet_name: device_name,
                        width: snapshot.area_width.clone(),
                        height: snapshot.area_height.clone(),
                        x: snapshot.area_x.clone(),
                        y: snapshot.area_y.clone(),
                        rotation: snapshot.area_rotation.clone(),
                        frequency: snapshot.area_frequency.clone(),
                        display: Some(display.clone()),
                    },
                )?;
                snapshot.monitor_index = display.index;
                snapshot.preview_width = display.width / 10.0;
                snapshot.preview_height = display.height / 10.0;
            }
        }
    }
    Ok(snapshot)
}

fn apply_area(
    client: &mut DaemonClient,
    settings: &mut Value,
    command: BackendCommand,
) -> io::Result<()> {
    let BackendCommand::ApplyArea {
        tablet_name,
        width,
        height,
        x,
        y,
        rotation,
        frequency,
        display,
    } = command
    else {
        return Ok(());
    };

    // SetSettings recreates OpenTabletDriver's input pipeline. Avoid doing
    // that when the requested configuration is already active.
    let original_settings = settings.clone();

    let profiles = json_member_mut(settings, "Profiles")
        .and_then(Value::as_array_mut)
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "Profiles missing"))?;
    let profile = profiles
        .iter_mut()
        .find(|profile| json_string(profile, "Tablet").as_deref() == Some(&tablet_name))
        .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "Tablet profile missing"))?;
    let area = json_member_mut(profile, "AbsoluteModeSettings")
        .and_then(|absolute| json_member_mut(absolute, "Tablet"))
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "Tablet area missing"))?;

    let mut requested_area = Vec::with_capacity(5);
    for (name, value) in [
        ("Width", width.as_str()),
        ("Height", height.as_str()),
        ("X", x.as_str()),
        ("Y", y.as_str()),
        ("Rotation", rotation.as_str()),
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
        requested_area.push((name, number));
    }

    let frequency = frequency
        .parse::<f64>()
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "Invalid frequency value"))?;
    if !frequency.is_finite() || !(1.0..=1000.0).contains(&frequency) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "Frequency must be between 1 and 1000 Hz",
        ));
    }
    if let Some(filters) = json_member_mut(profile, "Filters").and_then(Value::as_array_mut) {
        for filter in filters {
            let Some(filter_settings) =
                json_member_mut(filter, "Settings").and_then(Value::as_array_mut)
            else {
                continue;
            };
            for setting in filter_settings {
                if json_string(setting, "Property").as_deref() == Some("Frequency") {
                    if let Some(value) = json_member_mut(setting, "Value") {
                        *value = json!(frequency);
                    }
                }
            }
        }
    }

    if let Some(display) = display {
        let display_area = json_member_mut(profile, "AbsoluteModeSettings")
            .and_then(|absolute| json_member_mut(absolute, "Display"))
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "Display area missing"))?;
        for (name, number) in [
            ("Width", display.width),
            ("Height", display.height),
            ("X", display.x),
            ("Y", display.y),
        ] {
            let Some(field) = json_member_mut(display_area, name) else {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    format!("Display field {name} missing"),
                ));
            };
            *field = json!(number);
        }
    }

    if *settings == original_settings {
        eprintln!("apply_area: requested settings match current state, skipping SetSettings");
        return Ok(());
    }

    eprintln!(
        "apply_area: sending SetSettings for tablet '{tablet_name}' (width={:?}, height={:?}, x={:?}, y={:?}, rotation={:?}, frequency={frequency})",
        requested_area[0].1, requested_area[1].1, requested_area[2].1, requested_area[3].1, requested_area[4].1
    );
    let _ = client.call("SetSettings", json!([settings]))?;

    // SetSettings recreates OpenTabletDriver's input pipeline, which is not
    // guaranteed to complete by the time the RPC call returns (observed to
    // lag noticeably on Windows). Poll GetSettings briefly so the UI is told
    // about the actually-applied values instead of just echoing the request.
    for attempt in 1..=5 {
        thread::sleep(Duration::from_millis(300));
        let readback = match client.call("GetSettings", json!([])) {
            Ok(readback) => readback,
            Err(error) => {
                eprintln!("apply_area: readback attempt {attempt} failed: {error}");
                continue;
            }
        };
        let Some(profile) = settings_for_tablet(&readback, &tablet_name) else {
            eprintln!(
                "apply_area: readback attempt {attempt} found no profile for '{tablet_name}'"
            );
            continue;
        };
        let Some(area) = json_member(&profile, "AbsoluteModeSettings")
            .and_then(|absolute| json_member(absolute, "Tablet"))
        else {
            continue;
        };
        let matches = requested_area.iter().all(|(name, expected)| {
            json_member(area, name)
                .and_then(Value::as_f64)
                .is_some_and(|actual| (actual - expected).abs() < 0.005)
        });
        if matches {
            eprintln!("apply_area: readback confirmed on attempt {attempt}");
            *settings = readback;
            #[cfg(target_os = "windows")]
            windows_runtime::persist_driver_settings(settings)?;
            return Ok(());
        }
        eprintln!("apply_area: readback attempt {attempt} does not match requested area yet");
    }

    #[cfg(target_os = "windows")]
    return Err(io::Error::new(
        io::ErrorKind::TimedOut,
        "OpenTabletDriver did not confirm the requested area",
    ));

    #[cfg(not(target_os = "windows"))]
    {
        eprintln!("apply_area: readback did not confirm requested area within timeout, echoing requested values anyway");
        Ok(())
    }
}

fn publish_backend_snapshot(ui: &MainWindow, snapshot: BackendSnapshot) {
    ui.set_backend_state(snapshot.state.into());
    ui.set_device_name(snapshot.device_name.into());
    ui.set_area_preview_width(snapshot.preview_width);
    ui.set_area_preview_height(snapshot.preview_height);
    ui.set_tablet_bounds_width(snapshot.tablet_width);
    ui.set_tablet_bounds_height(snapshot.tablet_height);
    ui.set_area_width(snapshot.area_width.into());
    ui.set_area_height(snapshot.area_height.into());
    ui.set_area_x(snapshot.area_x.into());
    ui.set_area_y(snapshot.area_y.into());
    ui.set_area_rotation(snapshot.area_rotation.into());
    ui.set_area_frequency(snapshot.area_frequency.into());
    // Keep an unmatched virtual-screen mapping as "no specific display".
    // That preserves the driver's current mapping until the user chooses one.
    ui.set_monitor_index(snapshot.monitor_index);
    ui.set_pen_data_available(snapshot.pen_data_available);
}

fn publish_applied_area(ui: &MainWindow, command: BackendCommand) {
    let BackendCommand::ApplyArea {
        width,
        height,
        x,
        y,
        rotation,
        frequency,
        display,
        ..
    } = command
    else {
        return;
    };

    ui.set_area_width(width.into());
    ui.set_area_height(height.into());
    ui.set_area_x(x.into());
    ui.set_area_y(y.into());
    ui.set_area_rotation(rotation.into());
    ui.set_area_frequency(frequency.into());
    if let Some(display) = display {
        ui.set_monitor_index(display.index);
        ui.set_area_preview_width(display.width / 10.0);
        ui.set_area_preview_height(display.height / 10.0);
    }
}

fn start_backend_worker(
    ui: slint::Weak<MainWindow>,
    displays: Vec<DisplayInfo>,
) -> Sender<BackendCommand> {
    let (command_sender, command_receiver) = mpsc::channel();
    let backend_events = command_sender.clone();
    thread::spawn(move || {
        backend::run(ui, command_receiver, backend_events, displays);
    });
    command_sender
}

fn set_tray_visible(tray: &TrayIcon, visible: bool) {
    tray.set_shown(visible);
}

fn main() -> Result<(), slint::PlatformError> {
    #[cfg(target_os = "windows")]
    windows_runtime::initialize_process();

    if std::env::args().any(|argument| {
        argument == "--driver-self-test" || argument == "--windows-driver-self-test"
    }) {
        return match run_driver_self_test() {
            Ok(()) => Ok(()),
            Err(error) => {
                eprintln!("Driver self-test failed: {error}");
                Err(slint::PlatformError::Other(format!(
                    "Driver self-test failed: {error}"
                )))
            }
        };
    }

    let Some(_instance_guard) = acquire_instance_guard() else {
        return Ok(());
    };

    let mut settings = load_settings();
    if std::env::args().any(|argument| argument == "--start-minimized") {
        settings.start_with_system = true;
        settings.start_minimized = true;
    }

    let ui = MainWindow::new()?;
    apply_settings(&ui, &settings);
    // Create the shared settings file on first launch and migrate any values
    // that were loaded from the older platform-specific location.
    let _ = save_settings(&ui);
    let (input_monitoring_granted, accessibility_granted) = macos_permissions();
    ui.set_permission_platform(
        if cfg!(target_os = "macos") {
            "macOS"
        } else {
            "other"
        }
        .into(),
    );
    ui.set_input_monitoring_granted(input_monitoring_granted);
    ui.set_accessibility_granted(accessibility_granted);
    if cfg!(target_os = "macos") && !(input_monitoring_granted && accessibility_granted) {
        request_macos_permissions();
    }
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

    let displays = enumerate_displays();
    let monitor_options = ModelRc::new(VecModel::from_iter(
        displays
            .iter()
            .map(|display| SharedString::from(display.label.clone())),
    ));
    ui.set_monitor_options(monitor_options);
    let backend_commands = start_backend_worker(ui.as_weak(), displays.clone());

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
        let _ = retry_commands.send(BackendCommand::Detect);
        if let Some(ui) = weak_ui.upgrade() {
            ui.set_backend_state(backend_state().into());
        }
    });

    let weak_ui = ui.as_weak();
    let start_commands = backend_commands.clone();
    ui.on_start_daemon(move || {
        let started = start_daemon();
        if started {
            let _ = start_commands.send(BackendCommand::Detect);
        }
        if let Some(ui) = weak_ui.upgrade() {
            ui.set_backend_state(if started {
                "daemon-starting".into()
            } else {
                "daemon-not-running".into()
            });
        }
    });

    let apply_commands = backend_commands.clone();
    ui.on_apply_area(
        move |tablet, width, height, x, y, rotation, frequency, monitor| {
            let display = displays
                .iter()
                .find(|display| display.index == monitor)
                .cloned();
            let _ = apply_commands.send(BackendCommand::ApplyArea {
                tablet_name: tablet.to_string(),
                width: width.to_string(),
                height: height.to_string(),
                x: x.to_string(),
                y: y.to_string(),
                rotation: rotation.to_string(),
                frequency: frequency.to_string(),
                display,
            });
        },
    );

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

    let weak_ui = ui.as_weak();
    ui.on_check_permissions(move || {
        let (input_monitoring_granted, accessibility_granted) = macos_permissions();
        if let Some(ui) = weak_ui.upgrade() {
            ui.set_input_monitoring_granted(input_monitoring_granted);
            ui.set_accessibility_granted(accessibility_granted);
        }
    });

    let weak_ui = ui.as_weak();
    ui.on_request_permissions(move || {
        request_macos_permissions();
        let (input_monitoring_granted, accessibility_granted) = macos_permissions();
        if let Some(ui) = weak_ui.upgrade() {
            ui.set_input_monitoring_granted(input_monitoring_granted);
            ui.set_accessibility_granted(accessibility_granted);
        }
    });

    ui.show()?;
    if settings.start_with_system && settings.start_minimized {
        ui.window().set_minimized(true);
    }
    let event_loop_result = slint::run_event_loop();
    stop_daemon();
    event_loop_result
}
