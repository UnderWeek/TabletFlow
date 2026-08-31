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
    Command::new("open").arg(url).spawn().map(|_| ())
}
