//! Against a stand-in server on a loopback socket: each test says what the server answers.

use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpListener;
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

use serde_json::{json, Value};

use super::*;

/// How the server answers one connection.
type Answer = Box<dyn FnOnce(&mut std::net::TcpStream) + Send>;

/// What one connection received: the request line, headers, body.
#[derive(Debug)]
struct Seen {
    line: String,
    headers: Vec<String>,
    body: String,
}

/// A server that answers each connection with the next of `answers` (a function of the stream,
/// so it can write slowly), and reports what it was sent.
fn serve(answers: Vec<Answer>) -> (String, mpsc::Receiver<Seen>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let (tx, rx) = mpsc::channel();
    thread::spawn(move || {
        for answer in answers {
            let (mut stream, _) = listener.accept().unwrap();
            let mut reader = BufReader::new(stream.try_clone().unwrap());
            let mut line = String::new();
            reader.read_line(&mut line).unwrap();
            let mut headers = Vec::new();
            let mut len = 0;
            loop {
                let mut h = String::new();
                reader.read_line(&mut h).unwrap();
                let h = h.trim_end().to_string();
                if h.is_empty() {
                    break;
                }
                if let Some(v) = h.to_ascii_lowercase().strip_prefix("content-length:") {
                    len = v.trim().parse().unwrap();
                }
                headers.push(h);
            }
            let mut body = vec![0; len];
            reader.read_exact(&mut body).unwrap();
            tx.send(Seen {
                line: line.trim_end().to_string(),
                headers,
                body: String::from_utf8(body).unwrap(),
            })
            .unwrap();
            answer(&mut stream);
        }
    });
    (url, rx)
}

fn plain(status: u16, body: Value) -> Answer {
    Box::new(move |s| {
        let body = body.to_string();
        write!(
            s,
            "HTTP/1.1 {status} X\r\ncontent-type: application/json\r\ncontent-length: {}\r\n\r\n{body}",
            body.len()
        )
        .unwrap();
    })
}

fn chunked(status: u16, parts: Vec<String>) -> Answer {
    Box::new(move |s| {
        write!(s, "HTTP/1.1 {status} X\r\ntransfer-encoding: chunked\r\n\r\n").unwrap();
        for part in parts {
            write!(s, "{:x}\r\n{part}\r\n", part.len()).unwrap();
        }
        write!(s, "0\r\n\r\n").unwrap();
    })
}

fn handshake_ok() -> Answer {
    plain(200, json!({"credential": "cred-1", "client": 7}))
}

#[test]
fn connect_hands_over_the_token_and_the_versions_this_crate_speaks() {
    let (url, seen) = serve(vec![handshake_ok()]);
    let client = Client::connect(&url, "tok").unwrap();
    let request = seen.recv().unwrap();
    assert_eq!(request.line, "POST /handshake HTTP/1.1");
    let body: Value = serde_json::from_str(&request.body).unwrap();
    assert_eq!(
        body,
        json!({"token": "tok", "protocol_version": PROTOCOL_VERSION,
               "contrib_api_version": CONTRIB_API_VERSION, "topology": "split_streams"})
    );
    assert_eq!(client.url(), url);
    assert_eq!(client.token(), "tok");
}

#[test]
fn a_refused_handshake_says_why() {
    let (url, _seen) = serve(vec![plain(401, json!("bad token"))]);
    let e = Client::connect(&url, "nope").unwrap_err();
    assert_eq!(e.kind, ErrorKind::Handshake);
    assert_eq!(e.code, 401);
    let (url, _seen) = serve(vec![plain(
        200,
        json!({"error": {"code": -31001, "message": "protocol 3 is not 4"}}),
    )]);
    let e = Client::connect(&url, "tok").unwrap_err();
    assert_eq!((e.kind, e.code), (ErrorKind::Handshake, -31001));
    assert!(e.message.contains("protocol"));
}

#[test]
fn a_call_carries_the_credential_and_reads_a_chunked_answer() {
    let answer = json!({"jsonrpc": "2.0", "id": 1, "result": {"ok": "中文 ✓"}}).to_string();
    let (a, b) = answer.split_at(10);
    let (url, seen) = serve(vec![handshake_ok(), chunked(200, vec![a.to_string(), b.to_string()])]);
    let client = Client::connect(&url, "tok").unwrap();
    let result = client.call("app.x.do", json!({"n": 1})).unwrap();
    assert_eq!(result, json!({"ok": "中文 ✓"}));
    seen.recv().unwrap();
    let request = seen.recv().unwrap();
    assert_eq!(request.line, "POST /rpc HTTP/1.1");
    assert!(request.headers.iter().any(|h| h == "Authorization: Bearer cred-1"));
    let body: Value = serde_json::from_str(&request.body).unwrap();
    assert_eq!(body["method"], "app.x.do");
    assert_eq!(body["params"], json!({"n": 1}));
}

#[test]
fn an_error_answer_keeps_its_code_and_data_and_an_unknown_credential_is_unauthenticated() {
    let (url, _seen) = serve(vec![
        handshake_ok(),
        plain(200, json!({"jsonrpc": "2.0", "id": 1, "error": {"code": -32602, "message": "no", "data": {"code": "validation"}}})),
        plain(200, json!({"jsonrpc": "2.0", "id": null, "error": {"code": harnix_contract::codes::HANDSHAKE_REFUSED, "message": "unknown credential; call /handshake first"}})),
    ]);
    let client = Client::connect(&url, "tok").unwrap();
    let e = client.call("m", json!({})).unwrap_err();
    assert_eq!((e.kind, e.code), (ErrorKind::Rpc, -32602));
    assert_eq!(e.data["code"], "validation");
    assert!(e.to_string().contains("[validation]"));
    let e = client.call("m", json!({})).unwrap_err();
    assert_eq!(e.kind, ErrorKind::Unauthenticated);
}

#[test]
fn nothing_listening_is_a_transport_error() {
    let port = TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port();
    let e = Client::connect(&format!("127.0.0.1:{port}"), "tok").unwrap_err();
    assert_eq!(e.kind, ErrorKind::Transport);
    assert_eq!(Endpoint::parse("https://x:1").unwrap_err().kind, ErrorKind::Transport);
    assert_eq!(Endpoint::parse("http://[::1]:9/").unwrap().url(), "http://[::1]:9");
}

#[test]
fn a_launch_link_is_the_full_url() {
    let (url, seen) = serve(vec![plain(200, json!({"code": "c1", "path": "/auth/c1", "expires_in_secs": 60}))]);
    assert_eq!(launch_link(&url, "tok").unwrap(), format!("{url}/auth/c1"));
    let request = seen.recv().unwrap();
    assert_eq!(request.line, "POST /auth/link HTTP/1.1");
    assert!(request.headers.iter().any(|h| h == "Authorization: Bearer tok"));
}

#[test]
fn the_event_stream_gives_frames_across_slow_writes_and_idles_between_them() {
    let frame_a = json!({"method": "harnix.event", "params": {"seq": 1, "text": "多字节"}}).to_string();
    let frame_b = json!({"method": "harnix.turn_settled", "params": {"turn_id": "t"}}).to_string();
    let (go_on, wait) = mpsc::channel::<()>();
    let (url, seen) = serve(vec![
        handshake_ok(),
        Box::new(move |s| {
            write!(s, "HTTP/1.1 200 OK\r\ncontent-type: text/event-stream\r\ntransfer-encoding: chunked\r\n\r\n").unwrap();
            let send = |s: &mut std::net::TcpStream, bytes: &[u8]| {
                write!(s, "{:x}\r\n", bytes.len()).unwrap();
                s.write_all(bytes).unwrap();
                s.write_all(b"\r\n").unwrap();
                s.flush().unwrap();
            };
            send(s, b":\n\n");
            let event = format!("data: {frame_a}\n\n");
            // Split inside a multi-byte character, with a pause longer than the client's wait.
            let cut = event.find("多").unwrap() + 1;
            send(s, &event.as_bytes()[..cut]);
            wait.recv().unwrap();
            send(s, &event.as_bytes()[cut..]);
            send(s, format!("data: {frame_b}\n\n").as_bytes());
            wait.recv().unwrap();
        }),
    ]);
    let client = Client::connect(&url, "tok").unwrap();
    let mut events = client.events(Face::Session, Duration::from_millis(100)).unwrap();
    assert_eq!(events.next().unwrap(), StreamItem::Idle, "half an event is not one");
    go_on.send(()).unwrap();
    let mut frames = Vec::new();
    while frames.len() < 2 {
        if let StreamItem::Frame(f) = events.next().unwrap() {
            frames.push(f);
        }
    }
    assert_eq!(frames[0]["params"]["text"], "多字节");
    assert_eq!(frames[1]["method"], "harnix.turn_settled");
    seen.recv().unwrap();
    let request = seen.recv().unwrap();
    assert_eq!(request.line, "GET /events/session HTTP/1.1");
    assert!(request.headers.iter().any(|h| h == "Authorization: Bearer cred-1"));
    go_on.send(()).unwrap();
    // The server is gone: the stream says so.
    let ended = loop {
        match events.next() {
            Ok(_) => continue,
            Err(e) => break e,
        }
    };
    assert_eq!(ended.kind, ErrorKind::Transport);
}

#[test]
fn a_stream_refused_for_its_credential_is_unauthenticated() {
    let (url, _seen) = serve(vec![handshake_ok(), plain(401, json!("unknown credential"))]);
    let client = Client::connect(&url, "tok").unwrap();
    let e = client.events(Face::Host, Duration::from_millis(50)).err().unwrap();
    assert_eq!(e.kind, ErrorKind::Unauthenticated);
}
