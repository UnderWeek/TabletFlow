use crate::platform::Transport;
use std::fs::{File, OpenOptions};
use std::io::{self, Read, Write};
use std::os::windows::io::AsRawHandle;
use std::thread::JoinHandle;
use windows_sys::Win32::Foundation::{GetLastError, ERROR_SEM_TIMEOUT};
use windows_sys::Win32::System::Pipes::WaitNamedPipeW;
use windows_sys::Win32::System::IO::CancelSynchronousIo;

use super::runtime::wide;

const PIPE_PATH: &str = r"\\.\pipe\OpenTabletDriver.Daemon";

pub(super) struct WindowsTransport(File);

impl Read for WindowsTransport {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        self.0.read(buffer)
    }
}

impl Write for WindowsTransport {
    fn write(&mut self, buffer: &[u8]) -> io::Result<usize> {
        self.0.write(buffer)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.0.flush()
    }
}

impl Transport for WindowsTransport {
    fn try_clone_box(&self) -> io::Result<Box<dyn Transport>> {
        Ok(Box::new(Self(self.0.try_clone()?)))
    }

    fn interrupt(&self, reader: &JoinHandle<()>) {
        let handle = reader.as_raw_handle();
        while !reader.is_finished() {
            unsafe {
                let _ = CancelSynchronousIo(handle);
            }
            std::thread::yield_now();
        }
    }
}

pub(super) fn connect() -> io::Result<Box<dyn Transport>> {
    let pipe = wide(PIPE_PATH);
    if unsafe { WaitNamedPipeW(pipe.as_ptr(), 250) } == 0 {
        return Err(io::Error::last_os_error());
    }
    let file = OpenOptions::new().read(true).write(true).open(PIPE_PATH)?;
    Ok(Box::new(WindowsTransport(file)))
}

pub(super) fn is_available() -> bool {
    let pipe = wide(PIPE_PATH);
    if unsafe { WaitNamedPipeW(pipe.as_ptr(), 0) } != 0 {
        return true;
    }
    (unsafe { GetLastError() }) == ERROR_SEM_TIMEOUT
}
