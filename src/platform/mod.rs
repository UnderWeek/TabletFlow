use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};

pub trait Transport: Read + Write + Send {
    fn try_clone_box(&self) -> io::Result<Box<dyn Transport>>;
    fn interrupt(&self, reader: &std::thread::JoinHandle<()>);
}

pub trait Platform: Send + Sync {
    fn name(&self) -> &'static str;
    fn default_close_to_tray(&self) -> bool {
        false
    }
    fn settings_path(&self) -> Option<PathBuf>;
    fn legacy_settings_paths(&self) -> Vec<PathBuf> {
        Vec::new()
    }
    fn replace_file(&self, source: &Path, destination: &Path) -> io::Result<()> {
        std::fs::rename(source, destination)
    }
    fn initialize_process(&self) {}
    fn acquire_instance_guard(&self) -> Option<Box<dyn Send>>;
    fn connect_transport(&self) -> io::Result<Box<dyn Transport>>;
    fn ipc_available(&self) -> bool;
    fn owned_daemon_running(&self) -> bool;
    fn start_daemon(&self) -> io::Result<()>;
    fn stop_daemon(&self);
    fn configure_autostart(&self, enabled: bool, start_minimized: bool) -> io::Result<()>;
    fn permissions(&self) -> (bool, bool) {
        (true, true)
    }
    fn request_permissions(&self) {}
    fn open_url(&self, url: &str) -> io::Result<()>;
    fn run_driver_self_test(&self) -> io::Result<()>;
    fn persist_driver_settings(&self, _settings: &serde_json::Value) -> io::Result<()> {
        Ok(())
    }
    fn restore_pipeline_after_detect(&self) -> bool {
        false
    }
    fn auto_map_primary_display(&self) -> bool {
        true
    }
    fn rpc_timeout(&self, _method: &str) -> std::time::Duration {
        std::time::Duration::from_secs(15)
    }
    fn log(&self, message: &str) {
        eprintln!("TabletFlow: {message}");
    }
}

#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "macos")]
mod macos;
#[cfg(target_os = "windows")]
mod windows;

pub fn current() -> &'static dyn Platform {
    #[cfg(target_os = "linux")]
    {
        linux::platform()
    }
    #[cfg(target_os = "macos")]
    {
        macos::platform()
    }
    #[cfg(target_os = "windows")]
    {
        windows::platform()
    }
}
