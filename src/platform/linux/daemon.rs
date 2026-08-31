use super::ipc;
use std::io;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

static MANAGED: OnceLock<Mutex<Option<Child>>> = OnceLock::new();
fn managed() -> &'static Mutex<Option<Child>> {
    MANAGED.get_or_init(|| Mutex::new(None))
}

fn candidates() -> Vec<PathBuf> {
    let mut candidates = Vec::new();
    let runtime_id = match std::env::consts::ARCH {
        "aarch64" => "linux-arm64",
        "x86" => "linux-x86",
        _ => "linux-x64",
    };
    let target = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("target/otd")
        .join(runtime_id);
    candidates.push(target.join("OpenTabletDriver.Daemon"));
    candidates.push(target.join("OpenTabletDriver.Daemon.dll"));
    if let Ok(executable) = std::env::current_exe() {
        if let Some(directory) = executable.parent() {
            for relative in [
                "OpenTabletDriver.Daemon",
                "OpenTabletDriver.Daemon.dll",
                "resources/OpenTabletDriver.Daemon",
                "resources/OpenTabletDriver.Daemon.dll",
            ] {
                candidates.push(directory.join(relative));
            }
        }
    }
    if let Some(root) = std::env::var_os("TABLETFLOW_RESOURCE_DIR") {
        candidates.push(PathBuf::from(&root).join("OpenTabletDriver.Daemon"));
        candidates.push(PathBuf::from(root).join("OpenTabletDriver.Daemon.dll"));
    }
    candidates
}
fn daemon_path() -> Option<PathBuf> {
    candidates().into_iter().find(|path| path.is_file())
}

pub fn owned_is_running() -> bool {
    let Ok(mut slot) = managed().lock() else {
        return false;
    };
    let Some(child) = slot.as_mut() else {
        return false;
    };
    match child.try_wait() {
        Ok(None) => true,
        _ => {
            *slot = None;
            false
        }
    }
}

pub fn start() -> io::Result<()> {
    if owned_is_running() || ipc::is_available() {
        return Ok(());
    }
    let path = daemon_path().ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::NotFound,
            "OpenTabletDriver.Daemon is missing from the TabletFlow package",
        )
    })?;
    let mut command = if path.extension().is_some_and(|ext| ext == "dll") {
        let mut command = Command::new("dotnet");
        command.arg(&path);
        command
    } else {
        Command::new(&path)
    };
    command
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    if let Some(directory) = path.parent() {
        command.current_dir(directory);
    }
    let child = command.spawn()?;
    *managed()
        .lock()
        .map_err(|_| io::Error::other("daemon process lock is poisoned"))? = Some(child);
    Ok(())
}

pub fn stop() {
    let Ok(mut slot) = managed().lock() else {
        return;
    };
    if let Some(mut child) = slot.take() {
        let _ = child.kill();
        let _ = child.wait();
    }
}

pub fn self_test() -> io::Result<()> {
    let already_available = ipc::is_available();
    let owned_before = owned_is_running();
    if !already_available {
        start()?;
    }
    let started_owned = !owned_before && owned_is_running();
    let deadline = Instant::now() + Duration::from_secs(60);
    let result = loop {
        if ipc::is_available() {
            break Ok(());
        }
        if started_owned && !owned_is_running() {
            break Err(io::Error::other(
                "OpenTabletDriver exited before creating its IPC endpoint",
            ));
        }
        if Instant::now() >= deadline {
            break Err(io::Error::new(
                io::ErrorKind::TimedOut,
                "OpenTabletDriver did not create its IPC endpoint",
            ));
        }
        std::thread::sleep(Duration::from_millis(100));
    };
    if started_owned {
        stop();
    }
    result
}
