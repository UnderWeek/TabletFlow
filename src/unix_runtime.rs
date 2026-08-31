//! Unix (macOS/Linux) process, IPC, and single-instance integration.

use std::fs;
use std::io;
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

const DAEMON_PIPE_NAME: &str = "OpenTabletDriver.Daemon";
static MANAGED_DAEMON: OnceLock<Mutex<Option<Child>>> = OnceLock::new();

pub(super) struct InstanceGuard {
    _listener: UnixListener,
    path: PathBuf,
}

impl Drop for InstanceGuard {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
    }
}

fn managed_daemon() -> &'static Mutex<Option<Child>> {
    MANAGED_DAEMON.get_or_init(|| Mutex::new(None))
}

fn ipc_path() -> PathBuf {
    std::env::temp_dir().join(format!("CoreFxPipe_{DAEMON_PIPE_NAME}"))
}

fn instance_path() -> PathBuf {
    let uid = Command::new("id")
        .arg("-u")
        .output()
        .ok()
        .and_then(|output| String::from_utf8(output.stdout).ok())
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| "unknown".into());
    std::env::temp_dir().join(format!("tabletflow-{uid}.sock"))
}

pub(super) fn acquire_instance_guard() -> Option<InstanceGuard> {
    let path = instance_path();
    match UnixListener::bind(&path) {
        Ok(listener) => Some(InstanceGuard {
            _listener: listener,
            path,
        }),
        Err(error) if error.kind() == io::ErrorKind::AddrInUse => {
            // A successful connection means another TabletFlow instance owns
            // the socket. If nobody accepts it, the pathname is stale from a
            // crash and can be safely reclaimed.
            if UnixStream::connect(&path).is_ok() {
                return None;
            }
            let _ = fs::remove_file(&path);
            UnixListener::bind(&path)
                .ok()
                .map(|listener| InstanceGuard {
                    _listener: listener,
                    path,
                })
        }
        Err(error) => {
            eprintln!("TabletFlow instance guard unavailable: {error}");
            None
        }
    }
}

pub(super) fn connect_pipe() -> io::Result<UnixStream> {
    let stream = UnixStream::connect(ipc_path())?;
    stream.set_write_timeout(Some(std::time::Duration::from_secs(15)))?;
    Ok(stream)
}

pub(super) fn pipe_is_available() -> bool {
    UnixStream::connect(ipc_path()).is_ok()
}

fn daemon_candidates() -> Vec<PathBuf> {
    let mut candidates = Vec::new();
    let runtime_id = match (std::env::consts::OS, std::env::consts::ARCH) {
        ("macos", "aarch64") => Some("osx-arm64"),
        ("macos", "x86_64") => Some("osx-x64"),
        ("linux", "aarch64") => Some("linux-arm64"),
        ("linux", "x86_64") => Some("linux-x64"),
        ("linux", "x86") => Some("linux-x86"),
        _ => None,
    };
    if let Some(runtime_id) = runtime_id {
        let directory = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("target/otd")
            .join(runtime_id);
        candidates.push(directory.join("OpenTabletDriver.Daemon"));
        candidates.push(directory.join("OpenTabletDriver.Daemon.dll"));
    }

    if let Ok(executable) = std::env::current_exe() {
        if let Some(directory) = executable.parent() {
            candidates.push(directory.join("OpenTabletDriver.Daemon"));
            candidates.push(directory.join("OpenTabletDriver.Daemon.dll"));
            candidates.push(directory.join("resources/OpenTabletDriver.Daemon"));
            candidates.push(directory.join("resources/OpenTabletDriver.Daemon.dll"));
            if let Some(contents) = directory.parent() {
                candidates.push(contents.join("Resources/OpenTabletDriver.Daemon"));
                candidates.push(contents.join("Resources/OpenTabletDriver.Daemon.dll"));
            }
        }
    }

    if let Some(resource_root) = std::env::var_os("TABLETFLOW_RESOURCE_DIR") {
        let resource_root = PathBuf::from(resource_root);
        candidates.push(resource_root.join("OpenTabletDriver.Daemon"));
        candidates.push(resource_root.join("OpenTabletDriver.Daemon.dll"));
    }
    candidates
}

fn embedded_daemon_path() -> Option<PathBuf> {
    daemon_candidates()
        .into_iter()
        .find(|candidate| candidate.is_file())
}

fn process_ids_for_path(path: &Path) -> Vec<u32> {
    Command::new("pgrep")
        .args(["-f", path.to_string_lossy().as_ref()])
        .output()
        .map(|output| {
            String::from_utf8_lossy(&output.stdout)
                .lines()
                .filter_map(|line| line.trim().parse::<u32>().ok())
                .collect()
        })
        .unwrap_or_default()
}

fn any_daemon_process_exists() -> bool {
    Command::new("pgrep")
        .args(["-f", "OpenTabletDriver.Daemon"])
        .output()
        .is_ok_and(|output| output.status.success())
}

pub(super) fn owned_daemon_is_running() -> bool {
    let Ok(mut slot) = managed_daemon().lock() else {
        return false;
    };
    let Some(child) = slot.as_mut() else {
        return false;
    };
    match child.try_wait() {
        Ok(None) => true,
        Ok(Some(_)) | Err(_) => {
            *slot = None;
            false
        }
    }
}

pub(super) fn daemon_is_running() -> bool {
    owned_daemon_is_running() || pipe_is_available()
}

pub(super) fn start_daemon() -> io::Result<()> {
    if owned_daemon_is_running() || pipe_is_available() {
        return Ok(());
    }
    let path = embedded_daemon_path().ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::NotFound,
            "OpenTabletDriver.Daemon is missing from the TabletFlow package",
        )
    })?;

    // Do not kill or duplicate a daemon outside this process. The official OTD
    // UI/system service may own it and simply still be initializing its pipe.
    if any_daemon_process_exists() || !process_ids_for_path(&path).is_empty() {
        return Ok(());
    }

    let mut command = if path.extension().is_some_and(|extension| extension == "dll") {
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
    let mut slot = managed_daemon()
        .lock()
        .map_err(|_| io::Error::other("daemon process lock is poisoned"))?;
    *slot = Some(child);
    Ok(())
}

pub(super) fn stop_daemon() {
    let Ok(mut slot) = managed_daemon().lock() else {
        return;
    };
    let Some(mut child) = slot.take() else {
        return;
    };
    let _ = child.kill();
    let _ = child.wait();
}

/// Headless package smoke test used by release CI. It verifies that the
/// packaged daemon can start and create the same IPC endpoint TabletFlow uses.
pub(super) fn run_self_test() -> io::Result<()> {
    let was_available = pipe_is_available();
    let owned_before = owned_daemon_is_running();
    if !was_available {
        start_daemon()?;
    }
    let started_owned = !owned_before && owned_daemon_is_running();
    let deadline = Instant::now() + Duration::from_secs(60);
    let result = loop {
        if pipe_is_available() {
            break Ok(());
        }
        if started_owned && !owned_daemon_is_running() {
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
        stop_daemon();
    }
    result
}
