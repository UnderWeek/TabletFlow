mod autostart;
mod daemon;
mod ipc;
mod permissions;
mod runtime;

use super::{Platform, Transport};
use crate::core::rpc::RpcClient;
use serde_json::json;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::time::{Duration, Instant};

pub struct WindowsPlatform;
static PLATFORM: WindowsPlatform = WindowsPlatform;

pub fn platform() -> &'static dyn Platform {
    &PLATFORM
}

// The CI packaging workflow kills the whole self-test process after 240s.
// This deadline is the single source of truth for how long self_test() as a
// whole is allowed to run - the pipe wait AND every RPC call below all draw
// from the same remaining budget, so no combination of slow steps can add up
// to more than SELF_TEST_DEADLINE. That leaves headroom for daemon
// start/stop teardown and log flushing before the workflow's own timeout
// would otherwise kill the process mid-diagnostic.
const SELF_TEST_DEADLINE: Duration = Duration::from_secs(200);

// Production traffic uses a 180s rpc_timeout on DetectTablets/SetSettings to
// tolerate slow enumeration on real hardware. The self-test never has real
// hardware attached, so a slow response here means something is actually
// wedged, not that a device is taking its time - a much tighter cap both
// fails faster and (combined with SELF_TEST_DEADLINE) prevents the pipe wait
// plus four RPC calls from ever summing past the workflow's own timeout.
const SELF_TEST_RPC_TIMEOUT: Duration = Duration::from_secs(30);

fn self_test_progress(message: impl AsRef<str>) {
    let message = message.as_ref();
    eprintln!("[self-test] {message}");
    runtime::log_line(format!("self-test: {message}"));
}

fn self_test() -> io::Result<()> {
    let overall_deadline = Instant::now() + SELF_TEST_DEADLINE;
    let daemon_path = daemon::validate_bundle()?;
    self_test_progress("bundle validated");
    let available_before = ipc::is_available();
    let owned_before = daemon::owned_is_running();
    if !available_before {
        self_test_progress("daemon start requested");
        daemon::start()?;
    }
    let started_owned = !owned_before && daemon::owned_is_running();
    let result = loop {
        if ipc::is_available() {
            self_test_progress("named pipe available");
            let (events, _) = mpsc::channel();
            let mut client = RpcClient::connect(
                platform(),
                events,
                0,
                std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
            )?;
            self_test_progress("RpcClient connected");

            // Exercises the real pipeline sequence backend.rs relies on, not
            // just connectivity: DetectTablets (no physical tablet is
            // required - an empty result is fine), GetSettings, SetSettings
            // with those same settings, then GetSettings again to make sure
            // the round trip doesn't wedge the daemon. GetTablets alone
            // would only prove the pipe is open, not that this RPC sequence
            // actually works end to end. Each call's timeout is capped by
            // the remaining slice of the shared overall_deadline so a stall
            // on an earlier call can't let a later call still wait the full
            // SELF_TEST_RPC_TIMEOUT.
            let remaining = |deadline: Instant| -> io::Result<Duration> {
                let left = deadline.saturating_duration_since(Instant::now());
                if left.is_zero() {
                    Err(io::Error::new(
                        io::ErrorKind::TimedOut,
                        "self-test exceeded its overall deadline",
                    ))
                } else {
                    Ok(left.min(SELF_TEST_RPC_TIMEOUT))
                }
            };

            self_test_progress("DetectTablets begin");
            let timeout = remaining(overall_deadline)?;
            let tablets = client.call_with_timeout("DetectTablets", json!([]), timeout)?;
            self_test_progress("DetectTablets end");
            if !tablets.is_array() {
                break Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "DetectTablets returned a non-array result",
                ));
            }

            self_test_progress("GetSettings begin");
            let timeout = remaining(overall_deadline)?;
            let settings = client.call_with_timeout("GetSettings", json!([]), timeout)?;
            self_test_progress("GetSettings end");
            if settings.is_null() {
                break Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "GetSettings returned null",
                ));
            }

            self_test_progress("SetSettings begin");
            let timeout = remaining(overall_deadline)?;
            client.call_with_timeout("SetSettings", json!([settings]), timeout)?;
            self_test_progress("SetSettings end");

            self_test_progress("final GetSettings begin");
            let timeout = remaining(overall_deadline)?;
            let settings_after = client.call_with_timeout("GetSettings", json!([]), timeout)?;
            self_test_progress("final GetSettings end");
            if settings_after.is_null() {
                break Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "GetSettings returned null after SetSettings",
                ));
            }

            self_test_progress("RpcClient drop begin");
            drop(client);
            self_test_progress("RpcClient drop end");
            break Ok(());
        }
        if started_owned && !daemon::owned_is_running() {
            break Err(io::Error::other(
                "OpenTabletDriver exited before creating its named pipe",
            ));
        }
        if Instant::now() >= overall_deadline {
            break Err(io::Error::new(
                io::ErrorKind::TimedOut,
                "OpenTabletDriver did not create its named pipe",
            ));
        }
        std::thread::sleep(Duration::from_millis(100));
    };
    if started_owned {
        self_test_progress("daemon stop begin");
        daemon::stop();
        self_test_progress("daemon stop end");
    }
    match &result {
        Ok(()) => runtime::log_line(format!(
            "Windows package self-test passed daemon={}",
            daemon_path.display()
        )),
        Err(error) => runtime::log_line(format!("Windows package self-test failed: {error}")),
    }
    self_test_progress("self-test complete");
    result
}

impl Platform for WindowsPlatform {
    fn name(&self) -> &'static str {
        "Windows"
    }

    fn default_close_to_tray(&self) -> bool {
        true
    }

    fn settings_path(&self) -> Option<PathBuf> {
        std::env::var_os("APPDATA")
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("USERPROFILE").map(PathBuf::from))
            .map(|root| root.join("TabletFlow/settings.conf"))
    }

    fn legacy_settings_paths(&self) -> Vec<PathBuf> {
        std::env::var_os("USERPROFILE")
            .map(PathBuf::from)
            .map(|home| vec![home.join(".tabletflow/settings.conf")])
            .unwrap_or_default()
    }

    fn replace_file(&self, source: &Path, destination: &Path) -> io::Result<()> {
        runtime::replace_file(source, destination)
    }

    fn initialize_process(&self) {
        runtime::initialize_process();
    }

    fn acquire_instance_guard(&self) -> Option<Box<dyn Send>> {
        runtime::acquire_instance_guard().map(|guard| Box::new(guard) as Box<dyn Send>)
    }

    fn connect_transport(&self) -> io::Result<Box<dyn Transport>> {
        ipc::connect()
    }

    fn ipc_available(&self) -> bool {
        ipc::is_available()
    }

    fn owned_daemon_running(&self) -> bool {
        daemon::owned_is_running()
    }

    fn start_daemon(&self) -> io::Result<()> {
        daemon::start()
    }

    fn stop_daemon(&self) {
        daemon::stop();
    }

    fn configure_autostart(&self, enabled: bool, start_minimized: bool) -> io::Result<()> {
        autostart::configure(enabled, start_minimized)
    }

    fn permissions(&self) -> (bool, bool) {
        permissions::status()
    }

    fn request_permissions(&self) {
        permissions::request();
    }

    fn open_url(&self, url: &str) -> io::Result<()> {
        runtime::open_url(url)
    }

    fn run_driver_self_test(&self) -> io::Result<()> {
        self_test()
    }

    fn persist_driver_settings(&self, settings: &serde_json::Value) -> io::Result<()> {
        runtime::persist_driver_settings(settings)
    }

    fn restore_pipeline_after_detect(&self) -> bool {
        true
    }

    fn auto_map_primary_display(&self) -> bool {
        false
    }

    fn rpc_timeout(&self, method: &str) -> Duration {
        if matches!(method, "DetectTablets" | "SetSettings") {
            Duration::from_secs(180)
        } else {
            Duration::from_secs(30)
        }
    }

    fn log(&self, message: &str) {
        runtime::log_line(message);
    }
}
