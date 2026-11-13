//! The last lines a child wrote to its stderr: a few, kept for whoever asks why it went, and each
//! passed on to this process's log as it comes.
//!
//! Bounded every way a child could push it: [`TAIL_LINES`] lines, [`TAIL_BYTES`] in all, and a
//! line longer than [`LINE_BYTES`] is cut there, the rest of it read and dropped.

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

use tokio::io::{AsyncBufReadExt, AsyncRead, BufReader};

/// Lines kept.
pub const TAIL_LINES: usize = 40;

/// Bytes kept, all lines together.
pub const TAIL_BYTES: usize = 4096;

/// Bytes kept of one line.
pub const LINE_BYTES: usize = 512;

/// The last lines of one child's stderr, across every start of it that was given the same tail.
/// Cheap to clone; every clone is the same lines.
#[derive(Clone)]
pub struct Tail(Arc<Inner>);

impl std::fmt::Debug for Tail {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Tail").field("name", &self.0.name).field("lines", &self.ring().lines.len()).finish()
    }
}

struct Inner {
    /// Whose lines these are, as the log names them.
    name: String,
    ring: Mutex<Ring>,
}

#[derive(Default)]
struct Ring {
    lines: VecDeque<String>,
    bytes: usize,
}

impl Tail {
    /// An empty tail for the child the log calls `name`.
    pub fn new(name: impl Into<String>) -> Self {
        Self(Arc::new(Inner { name: name.into(), ring: Mutex::default() }))
    }

    /// The lines kept, oldest first.
    pub fn lines(&self) -> Vec<String> {
        self.ring().lines.iter().cloned().collect()
    }

    fn ring(&self) -> std::sync::MutexGuard<'_, Ring> {
        self.0.ring.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn push(&self, line: String) {
        let mut ring = self.ring();
        ring.bytes += line.len();
        ring.lines.push_back(line);
        while ring.lines.len() > TAIL_LINES || (ring.bytes > TAIL_BYTES && ring.lines.len() > 1) {
            let Some(old) = ring.lines.pop_front() else { break };
            ring.bytes -= old.len();
        }
    }

    /// Read `from` to its end, a line at a time: each one kept and logged.
    pub(crate) async fn follow(self, from: impl AsyncRead + Unpin, pid: u32) {
        let mut reader = BufReader::new(from);
        let mut line: Vec<u8> = Vec::new();
        loop {
            let buf = match reader.fill_buf().await {
                Ok(buf) => buf,
                Err(_) => break,
            };
            if buf.is_empty() {
                break;
            }
            let (part, used, ended) = match buf.iter().position(|&b| b == b'\n') {
                Some(at) => (&buf[..at], at + 1, true),
                None => (buf, buf.len(), false),
            };
            let room = LINE_BYTES.saturating_sub(line.len());
            line.extend_from_slice(&part[..part.len().min(room)]);
            reader.consume(used);
            if ended {
                self.take(&mut line, pid);
            }
        }
        if !line.is_empty() {
            self.take(&mut line, pid);
        }
    }

    fn take(&self, line: &mut Vec<u8>, pid: u32) {
        let text = String::from_utf8_lossy(line).trim_end_matches('\r').to_string();
        line.clear();
        log(&self.0.name, pid, &text);
        self.push(text);
    }
}

/// One line, at the level the child logged it at when it is one of its log's lines (a timestamp,
/// then a level), as a warning when it is not: a panic, say, or a line a library printed.
fn log(name: &str, pid: u32, line: &str) {
    let (level, message) = parse(line);
    match level {
        Some("ERROR") => tracing::error!(child = %name, pid, "{message}"),
        Some("INFO") => tracing::info!(child = %name, pid, "{message}"),
        Some("DEBUG") => tracing::debug!(child = %name, pid, "{message}"),
        Some("TRACE") => tracing::trace!(child = %name, pid, "{message}"),
        _ => tracing::warn!(child = %name, pid, "{message}"),
    }
}

/// The level of a line of a child's log and the rest of it after the level; no level, and the
/// whole line, for any other.
fn parse(line: &str) -> (Option<&str>, &str) {
    let rest = line.trim_start();
    let Some((_stamp, rest)) = rest.split_once(char::is_whitespace) else { return (None, line) };
    let rest = rest.trim_start();
    let (level, message) = rest.split_once(char::is_whitespace).unwrap_or((rest, ""));
    match level {
        "ERROR" | "WARN" | "INFO" | "DEBUG" | "TRACE" => (Some(level), message.trim_start()),
        _ => (None, line),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn a_tail_keeps_the_last_lines_within_its_bounds() {
        let tail = Tail::new("test");
        let text: String = (0..100).map(|n| format!("line {n}\n")).collect();
        tail.clone().follow(text.as_bytes(), 1).await;
        let lines = tail.lines();
        assert_eq!(lines.len(), TAIL_LINES);
        assert_eq!(lines.last().map(String::as_str), Some("line 99"));

        let tail = Tail::new("test");
        let long = format!("{}\nlast", "x".repeat(10 * LINE_BYTES));
        tail.clone().follow(long.as_bytes(), 1).await;
        assert_eq!(tail.lines(), ["x".repeat(LINE_BYTES), "last".to_string()]);

        let tail = Tail::new("test");
        let wide: String = (0..30).map(|n| format!("{n:>300}\n")).collect();
        tail.clone().follow(wide.as_bytes(), 1).await;
        let kept = tail.lines();
        assert!(kept.iter().map(String::len).sum::<usize>() <= TAIL_BYTES, "{}", kept.len());
        assert_eq!(kept.last().map(|l| l.trim()), Some("29"));
    }

    #[test]
    fn a_line_of_the_childs_log_is_read_for_its_level() {
        assert_eq!(parse("2026-11-13T10:00:00.1Z  WARN harnix_proc: gone"), (Some("WARN"), "harnix_proc: gone"));
        assert_eq!(parse("2026-11-13T10:00:00.1Z ERROR x: y"), (Some("ERROR"), "x: y"));
        assert_eq!(parse("thread 'main' panicked at src/lib.rs:1:1"), (None, "thread 'main' panicked at src/lib.rs:1:1"));
        assert_eq!(parse("one"), (None, "one"));
    }
}
