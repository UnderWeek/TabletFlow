use serde_json::Value;
use std::fs;
use std::io;
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::PathBuf;
use std::process::Command;

pub struct InstanceGuard {
    _listener: UnixListener,
    path: PathBuf,
}
impl Drop for InstanceGuard {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
    }
}

fn uid() -> String {
    Command::new("id")
        .arg("-u")
        .output()
        .ok()
        .and_then(|output| String::from_utf8(output.stdout).ok())
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| "unknown".into())
}

pub fn acquire_instance_guard() -> Option<InstanceGuard> {
    let path = std::env::temp_dir().join(format!("tabletflow-{}.sock", uid()));
    match UnixListener::bind(&path) {
        Ok(listener) => Some(InstanceGuard {
            _listener: listener,
            path,
        }),
        Err(error) if error.kind() == io::ErrorKind::AddrInUse => {
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

pub fn open_url(url: &str) -> io::Result<()> {
    Command::new("xdg-open").arg(url).spawn().map(|_| ())
}

fn local_data_root_from(xdg_data_home: Option<PathBuf>, home: Option<PathBuf>) -> PathBuf {
    xdg_data_home
        .or_else(|| home.map(|home| home.join(".local/share")))
        .unwrap_or_else(std::env::temp_dir)
        .join("TabletFlow")
}

fn local_data_root() -> PathBuf {
    local_data_root_from(
        std::env::var_os("XDG_DATA_HOME").map(PathBuf::from),
        std::env::var_os("HOME").map(PathBuf::from),
    )
}

/// The `--appdata` directory TabletFlow passes to the owned OpenTabletDriver
/// daemon it spawns (see `daemon::start`). `persist_driver_settings` writes
/// to exactly this directory's `settings.json` so the daemon TabletFlow
/// launches always reads back the same file TabletFlow just wrote - there is
/// no separate "TabletFlow's path" vs "the daemon's path".
pub fn otd_appdata_dir() -> PathBuf {
    local_data_root().join("OpenTabletDriver")
}

pub fn persist_driver_settings(settings: &Value) -> io::Result<()> {
    let path = otd_appdata_dir().join("settings.json");
    let parent = path.parent().ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            "driver settings path has no parent",
        )
    })?;
    fs::create_dir_all(parent)?;
    let temporary = path.with_extension("json.tmp");
    fs::write(
        &temporary,
        serde_json::to_vec_pretty(settings).map_err(io::Error::other)?,
    )?;
    fs::rename(&temporary, &path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prefers_xdg_data_home_when_set() {
        let root = local_data_root_from(
            Some(PathBuf::from("/custom/data")),
            Some(PathBuf::from("/home/test")),
        );
        assert_eq!(
            root.join("OpenTabletDriver"),
            PathBuf::from("/custom/data/TabletFlow/OpenTabletDriver")
        );
    }

    #[test]
    fn falls_back_to_home_local_share() {
        let root = local_data_root_from(None, Some(PathBuf::from("/home/test")));
        assert_eq!(
            root.join("OpenTabletDriver"),
            PathBuf::from("/home/test/.local/share/TabletFlow/OpenTabletDriver")
        );
    }

    #[test]
    fn missing_everything_falls_back_to_temp_dir_instead_of_panicking() {
        let root = local_data_root_from(None, None);
        assert!(root.starts_with(std::env::temp_dir()));
        assert!(root.ends_with("TabletFlow"));
    }
}
