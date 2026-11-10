//! `harnix.process`: this process's versions, uptime, memory, threads and who is connected — the
//! system space's strip. A method, and a topic computed again every few seconds while anyone
//! watches it: memory, threads and connections change without an event to say so.

use std::sync::Arc;
use std::time::{Duration, Instant};

use harnix_hub::Hub;
use harnix_kernel::{Rpc, TopicSource};
use serde_json::{json, Value};

/// How often the topic is computed again while watched.
const EVERY: Duration = Duration::from_secs(5);

pub(crate) struct Process {
    /// What does not change while it runs: the application's name and version, Harnix's and
    /// Agix's, when it started.
    facts: Value,
    started: Instant,
    /// Who is connected: the hub registers the clients.
    hub: Arc<Hub>,
}

impl Process {
    pub fn new(facts: Value, hub: Arc<Hub>) -> Self {
        Self {
            facts,
            started: Instant::now(),
            hub,
        }
    }

    /// Versions, uptime, memory and who is connected.
    pub fn view(&self) -> Value {
        let mut out = self.facts.clone();
        if let Some(obj) = out.as_object_mut() {
            obj.insert("uptime_secs".into(), json!(self.started.elapsed().as_secs()));
            obj.insert("peak_rss_bytes".into(), json!(peak_rss_bytes()));
            obj.insert("threads".into(), json!(os_threads()));
            obj.insert("connections".into(), json!(self.hub.connections()));
        }
        out
    }
}

#[async_trait::async_trait]
impl TopicSource for Process {
    async fn compute(&self, _params: &Value) -> Rpc<Value> {
        Ok(self.view())
    }

    fn every(&self) -> Option<Duration> {
        Some(EVERY)
    }
}

/// The most memory this process has held, in bytes; `None` where the platform does not say.
///
/// The peak rather than the current figure: it is what `getrusage` gives on every Unix without
/// a dependency, and for "is this process growing" it is the more useful of the two.
fn peak_rss_bytes() -> Option<u64> {
    #[cfg(unix)]
    {
        let mut usage = std::mem::MaybeUninit::<libc::rusage>::zeroed();
        // SAFETY: `getrusage` writes the whole struct for RUSAGE_SELF and touches nothing else.
        let rc = unsafe { libc::getrusage(libc::RUSAGE_SELF, usage.as_mut_ptr()) };
        if rc != 0 {
            return None;
        }
        let max = unsafe { usage.assume_init() }.ru_maxrss.max(0) as u64;
        // macOS reports bytes, Linux and the BSDs kilobytes.
        Some(if cfg!(target_os = "macos") {
            max
        } else {
            max * 1024
        })
    }
    #[cfg(not(unix))]
    {
        None
    }
}

/// How many threads the operating system counts in this process — the runtime's workers, the
/// blocking pool, and whatever the engine and its tools started.
fn os_threads() -> Option<u64> {
    #[cfg(target_os = "macos")]
    {
        let mut info = std::mem::MaybeUninit::<libc::proc_taskinfo>::zeroed();
        let size = std::mem::size_of::<libc::proc_taskinfo>() as libc::c_int;
        // SAFETY: `proc_pidinfo` writes at most `size` bytes into `info` and reports how many.
        let wrote = unsafe {
            libc::proc_pidinfo(
                libc::getpid(),
                libc::PROC_PIDTASKINFO,
                0,
                info.as_mut_ptr().cast(),
                size,
            )
        };
        if wrote != size {
            return None;
        }
        Some(unsafe { info.assume_init() }.pti_threadnum.max(0) as u64)
    }
    #[cfg(target_os = "linux")]
    {
        std::fs::read_to_string("/proc/self/status")
            .ok()?
            .lines()
            .find_map(|line| line.strip_prefix("Threads:"))
            .and_then(|n| n.trim().parse().ok())
    }
    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_process_counts_its_threads() {
        if cfg!(any(target_os = "macos", target_os = "linux")) {
            let _worker = std::thread::spawn(|| std::thread::sleep(std::time::Duration::from_millis(200)));
            assert!(os_threads().is_some_and(|n| n >= 2));
        }
    }
}
