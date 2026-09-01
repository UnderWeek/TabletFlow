use crate::platform::Transport;
use std::fs::{File, OpenOptions};
use std::io::{self, Read, Write};
use std::os::windows::io::AsRawHandle;
use std::thread::JoinHandle;
use std::time::{Duration, Instant};
use windows_sys::Win32::Foundation::{GetLastError, ERROR_SEM_TIMEOUT};
use windows_sys::Win32::System::Pipes::WaitNamedPipeW;
use windows_sys::Win32::System::IO::CancelSynchronousIo;

use super::runtime::wide;

const PIPE_PATH: &str = r"\\.\pipe\OpenTabletDriver.Daemon";

/// Upper bound on how long `WindowsTransport::interrupt` keeps retrying
/// `CancelSynchronousIo`. If the reader is still blocked after this, the
/// caller (`ReaderGuard::drop`'s watchdog) gives up on it too, so this loop
/// must not spin forever - just long enough to give a normally-behaving
/// pending read several chances to notice the cancellation.
const CANCEL_RETRY_DEADLINE: Duration = Duration::from_secs(2);
const CANCEL_RETRY_INTERVAL: Duration = Duration::from_millis(20);

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
        let deadline = Instant::now() + CANCEL_RETRY_DEADLINE;
        while !reader.is_finished() && Instant::now() < deadline {
            unsafe {
                let _ = CancelSynchronousIo(handle);
            }
            std::thread::sleep(CANCEL_RETRY_INTERVAL);
        }
        // If CancelSynchronousIo never manages to wake a wedged reader, this
        // returns without the reader having finished. That's fine: the
        // caller (ReaderGuard::drop) has its own bounded watchdog and treats
        // a still-blocked reader as a leaked thread rather than a hang, so
        // this loop must exit rather than spin on yield_now() forever.
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
    // A timeout of 0 is NMPWAIT_USE_DEFAULT_WAIT, not "return immediately" -
    // it makes this block for the pipe server's configured default timeout
    // whenever the pipe exists but every server instance is busy. Pass an
    // explicit 1ms timeout so this stays a cheap non-blocking probe, which
    // matters since it's polled from the backend supervisor loop.
    if unsafe { WaitNamedPipeW(pipe.as_ptr(), 1) } != 0 {
        return true;
    }
    (unsafe { GetLastError() }) == ERROR_SEM_TIMEOUT
}
