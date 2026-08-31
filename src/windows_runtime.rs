//! Windows-only process and operating-system integration.
//!
//! OpenTabletDriver is a console application.  Launching it (or helper tools such
//! as PowerShell/taskkill) from a GUI application without Windows creation flags
//! produces the flashing console windows that users were seeing.  This module
//! deliberately uses native Windows APIs and owns the daemon as a child process;
//! macOS and Linux never compile this code.

use std::ffi::OsStr;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::os::windows::ffi::OsStrExt;
use std::os::windows::io::AsRawHandle;
use std::os::windows::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::ptr::{null, null_mut};
use std::sync::{Mutex, OnceLock};
use std::thread::JoinHandle;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde_json::Value;
use windows_sys::Win32::Foundation::{
    CloseHandle, GetLastError, ERROR_ALREADY_EXISTS, ERROR_FILE_NOT_FOUND, ERROR_SEM_TIMEOUT,
    ERROR_SUCCESS, HANDLE, INVALID_HANDLE_VALUE,
};
use windows_sys::Win32::Storage::FileSystem::{
    MoveFileExW, MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH,
};
use windows_sys::Win32::System::Diagnostics::ToolHelp::{
    CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W, TH32CS_SNAPPROCESS,
};
use windows_sys::Win32::System::JobObjects::{
    AssignProcessToJobObject, CreateJobObjectW, JobObjectExtendedLimitInformation,
    SetInformationJobObject, JOBOBJECT_EXTENDED_LIMIT_INFORMATION,
    JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
};
use windows_sys::Win32::System::Pipes::WaitNamedPipeW;
use windows_sys::Win32::System::Registry::{
    RegCloseKey, RegCreateKeyExW, RegDeleteValueW, RegSetValueExW, HKEY, HKEY_CURRENT_USER,
    KEY_SET_VALUE, REG_OPTION_NON_VOLATILE, REG_SZ,
};
use windows_sys::Win32::System::Threading::{
    CreateMutexW, OpenProcess, QueryFullProcessImageNameW, TerminateProcess, WaitForSingleObject,
    CREATE_NO_WINDOW, PROCESS_QUERY_LIMITED_INFORMATION, PROCESS_SYNCHRONIZE, PROCESS_TERMINATE,
};
use windows_sys::Win32::System::IO::CancelSynchronousIo;
use windows_sys::Win32::UI::HiDpi::{SetProcessDpiAwareness, PROCESS_PER_MONITOR_DPI_AWARE};
use windows_sys::Win32::UI::Shell::ShellExecuteW;
use windows_sys::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;

const PIPE_PATH: &str = r"\\.\pipe\OpenTabletDriver.Daemon";
const INSTANCE_MUTEX_NAME: &str = r"Local\TabletFlow.Application.v2";
const AUTOSTART_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";
const AUTOSTART_VALUE: &str = "TabletFlow";

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

static MANAGED_DAEMON: OnceLock<Mutex<Option<ManagedDaemon>>> = OnceLock::new();

pub(super) struct InstanceGuard {
    handle: KernelHandle,
}

struct ManagedDaemon {
    child: Child,
    _job: Option<KernelHandle>,
    path: PathBuf,
}

/// A uniquely owned Win32 handle. Storing the pointer value as `isize` keeps the
/// wrapper Send, while Drop still closes exactly the handle that we own.
struct KernelHandle(isize);

impl KernelHandle {
    fn new(handle: HANDLE) -> Option<Self> {
        (!handle.is_null() && handle != INVALID_HANDLE_VALUE).then_some(Self(handle as isize))
    }

    fn raw(&self) -> HANDLE {
        self.0 as HANDLE
    }
}

impl Drop for KernelHandle {
    fn drop(&mut self) {
        // SAFETY: KernelHandle is created only for an owned Win32 handle and is
        // never cloned, so this is the single matching CloseHandle call.
        unsafe {
            CloseHandle(self.raw());
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum StartResult {
    AlreadyConnected,
    AlreadyStarting,
    Started,
}

fn managed_daemon() -> &'static Mutex<Option<ManagedDaemon>> {
    MANAGED_DAEMON.get_or_init(|| Mutex::new(None))
}

fn wide(value: impl AsRef<OsStr>) -> Vec<u16> {
    value
        .as_ref()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect()
}

fn local_data_root() -> PathBuf {
    std::env::var_os("LOCALAPPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir)
        .join("TabletFlow")
}

pub(super) fn supervisor_log_path() -> PathBuf {
    local_data_root().join("Logs/tabletflow-windows.log")
}

fn daemon_console_log_path() -> PathBuf {
    local_data_root().join("Logs/opentabletdriver-console.log")
}

fn driver_settings_path() -> PathBuf {
    local_data_root().join("OpenTabletDriver/settings.json")
}

pub(super) fn log_line(message: impl AsRef<str>) {
    let path = supervisor_log_path();
    let Some(parent) = path.parent() else {
        return;
    };
    if fs::create_dir_all(parent).is_err() {
        return;
    }
    let Ok(mut log) = OpenOptions::new().create(true).append(true).open(path) else {
        return;
    };
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or_default();
    let _ = writeln!(log, "[{timestamp}] {}", message.as_ref());
}

/// Match OpenTabletDriver's per-monitor coordinate system before any window or
/// monitor enumeration is created. Windows otherwise virtualizes coordinates
/// for a DPI-unaware process.
pub(super) fn initialize_process() {
    let _ = unsafe { SetProcessDpiAwareness(PROCESS_PER_MONITOR_DPI_AWARE) };
}

pub(super) fn acquire_instance_guard() -> Option<InstanceGuard> {
    let name = wide(INSTANCE_MUTEX_NAME);
    // SAFETY: pointers are valid for the duration of the call; a null security
    // descriptor requests the current user's default ACL.
    let handle = unsafe { CreateMutexW(null(), 1, name.as_ptr()) };
    let handle = KernelHandle::new(handle)?;
    // GetLastError must be read immediately after CreateMutexW.
    if unsafe { GetLastError() } == ERROR_ALREADY_EXISTS {
        return None;
    }
    Some(InstanceGuard { handle })
}

pub(super) fn connect_pipe() -> io::Result<File> {
    let pipe = wide(PIPE_PATH);
    // Avoid racing CreateFile against the daemon while it is creating its pipe.
    // This wait is bounded and happens on the backend worker, never the UI thread.
    if unsafe { WaitNamedPipeW(pipe.as_ptr(), 250) } == 0 {
        return Err(io::Error::last_os_error());
    }
    OpenOptions::new().read(true).write(true).open(PIPE_PATH)
}

/// Interrupt a reader blocked in a synchronous named-pipe read. The caller
/// must join the thread after this returns. Retrying handles the race where
/// the thread has checked its flag but has not issued ReadFile yet.
pub(super) fn cancel_reader(reader: &JoinHandle<()>) {
    let handle = reader.as_raw_handle();
    while !reader.is_finished() {
        // SAFETY: the JoinHandle owns a live thread handle for the duration of
        // this call, and CancelSynchronousIo only targets that thread's I/O.
        unsafe {
            let _ = CancelSynchronousIo(handle);
        }
        std::thread::yield_now();
    }
}

pub(super) fn pipe_is_available() -> bool {
    let pipe = wide(PIPE_PATH);
    if unsafe { WaitNamedPipeW(pipe.as_ptr(), 0) } != 0 {
        return true;
    }
    // ERROR_SEM_TIMEOUT means that the pipe exists but all instances are busy.
    // The daemon is still alive, so it must not be killed or duplicated.
    (unsafe { GetLastError() }) == ERROR_SEM_TIMEOUT
}

fn daemon_candidates() -> Vec<PathBuf> {
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
    let developer_runtime = match std::env::consts::ARCH {
        "aarch64" => "win-arm64",
        "x86_64" => "win-x64",
        "x86" => "win-x86",
        _ => "win-x64",
    };
    candidates.push(
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("target/otd")
            .join(developer_runtime)
            .join("OpenTabletDriver.Daemon.exe"),
    );
    candidates
}

pub(super) fn validate_embedded_daemon_bundle() -> io::Result<PathBuf> {
    let daemon = daemon_candidates()
        .into_iter()
        .find(|candidate| candidate.is_file())
        .ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::NotFound,
                "OpenTabletDriver.Daemon.exe is missing from the TabletFlow directory",
            )
        })?;
    validate_daemon_bundle(&daemon)?;
    Ok(daemon)
}

fn validate_daemon_bundle(daemon: &Path) -> io::Result<()> {
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
        Ok(())
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

fn daemon_process_exists() -> bool {
    let snapshot = unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) };
    let Some(snapshot) = KernelHandle::new(snapshot) else {
        return false;
    };

    let mut entry = PROCESSENTRY32W::default();
    entry.dwSize = std::mem::size_of::<PROCESSENTRY32W>() as u32;
    let mut has_entry = unsafe { Process32FirstW(snapshot.raw(), &mut entry) } != 0;
    while has_entry {
        let end = entry
            .szExeFile
            .iter()
            .position(|character| *character == 0)
            .unwrap_or(entry.szExeFile.len());
        if String::from_utf16_lossy(&entry.szExeFile[..end])
            .eq_ignore_ascii_case("OpenTabletDriver.Daemon.exe")
        {
            return true;
        }
        has_entry = unsafe { Process32NextW(snapshot.raw(), &mut entry) } != 0;
    }
    false
}

/// Remove only a stale daemon launched from this exact package path. A healthy
/// external OpenTabletDriver instance is detected by its pipe and never reaches
/// this function.
fn terminate_stale_packaged_daemons(daemon_path: &Path) {
    let target = normalized_path(daemon_path);
    let snapshot = unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) };
    let Some(snapshot) = KernelHandle::new(snapshot) else {
        return;
    };
    let mut entry = PROCESSENTRY32W::default();
    entry.dwSize = std::mem::size_of::<PROCESSENTRY32W>() as u32;
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
                log_line(format!(
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

fn reap_finished_daemon(slot: &mut Option<ManagedDaemon>) -> bool {
    let Some(daemon) = slot.as_mut() else {
        return false;
    };
    match daemon.child.try_wait() {
        Ok(None) => true,
        Ok(Some(status)) => {
            log_line(format!(
                "packaged daemon exited status={status} path={}",
                daemon.path.display()
            ));
            *slot = None;
            false
        }
        Err(error) => {
            log_line(format!("failed to query packaged daemon: {error}"));
            *slot = None;
            false
        }
    }
}

fn attach_kill_on_close_job(child: &Child) -> Option<KernelHandle> {
    let job = unsafe { CreateJobObjectW(null(), null()) };
    let job = KernelHandle::new(job)?;
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
        log_line(format!(
            "unable to configure daemon job object: {}",
            io::Error::last_os_error()
        ));
        return None;
    }
    let process = child.as_raw_handle() as HANDLE;
    if unsafe { AssignProcessToJobObject(job.raw(), process) } == 0 {
        // Some launchers already place TabletFlow in a restrictive job. Explicit
        // Child::kill on normal shutdown remains the fallback in that case.
        log_line(format!(
            "unable to assign daemon to job object: {}",
            io::Error::last_os_error()
        ));
        return None;
    }
    Some(job)
}

pub(super) fn start_daemon() -> io::Result<StartResult> {
    if pipe_is_available() {
        return Ok(StartResult::AlreadyConnected);
    }

    let mut slot = managed_daemon()
        .lock()
        .map_err(|_| io::Error::other("daemon process lock is poisoned"))?;
    if reap_finished_daemon(&mut slot) {
        return Ok(StartResult::AlreadyStarting);
    }
    // A daemon started by the official OpenTabletDriver UI or another
    // installation may take a long time to expose its pipe while it scans HID.
    // Attach to it once ready instead of starting a second process that exits
    // on OTD's daemon mutex.
    if daemon_process_exists() {
        return Ok(StartResult::AlreadyStarting);
    }

    let daemon_path = validate_embedded_daemon_bundle()?;
    terminate_stale_packaged_daemons(&daemon_path);

    let log_path = daemon_console_log_path();
    if let Some(parent) = log_path.parent() {
        fs::create_dir_all(parent)?;
    }
    let stdout = OpenOptions::new()
        .create(true)
        .append(true)
        .open(&log_path)?;
    let stderr = stdout.try_clone()?;
    let app_data = local_data_root().join("OpenTabletDriver");
    fs::create_dir_all(&app_data)?;

    let daemon_directory = daemon_path.parent().ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            "daemon path has no parent directory",
        )
    })?;
    let mut command = Command::new(&daemon_path);
    command
        .arg("--appdata")
        .arg(&app_data)
        .current_dir(daemon_directory)
        .stdin(Stdio::null())
        .stdout(Stdio::from(stdout))
        .stderr(Stdio::from(stderr))
        .creation_flags(CREATE_NO_WINDOW);

    log_line(format!(
        "starting packaged daemon path={}",
        daemon_path.display()
    ));
    let child = command.spawn()?;
    let job = attach_kill_on_close_job(&child);
    *slot = Some(ManagedDaemon {
        child,
        _job: job,
        path: daemon_path,
    });
    Ok(StartResult::Started)
}

pub(super) fn daemon_is_running() -> bool {
    if pipe_is_available() {
        return true;
    }
    managed_daemon()
        .lock()
        .map(|mut slot| reap_finished_daemon(&mut slot))
        .unwrap_or(false)
        || daemon_process_exists()
}

pub(super) fn owned_daemon_is_running() -> bool {
    managed_daemon()
        .lock()
        .map(|mut slot| reap_finished_daemon(&mut slot))
        .unwrap_or(false)
}

pub(super) fn stop_daemon() {
    let Ok(mut slot) = managed_daemon().lock() else {
        return;
    };
    let Some(mut daemon) = slot.take() else {
        return;
    };
    log_line(format!(
        "stopping packaged daemon pid={}",
        daemon.child.id()
    ));
    let _ = daemon.child.kill();
    let _ = daemon.child.wait();
}

/// Validate and exercise the packaged daemon without creating the Slint UI.
/// This is used by the release smoke test and is intentionally Windows-only.
pub(super) fn run_self_test() -> io::Result<()> {
    let daemon_path = validate_embedded_daemon_bundle()?;
    let pipe_was_ready = pipe_is_available();
    let started_result = if pipe_was_ready {
        None
    } else {
        Some(start_daemon()?)
    };
    let started_owned = started_result == Some(StartResult::Started);

    let deadline = Instant::now() + Duration::from_secs(180);
    let result = loop {
        if pipe_is_available() {
            // Exercise the same RPC contract used by the frontend. This catches
            // missing managed dependencies and initialization failures that
            // would still leave a named pipe present.
            let (backend_events, _) = std::sync::mpsc::channel();
            let mut client = match super::DaemonClient::connect(backend_events, 0) {
                Ok(client) => client,
                Err(error) => break Err(error),
            };
            break client
                .call("GetTablets", serde_json::json!([]))
                .and_then(|tablets| {
                    if !tablets.is_array() {
                        return Err(io::Error::new(
                            io::ErrorKind::InvalidData,
                            "GetTablets returned a non-array result",
                        ));
                    }
                    client.call("GetSettings", serde_json::json!([]))
                })
                .and_then(|settings| {
                    if settings.is_null() {
                        return Err(io::Error::new(
                            io::ErrorKind::InvalidData,
                            "GetSettings returned null",
                        ));
                    }
                    Ok(())
                });
        }
        if Instant::now() >= deadline {
            break Err(io::Error::new(
                io::ErrorKind::TimedOut,
                "OpenTabletDriver did not create its named pipe",
            ));
        }
        if started_owned && !owned_daemon_is_running() {
            break Err(io::Error::new(
                io::ErrorKind::Other,
                "OpenTabletDriver exited before creating its named pipe",
            ));
        }
        std::thread::sleep(Duration::from_millis(100));
    };

    // Never terminate a daemon that was already running before the test.
    if started_owned {
        stop_daemon();
    }
    if result.is_ok() {
        log_line(format!(
            "Windows package self-test passed daemon={}",
            daemon_path.display()
        ));
    } else if let Err(error) = &result {
        log_line(format!("Windows package self-test failed: {error}"));
    }
    result
}

pub(super) fn replace_file(source: &Path, destination: &Path) -> io::Result<()> {
    let source = wide(source.as_os_str());
    let destination = wide(destination.as_os_str());
    let replaced = unsafe {
        MoveFileExW(
            source.as_ptr(),
            destination.as_ptr(),
            MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
        )
    };
    if replaced == 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(())
    }
}

pub(super) fn persist_driver_settings(settings: &Value) -> io::Result<()> {
    let path = driver_settings_path();
    let parent = path.parent().ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            "driver settings path has no parent",
        )
    })?;
    fs::create_dir_all(parent)?;
    let temporary = path.with_extension("json.tmp");
    let serialized = serde_json::to_vec_pretty(settings).map_err(io::Error::other)?;
    fs::write(&temporary, serialized)?;
    replace_file(&temporary, &path)
}

pub(super) fn configure_autostart(enabled: bool, start_minimized: bool) -> io::Result<()> {
    let key_name = wide(AUTOSTART_KEY);
    let mut key: HKEY = null_mut();
    let mut disposition = 0;
    let result = unsafe {
        RegCreateKeyExW(
            HKEY_CURRENT_USER,
            key_name.as_ptr(),
            0,
            null(),
            REG_OPTION_NON_VOLATILE,
            KEY_SET_VALUE,
            null(),
            &mut key,
            &mut disposition,
        )
    };
    if result != ERROR_SUCCESS {
        return Err(io::Error::from_raw_os_error(result as i32));
    }

    let value_name = wide(AUTOSTART_VALUE);
    let operation = if enabled {
        let executable = std::env::current_exe()?;
        let mut command = format!("\"{}\"", executable.display());
        if start_minimized {
            command.push_str(" --start-minimized");
        }
        let command = wide(command);
        unsafe {
            RegSetValueExW(
                key,
                value_name.as_ptr(),
                0,
                REG_SZ,
                command.as_ptr().cast(),
                (command.len() * std::mem::size_of::<u16>()) as u32,
            )
        }
    } else {
        let result = unsafe { RegDeleteValueW(key, value_name.as_ptr()) };
        if result == ERROR_FILE_NOT_FOUND {
            ERROR_SUCCESS
        } else {
            result
        }
    };
    unsafe {
        RegCloseKey(key);
    }
    if operation == ERROR_SUCCESS {
        Ok(())
    } else {
        Err(io::Error::from_raw_os_error(operation as i32))
    }
}

pub(super) fn open_url(url: &str) -> io::Result<()> {
    let operation = wide("open");
    let url = wide(url);
    let result = unsafe {
        ShellExecuteW(
            null_mut(),
            operation.as_ptr(),
            url.as_ptr(),
            null(),
            null(),
            SW_SHOWNORMAL,
        )
    };
    if result as isize > 32 {
        Ok(())
    } else {
        Err(io::Error::other(format!(
            "ShellExecuteW failed with code {}",
            result as isize
        )))
    }
}
