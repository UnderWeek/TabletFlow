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
        if let Some(interrupter) = self.interrupter.take() {
            interrupter.interrupt(&reader);
        }
        let _ = reader.join();
    }
}

pub struct RpcClient {
    stream: Box<dyn Transport>,
    responses: Receiver<Value>,
    next_id: u64,
    _reader: ReaderGuard,
    platform: &'static dyn Platform,
}

impl RpcClient {
    pub fn connect(
        platform: &'static dyn Platform,
        backend_events: Sender<BackendCommand>,
        generation: u64,
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

        loop {
            let response = self
                .responses
                .recv_timeout(self.platform.rpc_timeout(method))
                .map_err(|error| match error {
                    RecvTimeoutError::Timeout => {
                        io::Error::new(io::ErrorKind::TimedOut, "OpenTabletDriver did not respond")
                    }
                    RecvTimeoutError::Disconnected => io::Error::new(
                        io::ErrorKind::BrokenPipe,
                        "OpenTabletDriver connection closed",
                    ),
                })?;
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
    let mut body = vec![0u8; content_length];
    stream.read_exact(&mut body)?;
    serde_json::from_slice(&body).map_err(io::Error::other)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    #[test]
    fn parses_content_length_frame() {
        let body = br#"{"jsonrpc":"2.0","id":1,"result":[]}"#;
        let mut frame = format!("Content-Length: {}\r\n\r\n", body.len()).into_bytes();
        frame.extend_from_slice(body);
        let value = read_message(&mut Cursor::new(frame)).unwrap();
        assert_eq!(value["id"], 1);
    }
}
