use std::fs;
use std::io;
use std::path::PathBuf;
use std::process::{Command, Stdio};

fn uid() -> String {
    Command::new("id")
        .arg("-u")
        .output()
        .ok()
        .and_then(|output| String::from_utf8(output.stdout).ok())
        .map(|uid| uid.trim().to_owned())
        .filter(|uid| !uid.is_empty())
        .unwrap_or_else(|| "0".into())
}

pub fn configure(enabled: bool, start_minimized: bool) -> io::Result<()> {
    let executable = std::env::current_exe()?;
    let Some(home) = std::env::var_os("HOME") else {
        return Ok(());
    };
    let path = PathBuf::from(home).join("Library/LaunchAgents/com.underweek.tabletflow.plist");
    let domain = format!("gui/{}", uid());
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
    let executable = executable
        .to_string_lossy()
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;");
    let argument = if start_minimized {
        "<string>--start-minimized</string>"
    } else {
        ""
    };
    fs::write(&path, format!("<?xml version=\"1.0\" encoding=\"UTF-8\"?><!DOCTYPE plist PUBLIC \"-//Apple//DTD PLIST 1.0//EN\" \"http://www.apple.com/DTDs/PropertyList-1.0.dtd\"><plist version=\"1.0\"><dict><key>Label</key><string>com.underweek.tabletflow</string><key>ProgramArguments</key><array><string>{executable}</string>{argument}</array><key>RunAtLoad</key><true/></dict></plist>"))?;
    // bootout is expected to fail with a nonzero status when no agent was
    // previously loaded (e.g. the very first time autostart is enabled), so
    // its result is intentionally not checked. bootstrap is the operation
    // that actually matters: if it fails, the user just turned "start with
    // system" on but nothing was registered with launchd, so that failure is
    // surfaced instead of being silently swallowed.
    let _ = Command::new("launchctl")
        .args(["bootout", &domain, path.to_string_lossy().as_ref()])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
    let status = Command::new("launchctl")
        .args(["bootstrap", &domain, path.to_string_lossy().as_ref()])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()?;
    if !status.success() {
        return Err(io::Error::other(format!(
            "launchctl bootstrap failed with {status}"
        )));
    }
    Ok(())
}
