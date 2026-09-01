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

fn self_test() -> io::Result<()> {
    let daemon_path = daemon::validate_bundle()?;
    let available_before = ipc::is_available();
    let owned_before = daemon::owned_is_running();
    if !available_before {
        daemon::start()?;
    }
    let started_owned = !owned_before && daemon::owned_is_running();
    let deadline = Instant::now() + Duration::from_secs(180);
    let result = loop {
        if ipc::is_available() {
            let (events, _) = mpsc::channel();
            let mut client = RpcClient::connect(
                platform(),
                events,
                0,
                std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
            )?;
            let tablets = client.call("GetTablets", json!([]))?;
            if !tablets.is_array() {
                break Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "GetTablets returned a non-array result",
                ));
            }
            let settings = client.call("GetSettings", json!([]))?;
            if settings.is_null() {
                break Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "GetSettings returned null",
                ));
            }
            break Ok(());
        }
        if started_owned && !daemon::owned_is_running() {
            break Err(io::Error::other(
                "OpenTabletDriver exited before creating its named pipe",
            ));
        }
        if Instant::now() >= deadline {
            break Err(io::Error::new(
                io::ErrorKind::TimedOut,
                "OpenTabletDriver did not create its named pipe",
            ));
        }
        std::thread::sleep(Duration::from_millis(100));
    };
    if started_owned {
        daemon::stop();
    }
    match &result {
        Ok(()) => runtime::log_line(format!(
            "Windows package self-test passed daemon={}",
            daemon_path.display()
        )),
        Err(error) => runtime::log_line(format!("Windows package self-test failed: {error}")),
    }
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
