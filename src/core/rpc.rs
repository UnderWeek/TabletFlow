use crate::core::models::BackendCommand;
use crate::platform::{Platform, Transport};
use serde_json::{json, Value};
use std::io::{self, Read, Write};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};
use std::thread;
use std::time::{Duration, Instant};

/// Longest a single `recv_timeout` slice in `call()` waits before re-checking
/// the shutdown flag and the overall deadline.
const CALL_POLL_INTERVAL: Duration = Duration::from_millis(200);

/// Longest ReaderGuard::drop waits for the reader thread to actually stop
/// before giving up on it. Each Transport::interrupt implementation (e.g.
/// the Windows one, which retries `CancelSynchronousIo` for up to its own
/// bounded deadline) is expected to return promptly even if it never
/// actually wakes the blocked reader, but this timeout is the last line of
/// defense: if a platform's interrupt somehow still blocks, or the reader
/// never notices the cancellation, shutdown/reconnect gives up on the
/// watchdog thread rather than hanging forever.
const READER_SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(2);

/// Upper bound on a single RPC message body. OpenTabletDriver's real
/// responses (tablet lists, settings) are at most a few hundred KB; this
/// just keeps a malformed or hostile local peer from making `read_message`
/// allocate an unbounded `Vec<u8>` off of a bogus Content-Length.
const MAX_MESSAGE_LEN: usize = 64 * 1024 * 1024;

struct ReaderGuard {
    cancelled: Arc<AtomicBool>,
    thread: Option<thread::JoinHandle<()>>,
    interrupter: Option<Box<dyn Transport>>,
}

impl Drop for ReaderGuard {
    fn drop(&mut self) {
        let Some(reader) = self.thread.take() else {
            return;
        };
        self.cancelled.store(true, Ordering::Release);
        let interrupter = self.interrupter.take();
        // interrupt()+join() run on a separate thread so an interrupt
        // implementation that never manages to unblock the reader (or a
        // reader wedged in a way no cancellation reaches) cannot hang this
        // thread forever. If the watchdog doesn't report back in time, the
        // reader thread (and this tiny watchdog) are left to finish on
        // their own - a leaked thread, not a hung shutdown.
        let (done_tx, done_rx) = mpsc::channel::<()>();
        thread::spawn(move || {
            if let Some(interrupter) = interrupter {
                interrupter.interrupt(&reader);
            }
            let _ = reader.join();
            let _ = done_tx.send(());
        });
        let _ = done_rx.recv_timeout(READER_SHUTDOWN_TIMEOUT);
    }
}

pub struct RpcClient {
    stream: Box<dyn Transport>,
    responses: Receiver<Value>,
    next_id: u64,
    _reader: ReaderGuard,
    platform: &'static dyn Platform,
    shutdown: Arc<AtomicBool>,
}

impl RpcClient {
    pub fn connect(
        platform: &'static dyn Platform,
        backend_events: Sender<BackendCommand>,
        generation: u64,
        shutdown: Arc<AtomicBool>,
    ) -> io::Result<Self> {
        let stream = platform.connect_transport()?;
        let mut reader = stream.try_clone_box()?;
        let interrupter = reader.try_clone_box()?;
        let (response_sender, responses) = mpsc::channel();
        let cancelled = Arc::new(AtomicBool::new(false));
        let reader_cancelled = Arc::clone(&cancelled);
        let resynchronize_is_tablet_change = platform.restore_pipeline_after_detect();
        let reader_thread = thread::spawn(move || loop {
            if reader_cancelled.load(Ordering::Acquire) {
                break;
            }
            match read_message(&mut *reader) {
                Ok(message) if message.get("id").is_some() => {
                    if response_sender.send(message).is_err() {
                        break;
                    }
                }
                Ok(message) => {
                    let method = message
                        .get("method")
                        .and_then(Value::as_str)
                        .unwrap_or_default();
                    if method.contains("TabletsChanged")
                        || (resynchronize_is_tablet_change && method.contains("Resynchronize"))
                    {
                        let _ = backend_events.send(BackendCommand::TabletChanged { generation });
                    }
                }
                Err(error) => {
                    if !reader_cancelled.load(Ordering::Acquire) {
                        let _ = backend_events.send(BackendCommand::DriverDisconnected {
                            generation,
                            reason: error.to_string(),
                        });
                    }
                    break;
                }
            }
        });

        Ok(Self {
            stream,
            responses,
            next_id: 1,
            _reader: ReaderGuard {
                cancelled,
                thread: Some(reader_thread),
                interrupter: Some(interrupter),
            },
            platform,
            shutdown,
        })
    }

    pub fn call(&mut self, method: &str, params: Value) -> io::Result<Value> {
        let id = self.next_id;
        self.next_id += 1;
        let body = serde_json::to_vec(&json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": method,
            "params": params,
        }))
        .map_err(io::Error::other)?;
        let header = format!("Content-Length: {}\r\n\r\n", body.len());
        self.stream.write_all(header.as_bytes())?;
        self.stream.write_all(&body)?;
        self.stream.flush()?;

        // Waits are sliced to CALL_POLL_INTERVAL (rather than one long
        // recv_timeout for the whole rpc_timeout) so a shutdown request can
        // interrupt the wait quickly, and so a stray response carrying an
        // unrelated id can't keep resetting the deadline.
        let deadline = Instant::now() + self.platform.rpc_timeout(method);
        loop {
            if self.shutdown.load(Ordering::Acquire) {
                return Err(io::Error::new(
                    io::ErrorKind::Interrupted,
                    "shutdown requested",
                ));
            }
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return Err(io::Error::new(
                    io::ErrorKind::TimedOut,
                    "OpenTabletDriver did not respond",
                ));
            }
            let response = match self
                .responses
                .recv_timeout(remaining.min(CALL_POLL_INTERVAL))
            {
                Ok(response) => response,
                Err(RecvTimeoutError::Timeout) => continue,
                Err(RecvTimeoutError::Disconnected) => {
                    return Err(io::Error::new(
                        io::ErrorKind::BrokenPipe,
                        "OpenTabletDriver connection closed",
                    ))
                }
            };
            if response.get("id") == Some(&json!(id)) {
                if let Some(error) = response.get("error") {
                    return Err(io::Error::other(error.to_string()));
                }
                return response
                    .get("result")
                    .cloned()
                    .ok_or_else(|| io::Error::other("RPC response has no result"));
            }
        }
    }
}

pub trait DriverRpc {
    fn rpc_call(&mut self, method: &str, params: Value) -> io::Result<Value>;
}

impl DriverRpc for RpcClient {
    fn rpc_call(&mut self, method: &str, params: Value) -> io::Result<Value> {
        self.call(method, params)
    }
}

pub fn query_tablets<C: DriverRpc>(
    client: &mut C,
    detect: bool,
    restore_pipeline_after_detect: bool,
    driver_settings: &mut Option<Value>,
) -> io::Result<Value> {
    if !detect {
        return client.rpc_call("GetTablets", json!([]));
    }
    let tablets = client.rpc_call("DetectTablets", json!([]))?;
    if restore_pipeline_after_detect {
        let settings = client.rpc_call("GetSettings", json!([]))?;
        client.rpc_call("SetSettings", json!([settings]))?;
        *driver_settings = Some(client.rpc_call("GetSettings", json!([]))?);
    }
    Ok(tablets)
}

pub fn read_message(stream: &mut dyn Read) -> io::Result<Value> {
    let mut headers = Vec::new();
    let mut delimiter = Vec::new();
    loop {
        let mut byte = [0u8; 1];
        stream.read_exact(&mut byte)?;
        headers.push(byte[0]);
        delimiter.push(byte[0]);
        if delimiter.ends_with(b"\r\n\r\n") || delimiter.ends_with(b"\n\n") {
            break;
        }
        if delimiter.len() > 4 {
            delimiter.remove(0);
        }
        if headers.len() > 8192 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "RPC headers are too large",
            ));
        }
    }
    let headers = String::from_utf8_lossy(&headers);
    let content_length = headers
        .lines()
        .find_map(|line| {
            let (name, value) = line.split_once(':')?;
            name.eq_ignore_ascii_case("content-length")
                .then(|| value.trim().parse::<usize>().ok())
                .flatten()
        })
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "RPC content length missing"))?;
    if content_length > MAX_MESSAGE_LEN {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("RPC content length {content_length} exceeds the {MAX_MESSAGE_LEN}-byte limit"),
        ));
    }
    let mut body = vec![0u8; content_length];
    stream.read_exact(&mut body)?;
    serde_json::from_slice(&body).map_err(io::Error::other)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::platform::Platform;
    use std::io::Cursor;
    use std::net::{TcpListener, TcpStream};
    use std::path::PathBuf;
    use std::sync::Mutex;

    /// Cross-platform stand-in for `UnixStream::pair()` (not available on
    /// Windows). A loopback TCP connection behaves the same way for these
    /// tests: both ends are `Read + Write`, support `try_clone`, and support
    /// `shutdown()` to unblock a peer stuck in a blocking read.
    fn socket_pair() -> (TcpStream, TcpStream) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let client = TcpStream::connect(addr).unwrap();
        let (server, _) = listener.accept().unwrap();
        client.set_nodelay(true).ok();
        server.set_nodelay(true).ok();
        (client, server)
    }

    #[test]
    fn rejects_content_length_above_the_limit() {
        let frame = format!("Content-Length: {}\r\n\r\n", MAX_MESSAGE_LEN + 1).into_bytes();
        let error = read_message(&mut Cursor::new(frame)).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::InvalidData);
    }

    #[test]
    fn parses_content_length_frame() {
        let body = br#"{"jsonrpc":"2.0","id":1,"result":[]}"#;
        let mut frame = format!("Content-Length: {}\r\n\r\n", body.len()).into_bytes();
        frame.extend_from_slice(body);
        let value = read_message(&mut Cursor::new(frame)).unwrap();
        assert_eq!(value["id"], 1);
    }

    struct StalledTransport(TcpStream);
    impl Read for StalledTransport {
        fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
            self.0.read(buf)
        }
    }
    impl Write for StalledTransport {
        fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
            self.0.write(buf)
        }
        fn flush(&mut self) -> io::Result<()> {
            self.0.flush()
        }
    }
    impl crate::platform::Transport for StalledTransport {
        fn try_clone_box(&self) -> io::Result<Box<dyn crate::platform::Transport>> {
            Ok(Box::new(StalledTransport(self.0.try_clone()?)))
        }
        fn interrupt(&self, _reader: &thread::JoinHandle<()>) {
            let _ = self.0.shutdown(std::net::Shutdown::Both);
        }
    }

    struct StalledPlatform(Mutex<Option<TcpStream>>);
    impl Platform for StalledPlatform {
        fn name(&self) -> &'static str {
            "stalled-test"
        }
        fn settings_path(&self) -> Option<PathBuf> {
            None
        }
        fn acquire_instance_guard(&self) -> Option<Box<dyn Send>> {
            None
        }
        fn connect_transport(&self) -> io::Result<Box<dyn crate::platform::Transport>> {
            let stream = self.0.lock().unwrap().take().unwrap();
            Ok(Box::new(StalledTransport(stream)))
        }
        fn ipc_available(&self) -> bool {
            true
        }
        fn owned_daemon_running(&self) -> bool {
            false
        }
        fn start_daemon(&self) -> io::Result<()> {
            Ok(())
        }
        fn stop_daemon(&self) {}
        fn configure_autostart(&self, _enabled: bool, _start_minimized: bool) -> io::Result<()> {
            Ok(())
        }
        fn open_url(&self, _url: &str) -> io::Result<()> {
            Ok(())
        }
        fn run_driver_self_test(&self) -> io::Result<()> {
            Ok(())
        }
        fn rpc_timeout(&self, _method: &str) -> std::time::Duration {
            // Stand-in for the real 180s DetectTablets/SetSettings timeout on
            // Windows; short here only so the test itself stays fast.
            std::time::Duration::from_millis(300)
        }
    }

    // Regression test: a call() to a daemon that never answers must not block
    // the calling thread for the full rpc_timeout once shutdown is requested.
    // On Windows rpc_timeout is 180s for DetectTablets/SetSettings (see
    // WindowsPlatform::rpc_timeout), and main.rs's shutdown path is
    // `send(Shutdown)` -> `backend_thread.join()`, which must not be stuck
    // behind an in-flight call() like this one.
    #[test]
    fn call_returns_promptly_once_shutdown_is_requested() {
        let (client_side, _server_side) = socket_pair();
        let platform = Box::leak(Box::new(StalledPlatform(Mutex::new(Some(client_side)))));
        let (events, _events_rx) = mpsc::channel();
        let shutdown = Arc::new(AtomicBool::new(false));
        let mut client = RpcClient::connect(platform, events, 0, Arc::clone(&shutdown)).unwrap();

        let shutdown_flag = Arc::clone(&shutdown);
        // Simulates main.rs sending BackendCommand::Shutdown while the backend
        // thread is stuck inside client.call(...).
        thread::spawn(move || {
            thread::sleep(std::time::Duration::from_millis(20));
            shutdown_flag.store(true, Ordering::Release);
        });

        let start = std::time::Instant::now();
        let result = client.call("DetectTablets", json!([]));
        let elapsed = start.elapsed();

        assert!(
            matches!(&result, Err(error) if error.kind() == io::ErrorKind::Interrupted),
            "expected an Interrupted error from the shutdown request, got {result:?}"
        );
        assert!(
            elapsed < Duration::from_millis(300),
            "call() took {elapsed:?} to return after shutdown was requested; expected it to \
             bail out within roughly CALL_POLL_INTERVAL instead of waiting out the full \
             rpc_timeout (300ms in this test, 180s for DetectTablets/SetSettings on Windows)"
        );
    }

    // Regression test: a response carrying an id from an earlier, already
    // timed-out call must not reset the deadline for the current call.
    #[test]
    fn stale_response_does_not_extend_the_deadline() {
        let (client_side, server_side) = socket_pair();
        let platform = Box::leak(Box::new(StalledPlatform(Mutex::new(Some(client_side)))));
        let (events, _events_rx) = mpsc::channel();
        let shutdown = Arc::new(AtomicBool::new(false));
        let mut client = RpcClient::connect(platform, events, 0, shutdown).unwrap();

        // Feed a burst of responses with a stale/unrelated id every 50ms,
        // well inside the 300ms test timeout, while call() waits for id=1.
        thread::spawn(move || {
            let mut server = server_side;
            for _ in 0..10 {
                thread::sleep(Duration::from_millis(50));
                let body = br#"{"jsonrpc":"2.0","id":999,"result":null}"#;
                let header = format!("Content-Length: {}\r\n\r\n", body.len());
                if server.write_all(header.as_bytes()).is_err() {
                    break;
                }
                if server.write_all(body).is_err() {
                    break;
                }
                let _ = server.flush();
            }
        });

        let start = std::time::Instant::now();
        let result = client.call("DetectTablets", json!([]));
        let elapsed = start.elapsed();

        assert!(
            matches!(&result, Err(error) if error.kind() == io::ErrorKind::TimedOut),
            "expected a timeout error, got {result:?}"
        );
        assert!(
            elapsed < Duration::from_millis(500),
            "call() took {elapsed:?} to time out despite a 300ms rpc_timeout; stale responses \
             with mismatched ids must not keep resetting the deadline"
        );
    }

    struct UninterruptibleTransport(TcpStream);
    impl Read for UninterruptibleTransport {
        fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
            self.0.read(buf)
        }
    }
    impl Write for UninterruptibleTransport {
        fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
            self.0.write(buf)
        }
        fn flush(&mut self) -> io::Result<()> {
            self.0.flush()
        }
    }
    impl crate::platform::Transport for UninterruptibleTransport {
        fn try_clone_box(&self) -> io::Result<Box<dyn crate::platform::Transport>> {
            Ok(Box::new(UninterruptibleTransport(self.0.try_clone()?)))
        }
        // Simulates a platform whose cancellation primitive (e.g. Windows'
        // CancelSynchronousIo) never manages to wake the blocked reader.
        fn interrupt(&self, _reader: &thread::JoinHandle<()>) {}
    }

    struct UninterruptiblePlatform(Mutex<Option<TcpStream>>);
    impl Platform for UninterruptiblePlatform {
        fn name(&self) -> &'static str {
            "uninterruptible-test"
        }
        fn settings_path(&self) -> Option<PathBuf> {
            None
        }
        fn acquire_instance_guard(&self) -> Option<Box<dyn Send>> {
            None
        }
        fn connect_transport(&self) -> io::Result<Box<dyn crate::platform::Transport>> {
            let stream = self.0.lock().unwrap().take().unwrap();
            Ok(Box::new(UninterruptibleTransport(stream)))
        }
        fn ipc_available(&self) -> bool {
            true
        }
        fn owned_daemon_running(&self) -> bool {
            false
        }
        fn start_daemon(&self) -> io::Result<()> {
            Ok(())
        }
        fn stop_daemon(&self) {}
        fn configure_autostart(&self, _enabled: bool, _start_minimized: bool) -> io::Result<()> {
            Ok(())
        }
        fn open_url(&self, _url: &str) -> io::Result<()> {
            Ok(())
        }
        fn run_driver_self_test(&self) -> io::Result<()> {
            Ok(())
        }
    }

    // Regression test: dropping an RpcClient whose reader thread is stuck in
    // a blocking read AND whose Transport::interrupt() fails to unblock it
    // (the exact Windows failure mode CancelSynchronousIo could hit) must
    // not hang forever. This is what backend.rs's `client = None;` on every
    // reconnect/error path relies on, and what main.rs's shutdown depends on
    // transitively.
    #[test]
    fn dropping_client_does_not_hang_when_interrupt_never_wakes_reader() {
        let (client_side, _server_side) = socket_pair();
        let platform = Box::leak(Box::new(UninterruptiblePlatform(Mutex::new(Some(
            client_side,
        )))));
        let (events, _events_rx) = mpsc::channel();
        let shutdown = Arc::new(AtomicBool::new(false));
        let client = RpcClient::connect(platform, events, 0, shutdown).unwrap();

        let start = std::time::Instant::now();
        drop(client);
        let elapsed = start.elapsed();

        assert!(
            elapsed < READER_SHUTDOWN_TIMEOUT + Duration::from_millis(500),
            "dropping the client took {elapsed:?}; ReaderGuard::drop must give up waiting on a \
             wedged reader within roughly READER_SHUTDOWN_TIMEOUT instead of blocking forever"
        );
    }
}
