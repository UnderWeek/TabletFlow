use crate::core::models::{AreaRequest, BackendCommand, BackendSnapshot};
use crate::core::rpc::{query_tablets, RpcClient};
use crate::core::state::{
    ConnectionAction, DaemonLifecycle, DetectionSchedule, BACKEND_RECONNECT_INTERVAL,
    TABLET_POLL_INTERVAL,
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

fn apply_area(
    client: &mut RpcClient,
    platform: &'static dyn Platform,
    settings: &mut Value,
    request: AreaRequest,
) -> io::Result<()> {
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

    let mut requested_area = Vec::with_capacity(5);
    for (name, value) in [
        ("Width", request.width.as_str()),
        ("Height", request.height.as_str()),
        ("X", request.x.as_str()),
        ("Y", request.y.as_str()),
        ("Rotation", request.rotation.as_str()),
    ] {
        let number = validation::finite_number(name, value)?;
        *json_member_mut(area, name).ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                format!("Area field {name} missing"),
            )
        })? = json!(number);
        requested_area.push((name, number));
    }
    let frequency = validation::frequency(&request.frequency)?;
    if let Some(filters) = json_member_mut(profile, "Filters").and_then(Value::as_array_mut) {
        for filter in filters {
            if let Some(filter_settings) =
                json_member_mut(filter, "Settings").and_then(Value::as_array_mut)
            {
                for setting in filter_settings {
                    if json_string(setting, "Property").as_deref() == Some("Frequency") {
                        if let Some(value) = json_member_mut(setting, "Value") {
                            *value = json!(frequency);
                        }
                    }
                }
            }
        }
    }
    if let Some(display) = &request.display {
        let display_area = json_member_mut(profile, "AbsoluteModeSettings")
            .and_then(|absolute| json_member_mut(absolute, "Display"))
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "Display area missing"))?;
        for (name, number) in [
            ("Width", display.width),
            ("Height", display.height),
            ("X", display.x),
            ("Y", display.y),
        ] {
            *json_member_mut(display_area, name).ok_or_else(|| {
                io::Error::new(
                    io::ErrorKind::InvalidData,
                    format!("Display field {name} missing"),
                )
            })? = json!(number);
        }
    }
    if *settings == original_settings {
        return Ok(());
    }
    client.call("SetSettings", json!([settings]))?;

    for _ in 0..5 {
        thread::sleep(Duration::from_millis(300));
        let Ok(readback) = client.call("GetSettings", json!([])) else {
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
        let matches = requested_area.iter().all(|(name, expected)| {
            json_member(area, name)
                .and_then(Value::as_f64)
                .is_some_and(|actual| (actual - expected).abs() < 0.005)
        });
        if matches {
            *settings = readback;
            platform.persist_driver_settings(settings)?;
            return Ok(());
        }
    }
    if platform.restore_pipeline_after_detect() {
        Err(io::Error::new(
            io::ErrorKind::TimedOut,
            "OpenTabletDriver did not confirm the requested area",
        ))
    } else {
        Ok(())
    }
}

pub fn run(
    platform: &'static dyn Platform,
    sink: std::sync::Arc<dyn BackendSink>,
    command_receiver: Receiver<BackendCommand>,
    backend_events: Sender<BackendCommand>,
    displays: Vec<DisplayInfo>,
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
                    lifecycle.disconnected();
                    refresh_requested = true;
                    driver_settings = None;
                    sink.connection_state("daemon-starting");
                }
                BackendCommand::TabletChanged { .. }
                | BackendCommand::DriverDisconnected { .. } => {}
                BackendCommand::ApplyArea(request) => queued_apply = Some(request),
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
            match RpcClient::connect(platform, backend_events.clone(), generation) {
                Ok(connection) => {
                    client = Some(connection);
                    active_generation = generation;
                    next_generation += 1;
                    lifecycle.connected();
                    refresh_requested = true;
                    driver_settings = None;
                    detection = DetectionSchedule::new(Instant::now());
                    sink.connection_state("daemon-starting");
                }
                Err(error) => {
                    lifecycle.disconnected();
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
                    }
                    sink.snapshot(snapshot);
                }
                Err(error) => {
                    platform.log(&format!("driver RPC query failed: {error}"));
                    client = None;
                    lifecycle.disconnected();
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
                        lifecycle.disconnected();
                        refresh_requested = true;
                        continue;
                    }
                }
            }
            let applied = request.clone();
            let result = driver_settings
                .as_mut()
                .ok_or_else(|| io::Error::other("driver settings cache is empty"))
                .and_then(|settings| apply_area(connection, platform, settings, request));
            match result {
                Ok(()) => sink.applied_area(applied),
                Err(error) => {
                    platform.log(&format!("failed to apply tablet area: {error}"));
                    client = None;
                    lifecycle.disconnected();
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
                if no_tablet && detection.take_due(Instant::now()) {
                    detect_requested = true;
                }
            }
            Err(RecvTimeoutError::Disconnected) => break,
        }
    }
    platform.log("backend supervisor stopped");
}
