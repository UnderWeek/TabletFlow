mod autostart;
mod daemon;
mod ipc;
mod runtime;

use super::{Platform, Transport};
use std::io;

pub struct LinuxPlatform;
static PLATFORM: LinuxPlatform = LinuxPlatform;

pub fn platform() -> &'static dyn Platform {
    &PLATFORM
}

impl Platform for LinuxPlatform {
    fn name(&self) -> &'static str {
        "Linux"
    }
    fn settings_path(&self) -> Option<std::path::PathBuf> {
        std::env::var_os("XDG_CONFIG_HOME")
            .map(std::path::PathBuf::from)
            .or_else(|| {
                std::env::var_os("HOME").map(|home| std::path::PathBuf::from(home).join(".config"))
            })
            .map(|root| root.join("tabletflow/settings.conf"))
    }
    fn legacy_settings_paths(&self) -> Vec<std::path::PathBuf> {
        let mut paths = Vec::new();
        if let Some(home) = std::env::var_os("HOME").map(std::path::PathBuf::from) {
            paths.push(home.join(".tabletflow/settings.conf"));
            paths.push(home.join(".config/TabletFlow/settings.conf"));
        }
        if let Some(config) = std::env::var_os("XDG_CONFIG_HOME").map(std::path::PathBuf::from) {
            paths.push(config.join("TabletFlow/settings.conf"));
        }
        paths
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
    fn open_url(&self, url: &str) -> io::Result<()> {
        runtime::open_url(url)
    }
    fn run_driver_self_test(&self) -> io::Result<()> {
        daemon::self_test()
    }
    fn persist_driver_settings(&self, settings: &serde_json::Value) -> io::Result<()> {
        runtime::persist_driver_settings(settings)
    }
}
