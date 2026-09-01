use std::fs::{self, OpenOptions};
use std::io;
use std::os::windows::io::AsRawHandle;
use std::os::windows::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::ptr::null;
use std::sync::{Mutex, OnceLock};

use windows_sys::Win32::Foundation::HANDLE;
use windows_sys::Win32::System::Diagnostics::ToolHelp::{
    CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W, TH32CS_SNAPPROCESS,
};
use windows_sys::Win32::System::JobObjects::{
    AssignProcessToJobObject, CreateJobObjectW, JobObjectExtendedLimitInformation,
    SetInformationJobObject, JOBOBJECT_EXTENDED_LIMIT_INFORMATION,
    JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
};
use windows_sys::Win32::System::Threading::{
    OpenProcess, QueryFullProcessImageNameW, TerminateProcess, WaitForSingleObject,
    CREATE_NO_WINDOW, PROCESS_QUERY_LIMITED_INFORMATION, PROCESS_SYNCHRONIZE, PROCESS_TERMINATE,
};

use super::{ipc, runtime};
use runtime::KernelHandle;

const REQUIRED_DAEMON_FILES: &[&str] = &[
    "OpenTabletDriver.Daemon.exe",
    "OpenTabletDriver.Daemon.dll",
    "OpenTabletDriver.Daemon.deps.json",
    "OpenTabletDriver.Daemon.runtimeconfig.json",
    "OpenTabletDriver.dll",
    "OpenTabletDriver.Configurations.dll",
    "OpenTabletDriver.Desktop.dll",
    "OpenTabletDriver.Native.dll",
    "OpenTabletDriver.Plugin.dll",
    "HidSharpCore.dll",
    "hostfxr.dll",
    "hostpolicy.dll",
    "coreclr.dll",
];

static MANAGED: OnceLock<Mutex<Option<ManagedDaemon>>> = OnceLock::new();

struct ManagedDaemon {
    child: Child,
    _job: Option<KernelHandle>,
    path: PathBuf,
}

fn managed() -> &'static Mutex<Option<ManagedDaemon>> {
    MANAGED.get_or_init(|| Mutex::new(None))
}

fn candidates() -> Vec<PathBuf> {
    let mut candidates = Vec::new();
    if let Some(path) = std::env::var_os("TABLETFLOW_DAEMON_PATH") {
        candidates.push(PathBuf::from(path));
    }
    if let Ok(executable) = std::env::current_exe() {
        if let Some(directory) = executable.parent() {
            candidates.push(directory.join("OpenTabletDriver.Daemon.exe"));
            candidates.push(directory.join("resources/OpenTabletDriver.Daemon.exe"));
        }
    }
    if let Some(resource_root) = std::env::var_os("TABLETFLOW_RESOURCE_DIR") {
        candidates.push(PathBuf::from(resource_root).join("OpenTabletDriver.Daemon.exe"));
    }
    let runtime = match std::env::consts::ARCH {
        "aarch64" => "win-arm64",
        "x86_64" => "win-x64",
        "x86" => "win-x86",
        _ => "win-x64",
    };
    candidates.push(
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("target/otd")
            .join(runtime)
            .join("OpenTabletDriver.Daemon.exe"),
    );
    candidates
}

pub(super) fn validate_bundle() -> io::Result<PathBuf> {
    let daemon = candidates()
        .into_iter()
        .find(|candidate| candidate.is_file())
        .ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::NotFound,
                "OpenTabletDriver.Daemon.exe is missing from the TabletFlow directory",
            )
        })?;
    let directory = daemon.parent().ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            "daemon path has no parent directory",
        )
    })?;
    let missing = REQUIRED_DAEMON_FILES
        .iter()
        .filter(|name| !directory.join(name).is_file())
        .copied()
        .collect::<Vec<_>>();
    if missing.is_empty() {
        Ok(daemon)
    } else {
        Err(io::Error::new(
            io::ErrorKind::NotFound,
            format!(
                "incomplete OpenTabletDriver runtime; missing: {}",
                missing.join(", ")
            ),
        ))
    }
}

fn normalized_path(path: &Path) -> String {
    fs::canonicalize(path)
        .unwrap_or_else(|_| path.to_path_buf())
        .to_string_lossy()
        .trim_start_matches(r"\\?\")
        .replace('/', r"\")
        .to_lowercase()
}

fn executable_path(process: HANDLE) -> Option<PathBuf> {
    let mut buffer = vec![0u16; 32_768];
    let mut length = buffer.len() as u32;
    if unsafe { QueryFullProcessImageNameW(process, 0, buffer.as_mut_ptr(), &mut length) } == 0 {
        return None;
    }
    buffer.truncate(length as usize);
    Some(PathBuf::from(String::from_utf16_lossy(&buffer)))
}

fn terminate_stale_packaged_daemons(daemon_path: &Path) {
    let target = normalized_path(daemon_path);
    let snapshot = unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) };
    let Some(snapshot) = KernelHandle::new(snapshot) else {
        return;
    };
    let mut entry = PROCESSENTRY32W {
        dwSize: std::mem::size_of::<PROCESSENTRY32W>() as u32,
        ..Default::default()
    };
    let mut has_entry = unsafe { Process32FirstW(snapshot.raw(), &mut entry) } != 0;
    while has_entry {
        let process = unsafe {
            OpenProcess(
                PROCESS_QUERY_LIMITED_INFORMATION | PROCESS_TERMINATE | PROCESS_SYNCHRONIZE,
                0,
                entry.th32ProcessID,
            )
        };
        if let Some(process) = KernelHandle::new(process) {
            let matches = executable_path(process.raw())
                .map(|path| normalized_path(&path) == target)
                .unwrap_or(false);
            if matches {
                runtime::log_line(format!(
                    "terminating stale packaged daemon pid={}",
                    entry.th32ProcessID
                ));
                unsafe {
                    TerminateProcess(process.raw(), 1);
                    WaitForSingleObject(process.raw(), 2_000);
                }
            }
        }
        has_entry = unsafe { Process32NextW(snapshot.raw(), &mut entry) } != 0;
    }
}

fn reap_finished(slot: &mut Option<ManagedDaemon>) -> bool {
    let Some(daemon) = slot.as_mut() else {
        return false;
    };
    match daemon.child.try_wait() {
        Ok(None) => true,
        Ok(Some(status)) => {
            runtime::log_line(format!(
                "packaged daemon exited status={status} path={}",
                daemon.path.display()
            ));
            *slot = None;
            false
        }
        Err(error) => {
            runtime::log_line(format!("failed to query packaged daemon: {error}"));
            *slot = None;
            false
        }
    }
}

fn attach_job(child: &Child) -> Option<KernelHandle> {
    let job = KernelHandle::new(unsafe { CreateJobObjectW(null(), null()) })?;
    let mut information = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
    information.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
    let configured = unsafe {
        SetInformationJobObject(
            job.raw(),
            JobObjectExtendedLimitInformation,
            (&information as *const JOBOBJECT_EXTENDED_LIMIT_INFORMATION).cast(),
            std::mem::size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
        )
    } != 0;
    if !configured {
        runtime::log_line(format!(
            "unable to configure daemon job object: {}",
            io::Error::last_os_error()
        ));
        return None;
    }
    let process = child.as_raw_handle() as HANDLE;
    if unsafe { AssignProcessToJobObject(job.raw(), process) } == 0 {
        runtime::log_line(format!(
            "unable to assign daemon to job object: {}",
            io::Error::last_os_error()
        ));
        return None;
    }
    Some(job)
}

pub(super) fn owned_is_running() -> bool {
    managed()
        .lock()
        .map(|mut slot| reap_finished(&mut slot))
        .unwrap_or(false)
}

pub(super) fn start() -> io::Result<()> {
    if ipc::is_available() || owned_is_running() {
        return Ok(());
    }
    let daemon_path = validate_bundle()?;
    let mut slot = managed()
        .lock()
        .map_err(|_| io::Error::other("daemon process lock is poisoned"))?;
    if reap_finished(&mut slot) {
        return Ok(());
    }

    // Only an orphan from TabletFlow's exact packaged path is terminated.
    // Arbitrary external OTD processes are never used as a readiness signal and
    // are never killed; IPC availability is the only external-ready condition.
    terminate_stale_packaged_daemons(&daemon_path);

    let log_path = runtime::local_data_root().join("Logs/opentabletdriver-console.log");
    if let Some(parent) = log_path.parent() {
        fs::create_dir_all(parent)?;
    }
    let stdout = OpenOptions::new()
        .create(true)
        .append(true)
        .open(&log_path)?;
    let stderr = stdout.try_clone()?;
    let app_data = runtime::local_data_root().join("OpenTabletDriver");
    fs::create_dir_all(&app_data)?;
    let directory = daemon_path.parent().ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            "daemon path has no parent directory",
        )
    })?;
    let mut command = Command::new(&daemon_path);
    command
        .arg("--appdata")
        .arg(&app_data)
        .current_dir(directory)
        .stdin(Stdio::null())
        .stdout(Stdio::from(stdout))
        .stderr(Stdio::from(stderr))
        .creation_flags(CREATE_NO_WINDOW);
    runtime::log_line(format!(
        "starting packaged daemon path={}",
        daemon_path.display()
    ));
    let child = command.spawn()?;
    let job = attach_job(&child);
    *slot = Some(ManagedDaemon {
        child,
        _job: job,
        path: daemon_path,
    });
    Ok(())
}

pub(super) fn stop() {
    let Ok(mut slot) = managed().lock() else {
        return;
    };
    let Some(mut daemon) = slot.take() else {
        return;
    };
    runtime::log_line(format!(
        "stopping packaged daemon pid={}",
        daemon.child.id()
    ));
    let _ = daemon.child.kill();
    let _ = daemon.child.wait();
}
