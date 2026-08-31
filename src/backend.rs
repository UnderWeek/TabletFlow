//! Cross-platform OpenTabletDriver supervisor.
//!
//! All desktop platforms use the same reconnect, detection, and queued-apply
//! state machine. Platform-specific process/IPC details stay behind helpers in
//! `main`/`windows_runtime`, so behavior does not silently diverge by OS.

use super::*;
use std::time::Instant;

const TABLET_POLL_INTERVAL: Duration = Duration::from_millis(900);
const START_RETRY_INTERVAL: Duration = Duration::from_millis(1500);
const IPC_STARTUP_DEADLINE: Duration = Duration::from_secs(180);
const DETECT_RETRY_DELAYS: [Duration; 6] = [
    Duration::ZERO,
    Duration::from_secs(2),
    Duration::from_secs(4),
    Duration::from_secs(7),
    Duration::from_secs(12),
    Duration::from_secs(20),
];
const STEADY_STATE_DETECT_INTERVAL: Duration = Duration::from_secs(15);

#[derive(Debug)]
struct DetectionSchedule {
    attempts: usize,
    next: Option<Instant>,
}

impl DetectionSchedule {
    fn new(now: Instant) -> Self {
        Self {
            attempts: 0,
            next: Some(now),
        }
    }

    fn reset_after_explicit_detect(&mut self, now: Instant) {
        self.attempts = 1;
        self.next = Some(now + DETECT_RETRY_DELAYS[1]);
    }

    fn tablet_found(&mut self) {
        self.attempts = 0;
        self.next = None;
    }

    fn no_tablet(&mut self, now: Instant) {
        self.next = DETECT_RETRY_DELAYS
            .get(self.attempts)
            .copied()
            .map(|delay| now + delay)
            .or(Some(now + STEADY_STATE_DETECT_INTERVAL));
    }

    fn take_due(&mut self, now: Instant) -> bool {
        if self.next.is_none_or(|deadline| now < deadline) {
            return false;
        }
        self.attempts = self
            .attempts
            .saturating_add(1)
            .min(DETECT_RETRY_DELAYS.len());
        self.next = None;
        true
    }

    fn wait_duration(&self, now: Instant) -> Duration {
        self.next
            .map(|deadline| deadline.saturating_duration_since(now))
            .unwrap_or(TABLET_POLL_INTERVAL)
            .min(TABLET_POLL_INTERVAL)
    }
}

fn publish_connection_state(ui: &slint::Weak<MainWindow>, state: &'static str) {
    let weak_ui = ui.clone();
    let _ = slint::invoke_from_event_loop(move || {
        if let Some(ui) = weak_ui.upgrade() {
            ui.set_backend_state(state.into());
            if state != "ready" {
                ui.set_device_name("".into());
                ui.set_pen_data_available(false);
            }
        }
    });
}

fn wait_for_command(
    command_receiver: &Receiver<BackendCommand>,
    timeout: Duration,
) -> Option<BackendCommand> {
    command_receiver.recv_timeout(timeout).ok()
}

pub(super) fn run(
    ui: slint::Weak<MainWindow>,
    command_receiver: Receiver<BackendCommand>,
    backend_events: Sender<BackendCommand>,
    displays: Vec<DisplayInfo>,
) {
    let mut client: Option<DaemonClient> = None;
    let mut pending_command = None;
    let mut queued_apply: Option<BackendCommand> = None;
    let mut refresh_requested = true;
    let mut detect_requested = false;
    let mut no_tablet = false;
    let mut driver_settings = None;
    // Windows needs SetSettings after DetectTablets to recreate its output
    // pipeline. Other platforms retain the one-time automatic display mapping.
    let mut automatic_mapping_attempted = cfg!(target_os = "windows");
    let mut active_generation = 0;
    let mut next_generation = 1;
    let mut last_ipc_error = String::new();
    let mut last_start_attempt = Instant::now()
        .checked_sub(START_RETRY_INTERVAL)
        .unwrap_or_else(Instant::now);
    let mut startup_wait_started = Some(Instant::now());
    let mut detection = DetectionSchedule::new(Instant::now());

    runtime_log("backend supervisor started");

    loop {
        let commands = pending_command
            .take()
            .into_iter()
            .chain(command_receiver.try_iter())
            .collect::<Vec<_>>();
        for command in commands {
            match command {
                BackendCommand::Detect => {
                    detect_requested = true;
                    refresh_requested = true;
                    driver_settings = None;
                    if !cfg!(target_os = "windows") {
                        automatic_mapping_attempted = false;
                    }
                    detection.reset_after_explicit_detect(Instant::now());
                }
                BackendCommand::TabletChanged { generation } if generation == active_generation => {
                    detect_requested = cfg!(target_os = "windows");
                    refresh_requested = true;
                    driver_settings = None;
                    if !cfg!(target_os = "windows") {
                        automatic_mapping_attempted = false;
                    }
                }
                BackendCommand::DriverDisconnected { generation, reason }
                    if generation == active_generation =>
                {
                    runtime_log(format!(
                        "driver IPC disconnected generation={generation}: {reason}"
                    ));
                    client = None;
                    refresh_requested = true;
                    driver_settings = None;
                    startup_wait_started = Some(Instant::now());
                    publish_connection_state(&ui, "daemon-starting");
                }
                BackendCommand::TabletChanged { .. }
                | BackendCommand::DriverDisconnected { .. } => {}
                command @ BackendCommand::ApplyArea { .. } => {
                    // Never drop user input while reconnecting; only the newest
                    // requested state matters.
                    queued_apply = Some(command);
                }
            }
        }

        if client.is_none() {
            if !daemon_is_running() {
                if last_start_attempt.elapsed() < START_RETRY_INTERVAL {
                    pending_command = wait_for_command(
                        &command_receiver,
                        START_RETRY_INTERVAL.saturating_sub(last_start_attempt.elapsed()),
                    );
                    continue;
                }

                last_start_attempt = Instant::now();
                if start_daemon() {
                    startup_wait_started = Some(Instant::now());
                    publish_connection_state(&ui, "daemon-starting");
                } else {
                    publish_connection_state(&ui, "daemon-not-running");
                    pending_command = wait_for_command(&command_receiver, START_RETRY_INTERVAL);
                    continue;
                }
            }

            if owned_daemon_is_running()
                && startup_wait_started
                    .is_some_and(|started| started.elapsed() >= IPC_STARTUP_DEADLINE)
                && !daemon_ipc_is_available()
            {
                runtime_log("packaged daemon missed IPC startup deadline; restarting");
                stop_daemon();
                startup_wait_started = None;
                publish_connection_state(&ui, "daemon-crashed");
                continue;
            }

            let generation = next_generation;
            match DaemonClient::connect(backend_events.clone(), generation) {
                Ok(connection) => {
                    runtime_log(format!("driver IPC connected generation={generation}"));
                    client = Some(connection);
                    active_generation = generation;
                    next_generation += 1;
                    refresh_requested = true;
                    driver_settings = None;
                    last_ipc_error.clear();
                    startup_wait_started = None;
                    detection = DetectionSchedule::new(Instant::now());
                    publish_connection_state(&ui, "daemon-starting");
                }
                Err(error) => {
                    let message = error.to_string();
                    if message != last_ipc_error {
                        runtime_log(format!(
                            "waiting for driver IPC generation={generation}: {message}"
                        ));
                        last_ipc_error = message;
                    }
                    publish_connection_state(&ui, "daemon-starting");
                    pending_command =
                        wait_for_command(&command_receiver, BACKEND_RECONNECT_INTERVAL);
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
                    last_ipc_error.clear();
                    let weak_ui = ui.clone();
                    let _ = slint::invoke_from_event_loop(move || {
                        if let Some(ui) = weak_ui.upgrade() {
                            publish_backend_snapshot(&ui, snapshot);
                        }
                    });
                }
                Err(error) => {
                    runtime_log(format!("driver RPC query failed: {error}"));
                    client = None;
                    driver_settings = None;
                    refresh_requested = true;
                    startup_wait_started = Some(Instant::now());
                    publish_connection_state(&ui, "daemon-starting");
                    continue;
                }
            }
        }

        if let Some(command) = queued_apply.take() {
            let Some(connection) = client.as_mut() else {
                queued_apply = Some(command);
                continue;
            };
            if driver_settings.is_none() {
                match connection.call("GetSettings", json!([])) {
                    Ok(settings) => driver_settings = Some(settings),
                    Err(error) => {
                        runtime_log(format!("failed to load settings before apply: {error}"));
                        queued_apply = Some(command);
                        client = None;
                        refresh_requested = true;
                        publish_connection_state(&ui, "daemon-starting");
                        continue;
                    }
                }
            }

            let applied = command.clone();
            let result = driver_settings
                .as_mut()
                .ok_or_else(|| io::Error::other("driver settings cache is empty"))
                .and_then(|settings| apply_area(connection, settings, command));
            match result {
                Ok(()) => {
                    let weak_ui = ui.clone();
                    let _ = slint::invoke_from_event_loop(move || {
                        if let Some(ui) = weak_ui.upgrade() {
                            publish_applied_area(&ui, applied);
                        }
                    });
                }
                Err(error) => {
                    runtime_log(format!("failed to apply tablet area: {error}"));
                    client = None;
                    driver_settings = None;
                    refresh_requested = true;
                    startup_wait_started = Some(Instant::now());
                    publish_connection_state(&ui, "daemon-starting");
                    continue;
                }
            }
        }

        if no_tablet {
            let now = Instant::now();
            match command_receiver.recv_timeout(detection.wait_duration(now)) {
                Ok(command) => pending_command = Some(command),
                Err(RecvTimeoutError::Timeout) => {
                    refresh_requested = true;
                    if detection.take_due(Instant::now()) {
                        detect_requested = true;
                    }
                }
                Err(RecvTimeoutError::Disconnected) => break,
            }
        } else {
            match command_receiver.recv_timeout(TABLET_POLL_INTERVAL) {
                Ok(command) => pending_command = Some(command),
                Err(RecvTimeoutError::Timeout) => {
                    // Polling also catches missed pipe notifications and hotplug
                    // events on all platforms.
                    refresh_requested = true;
                }
                Err(RecvTimeoutError::Disconnected) => break,
            }
        }
    }

    runtime_log("backend supervisor stopped");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detection_retries_back_off_then_enter_steady_state() {
        let start = Instant::now();
        let mut schedule = DetectionSchedule::new(start);
        for delay in DETECT_RETRY_DELAYS {
            let due = start + delay;
            schedule.next = Some(due);
            assert!(schedule.take_due(due));
            schedule.no_tablet(due);
        }
        let due = start + STEADY_STATE_DETECT_INTERVAL + Duration::from_secs(30);
        schedule.next = Some(due);
        assert!(schedule.take_due(due));
    }

    #[test]
    fn finding_a_tablet_stops_detection_retries() {
        let now = Instant::now();
        let mut schedule = DetectionSchedule::new(now);
        schedule.tablet_found();
        assert!(!schedule.take_due(now + Duration::from_secs(300)));
    }
}
