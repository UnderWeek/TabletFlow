mod autostart;
mod daemon;
mod ipc;
mod permissions;
mod runtime;

use super::{Platform, Transport};
use std::io;

pub struct MacOsPlatform;
static PLATFORM: MacOsPlatform = MacOsPlatform;

pub fn platform() -> &'static dyn Platform {
    &PLATFORM
}

impl Platform for MacOsPlatform {
    fn name(&self) -> &'static str {
        "macOS"
    }
    fn settings_path(&self) -> Option<std::path::PathBuf> {
        std::env::var_os("HOME")
            .map(std::path::PathBuf::from)
            .map(|home| home.join("Library/Application Support/TabletFlow/settings.conf"))
    }
    fn legacy_settings_paths(&self) -> Vec<std::path::PathBuf> {
        std::env::var_os("HOME")
            .map(std::path::PathBuf::from)
            .map(|home| vec![home.join(".tabletflow/settings.conf")])
            .unwrap_or_default()
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
        daemon::stop()
    }
    fn configure_autostart(&self, enabled: bool, start_minimized: bool) -> io::Result<()> {
        autostart::configure(enabled, start_minimized)
    }
    fn permissions(&self) -> (bool, bool) {
        permissions::status()
    }
    fn request_permissions(&self) {
        permissions::request()
    }
    fn open_url(&self, url: &str) -> io::Result<()> {
        runtime::open_url(url)
    }
    fn run_driver_self_test(&self) -> io::Result<()> {
        daemon::self_test()
    }
}
