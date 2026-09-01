use super::{ipc, runtime};
use std::fs::{self, OpenOptions};
use std::io;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

struct ManagedDaemon {
    child: Child,
    path: PathBuf,
}

static MANAGED: OnceLock<Mutex<Option<ManagedDaemon>>> = OnceLock::new();
fn managed() -> &'static Mutex<Option<ManagedDaemon>> {
    MANAGED.get_or_init(|| Mutex::new(None))
}

fn candidates() -> Vec<PathBuf> {
    // Packaged/current-exe-relative locations are checked before the
    // target/otd dev-build fallback. CARGO_MANIFEST_DIR is baked in at
    // compile time: on a machine that both built and packaged the app (the
    // common local dev workflow), that path still exists at runtime, so if
    // it were checked first a packaged TabletFlow.app would silently launch
    // the developer's target/otd daemon instead of the one bundled inside
    // its own .app - a different build, potentially a different version.
    let mut candidates = Vec::new();
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
            if let Some(contents) = directory.parent() {
                candidates.push(contents.join("Resources/OpenTabletDriver.Daemon"));
                candidates.push(contents.join("Resources/OpenTabletDriver.Daemon.dll"));
            }
        }
    }
    if let Some(root) = std::env::var_os("TABLETFLOW_RESOURCE_DIR") {
        candidates.push(PathBuf::from(&root).join("OpenTabletDriver.Daemon"));
        candidates.push(PathBuf::from(root).join("OpenTabletDriver.Daemon.dll"));
    }
    let runtime_id = match std::env::consts::ARCH {
        "aarch64" => "osx-arm64",
        _ => "osx-x64",
    };
    let target = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("target/otd")
        .join(runtime_id);
    candidates.push(target.join("OpenTabletDriver.Daemon"));
    candidates.push(target.join("OpenTabletDriver.Daemon.dll"));
    candidates
}

fn daemon_path() -> Option<PathBuf> {
    candidates().into_iter().find(|path| path.is_file())
}

pub fn owned_is_running() -> bool {
    let Ok(mut slot) = managed().lock() else {
        return false;
    };
    let Some(daemon) = slot.as_mut() else {
        return false;
    };
    match daemon.child.try_wait() {
        Ok(None) => true,
        Ok(Some(status)) => {
            eprintln!(
                "TabletFlow: packaged daemon exited status={status} path={}",
                daemon.path.display()
            );
            *slot = None;
            false
        }
        Err(error) => {
            eprintln!("TabletFlow: failed to query packaged daemon: {error}");
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
    let app_data = runtime::otd_appdata_dir();
    fs::create_dir_all(&app_data)?;
    let log_dir = app_data
        .parent()
        .map(|parent| parent.join("Logs"))
        .unwrap_or_else(|| app_data.join("Logs"));
    fs::create_dir_all(&log_dir)?;
    let log_path = log_dir.join("opentabletdriver-console.log");
    let stdout = OpenOptions::new()
        .create(true)
        .append(true)
        .open(&log_path)?;
    let stderr = stdout.try_clone()?;
    command
        .arg("--appdata")
        .arg(&app_data)
        .stdin(Stdio::null())
        .stdout(Stdio::from(stdout))
        .stderr(Stdio::from(stderr));
    if let Some(directory) = path.parent() {
        command.current_dir(directory);
    }
    eprintln!(
        "TabletFlow: starting packaged daemon path={} log={}",
        path.display(),
        log_path.display()
    );
    let child = command.spawn()?;
    eprintln!("TabletFlow: packaged daemon pid={}", child.id());
    *managed()
        .lock()
        .map_err(|_| io::Error::other("daemon process lock is poisoned"))? =
        Some(ManagedDaemon { child, path });
    Ok(())
}

pub fn stop() {
    let Ok(mut slot) = managed().lock() else {
        return;
    };
    if let Some(mut daemon) = slot.take() {
        eprintln!(
            "TabletFlow: stopping packaged daemon pid={}",
            daemon.child.id()
        );
        let _ = daemon.child.kill();
        let _ = daemon.child.wait();
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
