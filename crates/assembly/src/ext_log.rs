//! Each extension's log: what this process logged as one extension's, the newest lines kept in a
//! ring bounded per extension and in all (`harnix.extensions.log`, which the system space reads).
//!
//! A line is an extension's when the event names it — the `extension` field (the host's lines
//! about it, and its process's stderr, `stream = "stderr"`), or `app` (a WebAssembly package's
//! log and its carrier's lines about it) — or when it was logged inside the span the host opens
//! around everything the extension is asked to do ([`harnix_kernel::EXTENSION_SPAN`]): a call, a
//! topic, its start and stop. Work an extension spawns itself is outside that span unless it
//! carries it along. Kept apart from the process's own log and its filter: `RUST_LOG` does not
//! decide what an extension's log holds.

use std::collections::{BTreeMap, VecDeque};
use std::fmt::Write as _;
use std::sync::{Arc, LazyLock, Mutex};

use harnix_contract::wire::{ExtensionLog, ExtensionLogCount, ExtensionLogLine, ExtensionLogOrigin};
use harnix_contract::RpcError;
use harnix_kernel::{CallContext, Handler, Rpc, EXTENSION_SPAN};
use serde_json::Value;
use tracing::field::{Field, Visit};
use tracing::span::{Attributes, Id};
use tracing::{Event, Level, Metadata, Subscriber};
use tracing_subscriber::filter::{FilterFn, Filtered};
use tracing_subscriber::layer::{Context, Layer};
use tracing_subscriber::registry::LookupSpan;

/// Lines kept of one extension.
pub const PER_EXTENSION: usize = 500;
/// Lines kept in all: past it, the oldest of any extension goes first.
pub const TOTAL: usize = 4000;
/// Bytes kept of one line's message.
pub const LINE_BYTES: usize = 2048;
/// The most lines one read answers.
pub const READ_MAX: usize = 1000;
/// Lines one read answers when it names no `limit`.
const READ_DEFAULT: usize = 500;

/// This process's extension log: one for the process, as its log is.
pub(crate) static LOG: LazyLock<Arc<ExtLog>> = LazyLock::new(|| Arc::new(ExtLog::default()));

/// The ring.
#[derive(Default)]
pub(crate) struct ExtLog {
    ring: Mutex<Ring>,
}

#[derive(Default)]
struct Ring {
    /// The last `seq` given.
    last: u64,
    dropped: u64,
    total: usize,
    by: BTreeMap<String, VecDeque<ExtensionLogLine>>,
}

/// What one read asks for.
#[derive(Debug, Default, Clone, PartialEq)]
pub(crate) struct Query {
    pub extension: Option<String>,
    /// The least severe level wanted.
    pub level: Option<Level>,
    /// Only lines after this `seq`.
    pub after: u64,
    pub limit: usize,
}

impl ExtLog {
    /// One line of `extension`'s, cut to [`LINE_BYTES`]; the oldest let go past the bounds.
    pub(crate) fn push(&self, extension: &str, level: Level, origin: ExtensionLogOrigin, target: &str, message: String) {
        let mut ring = self.ring.lock().unwrap_or_else(|e| e.into_inner());
        ring.last += 1;
        let line = ExtensionLogLine {
            seq: ring.last,
            at_ms: now_ms(),
            extension: extension.to_string(),
            level: level.as_str().to_ascii_lowercase(),
            origin,
            target: target.to_string(),
            message: cut(message),
        };
        let lines = ring.by.entry(extension.to_string()).or_default();
        lines.push_back(line);
        let over = lines.len() > PER_EXTENSION;
        if over {
            lines.pop_front();
        }
        match over {
            true => ring.dropped += 1,
            false => ring.total += 1,
        }
        while ring.total > TOTAL {
            let oldest = ring
                .by
                .iter()
                .filter_map(|(id, lines)| lines.front().map(|l| (l.seq, id.clone())))
                .min()
                .map(|(_, id)| id);
            let Some(id) = oldest else { break };
            if let Some(lines) = ring.by.get_mut(&id) {
                lines.pop_front();
                if lines.is_empty() {
                    ring.by.remove(&id);
                }
            }
            ring.total -= 1;
            ring.dropped += 1;
        }
    }

    /// The lines `query` asks for, oldest first: the newest `limit` of those after its `after`.
    pub(crate) fn read(&self, query: &Query) -> ExtensionLog {
        let ring = self.ring.lock().unwrap_or_else(|e| e.into_inner());
        let wanted = |line: &&ExtensionLogLine| {
            line.seq > query.after
                && query.level.is_none_or(|least| level_of(&line.level).is_some_and(|l| l <= least))
        };
        let mut lines: Vec<ExtensionLogLine> = match &query.extension {
            Some(id) => ring.by.get(id).into_iter().flatten().filter(wanted).cloned().collect(),
            None => ring.by.values().flatten().filter(wanted).cloned().collect(),
        };
        lines.sort_by_key(|l| l.seq);
        let limit = query.limit.clamp(1, READ_MAX);
        if lines.len() > limit {
            lines.drain(..lines.len() - limit);
        }
        ExtensionLog {
            lines,
            last: ring.last,
            extensions: ring.by.iter().map(|(id, lines)| ExtensionLogCount { extension: id.clone(), lines: lines.len() }).collect(),
            dropped: ring.dropped,
        }
    }
}

/// A read's parameters: `{extension?, level?, after?, limit?}`.
pub(crate) fn query(params: &Value) -> Result<Query, RpcError> {
    let extension = match params.get("extension") {
        None | Some(Value::Null) => None,
        Some(Value::String(id)) if !id.is_empty() => Some(id.clone()),
        Some(_) => return Err(RpcError::invalid_params("`extension` is an extension's id")),
    };
    let level = match params.get("level") {
        None | Some(Value::Null) => None,
        Some(Value::String(name)) => Some(
            level_of(name).ok_or_else(|| RpcError::invalid_params("`level` is one of error, warn, info, debug, trace"))?,
        ),
        Some(_) => return Err(RpcError::invalid_params("`level` is one of error, warn, info, debug, trace")),
    };
    let number = |key: &str| match params.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(v) => v.as_u64().map(Some).ok_or_else(|| RpcError::invalid_params(format!("`{key}` is a whole number"))),
    };
    Ok(Query {
        extension,
        level,
        after: number("after")?.unwrap_or(0),
        limit: number("limit")?.map_or(READ_DEFAULT, |n| n.min(READ_MAX as u64) as usize),
    })
}

fn level_of(name: &str) -> Option<Level> {
    name.parse().ok()
}

/// What a line keeps of its message.
fn cut(mut message: String) -> String {
    if message.len() > LINE_BYTES {
        let mut at = LINE_BYTES;
        while !message.is_char_boundary(at) {
            at -= 1;
        }
        message.truncate(at);
        message.push('…');
    }
    message
}

fn now_ms() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0)
}

/// `harnix.extensions.log`: the ring, read.
pub(crate) struct Read(pub Arc<ExtLog>);

#[async_trait::async_trait]
impl Handler for Read {
    async fn call(&self, _: CallContext, params: Value) -> Rpc<Value> {
        let query = query(&params)?;
        Ok(serde_json::to_value(self.0.read(&query)).unwrap_or(Value::Null))
    }
}

/* ── capture ────────────────────────────────────────────────────────────── */

/// What [`wanted`] is, as the filter holds it.
type Wanted = fn(&Metadata<'_>) -> bool;

/// The layer that fills the ring, with its own filter: the extension spans, and the events at
/// `info` and above — and any that names an extension, whatever its level.
pub(crate) fn layer<S>(log: Arc<ExtLog>) -> Filtered<Capture, FilterFn<Wanted>, S>
where
    S: Subscriber + for<'a> LookupSpan<'a>,
{
    Capture { log }.with_filter(FilterFn::new(wanted as Wanted))
}

fn wanted(meta: &Metadata<'_>) -> bool {
    if meta.is_span() {
        return meta.name() == EXTENSION_SPAN;
    }
    *meta.level() <= Level::INFO || meta.fields().field("extension").is_some() || meta.fields().field("app").is_some()
}

/// Fills the ring from what is logged.
pub(crate) struct Capture {
    log: Arc<ExtLog>,
}

/// The extension an [`EXTENSION_SPAN`] is about, kept on the span.
struct Owner(String);

impl<S> Layer<S> for Capture
where
    S: Subscriber + for<'a> LookupSpan<'a>,
{
    fn on_new_span(&self, attrs: &Attributes<'_>, id: &Id, ctx: Context<'_, S>) {
        if attrs.metadata().name() != EXTENSION_SPAN {
            return;
        }
        let mut fields = Fields::default();
        attrs.record(&mut fields);
        if let (Some(owner), Some(span)) = (fields.extension, ctx.span(id)) {
            span.extensions_mut().insert(Owner(owner));
        }
    }

    fn on_event(&self, event: &Event<'_>, ctx: Context<'_, S>) {
        let mut fields = Fields::default();
        event.record(&mut fields);
        let (owner, origin) = match (fields.extension.take(), fields.app.take()) {
            (Some(id), _) if fields.stderr => (id, ExtensionLogOrigin::Process),
            (Some(id), _) => (id, ExtensionLogOrigin::Host),
            (None, Some(app)) => (app, ExtensionLogOrigin::Package),
            (None, None) => {
                // The innermost extension span: a call one extension makes into another is the
                // other's while it answers.
                let Some(scope) = ctx.event_scope(event) else { return };
                let owner = scope.into_iter().find_map(|span| span.extensions().get::<Owner>().map(|o| o.0.clone()));
                match owner {
                    Some(id) => (id, ExtensionLogOrigin::Call),
                    None => return,
                }
            }
        };
        let meta = event.metadata();
        self.log.push(&owner, *meta.level(), origin, meta.target(), fields.text());
    }
}

/// An event's fields, as a line: the message, then the rest as `name=value`; the ones that say
/// whose it is taken out.
#[derive(Default)]
struct Fields {
    message: String,
    rest: String,
    extension: Option<String>,
    app: Option<String>,
    stderr: bool,
}

impl Fields {
    fn text(self) -> String {
        match (self.message.is_empty(), self.rest.is_empty()) {
            (_, true) => self.message,
            (true, false) => self.rest,
            (false, false) => format!("{} {}", self.message, self.rest),
        }
    }

    fn put(&mut self, field: &Field, value: String) {
        match field.name() {
            "message" => self.message = value,
            "extension" => self.extension = Some(value),
            "app" => self.app = Some(value),
            "stream" => self.stderr = value == "stderr",
            // Whose process it was: the line is the extension's whatever its pid.
            "pid" => {}
            name => {
                if !self.rest.is_empty() {
                    self.rest.push(' ');
                }
                let _ = write!(self.rest, "{name}={value}");
            }
        }
    }
}

impl Visit for Fields {
    fn record_str(&mut self, field: &Field, value: &str) {
        self.put(field, value.to_string());
    }

    fn record_debug(&mut self, field: &Field, value: &dyn std::fmt::Debug) {
        self.put(field, format!("{value:?}"));
    }
}

#[cfg(test)]
mod tests {
    use tracing_subscriber::layer::SubscriberExt;

    use super::*;

    fn push(log: &ExtLog, ext: &str, n: usize) {
        for i in 0..n {
            log.push(ext, Level::INFO, ExtensionLogOrigin::Call, "t", format!("{ext} {i}"));
        }
    }

    fn all() -> Query {
        Query { limit: READ_MAX, ..Query::default() }
    }

    #[test]
    fn one_extension_keeps_its_newest_lines() {
        let log = ExtLog::default();
        push(&log, "chatty", PER_EXTENSION + 7);
        let read = log.read(&all());
        assert_eq!(read.extensions, [ExtensionLogCount { extension: "chatty".into(), lines: PER_EXTENSION }]);
        assert_eq!(read.dropped, 7);
        assert_eq!(read.last, (PER_EXTENSION + 7) as u64);
        let first = log.read(&Query { limit: 1, extension: Some("chatty".into()), after: 0, level: None });
        assert_eq!(first.lines[0].message, format!("chatty {}", PER_EXTENSION + 6), "the newest of what is asked");
    }

    #[test]
    fn past_the_total_the_oldest_of_any_extension_goes_first() {
        let log = ExtLog::default();
        push(&log, "early", 10);
        let others = TOTAL / PER_EXTENSION;
        for n in 0..others {
            push(&log, &format!("x{n:02}"), PER_EXTENSION);
        }
        let read = log.read(&all());
        let total: usize = read.extensions.iter().map(|c| c.lines).sum();
        assert_eq!(total, TOTAL);
        assert!(read.extensions.iter().all(|c| c.extension != "early"), "the oldest lines were the early extension's");
        assert_eq!(read.dropped, 10);
    }

    #[test]
    fn a_read_filters_by_extension_level_and_seq() {
        let log = ExtLog::default();
        log.push("a", Level::ERROR, ExtensionLogOrigin::Host, "t", "a broke".into());
        log.push("b", Level::DEBUG, ExtensionLogOrigin::Package, "t", "b detail".into());
        log.push("a", Level::INFO, ExtensionLogOrigin::Call, "t", "a fine".into());
        let messages = |q: Query| log.read(&q).lines.into_iter().map(|l| l.message).collect::<Vec<_>>();
        assert_eq!(messages(all()), ["a broke", "b detail", "a fine"]);
        assert_eq!(messages(Query { extension: Some("a".into()), ..all() }), ["a broke", "a fine"]);
        assert_eq!(messages(Query { level: Some(Level::WARN), ..all() }), ["a broke"]);
        assert_eq!(messages(Query { level: Some(Level::INFO), ..all() }), ["a broke", "a fine"]);
        assert_eq!(messages(Query { after: 2, ..all() }), ["a fine"]);
        assert!(messages(Query { extension: Some("nobody".into()), ..all() }).is_empty());
        assert_eq!(log.read(&all()).lines[0].level, "error");
    }

    #[test]
    fn a_long_line_is_cut_on_a_character() {
        let log = ExtLog::default();
        log.push("a", Level::INFO, ExtensionLogOrigin::Call, "t", "é".repeat(LINE_BYTES));
        let message = &log.read(&all()).lines[0].message;
        assert!(message.len() <= LINE_BYTES + '…'.len_utf8() && message.ends_with('…'));
    }

    #[test]
    fn the_parameters_are_checked() {
        let q = query(&serde_json::json!({"extension": "a", "level": "warn", "after": 3, "limit": 5000})).unwrap();
        assert_eq!(q, Query { extension: Some("a".into()), level: Some(Level::WARN), after: 3, limit: READ_MAX });
        assert_eq!(query(&serde_json::json!({})).unwrap().limit, READ_DEFAULT);
        assert!(query(&serde_json::json!({"level": "loud"})).is_err());
        assert!(query(&serde_json::json!({"after": -1})).is_err());
        assert!(query(&serde_json::json!({"extension": 3})).is_err());
    }

    /// Who a line is, by the span it was logged in or the field that names it; a line of nobody's
    /// is not kept, nor one below `info` that names nobody.
    #[test]
    fn each_line_goes_to_the_extension_it_is_about() {
        let log = Arc::new(ExtLog::default());
        let subscriber = tracing_subscriber::registry().with(layer(log.clone()));
        tracing::subscriber::with_default(subscriber, || {
            tracing::info!("nobody's");
            let outer = tracing::info_span!(EXTENSION_SPAN, extension = %"flow");
            let _o = outer.enter();
            tracing::info!(step = 2, "flow answers");
            tracing::debug!("flow's detail, below what is kept");
            {
                let inner = tracing::info_span!(EXTENSION_SPAN, extension = "settings");
                let _i = inner.enter();
                tracing::warn!("settings answers flow's call");
            }
            tracing::info!(extension = "acme.proc", stream = "stderr", pid = 7, "from its stderr");
            tracing::debug!(app = "acme.wasm", "a package's own debug line");
            tracing::error!(extension = "acme.proc", "the host on it");
        });
        let lines = log.read(&all()).lines;
        let seen: Vec<(&str, ExtensionLogOrigin, &str)> =
            lines.iter().map(|l| (l.extension.as_str(), l.origin, l.message.as_str())).collect();
        assert_eq!(
            seen,
            [
                ("flow", ExtensionLogOrigin::Call, "flow answers step=2"),
                ("settings", ExtensionLogOrigin::Call, "settings answers flow's call"),
                ("acme.proc", ExtensionLogOrigin::Process, "from its stderr"),
                ("acme.wasm", ExtensionLogOrigin::Package, "a package's own debug line"),
                ("acme.proc", ExtensionLogOrigin::Host, "the host on it"),
            ]
        );
        assert_eq!(lines[3].level, "debug");
    }
}
