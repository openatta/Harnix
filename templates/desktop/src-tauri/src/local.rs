//! Local mode: the app runs as this shell's child, on a loopback port.
//!
//! # How the window is admitted
//!
//! The child is started with `--launch-link`, so its page carries no token.
//! The shell reads the token from the file the app writes under its state
//! directory (owner-only), asks `POST /auth/link` for a one-time link, and
//! points the window at it; following the link sets the session cookie and
//! lands on the interface. That endpoint is the app's program-facing contract
//! for exactly this ("a launcher, a desktop shell, a CLI"): JSON, versioned
//! with the server, and it doubles as the readiness check — it answers once
//! the server is serving and the token on disk is this process's. The other
//! way, scraping the `open → …` line the app prints, reads a console message
//! meant for a person, and would tie the shell to its wording.
//!
//! Nothing crosses Tauri IPC: the token goes from a file to one loopback
//! request, and the window only ever sees a link that is spent once used.

use std::io::{Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use crate::config::Local;

/// How long the app gets to start serving. Startup builds instances and runs
/// module migrations; a first start on a slow disk takes seconds, not minutes.
const READY_TIMEOUT: Duration = Duration::from_secs(60);

/// How long a clean shutdown gets before the child is killed.
const STOP_TIMEOUT: Duration = Duration::from_secs(5);

/// The child, shared between whoever started it, whoever watches it and
/// whoever stops it on exit.
#[derive(Default, Clone)]
pub struct Slot(Arc<Mutex<SlotState>>);

#[derive(Default)]
struct SlotState {
    child: Option<Child>,
    /// Set by [`Slot::stop`]. A child started after the shell decided to exit
    /// would outlive it, so starting checks this under the same lock.
    closed: bool,
}

/// Where a started app can be reached.
pub struct Started {
    pub addr: SocketAddr,
    token_file: PathBuf,
}

/// What [`Slot::poll`] found.
pub enum Running {
    Yes,
    /// The child ended by itself; the text says how.
    Exited(String),
    /// The shell stopped it, or never started one.
    Released,
}

impl Slot {
    /// Start the app on loopback with launch links on.
    pub fn start(&self, local: &Local) -> Result<Started, String> {
        if !local.program.is_file() {
            return Err(format!(
                "找不到应用程序 {}。在 desktop.toml 的 [local] program 写上它的路径，或把它打包成 sidecar。",
                local.program.display()
            ));
        }
        let port = match local.port {
            Some(port) => port,
            None => free_port().map_err(|e| format!("找不到空闲端口：{e}"))?,
        };
        let mut command = Command::new(&local.program);
        if let Some(profile) = &local.profile {
            command.arg("--profile").arg(profile);
        }
        // Loopback whatever the profile says: the window is on this machine,
        // and a reachable bind would need TLS and paired devices it does not
        // have.
        command
            .args([
                "--host",
                "127.0.0.1",
                "--port",
                &port.to_string(),
                "--launch-link",
                "--exit-with-stdin",
            ])
            .args(&local.args)
            // The app's stdin is a pipe this process holds (inside `Child`):
            // when the shell goes, however it goes — a crash or `kill -9`
            // included — the pipe closes and the app stops itself. Logs go
            // wherever the shell's go.
            .stdin(Stdio::piped());
        // A desktop launch starts in `/`, and the engine keeps per-directory
        // state (`.agix/`) under the working directory it is given. Home is
        // where a person would expect that to land, wherever the shell was
        // started from.
        if let Some(home) = dirs::home_dir() {
            command.current_dir(home);
        }

        let mut state = self.0.lock().unwrap_or_else(|e| e.into_inner());
        if state.closed {
            return Err("正在退出".into());
        }
        let child = command
            .spawn()
            .map_err(|e| format!("启动 {} 失败：{e}", local.program.display()))?;
        state.child = Some(child);
        Ok(Started {
            addr: SocketAddr::from(([127, 0, 0, 1], port)),
            token_file: state_dir(local.profile.as_deref()).join("token"),
        })
    }

    /// Wait for the app to serve, then get a launch link for the window.
    pub fn launch_link(&self, started: &Started) -> Result<String, String> {
        let deadline = Instant::now() + READY_TIMEOUT;
        let mut last = String::from("没有响应");
        while Instant::now() < deadline {
            match self.poll() {
                Running::Yes => {}
                Running::Exited(how) => return Err(format!("应用在就绪之前退出了（{how}）")),
                Running::Released => return Err("正在退出".into()),
            }
            // Read afresh every time: until the server is up, the file may
            // still hold the token of an earlier run.
            match std::fs::read_to_string(&started.token_file) {
                Err(e) => last = format!("读不到 {}：{e}", started.token_file.display()),
                Ok(token) => match mint(started.addr, token.trim()) {
                    Ok(path) => return Ok(format!("http://{}{path}", started.addr)),
                    Err(e) => last = e,
                },
            }
            std::thread::sleep(Duration::from_millis(200));
        }
        Err(format!(
            "应用 {} 秒内没有就绪：{last}",
            READY_TIMEOUT.as_secs()
        ))
    }

    pub fn poll(&self) -> Running {
        let mut state = self.0.lock().unwrap_or_else(|e| e.into_inner());
        let Some(child) = state.child.as_mut() else {
            return Running::Released;
        };
        match child.try_wait() {
            Ok(None) => Running::Yes,
            Ok(Some(status)) => {
                state.child = None;
                Running::Exited(status.to_string())
            }
            Err(e) => Running::Exited(e.to_string()),
        }
    }

    /// Stop the child, cleanly if it will: SIGINT is what a Harnix process
    /// shuts down on (modules stop in reverse, the hub closes, uploads are
    /// cleared). Killed if it has not finished within [`STOP_TIMEOUT`].
    /// `None` if there was nothing to stop.
    pub fn stop(&self) -> Option<Stopped> {
        let mut child = {
            let mut state = self.0.lock().unwrap_or_else(|e| e.into_inner());
            state.closed = true;
            state.child.take()?
        };
        interrupt(&child);
        let deadline = Instant::now() + STOP_TIMEOUT;
        while Instant::now() < deadline {
            if let Ok(Some(status)) = child.try_wait() {
                return Some(Stopped {
                    clean: status.success(),
                    how: status.to_string(),
                });
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        let _ = child.kill();
        let how = match child.wait() {
            Ok(status) => status.to_string(),
            Err(e) => e.to_string(),
        };
        Some(Stopped {
            clean: false,
            how: format!(
                "{how}（{} 秒内没有退出，已强制结束）",
                STOP_TIMEOUT.as_secs()
            ),
        })
    }
}

/// How a child the shell stopped ended.
pub struct Stopped {
    /// Exited by itself, successfully, after being asked to.
    pub clean: bool,
    pub how: String,
}

#[cfg(unix)]
fn interrupt(child: &Child) {
    // Safe: a signal to a pid this process spawned and has not reaped, so it
    // cannot have been reused for another process.
    unsafe {
        libc::kill(child.id() as libc::pid_t, libc::SIGINT);
    }
}

#[cfg(not(unix))]
fn interrupt(_child: &Child) {
    // Windows has no signal for a child without a console of its own; it goes
    // straight to the kill after the timeout. (README: 已知的限制.)
}

/// A port nothing is listening on right now. Released before the child binds
/// it, so another process could take it in between; the child then fails to
/// start and the error page says so, which is better than a fixed port that
/// collides every time a second copy runs.
fn free_port() -> std::io::Result<u16> {
    Ok(TcpListener::bind(("127.0.0.1", 0))?.local_addr()?.port())
}

/// Where the app keeps its token, by the rules the app itself follows
/// (`crates/app/src/paths.rs` in Harnix): `HARNIX_STATE_DIR`, else
/// `~/.<product_id>`, the product id being the profile's `[brand] product_id`,
/// else [`crate::config::PRODUCT_ID`]. Mirrored rather than shared, because the shell does not link
/// Harnix; the child inherits this environment, so both sides read the same
/// variable.
fn state_dir(profile: Option<&std::path::Path>) -> PathBuf {
    if let Some(dir) = std::env::var_os("HARNIX_STATE_DIR") {
        return dir.into();
    }
    let product = profile
        .and_then(|path| std::fs::read_to_string(path).ok())
        .and_then(|text| text.parse::<toml::Table>().ok())
        .and_then(|table| {
            table
                .get("brand")?
                .get("product_id")?
                .as_str()
                .map(str::to_string)
        })
        .unwrap_or_else(|| crate::config::PRODUCT_ID.to_string());
    dirs::home_dir()
        .unwrap_or_default()
        .join(format!(".{product}"))
}

/// `POST /auth/link` with the token as a bearer header; the link's path.
fn mint(addr: SocketAddr, token: &str) -> Result<String, String> {
    let request = format!(
        "POST /auth/link HTTP/1.1\r\nHost: {addr}\r\nAuthorization: Bearer {token}\r\n\
         Content-Length: 0\r\nConnection: close\r\n\r\n"
    );
    let response = exchange(addr, &request).map_err(|e| format!("连不上 {addr}：{e}"))?;
    match response.status {
        200 => {}
        // The server is up but the file still held an earlier run's token,
        // or another copy of the app sharing this state directory replaced it.
        401 => return Err("令牌不对（还是上一次运行的，或被另一个进程覆盖了）".into()),
        other => return Err(format!("POST /auth/link 返回 {other}")),
    }
    let body: serde_json::Value = serde_json::from_str(&response.body)
        .map_err(|e| format!("POST /auth/link 的应答不对：{e}"))?;
    body["path"]
        .as_str()
        .map(str::to_string)
        .ok_or_else(|| "POST /auth/link 的应答里没有 path".into())
}

/// A response, as much of it as the shell looks at.
pub struct Response {
    pub status: u16,
    pub head: String,
    pub body: String,
}

/// `GET` on the child, for `--smoke` to follow the link the way the window
/// would.
pub fn get(addr: SocketAddr, path: &str, cookie: Option<&str>) -> std::io::Result<Response> {
    let cookie = cookie
        .map(|c| format!("Cookie: {c}\r\n"))
        .unwrap_or_default();
    exchange(
        addr,
        &format!("GET {path} HTTP/1.1\r\nHost: {addr}\r\n{cookie}Connection: close\r\n\r\n"),
    )
}

/// One request on loopback, plaintext, `Connection: close`, read to the end.
/// That is everything the shell ever says to its own child, so an HTTP client
/// crate (TLS, pools, an async runtime) would be weight for nothing.
fn exchange(addr: SocketAddr, request: &str) -> std::io::Result<Response> {
    let mut stream = TcpStream::connect_timeout(&addr, Duration::from_secs(1))?;
    stream.set_read_timeout(Some(Duration::from_secs(5)))?;
    stream.write_all(request.as_bytes())?;
    let mut raw = Vec::new();
    stream.read_to_end(&mut raw)?;
    let raw = String::from_utf8_lossy(&raw);
    let (head, body) = raw.split_once("\r\n\r\n").unwrap_or((&raw, ""));
    let status = head
        .split_whitespace()
        .nth(1)
        .and_then(|s| s.parse().ok())
        .ok_or_else(|| std::io::Error::other("not an HTTP response"))?;
    Ok(Response {
        status,
        head: head.to_string(),
        body: body.to_string(),
    })
}

/// The value of the first `Set-Cookie` in a response head, `name=value` only.
pub fn set_cookie(head: &str) -> Option<String> {
    head.lines()
        .find_map(|l| {
            l.split_once(':')
                .filter(|(k, _)| k.eq_ignore_ascii_case("set-cookie"))
        })
        .and_then(|(_, v)| v.trim().split(';').next())
        .map(str::to_string)
}
