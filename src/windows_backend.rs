//! Windows-specific driver supervisor and recovery loop.
//!
//! The previous shared worker stopped reconnecting after one IPC error and went
//! to sleep forever after publishing `no-tablet`.  Windows gets an independent
//! state machine so the stable macOS/Linux paths stay byte-for-byte in their
//! existing implementation.

use super::*;
use std::time::Instant;

const TABLET_POLL_INTERVAL: Duration = Duration::from_millis(900);
const START_RETRY_INTERVAL: Duration = Duration::from_millis(1500);
// DriverDaemon performs a synchronous HID scan before it creates the RPC pipe.
// On Windows this can legitimately take a couple of minutes while HID-class
// drivers settle after login or USB resume. Do not restart a living process
// during that initialization window.
const PIPE_STARTUP_DEADLINE: Duration = Duration::from_secs(180);
const MAX_AUTOMATIC_RESTARTS: usize = 3;
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
        // The explicit request itself is the first attempt in the new burst.
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
    match command_receiver.recv_timeout(timeout) {
        Ok(command) => Some(command),
        Err(RecvTimeoutError::Timeout) => None,
        Err(RecvTimeoutError::Disconnected) => None,
    }
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
    let mut automatic_mapping_attempted = true;
    let mut active_generation = 0;
    let mut next_generation = 1;
    let mut last_ipc_error = String::new();
    let mut last_start_attempt = Instant::now()
        .checked_sub(START_RETRY_INTERVAL)
        .unwrap_or_else(Instant::now);
    let mut startup_wait_started = Some(Instant::now());
    let mut automatic_restarts = 0usize;
    let mut detection = DetectionSchedule::new(Instant::now());

    windows_runtime::log_line("Windows backend supervisor started");

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
                    automatic_restarts = 0;
                    detection.reset_after_explicit_detect(Instant::now());
                }
                BackendCommand::TabletChanged { generation } if generation == active_generation => {
                    detect_requested = true;
                    refresh_requested = true;
                    driver_settings = None;
                }
                BackendCommand::DriverDisconnected { generation, reason }
                    if generation == active_generation =>
                {
                    windows_runtime::log_line(format!(
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
                    // Keep the most recent user request while the daemon is starting
                    // or reconnecting instead of silently dropping it.
                    queued_apply = Some(command);
                }
            }
        }

        if client.is_none() {
            if !windows_runtime::daemon_is_running() {
                if automatic_restarts >= MAX_AUTOMATIC_RESTARTS {
                    publish_connection_state(&ui, "daemon-crashed");
                    pending_command = wait_for_command(&command_receiver, START_RETRY_INTERVAL);
                    continue;
                }

                if last_start_attempt.elapsed() < START_RETRY_INTERVAL {
                    pending_command = wait_for_command(
                        &command_receiver,
                        START_RETRY_INTERVAL.saturating_sub(last_start_attempt.elapsed()),
                    );
                    continue;
                }

                last_start_attempt = Instant::now();
                match windows_runtime::start_daemon() {
                    Ok(windows_runtime::StartResult::Started) => {
                        automatic_restarts += 1;
                        startup_wait_started = Some(Instant::now());
                        publish_connection_state(&ui, "daemon-starting");
                    }
                    Ok(windows_runtime::StartResult::AlreadyConnected) => {
                        startup_wait_started = None;
                    }
                    Ok(windows_runtime::StartResult::AlreadyStarting) => {
                        startup_wait_started.get_or_insert_with(Instant::now);
                    }
                    Err(error) => {
                        automatic_restarts += 1;
                        windows_runtime::log_line(format!("unable to start driver: {error}"));
                        publish_connection_state(
                            &ui,
                            if error.kind() == io::ErrorKind::NotFound {
                                "daemon-not-running"
                            } else {
                                "daemon-crashed"
                            },
                        );
                        pending_command = wait_for_command(&command_receiver, START_RETRY_INTERVAL);
                        continue;
                    }
                }
            }

            if windows_runtime::owned_daemon_is_running()
                && startup_wait_started
                    .is_some_and(|started| started.elapsed() >= PIPE_STARTUP_DEADLINE)
                && !windows_runtime::pipe_is_available()
            {
                windows_runtime::log_line(
                    "packaged daemon missed pipe startup deadline; performing controlled restart",
                );
                windows_runtime::stop_daemon();
                startup_wait_started = None;
                publish_connection_state(&ui, "daemon-crashed");
                continue;
            }

            let generation = next_generation;
            match DaemonClient::connect(backend_events.clone(), generation) {
                Ok(connection) => {
                    windows_runtime::log_line(format!(
                        "driver IPC connected generation={generation}"
                    ));
                    client = Some(connection);
                    active_generation = generation;
                    next_generation += 1;
                    refresh_requested = true;
                    detect_requested = false;
                    driver_settings = None;
                    last_ipc_error.clear();
                    startup_wait_started = None;
                    detection = DetectionSchedule::new(Instant::now());
                    publish_connection_state(&ui, "daemon-starting");
                }
                Err(error) => {
                    let message = error.to_string();
                    if message != last_ipc_error {
                        windows_runtime::log_line(format!(
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
            let result = query_backend(
                client.as_mut().expect("client checked above"),
                detect,
                &displays,
                &mut automatic_mapping_attempted,
                &mut driver_settings,
            );
            match result {
                Ok(snapshot) => {
                    no_tablet = snapshot.state == "no-tablet";
                    if no_tablet {
                        detection.no_tablet(Instant::now());
                    } else {
                        detection.tablet_found();
                    }
                    automatic_restarts = 0;
                    last_ipc_error.clear();
                    let weak_ui = ui.clone();
                    let _ = slint::invoke_from_event_loop(move || {
                        if let Some(ui) = weak_ui.upgrade() {
                            publish_backend_snapshot(&ui, snapshot);
                        }
                    });
                }
                Err(error) => {
                    windows_runtime::log_line(format!("driver RPC query failed: {error}"));
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
            let connection = client.as_mut().expect("client checked above");
            if driver_settings.is_none() {
                match connection.call("GetSettings", json!([])) {
                    Ok(settings) => driver_settings = Some(settings),
                    Err(error) => {
                        windows_runtime::log_line(format!(
                            "failed to load settings before apply: {error}"
                        ));
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
                    windows_runtime::log_line(format!("failed to apply tablet area: {error}"));
                    // A timed-out/unconfirmed operation must not be echoed as a
                    // success. Reconnect and read the actual driver state.
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
            let timeout = detection.wait_duration(now);
            match command_receiver.recv_timeout(timeout) {
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
            match command_receiver.recv() {
                Ok(command) => pending_command = Some(command),
                Err(_) => break,
            }
        }
    }

    windows_runtime::log_line("Windows backend supervisor stopped");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detection_burst_is_bounded_and_backed_off() {
        let start = Instant::now();
        let mut schedule = DetectionSchedule::new(start);
        for attempt in 0..DETECT_RETRY_DELAYS.len() {
            let due = start
                + DETECT_RETRY_DELAYS[..=attempt]
                    .iter()
                    .copied()
                    .fold(Duration::ZERO, |sum, delay| sum + delay);
            schedule.next = Some(due);
            assert!(schedule.take_due(due));
            schedule.no_tablet(due);
        }
        assert!(!schedule.take_due(start + Duration::from_secs(1)));
        schedule.no_tablet(start + Duration::from_secs(1));
        let steady_state_due = start + Duration::from_secs(1) + STEADY_STATE_DETECT_INTERVAL;
        assert!(schedule.take_due(steady_state_due));
        schedule.no_tablet(steady_state_due);
        assert!(schedule.take_due(steady_state_due + STEADY_STATE_DETECT_INTERVAL));
    }

    #[test]
    fn finding_a_tablet_stops_detection_retries() {
        let now = Instant::now();
        let mut schedule = DetectionSchedule::new(now);
        schedule.tablet_found();
        assert!(!schedule.take_due(now + Duration::from_secs(300)));
        assert_eq!(schedule.wait_duration(now), TABLET_POLL_INTERVAL);
    }
}
