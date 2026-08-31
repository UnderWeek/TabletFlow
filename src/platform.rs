use super::*;

#[cfg(not(target_os = "windows"))]
use std::fs;
#[cfg(not(target_os = "windows"))]
use std::path::PathBuf;
#[cfg(not(target_os = "windows"))]
use std::process::{Command, Stdio};

#[cfg(target_os = "windows")]
pub(super) fn configure_autostart(enabled: bool, start_minimized: bool) -> io::Result<()> {
    windows_runtime::configure_autostart(enabled, start_minimized)
}

#[cfg(target_os = "macos")]
pub(super) fn configure_autostart(enabled: bool, start_minimized: bool) -> io::Result<()> {
    let executable = std::env::current_exe()?;
    let Some(home) = std::env::var_os("HOME") else {
        return Ok(());
    };
    let path = PathBuf::from(home).join("Library/LaunchAgents/com.underweek.tabletflow.plist");
    let domain = format!("gui/{}", current_uid());
    if !enabled {
        let _ = Command::new("launchctl")
            .args(["bootout", &domain, path.to_string_lossy().as_ref()])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
        if path.exists() {
            fs::remove_file(path)?;
        }
        return Ok(());
    }

    let Some(parent) = path.parent() else {
        return Ok(());
    };
    fs::create_dir_all(parent)?;
    let escaped_executable = executable
        .to_string_lossy()
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;");
    let argument = if start_minimized {
        "            <string>--start-minimized</string>\n"
    } else {
        ""
    };
    let plist = format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<!DOCTYPE plist PUBLIC \"-//Apple//DTD PLIST 1.0//EN\" \"http://www.apple.com/DTDs/PropertyList-1.0.dtd\">\n<plist version=\"1.0\"><dict>\n<key>Label</key><string>com.underweek.tabletflow</string>\n<key>ProgramArguments</key><array>\n            <string>{escaped_executable}</string>\n{argument}</array>\n<key>RunAtLoad</key><true/>\n</dict></plist>\n"
    );
    fs::write(&path, plist)?;
    let _ = Command::new("launchctl")
        .args(["bootout", &domain, path.to_string_lossy().as_ref()])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
    let _ = Command::new("launchctl")
        .args(["bootstrap", &domain, path.to_string_lossy().as_ref()])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();

    Ok(())
}

#[cfg(all(unix, not(target_os = "macos")))]
pub(super) fn configure_autostart(enabled: bool, start_minimized: bool) -> io::Result<()> {
    let executable = std::env::current_exe()?;
    let config_home = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".config")));
    let Some(config_home) = config_home else {
        return Ok(());
    };
    let path = config_home.join("autostart/TabletFlow.desktop");
    if !enabled {
        if path.exists() {
            fs::remove_file(path)?;
        }
        return Ok(());
    }
    let Some(parent) = path.parent() else {
        return Ok(());
    };
    fs::create_dir_all(parent)?;
    let executable = desktop_exec_quote(&executable.to_string_lossy());
    let minimized_argument = if start_minimized {
        " --start-minimized"
    } else {
        ""
    };
    fs::write(
        path,
        format!(
            "[Desktop Entry]\nType=Application\nName=TabletFlow\nExec={executable}{minimized_argument}\nX-GNOME-Autostart-enabled=true\n"
        ),
    )?;
    Ok(())
}

#[cfg(all(unix, not(target_os = "macos")))]
fn desktop_exec_quote(value: &str) -> String {
    let escaped = value
        .replace('\\', "\\\\")
        .replace('`', "\\`")
        .replace('$', "\\$")
        .replace('"', "\\\"");
    format!("\"{escaped}\"")
}

#[cfg(target_os = "macos")]
fn current_uid() -> String {
    Command::new("id")
        .arg("-u")
        .output()
        .ok()
        .and_then(|output| String::from_utf8(output.stdout).ok())
        .map(|uid| uid.trim().to_owned())
        .filter(|uid| !uid.is_empty())
        .unwrap_or_else(|| "0".into())
}

pub(super) fn daemon_is_running() -> bool {
    #[cfg(target_os = "windows")]
    {
        windows_runtime::daemon_is_running()
    }
    #[cfg(not(target_os = "windows"))]
    {
        unix_runtime::daemon_is_running()
    }
}

pub(super) fn daemon_ipc_is_available() -> bool {
    #[cfg(target_os = "windows")]
    {
        windows_runtime::pipe_is_available()
    }
    #[cfg(not(target_os = "windows"))]
    {
        unix_runtime::pipe_is_available()
    }
}

pub(super) fn owned_daemon_is_running() -> bool {
    #[cfg(target_os = "windows")]
    {
        windows_runtime::owned_daemon_is_running()
    }
    #[cfg(not(target_os = "windows"))]
    {
        unix_runtime::owned_daemon_is_running()
    }
}

pub(super) fn runtime_log(message: impl AsRef<str>) {
    #[cfg(target_os = "windows")]
    windows_runtime::log_line(message.as_ref());
    #[cfg(not(target_os = "windows"))]
    eprintln!("TabletFlow: {}", message.as_ref());
}

pub(super) fn backend_state() -> &'static str {
    if daemon_is_running() {
        "daemon-running"
    } else {
        "daemon-not-running"
    }
}

pub(super) fn start_daemon() -> bool {
    #[cfg(target_os = "windows")]
    {
        match windows_runtime::start_daemon() {
            Ok(_) => true,
            Err(error) => {
                windows_runtime::log_line(format!("driver launch failed: {error}"));
                false
            }
        }
    }
    #[cfg(not(target_os = "windows"))]
    {
        unix_runtime::start_daemon().is_ok()
    }
}

pub(super) fn stop_daemon() {
    #[cfg(target_os = "windows")]
    windows_runtime::stop_daemon();
    #[cfg(not(target_os = "windows"))]
    unix_runtime::stop_daemon();
}

#[cfg(target_os = "macos")]
#[link(name = "ApplicationServices", kind = "framework")]
unsafe extern "C" {
    fn AXIsProcessTrusted() -> bool;
}

#[cfg(target_os = "macos")]
#[link(name = "IOKit", kind = "framework")]
unsafe extern "C" {
    fn IOHIDCheckAccess(request_type: u32) -> u32;
    fn IOHIDRequestAccess(request_type: u32) -> bool;
}

pub(super) fn macos_permissions() -> (bool, bool) {
    #[cfg(target_os = "macos")]
    {
        const IOHID_REQUEST_TYPE_LISTEN_EVENT: u32 = 1;
        let input_monitoring = unsafe { IOHIDCheckAccess(IOHID_REQUEST_TYPE_LISTEN_EVENT) } == 0;
        let accessibility = unsafe { AXIsProcessTrusted() };
        (input_monitoring, accessibility)
    }

    #[cfg(not(target_os = "macos"))]
    {
        (true, true)
    }
}

pub(super) fn request_macos_permissions() {
    #[cfg(target_os = "macos")]
    {
        const IOHID_REQUEST_TYPE_LISTEN_EVENT: u32 = 1;
        let (input_monitoring_granted, accessibility_granted) = macos_permissions();
        if !input_monitoring_granted {
            let _ = unsafe { IOHIDRequestAccess(IOHID_REQUEST_TYPE_LISTEN_EVENT) };
            let _ = Command::new("open")
                .arg("x-apple.systempreferences:com.apple.preference.security?Privacy_ListenEvent")
                .spawn();
        } else if !accessibility_granted {
            let _ = Command::new("open")
                .arg(
                    "x-apple.systempreferences:com.apple.preference.security?Privacy_Accessibility",
                )
                .spawn();
        }
    }
}

pub(super) fn open_github() {
    const URL: &str = "https://github.com/UnderWeek/TabletFlow";

    #[cfg(target_os = "macos")]
    let _ = Command::new("open").arg(URL).spawn();

    #[cfg(target_os = "windows")]
    let _ = windows_runtime::open_url(URL);

    #[cfg(all(unix, not(target_os = "macos")))]
    let _ = Command::new("xdg-open").arg(URL).spawn();
}

pub(super) fn run_driver_self_test() -> io::Result<()> {
    #[cfg(target_os = "windows")]
    {
        windows_runtime::run_self_test()
    }
    #[cfg(unix)]
    {
        unix_runtime::run_self_test()
    }
    #[cfg(not(any(unix, target_os = "windows")))]
    {
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "driver self-test is unsupported on this platform",
        ))
    }
}

#[cfg(test)]
mod tests {
    #[cfg(all(unix, not(target_os = "macos")))]
    #[test]
    fn desktop_exec_paths_are_quoted_and_escaped() {
        assert_eq!(
            super::desktop_exec_quote(r#"/tmp/Tablet Flow/$test\"#),
            r#"\"/tmp/Tablet Flow/\$test\\\""#
        );
    }
}
