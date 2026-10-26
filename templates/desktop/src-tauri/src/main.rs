//! A desktop window around a Harnix app (design §12).
//!
//! The shell owns three things: the window, the child process (local mode)
//! and its configuration. Everything the app does — every call, every event,
//! every upload — is the page's own HTTP and WebSocket to the server, exactly
//! as in a browser tab. There are no Tauri commands, no invoke handler and no
//! global Tauri object in the page, and `check-no-ipc.mjs` fails as soon as
//! one appears. That is what keeps the desktop app and the browser the same
//! product: one interface, one admission path, one set of tests.
//!
//! Two modes (`desktop.toml`, see `config.rs`):
//!
//! - **local** — start the app on a loopback port, wait for it, open a
//!   one-time launch link (`local.rs`). Closing the window stops the app.
//! - **remote** — open the configured `https://` address of a Harnix running
//!   elsewhere. Pairing is the web interface's own flow.
//!
//! `--smoke` does the same without a window: resolve, start, admit, print,
//! stop. It is how the template is checked where no window can be seen.

// No console window behind the app on Windows, in release builds.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod config;
mod local;

use std::net::{SocketAddr, TcpStream, ToSocketAddrs};
use std::time::Duration;

use tauri::{RunEvent, Url, WebviewUrl, WebviewWindow, WebviewWindowBuilder};

use config::Config;
use local::{Running, Slot};

fn main() {
    let context = tauri::generate_context!();
    let config = config::load(&context.config().identifier);
    if std::env::args().skip(1).any(|a| a == "--smoke") {
        std::process::exit(smoke(config));
    }

    let child = Slot::default();
    let app = tauri::Builder::default()
        .setup({
            let child = child.clone();
            move |app| {
                // Made here rather than in tauri.conf.json so it can start on
                // the bundled page: the server's address is not known yet.
                let window =
                    WebviewWindowBuilder::new(app, "main", WebviewUrl::App("index.html".into()))
                        .title(app.package_info().name.clone())
                        .inner_size(1280.0, 820.0)
                        .min_inner_size(640.0, 480.0)
                        .build()?;
                std::thread::spawn(move || connect(&window, config, &child));
                exit_on_signal(app.handle().clone());
                Ok(())
            }
        })
        .build(context)
        .expect("the shell could not start");

    // The last window closing, Cmd-Q, a signal: they all end in `Exit`, and
    // the child goes with the shell.
    app.run(move |_, event| {
        if let RunEvent::Exit = event {
            if let Some(stopped) = child.stop() {
                eprintln!("desktop: app stopped: {}", stopped.how);
            }
        }
    });
}

/// Point the window at the server, or at the error page saying why not.
/// Runs on its own thread: waiting for a child must not hold the event loop.
fn connect(window: &WebviewWindow, config: Result<Config, String>, child: &Slot) {
    let go = |url: Url| {
        if let Err(e) = window.navigate(url) {
            eprintln!("desktop: the window would not navigate: {e}");
        }
    };
    let fail = |why: String| {
        eprintln!("desktop: {why}");
        go(error_page(&why));
    };
    let local = match config {
        Err(why) => return fail(why),
        Ok(Config::Remote(remote)) => {
            return match reachable(&remote.url) {
                Ok(()) => go(remote.url),
                Err(why) => fail(why),
            };
        }
        Ok(Config::Local(local)) => local,
    };
    let link = child
        .start(&local)
        .and_then(|started| child.launch_link(&started))
        .and_then(|link| Url::parse(&link).map_err(|e| format!("{link}：{e}")));
    match link {
        Ok(link) => go(link),
        Err(why) => return fail(why),
    }
    // Otherwise the page would sit on a dead connection, retrying, with
    // nothing to say the app behind it has gone.
    loop {
        std::thread::sleep(Duration::from_millis(500));
        match child.poll() {
            Running::Yes => {}
            Running::Exited(how) => return fail(format!("应用已退出（{how}）")),
            Running::Released => return,
        }
    }
}

/// The bundled page, in its error state. Where Tauri serves `dist/` depends
/// on the platform: a custom scheme where the webview allows one, a reserved
/// host on Windows, whose WebView2 does not.
fn error_page(why: &str) -> Url {
    let base = if cfg!(windows) {
        "http://tauri.localhost/index.html"
    } else {
        "tauri://localhost/index.html"
    };
    let mut url = Url::parse(base).expect("a constant URL");
    url.query_pairs_mut()
        .append_pair("state", "error")
        .append_pair("detail", why);
    url
}

/// Whether anything answers at the remote address. Only a TCP connect: TLS
/// and pairing are the page's business, and a webview's own error for an
/// unreachable host says less than the bundled page does.
fn reachable(url: &Url) -> Result<(), String> {
    let (Some(host), Some(port)) = (url.host_str(), url.port_or_known_default()) else {
        return Err(format!("{url} 里没有主机"));
    };
    let host = host.trim_start_matches('[').trim_end_matches(']');
    let addrs = (host, port)
        .to_socket_addrs()
        .map_err(|e| format!("解析不了 {host}：{e}"))?;
    for addr in addrs {
        if TcpStream::connect_timeout(&addr, Duration::from_secs(5)).is_ok() {
            return Ok(());
        }
    }
    Err(format!("连不上 {url}"))
}

/// SIGINT, SIGTERM, SIGHUP: exit through the event loop, so the child is
/// stopped as it is when the window closes, instead of being left running.
#[cfg(unix)]
fn exit_on_signal(app: tauri::AppHandle) {
    use signal_hook::consts::{SIGHUP, SIGINT, SIGTERM};
    match signal_hook::iterator::Signals::new([SIGINT, SIGTERM, SIGHUP]) {
        Ok(mut signals) => {
            std::thread::spawn(move || {
                if signals.forever().next().is_some() {
                    app.exit(0);
                }
            });
        }
        Err(e) => eprintln!("desktop: signals not handled: {e}"),
    }
}

#[cfg(not(unix))]
fn exit_on_signal(_app: tauri::AppHandle) {}

/// `--smoke`: everything but the window. Exit status 0 when the page was
/// reached the way the window would reach it, and the child then stopped
/// cleanly.
fn smoke(config: Result<Config, String>) -> i32 {
    let ok = match config {
        Err(why) => report(false, format!("config: {why}")),
        Ok(Config::Remote(remote)) => match reachable(&remote.url) {
            Ok(()) => report(true, format!("remote {} answers", remote.url)),
            Err(why) => report(false, format!("remote: {why}")),
        },
        Ok(Config::Local(local)) => {
            println!("     program {}", local.program.display());
            let child = Slot::default();
            match child.start(&local) {
                Err(why) => report(false, format!("start: {why}")),
                Ok(started) => {
                    let admitted = match child.launch_link(&started) {
                        Err(why) => report(false, format!("launch link: {why}")),
                        Ok(link) => {
                            report(true, format!("launch link {link}"));
                            follow(started.addr, &link)
                        }
                    };
                    let stopped = match child.stop() {
                        Some(stopped) => report(stopped.clean, format!("stop: {}", stopped.how)),
                        None => report(false, "stop: the app had already exited".into()),
                    };
                    admitted && stopped
                }
            }
        }
    };
    if ok {
        0
    } else {
        1
    }
}

/// Follow a launch link as the window does: the link sets a cookie, the
/// cookie admits the page, the page is refused without it, and the link does
/// not work twice.
fn follow(addr: SocketAddr, link: &str) -> bool {
    let path = &link[link.find("/auth/").unwrap_or(0)..];
    let redeemed = match local::get(addr, path, None) {
        Ok(response) => response,
        Err(e) => return report(false, format!("GET /auth/<code>: {e}")),
    };
    let Some(cookie) = local::set_cookie(&redeemed.head) else {
        return report(
            false,
            format!("GET /auth/<code> → {}, no cookie", redeemed.status),
        );
    };
    let status = |path: &str, cookie: Option<&str>| {
        local::get(addr, path, cookie)
            .map(|r| (r.status, r.body.contains("<html")))
            .unwrap_or((0, false))
    };
    report(
        redeemed.status == 200,
        format!("GET /auth/<code> → {}, cookie set", redeemed.status),
    ) && {
        let (code, html) = status("/", Some(&cookie));
        report(
            code == 200 && html,
            format!("GET / with the cookie → {code}"),
        )
    } && {
        let (code, _) = status("/", None);
        report(code == 401, format!("GET / without it → {code}"))
    } && {
        let (code, _) = status(path, None);
        report(code == 401, format!("the same link again → {code}"))
    }
}

fn report(ok: bool, what: String) -> bool {
    println!("{} {what}", if ok { "ok  " } else { "FAIL" });
    ok
}
