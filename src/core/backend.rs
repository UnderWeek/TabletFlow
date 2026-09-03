use crate::core::models::{AreaRequest, BackendCommand, BackendSnapshot};
use crate::core::rpc::{query_tablets, DriverRpc, RpcClient};
use crate::core::state::{
    ConnectionAction, DaemonLifecycle, DetectionSchedule, BACKEND_RECONNECT_INTERVAL,
    PIPELINE_REBUILD_INTERVAL, TABLET_POLL_INTERVAL,
};
use crate::core::validation;
use crate::display::{selected_display_index, DisplayInfo};
use crate::platform::Platform;
use serde_json::{json, Value};
use std::io;
use std::sync::mpsc::{Receiver, RecvTimeoutError, Sender};
use std::thread;
use std::time::{Duration, Instant};

pub trait BackendSink: Send + Sync + 'static {
    fn connection_state(&self, state: &'static str);
    fn snapshot(&self, snapshot: BackendSnapshot);
    fn applied_area(&self, request: AreaRequest);
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
    Some((json_string(properties, "Name")?, tablet.clone()))
}

fn settings_for_tablet(settings: &Value, tablet_name: &str) -> Option<Value> {
    json_member(settings, "Profiles")?
        .as_array()?
        .iter()
        .find_map(|profile| {
            (json_string(profile, "Tablet").as_deref() == Some(tablet_name))
                .then(|| profile.clone())
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
    client: &mut RpcClient,
    platform: &'static dyn Platform,
    detect: bool,
    displays: &[DisplayInfo],
    automatic_mapping_attempted: &mut bool,
    driver_settings: &mut Option<Value>,
) -> io::Result<BackendSnapshot> {
    let tablets = query_tablets(
        client,
        detect,
        platform.restore_pipeline_after_detect(),
        driver_settings,
    )?;
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
    if let Some(profile) = driver_settings
        .as_ref()
        .and_then(|settings| settings_for_tablet(settings, &device_name))
    {
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

    if platform.auto_map_primary_display()
        && snapshot.pen_data_available
        && !*automatic_mapping_attempted
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
                    platform,
                    settings,
                    AreaRequest {
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

/// Applies `request` to `settings`, sends `SetSettings`, then confirms via a
/// `GetSettings` readback that OpenTabletDriver actually accepted every field
/// TabletFlow changed (tablet area, display area if requested, and frequency
/// if a filter setting for it was actually found and changed). On success
/// returns an `AreaRequest` built from the *actual* readback rather than the
/// caller's original request, so the UI never reports "confirmed" values
/// that don't match what the driver is really using.
fn apply_area<C: DriverRpc>(
    client: &mut C,
    platform: &'static dyn Platform,
    settings: &mut Value,
    request: AreaRequest,
) -> io::Result<AreaRequest> {
    let original_settings = settings.clone();
    let profiles = json_member_mut(settings, "Profiles")
        .and_then(Value::as_array_mut)
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "Profiles missing"))?;
    let profile = profiles
        .iter_mut()
        .find(|profile| json_string(profile, "Tablet").as_deref() == Some(&request.tablet_name))
        .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "Tablet profile missing"))?;
    let area = json_member_mut(profile, "AbsoluteModeSettings")
        .and_then(|absolute| json_member_mut(absolute, "Tablet"))
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "Tablet area missing"))?;

    let requested_area = [
        (
            "Width",
            validation::positive_dimension("Width", &request.width)?,
        ),
        (
            "Height",
            validation::positive_dimension("Height", &request.height)?,
        ),
        ("X", validation::finite_number("X", &request.x)?),
        ("Y", validation::finite_number("Y", &request.y)?),
        ("Rotation", validation::rotation(&request.rotation)?),
    ];
    for (name, number) in requested_area {
        *json_member_mut(area, name).ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                format!("Area field {name} missing"),
            )
        })? = json!(number);
    }

    let frequency = validation::frequency(&request.frequency)?;
    let mut frequency_updated = false;
    if let Some(filters) = json_member_mut(profile, "Filters").and_then(Value::as_array_mut) {
        for filter in filters {
            if let Some(filter_settings) =
                json_member_mut(filter, "Settings").and_then(Value::as_array_mut)
            {
                for setting in filter_settings {
                    if json_string(setting, "Property").as_deref() == Some("Frequency") {
                        if let Some(value) = json_member_mut(setting, "Value") {
                            *value = json!(frequency);
                            frequency_updated = true;
                        }
                    }
                }
            }
        }
    }

    let mut requested_display: Option<[(&str, f32); 4]> = None;
    if let Some(display) = &request.display {
        let display_area = json_member_mut(profile, "AbsoluteModeSettings")
            .and_then(|absolute| json_member_mut(absolute, "Display"))
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "Display area missing"))?;
        let values = [
            ("Width", display.width),
            ("Height", display.height),
            ("X", display.x),
            ("Y", display.y),
        ];
        for (name, number) in values {
            *json_member_mut(display_area, name).ok_or_else(|| {
                io::Error::new(
                    io::ErrorKind::InvalidData,
                    format!("Display field {name} missing"),
                )
            })? = json!(number);
        }
        requested_display = Some(values);
    }

    if *settings == original_settings {
        return Ok(request);
    }
    client.rpc_call("SetSettings", json!([settings]))?;

    for _ in 0..5 {
        thread::sleep(Duration::from_millis(300));
        let Ok(readback) = client.rpc_call("GetSettings", json!([])) else {
            continue;
        };
        let Some(profile) = settings_for_tablet(&readback, &request.tablet_name) else {
            continue;
        };
        let Some(area) = json_member(&profile, "AbsoluteModeSettings")
            .and_then(|absolute| json_member(absolute, "Tablet"))
        else {
            continue;
        };
        let tablet_matches = requested_area.iter().all(|(name, expected)| {
            json_member(area, name)
                .and_then(Value::as_f64)
                .is_some_and(|actual| (actual - expected).abs() < 0.005)
        });
        let display_matches = match requested_display {
            None => true,
            Some(values) => json_member(&profile, "AbsoluteModeSettings")
                .and_then(|absolute| json_member(absolute, "Display"))
                .is_some_and(|display_area| {
                    values.iter().all(|(name, expected)| {
                        json_member(display_area, name)
                            .and_then(Value::as_f64)
                            .is_some_and(|actual| (actual - *expected as f64).abs() < 0.5)
                    })
                }),
        };
        let frequency_matches = !frequency_updated
            || filter_frequency(&profile)
                .parse::<f64>()
                .is_ok_and(|actual| (actual - frequency).abs() < 0.5);

        if tablet_matches && display_matches && frequency_matches {
            let actual = AreaRequest {
                tablet_name: request.tablet_name,
                width: json_number_string(area, "Width"),
                height: json_number_string(area, "Height"),
                x: json_number_string(area, "X"),
                y: json_number_string(area, "Y"),
                rotation: json_number_string(area, "Rotation"),
                frequency: filter_frequency(&profile),
                display: request.display,
            };
            *settings = readback;
            platform.persist_driver_settings(settings)?;
            return Ok(actual);
        }
    }
    Err(io::Error::new(
        io::ErrorKind::TimedOut,
        "OpenTabletDriver did not confirm the requested area",
    ))
}

pub fn run(
    platform: &'static dyn Platform,
    sink: std::sync::Arc<dyn BackendSink>,
    command_receiver: Receiver<BackendCommand>,
    backend_events: Sender<BackendCommand>,
    displays: Vec<DisplayInfo>,
    shutdown: std::sync::Arc<std::sync::atomic::AtomicBool>,
) {
    let mut client: Option<RpcClient> = None;
    let mut pending_command = None;
    let mut queued_apply: Option<AreaRequest> = None;
    let mut refresh_requested = true;
    let mut detect_requested = false;
    let mut no_tablet = false;
    let mut driver_settings = None;
    let mut automatic_mapping_attempted = !platform.auto_map_primary_display();
    let mut active_generation = 0;
    let mut next_generation = 1;
    let mut lifecycle = DaemonLifecycle::new();
    let mut detection = DetectionSchedule::new(Instant::now());
    let mut last_pipeline_rebuild = Instant::now();
    platform.log("backend supervisor started");

    'supervisor: loop {
        let commands = pending_command
            .take()
            .into_iter()
            .chain(command_receiver.try_iter())
            .collect::<Vec<_>>();
        for command in commands {
            match command {
                BackendCommand::Shutdown => break 'supervisor,
                BackendCommand::Detect => {
                    detect_requested = true;
                    refresh_requested = true;
                    driver_settings = None;
                    automatic_mapping_attempted = !platform.auto_map_primary_display();
                    detection.reset_after_explicit_detect(Instant::now());
                }
                BackendCommand::StartDaemon => {
                    if client.is_none() {
                        let now = Instant::now();
                        lifecycle.start_attempted(now);
                        match platform.start_daemon() {
                            Ok(()) => {
                                lifecycle.spawn_succeeded(now);
                                sink.connection_state("daemon-starting");
                            }
                            Err(error) => {
                                lifecycle.spawn_failed();
                                platform.log(&format!("manual driver launch failed: {error}"));
                                sink.connection_state("daemon-not-running");
                            }
                        }
                    } else {
                        detect_requested = true;
                        refresh_requested = true;
                    }
                }
                BackendCommand::TabletChanged { generation } if generation == active_generation => {
                    detect_requested = platform.restore_pipeline_after_detect();
                    refresh_requested = true;
                    driver_settings = None;
                    automatic_mapping_attempted = !platform.auto_map_primary_display();
                }
                BackendCommand::DriverDisconnected { generation, reason }
                    if generation == active_generation =>
                {
                    platform.log(&format!(
                        "driver IPC disconnected generation={generation}: {reason}"
                    ));
                    client = None;
                    lifecycle.disconnected(Instant::now(), platform.owned_daemon_running());
                    refresh_requested = true;
                    driver_settings = None;
                    sink.connection_state("daemon-starting");
                }
                BackendCommand::TabletChanged { .. }
                | BackendCommand::DriverDisconnected { .. } => {}
                BackendCommand::ApplyArea(request) => queued_apply = Some(request),
                BackendCommand::RefreshSettings => {
                    driver_settings = None;
                    refresh_requested = true;
                }
            }
        }

        if client.is_none() {
            let now = Instant::now();
            match lifecycle.next_action(
                now,
                platform.ipc_available(),
                platform.owned_daemon_running(),
            ) {
                ConnectionAction::Connect => {}
                ConnectionAction::StartOwnedDaemon => {
                    lifecycle.start_attempted(now);
                    match platform.start_daemon() {
                        Ok(()) => {
                            lifecycle.spawn_succeeded(now);
                            sink.connection_state("daemon-starting");
                        }
                        Err(error) => {
                            lifecycle.spawn_failed();
                            platform.log(&format!("driver launch failed: {error}"));
                            sink.connection_state("daemon-not-running");
                        }
                    }
                    pending_command = command_receiver
                        .recv_timeout(BACKEND_RECONNECT_INTERVAL)
                        .ok();
                    continue;
                }
                ConnectionAction::WaitForIpc => {
                    sink.connection_state("daemon-starting");
                    pending_command = command_receiver
                        .recv_timeout(BACKEND_RECONNECT_INTERVAL)
                        .ok();
                    continue;
                }
                ConnectionAction::WaitForRetry => {
                    pending_command = command_receiver
                        .recv_timeout(lifecycle.retry_wait(now))
                        .ok();
                    continue;
                }
                ConnectionAction::RestartOwnedDaemon => {
                    platform.log("packaged daemon missed IPC startup deadline; restarting");
                    platform.stop_daemon();
                    lifecycle.owned_daemon_stopped();
                    sink.connection_state("daemon-crashed");
                    continue;
                }
            }
            let generation = next_generation;
            match RpcClient::connect(
                platform,
                backend_events.clone(),
                generation,
                std::sync::Arc::clone(&shutdown),
            ) {
                Ok(connection) => {
                    client = Some(connection);
                    active_generation = generation;
                    next_generation += 1;
                    lifecycle.connected();
                    refresh_requested = true;
                    // On platforms whose output pipeline needs rebuilding
                    // after detection (Windows), a fresh connection must
                    // route its first query through DetectTablets, not
                    // GetTablets: the daemon may already have a tablet
                    // cached from its own startup auto-detect (or from a
                    // previous client), in which case GetTablets alone
                    // would report "ready" without ever re-running the
                    // GetSettings/SetSettings round trip that rebuilds the
                    // pipeline. `||` preserves an explicit Detect the user
                    // may already have queued.
                    detect_requested = detect_requested || platform.restore_pipeline_after_detect();
                    driver_settings = None;
                    detection = DetectionSchedule::new(Instant::now());
                    sink.connection_state("daemon-starting");
                }
                Err(error) => {
                    lifecycle.disconnected(Instant::now(), platform.owned_daemon_running());
                    platform.log(&format!(
                        "waiting for driver IPC generation={generation}: {error}"
                    ));
                    sink.connection_state("daemon-starting");
                    pending_command = command_receiver
                        .recv_timeout(BACKEND_RECONNECT_INTERVAL)
                        .ok();
                    continue;
                }
            }
        }

        let detect = std::mem::take(&mut detect_requested);
        let refresh = std::mem::take(&mut refresh_requested);
        if detect || refresh {
            let Some(connection) = client.as_mut() else {
                continue;
            };
            match query_backend(
                connection,
                platform,
                detect,
                &displays,
                &mut automatic_mapping_attempted,
                &mut driver_settings,
            ) {
                Ok(snapshot) => {
                    no_tablet = snapshot.state == "no-tablet";
                    if no_tablet {
                        detection.no_tablet(Instant::now());
                    } else {
                        detection.tablet_found();
                        if detect {
                            last_pipeline_rebuild = Instant::now();
                        }
                    }
                    sink.snapshot(snapshot);
                }
                Err(error) => {
                    platform.log(&format!("driver RPC query failed: {error}"));
                    client = None;
                    lifecycle.disconnected(Instant::now(), platform.owned_daemon_running());
                    driver_settings = None;
                    refresh_requested = true;
                    sink.connection_state("daemon-starting");
                    continue;
                }
            }
        }

        if let Some(request) = queued_apply.take() {
            let Some(connection) = client.as_mut() else {
                queued_apply = Some(request);
                continue;
            };
            if driver_settings.is_none() {
                match connection.call("GetSettings", json!([])) {
                    Ok(settings) => driver_settings = Some(settings),
                    Err(_) => {
                        queued_apply = Some(request);
                        client = None;
                        lifecycle.disconnected(Instant::now(), platform.owned_daemon_running());
                        refresh_requested = true;
                        continue;
                    }
                }
            }
            let result = driver_settings
                .as_mut()
                .ok_or_else(|| io::Error::other("driver settings cache is empty"))
                .and_then(|settings| apply_area(connection, platform, settings, request));
            match result {
                Ok(actual) => sink.applied_area(actual),
                Err(error) => {
                    platform.log(&format!("failed to apply tablet area: {error}"));
                    client = None;
                    lifecycle.disconnected(Instant::now(), platform.owned_daemon_running());
                    driver_settings = None;
                    refresh_requested = true;
                    sink.connection_state("daemon-starting");
                    continue;
                }
            }
        }

        let timeout = if no_tablet {
            detection.wait_duration(Instant::now())
        } else {
            TABLET_POLL_INTERVAL
        };
        match command_receiver.recv_timeout(timeout) {
            Ok(command) => pending_command = Some(command),
            Err(RecvTimeoutError::Timeout) => {
                refresh_requested = true;
                let now = Instant::now();
                let due_for_no_tablet_retry = no_tablet && detection.take_due(now);
                let due_for_pipeline_rebuild = !no_tablet
                    && platform.restore_pipeline_after_detect()
                    && now.saturating_duration_since(last_pipeline_rebuild)
                        >= PIPELINE_REBUILD_INTERVAL;
                if due_for_no_tablet_retry || due_for_pipeline_rebuild {
                    detect_requested = true;
                }
            }
            Err(RecvTimeoutError::Disconnected) => break,
        }
    }
    platform.log("backend supervisor stopped");
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;
    use std::path::PathBuf;

    struct NoopPlatform;
    impl Platform for NoopPlatform {
        fn name(&self) -> &'static str {
            "test"
        }
        fn settings_path(&self) -> Option<PathBuf> {
            None
        }
        fn acquire_instance_guard(&self) -> Option<Box<dyn Send>> {
            None
        }
        fn connect_transport(&self) -> io::Result<Box<dyn crate::platform::Transport>> {
            Err(io::Error::other("not used in this test"))
        }
        fn ipc_available(&self) -> bool {
            false
        }
        fn owned_daemon_running(&self) -> bool {
            false
        }
        fn start_daemon(&self) -> io::Result<()> {
            Ok(())
        }
        fn stop_daemon(&self) {}
        fn configure_autostart(&self, _enabled: bool, _start_minimized: bool) -> io::Result<()> {
            Ok(())
        }
        fn open_url(&self, _url: &str) -> io::Result<()> {
            Ok(())
        }
        fn run_driver_self_test(&self) -> io::Result<()> {
            Ok(())
        }
    }

    fn platform() -> &'static dyn Platform {
        Box::leak(Box::new(NoopPlatform))
    }

    const TABLET: &str = "Test Tablet";

    /// `tablet` is (Width, Height, X, Y, Rotation); `display` is
    /// (Width, Height, X, Y).
    fn settings_with(tablet: [f64; 5], display: [f64; 4], frequency: f64) -> Value {
        let [tablet_w, tablet_h, tablet_x, tablet_y, rotation] = tablet;
        let [display_w, display_h, display_x, display_y] = display;
        json!({
            "Profiles": [{
                "Tablet": TABLET,
                "AbsoluteModeSettings": {
                    "Tablet": {
                        "Width": tablet_w,
                        "Height": tablet_h,
                        "X": tablet_x,
                        "Y": tablet_y,
                        "Rotation": rotation,
                    },
                    "Display": {
                        "Width": display_w,
                        "Height": display_h,
                        "X": display_x,
                        "Y": display_y,
                    },
                },
                "Filters": [{
                    "Enable": true,
                    "Settings": [{
                        "Property": "Frequency",
                        "Value": frequency,
                    }],
                }],
            }],
        })
    }

    fn area_request(display: Option<DisplayInfo>) -> AreaRequest {
        AreaRequest {
            tablet_name: TABLET.to_string(),
            width: "150".to_string(),
            height: "90".to_string(),
            x: "75".to_string(),
            y: "45".to_string(),
            rotation: "0".to_string(),
            frequency: "500".to_string(),
            display,
        }
    }

    fn display_info() -> DisplayInfo {
        DisplayInfo {
            index: 0,
            label: "Display 1".into(),
            width: 1920.0,
            height: 1080.0,
            x: 960.0,
            y: 540.0,
            detected: true,
            primary: true,
        }
    }

    /// A scripted `DriverRpc` double: `SetSettings` is a no-op, and
    /// `GetSettings` returns the next canned readback from `responses` (or an
    /// error once exhausted, which `apply_area`'s retry loop treats the same
    /// as a transient RPC failure).
    struct ScriptedClient {
        responses: RefCell<std::vec::IntoIter<Value>>,
    }
    impl ScriptedClient {
        fn new(responses: Vec<Value>) -> Self {
            Self {
                responses: RefCell::new(responses.into_iter()),
            }
        }
    }
    impl DriverRpc for ScriptedClient {
        fn rpc_call(&mut self, method: &str, _params: Value) -> io::Result<Value> {
            match method {
                "SetSettings" => Ok(Value::Null),
                "GetSettings" => self
                    .responses
                    .borrow_mut()
                    .next()
                    .ok_or_else(|| io::Error::other("no more scripted responses")),
                other => panic!("unexpected RPC method {other}"),
            }
        }
    }

    #[test]
    fn confirmed_readback_reports_actual_tablet_display_and_frequency() {
        let mut client = ScriptedClient::new(vec![settings_with(
            [150.0, 90.0, 75.0, 45.0, 0.0],
            [1920.0, 1080.0, 960.0, 540.0],
            500.0,
        )]);
        let mut settings = settings_with(
            [152.0, 95.0, 0.0, 0.0, 0.0],
            [1920.0, 1080.0, 0.0, 0.0],
            1000.0,
        );
        let result = apply_area(
            &mut client,
            platform(),
            &mut settings,
            area_request(Some(display_info())),
        );
        let actual = result.expect("readback should confirm the requested area");
        assert_eq!(actual.width, "150");
        assert_eq!(actual.height, "90");
        assert_eq!(actual.frequency, "500");
    }

    #[test]
    fn tablet_matches_but_display_mismatch_is_a_failure() {
        // Display X/Y never move off their original (unrequested) values,
        // simulating OTD accepting the tablet area but not the display area.
        let mut client = ScriptedClient::new(vec![settings_with(
            [150.0, 90.0, 75.0, 45.0, 0.0],
            [1920.0, 1080.0, 0.0, 0.0],
            500.0,
        )]);
        let mut settings = settings_with(
            [152.0, 95.0, 0.0, 0.0, 0.0],
            [1920.0, 1080.0, 0.0, 0.0],
            1000.0,
        );
        let result = apply_area(
            &mut client,
            platform(),
            &mut settings,
            area_request(Some(display_info())),
        );
        assert!(
            result.is_err(),
            "a display-area mismatch must fail confirmation even though the tablet area matched"
        );
    }

    #[test]
    fn tablet_matches_but_frequency_mismatch_is_a_failure() {
        // Frequency stays at its original value while the tablet area moved,
        // simulating OTD dropping the frequency change.
        let mut client = ScriptedClient::new(vec![settings_with(
            [150.0, 90.0, 75.0, 45.0, 0.0],
            [1920.0, 1080.0, 960.0, 540.0],
            1000.0,
        )]);
        let mut settings = settings_with(
            [152.0, 95.0, 0.0, 0.0, 0.0],
            [1920.0, 1080.0, 960.0, 540.0],
            1000.0,
        );
        let result = apply_area(&mut client, platform(), &mut settings, area_request(None));
        assert!(
            result.is_err(),
            "a frequency mismatch must fail confirmation even though the tablet area matched"
        );
    }

    // Regression test: when apply_area's readback never confirms the
    // requested values, backend::run's `Ok(actual) => sink.applied_area(actual)`
    // match arm is never reached - callers must treat Err as "do not publish
    // an applied-area event", not silently succeed as pre-fix macOS/Linux did.
    #[test]
    fn failed_readback_never_yields_an_ok_result() {
        let mut client = ScriptedClient::new(Vec::new());
        let mut settings = settings_with(
            [152.0, 95.0, 0.0, 0.0, 0.0],
            [1920.0, 1080.0, 960.0, 540.0],
            1000.0,
        );
        let result = apply_area(&mut client, platform(), &mut settings, area_request(None));
        assert!(matches!(result, Err(error) if error.kind() == io::ErrorKind::TimedOut));
    }
}
