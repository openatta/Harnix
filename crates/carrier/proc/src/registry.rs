//! What is running: the packages whose processes are up, each with the wire it is reached by, read
//! by the tool provider (`tools.rs`), the routes (`routes.rs`) and the interface entries
//! (`frames.rs`). A package is in it exactly while its process answers: one whose process exited
//! is out until it is up again. One whose process was let go while idle stays in, asleep: what
//! needs its process wakes it ([`Registry::awake`]), and the rest — its interface, its tools as
//! sessions list them — go on as they were.

use std::collections::{BTreeMap, HashMap};
use std::ops::Deref;
use std::sync::{Arc, Mutex, RwLock};
use std::time::{Duration, Instant};

use harnix_carrier_shared::sync::{lock, read, write};
use harnix_carrier_shared::{Loaded, Roster};
use harnix_host_api::Waker;
use harnix_wasm_port::Control;

/// What is running, shared with the tool provider and the routes.
#[derive(Default)]
pub struct Registry {
    roster: Roster,
    /// Each running package's process, as its wire reaches it.
    controls: RwLock<BTreeMap<String, Arc<dyn Control>>>,
    /// Packages whose processes were let go while idle, with what wakes each.
    sleeping: RwLock<BTreeMap<String, Arc<dyn Waker>>>,
    /// What the product has in flight into each package's process, and when the last of it ended.
    activity: Mutex<HashMap<String, (usize, Instant)>>,
}

/// One call into a package's process in flight, for as long as it is held.
pub(crate) struct Held<'a> {
    registry: &'a Registry,
    id: String,
}

impl Drop for Held<'_> {
    fn drop(&mut self) {
        if let Some((busy, last)) = lock(&self.registry.activity).get_mut(&self.id) {
            *busy = busy.saturating_sub(1);
            *last = Instant::now();
        }
    }
}

/// The roster (`loaded`, `get`, `progress`, `listen`, …) is the shared one's.
impl Deref for Registry {
    type Target = Roster;

    fn deref(&self) -> &Roster {
        &self.roster
    }
}

impl Registry {
    /// Its process is up: what reaches it goes over `control`.
    pub(crate) fn insert(&self, loaded: Arc<Loaded>, control: Arc<dyn Control>) {
        write(&self.sleeping).remove(&loaded.id);
        lock(&self.activity).entry(loaded.id.clone()).or_insert((0, Instant::now())).1 = Instant::now();
        write(&self.controls).insert(loaded.id.clone(), control);
        self.roster.insert(loaded);
    }

    pub(crate) fn remove(&self, id: &str) -> Option<Arc<Loaded>> {
        write(&self.sleeping).remove(id);
        write(&self.controls).remove(id);
        self.roster.remove(id)
    }

    /// Its process was let go while idle: still listed, and woken by `waker` for what needs it.
    pub(crate) fn sleep(&self, id: &str, waker: Arc<dyn Waker>) {
        write(&self.controls).remove(id);
        write(&self.sleeping).insert(id.to_string(), waker);
    }

    /// The process that runs `id` now, while it is up.
    pub fn control(&self, id: &str) -> Option<Arc<dyn Control>> {
        read(&self.controls).get(id).cloned()
    }

    /// The process that runs `id`, woken first when it sleeps; `None` when it is not up and does
    /// not wake.
    pub(crate) async fn awake(&self, id: &str) -> Option<Arc<dyn Control>> {
        if let Some(control) = self.control(id) {
            return Some(control);
        }
        let waker = read(&self.sleeping).get(id).cloned()?;
        if let Err(e) = waker.wake().await {
            tracing::warn!(package = %id, error = %e.message, "its extension process did not wake");
            return None;
        }
        self.control(id)
    }

    /// A call into `id`'s process, in flight until the guard drops.
    pub(crate) fn hold(&self, id: &str) -> Held<'_> {
        lock(&self.activity).entry(id.to_string()).or_insert((0, Instant::now())).0 += 1;
        Held { registry: self, id: id.to_string() }
    }

    /// How long nothing has been in flight into `id`'s process; `None` while something is.
    pub(crate) fn idle_for(&self, id: &str) -> Option<Duration> {
        match lock(&self.activity).get(id) {
            Some((0, last)) => Some(last.elapsed()),
            Some(_) => None,
            None => Some(Duration::ZERO),
        }
    }
}
