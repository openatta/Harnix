//! A download-only event stream (`/events/session`, `/events/host`): server-sent events, one
//! JSON frame per event, read with a bounded wait.

use std::io::{BufRead, BufReader, ErrorKind as IoKind, Read, Write};
use std::net::TcpStream;
use std::time::Duration;

use serde_json::Value;

use crate::http::{self, Chunked, Endpoint};
use crate::{ClientError, ErrorKind};

/// Which stream.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Face {
    /// Frames of the sessions this client watches (`harnix.event`, `harnix.turn_settled`, …).
    Session,
    /// Everything not tied to a session: topics, instance changes.
    Host,
}

/// What one read gave.
#[derive(Debug, Clone, PartialEq)]
pub enum StreamItem {
    /// One frame, `{method, params}` as on any other topology.
    Frame(Value),
    /// Nothing within the wait.
    Idle,
}

pub struct EventStream {
    reader: BufReader<Box<dyn Read + Send>>,
    /// A line read up to a timeout: it continues on the next read.
    line: Vec<u8>,
    data: String,
}

impl EventStream {
    pub(crate) fn open(endpoint: &Endpoint, credential: &str, face: Face, idle: Duration) -> Result<Self, ClientError> {
        let path = match face {
            Face::Session => "/events/session",
            Face::Host => "/events/host",
        };
        let mut stream: TcpStream = endpoint.connect(Duration::from_secs(10))?;
        let head = http::head(endpoint, "GET", path, Some(credential), "Accept: text/event-stream\r\n", 0);
        stream
            .write_all(head.as_bytes())
            .map_err(|e| ClientError::transport(format!("{path}: {e}")))?;
        let mut reader = BufReader::new(stream.try_clone().map_err(|e| ClientError::transport(e.to_string()))?);
        let (status, headers) =
            http::read_head(&mut reader).map_err(|e| ClientError::transport(format!("{path}: {e}")))?;
        if status != 200 {
            let kind = if status == 401 {
                ErrorKind::Unauthenticated
            } else {
                ErrorKind::Transport
            };
            return Err(ClientError {
                kind,
                code: i64::from(status),
                message: format!("{path}: HTTP {status}"),
                data: Value::Null,
            });
        }
        stream
            .set_read_timeout(Some(idle.max(Duration::from_millis(1))))
            .map_err(|e| ClientError::transport(e.to_string()))?;
        let body: Box<dyn Read + Send> = if http::is_chunked(&headers) {
            Box::new(Chunked::new(reader))
        } else {
            Box::new(reader)
        };
        Ok(Self {
            reader: BufReader::new(body),
            line: Vec::new(),
            data: String::new(),
        })
    }

    /// The next frame, or [`StreamItem::Idle`] when the wait ran out; an error when the stream
    /// ended (the process went away) — open it again after connecting again. An event whose data
    /// is not JSON is skipped.
    #[allow(clippy::should_implement_trait)]
    pub fn next(&mut self) -> Result<StreamItem, ClientError> {
        loop {
            match self.reader.read_until(b'\n', &mut self.line) {
                Ok(0) => return Err(ClientError::transport("the event stream ended")),
                Ok(_) if !self.line.ends_with(b"\n") => continue,
                Ok(_) => {}
                Err(e) if matches!(e.kind(), IoKind::WouldBlock | IoKind::TimedOut) => {
                    return Ok(StreamItem::Idle);
                }
                Err(e) => return Err(ClientError::transport(e.to_string())),
            }
            let line = String::from_utf8_lossy(&std::mem::take(&mut self.line)).into_owned();
            let text = line.trim_end_matches(['\r', '\n']);
            if text.is_empty() {
                if self.data.is_empty() {
                    continue;
                }
                // Every event the server sends is one JSON frame. One that is not is no frame, and
                // not the wait running out either: it is left out, and the read goes on.
                match serde_json::from_str(&std::mem::take(&mut self.data)) {
                    Ok(frame) => return Ok(StreamItem::Frame(frame)),
                    Err(_) => continue,
                }
            }
            if let Some(rest) = text.strip_prefix("data:") {
                if !self.data.is_empty() {
                    self.data.push('\n');
                }
                self.data.push_str(rest.strip_prefix(' ').unwrap_or(rest));
            }
        }
    }
}
