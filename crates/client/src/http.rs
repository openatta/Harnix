//! Just enough HTTP/1.1 for one request per connection on a trusted link: a request written
//! whole with `Connection: close`, an answer read to the end, chunked or not.

use std::io::{BufRead, BufReader, Read, Write};
use std::net::{SocketAddr, TcpStream, ToSocketAddrs};
use std::time::Duration;

use crate::ClientError;

/// Where a Harnix process listens: `http://host:port` or `host:port`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Endpoint {
    pub host: String,
    pub port: u16,
}

impl Endpoint {
    pub fn parse(url: &str) -> Result<Self, ClientError> {
        let rest = url.trim().trim_end_matches('/');
        if rest.starts_with("https://") {
            return Err(ClientError::transport(format!(
                "{url}: harnix-client speaks plain HTTP only (a local or LAN process)"
            )));
        }
        let rest = rest.strip_prefix("http://").unwrap_or(rest);
        let (host, port) = rest
            .rsplit_once(':')
            .and_then(|(h, p)| p.parse::<u16>().ok().map(|p| (h, p)))
            .ok_or_else(|| ClientError::transport(format!("{url}: expected http://host:port")))?;
        Ok(Self {
            host: host.trim_start_matches('[').trim_end_matches(']').to_string(),
            port,
        })
    }

    pub fn url(&self) -> String {
        if self.host.contains(':') {
            format!("http://[{}]:{}", self.host, self.port)
        } else {
            format!("http://{}:{}", self.host, self.port)
        }
    }

    fn addr(&self) -> Result<SocketAddr, ClientError> {
        (self.host.as_str(), self.port)
            .to_socket_addrs()
            .map_err(|e| ClientError::transport(format!("{}: {e}", self.url())))?
            .next()
            .ok_or_else(|| ClientError::transport(format!("{}: no address", self.url())))
    }

    pub(crate) fn connect(&self, wait: Duration) -> Result<TcpStream, ClientError> {
        let stream = TcpStream::connect_timeout(&self.addr()?, wait.min(Duration::from_secs(5)))
            .map_err(|e| ClientError::transport(format!("{}: {e}", self.url())))?;
        stream
            .set_read_timeout(Some(wait))
            .and_then(|_| stream.set_write_timeout(Some(wait)))
            .map_err(|e| ClientError::transport(e.to_string()))?;
        Ok(stream)
    }
}

pub(crate) struct Response {
    pub status: u16,
    pub body: Vec<u8>,
}

pub(crate) fn head(
    endpoint: &Endpoint,
    method: &str,
    path: &str,
    bearer: Option<&str>,
    extra: &str,
    body_len: usize,
) -> String {
    let auth = bearer
        .map(|b| format!("Authorization: Bearer {b}\r\n"))
        .unwrap_or_default();
    format!(
        "{method} {path} HTTP/1.1\r\nHost: {}:{}\r\n{auth}{extra}Content-Length: {body_len}\r\n\r\n",
        endpoint.host, endpoint.port
    )
}

pub(crate) fn post(
    endpoint: &Endpoint,
    path: &str,
    bearer: Option<&str>,
    body: &[u8],
    wait: Duration,
) -> Result<Response, ClientError> {
    let mut stream = endpoint.connect(wait)?;
    let head = head(
        endpoint,
        "POST",
        path,
        bearer,
        "Content-Type: application/json\r\nConnection: close\r\n",
        body.len(),
    );
    stream
        .write_all(head.as_bytes())
        .and_then(|_| stream.write_all(body))
        .map_err(|e| ClientError::transport(format!("{path}: {e}")))?;
    let mut reader = BufReader::new(stream);
    let (status, headers) = read_head(&mut reader).map_err(|e| ClientError::transport(format!("{path}: {e}")))?;
    let body = read_body(&mut reader, &headers).map_err(|e| ClientError::transport(format!("{path}: {e}")))?;
    Ok(Response { status, body })
}

/// The status and the headers (names lower-cased).
pub(crate) fn read_head(reader: &mut impl BufRead) -> std::io::Result<(u16, Vec<(String, String)>)> {
    let mut line = String::new();
    reader.read_line(&mut line)?;
    let status = line
        .split_whitespace()
        .nth(1)
        .and_then(|s| s.parse().ok())
        .ok_or_else(|| std::io::Error::new(std::io::ErrorKind::InvalidData, format!("not HTTP: {line:?}")))?;
    let mut headers = Vec::new();
    loop {
        line.clear();
        if reader.read_line(&mut line)? == 0 {
            break;
        }
        let trimmed = line.trim_end();
        if trimmed.is_empty() {
            break;
        }
        if let Some((name, value)) = trimmed.split_once(':') {
            headers.push((name.trim().to_ascii_lowercase(), value.trim().to_string()));
        }
    }
    Ok((status, headers))
}

pub(crate) fn header<'a>(headers: &'a [(String, String)], name: &str) -> Option<&'a str> {
    headers
        .iter()
        .find(|(n, _)| n == name)
        .map(|(_, v)| v.as_str())
}

pub(crate) fn is_chunked(headers: &[(String, String)]) -> bool {
    header(headers, "transfer-encoding").is_some_and(|v| v.to_ascii_lowercase().contains("chunked"))
}

fn read_body(reader: &mut impl BufRead, headers: &[(String, String)]) -> std::io::Result<Vec<u8>> {
    let mut body = Vec::new();
    if is_chunked(headers) {
        let mut chunks = Chunked::new(reader);
        chunks.read_to_end(&mut body)?;
    } else if let Some(len) = header(headers, "content-length").and_then(|v| v.parse::<usize>().ok()) {
        body.resize(len, 0);
        reader.read_exact(&mut body)?;
    } else {
        reader.read_to_end(&mut body)?;
    }
    Ok(body)
}

/// A chunked body as a plain byte stream.
pub(crate) struct Chunked<R> {
    inner: R,
    left: usize,
    done: bool,
    /// A size line read up to a timeout: it continues on the next read.
    line: Vec<u8>,
}

impl<R: BufRead> Chunked<R> {
    pub fn new(inner: R) -> Self {
        Self {
            inner,
            left: 0,
            done: false,
            line: Vec::new(),
        }
    }
}

impl<R: BufRead> Read for Chunked<R> {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        if self.done || buf.is_empty() {
            return Ok(0);
        }
        while self.left == 0 {
            if self.inner.read_until(b'\n', &mut self.line)? == 0 {
                self.done = true;
                return Ok(0);
            }
            if !self.line.ends_with(b"\n") {
                continue;
            }
            let line = String::from_utf8_lossy(&std::mem::take(&mut self.line)).into_owned();
            if line.trim().is_empty() {
                // The CRLF that ends the previous chunk.
                continue;
            }
            let size = line.trim().split(';').next().unwrap_or("");
            self.left = usize::from_str_radix(size, 16).map_err(|_| {
                std::io::Error::new(std::io::ErrorKind::InvalidData, format!("bad chunk size {line:?}"))
            })?;
            if self.left == 0 {
                self.done = true;
                return Ok(0);
            }
        }
        let want = buf.len().min(self.left);
        let n = self.inner.read(&mut buf[..want])?;
        if n == 0 {
            self.done = true;
        }
        self.left -= n;
        Ok(n)
    }
}
