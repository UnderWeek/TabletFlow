use crate::platform::Transport;
use std::io::{self, Read, Write};
use std::net::Shutdown;
use std::os::unix::net::UnixStream;
use std::path::PathBuf;

fn path() -> PathBuf {
    std::env::temp_dir().join("CoreFxPipe_OpenTabletDriver.Daemon")
}

pub struct LinuxTransport(UnixStream);
impl Read for LinuxTransport {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        self.0.read(buf)
    }
}
impl Write for LinuxTransport {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.0.write(buf)
    }
    fn flush(&mut self) -> io::Result<()> {
        self.0.flush()
    }
}
impl Transport for LinuxTransport {
    fn try_clone_box(&self) -> io::Result<Box<dyn Transport>> {
        Ok(Box::new(Self(self.0.try_clone()?)))
    }
    fn interrupt(&self, _reader: &std::thread::JoinHandle<()>) {
        let _ = self.0.shutdown(Shutdown::Both);
    }
}

pub fn connect() -> io::Result<Box<dyn Transport>> {
    let stream = UnixStream::connect(path())?;
    stream.set_write_timeout(Some(std::time::Duration::from_secs(15)))?;
    Ok(Box::new(LinuxTransport(stream)))
}
pub fn is_available() -> bool {
    UnixStream::connect(path()).is_ok()
}
