use std::time::{Duration, Instant};

pub const TABLET_POLL_INTERVAL: Duration = Duration::from_millis(900);
pub const START_RETRY_INTERVAL: Duration = Duration::from_millis(1500);
pub const IPC_STARTUP_DEADLINE: Duration = Duration::from_secs(180);
/// Grace period given to an owned daemon that was previously connected and
/// then lost IPC (as opposed to one still starting up). Shorter than
/// `IPC_STARTUP_DEADLINE` because a daemon that already proved it can create
/// its IPC endpoint once is expected to recover quickly or not at all -
/// waiting the full startup deadline here would leave TabletFlow stuck
/// pointing at a hung-but-alive process for minutes.
pub const IPC_RECONNECT_GRACE: Duration = Duration::from_secs(20);
pub const BACKEND_RECONNECT_INTERVAL: Duration = Duration::from_millis(1200);
const DETECT_RETRY_DELAYS: [Duration; 6] = [
    Duration::ZERO,
    Duration::from_secs(2),
    Duration::from_secs(4),
    Duration::from_secs(7),
    Duration::from_secs(12),
    Duration::from_secs(20),
];
const STEADY_STATE_DETECT_INTERVAL: Duration = Duration::from_secs(15);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ConnectionAction {
    Connect,
    StartOwnedDaemon,
    WaitForIpc,
    WaitForRetry,
    RestartOwnedDaemon,
}

#[derive(Debug)]
pub struct DaemonLifecycle {
    last_start_attempt: Option<Instant>,
    // Absolute deadline by which an owned daemon must have usable IPC, set
    // both while it is starting up (`spawn_succeeded`) and again if it was
    // connected and then lost IPC while still alive (`disconnected`). Without
    // the latter, `next_action` would return `WaitForIpc` forever once
    // `connected()` had cleared this field, even though the owned process is
    // still running and unresponsive.
    ipc_wait_deadline: Option<Instant>,
    connected: bool,
}

impl DaemonLifecycle {
    pub fn new() -> Self {
        Self {
            last_start_attempt: None,
            ipc_wait_deadline: None,
            connected: false,
        }
    }

    pub fn next_action(
        &self,
        now: Instant,
        ipc_available: bool,
        owned_daemon_running: bool,
    ) -> ConnectionAction {
        if ipc_available {
            return ConnectionAction::Connect;
        }
        if owned_daemon_running {
            if self
                .ipc_wait_deadline
                .is_some_and(|deadline| now >= deadline)
            {
                ConnectionAction::RestartOwnedDaemon
            } else {
                ConnectionAction::WaitForIpc
            }
        } else if self
            .last_start_attempt
            .is_some_and(|attempt| now.saturating_duration_since(attempt) < START_RETRY_INTERVAL)
        {
            ConnectionAction::WaitForRetry
        } else {
            ConnectionAction::StartOwnedDaemon
        }
    }

    pub fn retry_wait(&self, now: Instant) -> Duration {
        self.last_start_attempt
            .map(|attempt| {
                START_RETRY_INTERVAL.saturating_sub(now.saturating_duration_since(attempt))
            })
            .unwrap_or(Duration::ZERO)
    }

    pub fn start_attempted(&mut self, now: Instant) {
        self.last_start_attempt = Some(now);
    }

    pub fn spawn_succeeded(&mut self, now: Instant) {
        self.ipc_wait_deadline = Some(now + IPC_STARTUP_DEADLINE);
    }

    pub fn spawn_failed(&mut self) {
        self.ipc_wait_deadline = None;
    }

    pub fn connected(&mut self) {
        self.connected = true;
        self.ipc_wait_deadline = None;
    }

    /// Must be called with the daemon-ownership state observed at disconnect
    /// time: if an owned daemon process is still alive, this starts (or
    /// keeps) a bounded grace period after which `next_action` will report
    /// `RestartOwnedDaemon` instead of waiting for IPC indefinitely.
    pub fn disconnected(&mut self, now: Instant, owned_daemon_running: bool) {
        self.connected = false;
        if owned_daemon_running {
            self.ipc_wait_deadline
                .get_or_insert(now + IPC_RECONNECT_GRACE);
        } else {
            self.ipc_wait_deadline = None;
        }
    }

    pub fn owned_daemon_stopped(&mut self) {
        self.ipc_wait_deadline = None;
    }

    #[cfg(test)]
    fn is_connected(&self) -> bool {
        self.connected
    }
}

#[derive(Debug)]
pub struct DetectionSchedule {
    attempts: usize,
    next: Option<Instant>,
}

impl DetectionSchedule {
    pub fn new(now: Instant) -> Self {
        Self {
            attempts: 0,
            next: Some(now),
        }
    }

    pub fn reset_after_explicit_detect(&mut self, now: Instant) {
        self.attempts = 1;
        self.next = Some(now + DETECT_RETRY_DELAYS[1]);
    }

    pub fn tablet_found(&mut self) {
        self.attempts = 0;
        self.next = None;
    }

    pub fn no_tablet(&mut self, now: Instant) {
        self.next = DETECT_RETRY_DELAYS
            .get(self.attempts)
            .copied()
            .map(|delay| now + delay)
            .or(Some(now + STEADY_STATE_DETECT_INTERVAL));
    }

    pub fn take_due(&mut self, now: Instant) -> bool {
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

    pub fn wait_duration(&self, now: Instant) -> Duration {
        self.next
            .map(|deadline| deadline.saturating_duration_since(now))
            .unwrap_or(TABLET_POLL_INTERVAL)
            .min(TABLET_POLL_INTERVAL)
    }
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

    #[test]
    fn ipc_already_available_connects_without_spawn() {
        let lifecycle = DaemonLifecycle::new();
        assert_eq!(
            lifecycle.next_action(Instant::now(), true, false),
            ConnectionAction::Connect
        );
    }

    #[test]
    fn no_daemon_spawns_and_connects_when_ipc_appears() {
        let now = Instant::now();
        let mut lifecycle = DaemonLifecycle::new();
        assert_eq!(
            lifecycle.next_action(now, false, false),
            ConnectionAction::StartOwnedDaemon
        );
        lifecycle.start_attempted(now);
        lifecycle.spawn_succeeded(now);
        assert_eq!(
            lifecycle.next_action(now + Duration::from_millis(10), true, true),
            ConnectionAction::Connect
        );
    }

    #[test]
    fn spawn_failure_is_bounded_by_retry_interval() {
        let now = Instant::now();
        let mut lifecycle = DaemonLifecycle::new();
        lifecycle.start_attempted(now);
        lifecycle.spawn_failed();
        assert_eq!(
            lifecycle.next_action(now + Duration::from_millis(100), false, false),
            ConnectionAction::WaitForRetry
        );
        assert_eq!(
            lifecycle.next_action(now + START_RETRY_INTERVAL, false, false),
            ConnectionAction::StartOwnedDaemon
        );
    }

    #[test]
    fn owned_daemon_waits_for_ipc_then_restarts_after_deadline() {
        let now = Instant::now();
        let mut lifecycle = DaemonLifecycle::new();
        lifecycle.start_attempted(now);
        lifecycle.spawn_succeeded(now);
        assert_eq!(
            lifecycle.next_action(now + Duration::from_secs(1), false, true),
            ConnectionAction::WaitForIpc
        );
        assert_eq!(
            lifecycle.next_action(now + IPC_STARTUP_DEADLINE, false, true),
            ConnectionAction::RestartOwnedDaemon
        );
    }

    #[test]
    fn disconnect_returns_to_reconnect_flow() {
        let now = Instant::now();
        let mut lifecycle = DaemonLifecycle::new();
        lifecycle.connected();
        assert!(lifecycle.is_connected());
        lifecycle.disconnected(now, false);
        assert!(!lifecycle.is_connected());
        assert_eq!(
            lifecycle.next_action(now, true, false),
            ConnectionAction::Connect
        );
    }

    // Regression test: spawn -> connect -> IPC lost while the owned daemon
    // process remains alive must not wait for IPC forever. Before this fix,
    // `connected()` cleared the only deadline tracked, so a lost-but-alive
    // owned daemon produced `WaitForIpc` indefinitely and TabletFlow never
    // restarted it.
    #[test]
    fn owned_daemon_disconnect_after_connect_restarts_after_grace_period() {
        let now = Instant::now();
        let mut lifecycle = DaemonLifecycle::new();
        lifecycle.start_attempted(now);
        lifecycle.spawn_succeeded(now);
        assert_eq!(
            lifecycle.next_action(now + Duration::from_millis(10), true, true),
            ConnectionAction::Connect
        );
        lifecycle.connected();

        let disconnect_time = now + Duration::from_secs(30);
        lifecycle.disconnected(disconnect_time, true);

        // Still within the grace period: keep waiting for IPC to come back,
        // not StartOwnedDaemon (the process is still alive) and not stuck
        // forever either.
        assert_eq!(
            lifecycle.next_action(disconnect_time + Duration::from_secs(1), false, true),
            ConnectionAction::WaitForIpc
        );

        assert_eq!(
            lifecycle.next_action(disconnect_time + IPC_RECONNECT_GRACE, false, true),
            ConnectionAction::RestartOwnedDaemon
        );
    }

    // Regression test: a disconnect from an external (non-owned) daemon must
    // not start an IPC-wait deadline - TabletFlow never restarts processes it
    // doesn't own, so it should fall through to StartOwnedDaemon once the
    // usual retry interval has passed.
    #[test]
    fn external_daemon_disconnect_does_not_arm_a_restart_deadline() {
        let now = Instant::now();
        let mut lifecycle = DaemonLifecycle::new();
        lifecycle.connected();
        lifecycle.disconnected(now, false);
        assert_eq!(
            lifecycle.next_action(now + IPC_RECONNECT_GRACE * 10, false, false),
            ConnectionAction::StartOwnedDaemon
        );
    }

    #[test]
    fn healthy_external_daemon_is_reused() {
        let lifecycle = DaemonLifecycle::new();
        assert_eq!(
            lifecycle.next_action(Instant::now(), true, false),
            ConnectionAction::Connect
        );
    }

    #[test]
    fn stale_external_process_without_ipc_cannot_block_startup() {
        let lifecycle = DaemonLifecycle::new();
        // External process existence is deliberately not an input. Only usable
        // IPC or a daemon owned by TabletFlow affects the lifecycle decision.
        assert_eq!(
            lifecycle.next_action(Instant::now(), false, false),
            ConnectionAction::StartOwnedDaemon
        );
    }
}
