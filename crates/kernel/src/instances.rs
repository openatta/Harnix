//! The seam between the instance registry and the hub.
//!
//! The registry builds, lists and stops instances; the hub holds their links and their sessions.
//! The registry tells the hub of each instance through an [`InstanceObserver`], and three things
//! hold across that seam:
//!
//! 1. **Visible after taken over.** An instance is listed, published and routed by name only
//!    after [`InstanceObserver::added`] returned: whoever can see it, the hub is already reading
//!    its events.
//! 2. **Only the hub holds a link.** It is handed over once, in `added` or `restarted`; the
//!    registry keeps what it needs to stop it.
//! 3. **Neither calls the other holding a lock of its own.** Each side marks its lock held with
//!    [`holding`], and each side's entry points assert with [`holds`] that the other's is not.
//!
//! The hub asks the registry back through an [`InstanceBook`]: which instances there are, whose,
//! and to build a space's default one. It knows the registry by that and by nothing else.
//!
//! Generic over the link, which is the engine's vocabulary and not this crate's.

use std::sync::Arc;

use harnix_contract::ExtId;

use crate::{Rpc, TopicChanged};

/// Whose engine an instance runs: what the hub trusts its frames as far as.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InstanceTrust {
    /// One the product started or connected to: in this process, spawned, or attached.
    Product,
    /// An engine in an extension's own process: under third-party control.
    Extension(ExtId),
}

impl InstanceTrust {
    pub fn is_extension(&self) -> bool {
        matches!(self, InstanceTrust::Extension(_))
    }

    /// As the wire says it: `"product"` or `"extension"`.
    pub fn label(&self) -> &'static str {
        match self {
            InstanceTrust::Product => "product",
            InstanceTrust::Extension(_) => "extension",
        }
    }
}

/// What the hub asks of the instance registry, holding none of its own locks.
#[async_trait::async_trait]
pub trait InstanceBook: Send + Sync {
    /// One listed instance.
    fn listed(&self, id: &str) -> Option<Listed>;

    /// Every listed instance.
    fn all(&self) -> Vec<Listed>;

    /// Build `name` from `template` as `space`'s own — its default instance — or answer with it,
    /// when it was built so already.
    async fn ensure_for_space(&self, template: &str, name: &str, space: &str) -> Rpc<()>;

    /// An instance's engine sent more than its trust allows, `consecutive` windows in a row; `0`:
    /// it is within bounds again. The hub dropped what was over. Whoever carries it decides what
    /// that costs the engine.
    fn flooding(&self, id: &str, consecutive: u32);
}

/// A listed instance, as the hub needs it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Listed {
    pub id: String,
    /// The space it was built for, when it was: its sessions are that space's.
    pub space: Option<String>,
}

/// What the registry tells the hub. Never called while the registry holds its lock, and none of
/// it calls back into the registry.
///
/// Besides the three callbacks, the registry asks one question of the hub — which spaces there
/// are, for the space a caller claims — and says through [`TopicChanged`] when the instance list
/// it publishes changed.
#[async_trait::async_trait]
pub trait InstanceObserver<L: ?Sized + Send + Sync + 'static>: TopicChanged {
    /// A new instance, and its link: take its events before returning.
    async fn added(&self, id: &str, link: Arc<L>, trust: InstanceTrust);

    /// The instance is gone: forget its sessions.
    async fn removed(&self, id: &str);

    /// The instance is carried by a new link (a restart, or down): take it, and bring the
    /// sessions anyone watches back in step.
    async fn restarted(&self, id: &str, link: Arc<L>);

    /// Whether `space` is one this product has.
    fn has_space(&self, space: &str) -> bool;
}

/// The two sides of the seam, each with a lock of its own.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Side {
    Hub,
    Registry,
}

/// `side`'s lock, marked held by this task (or thread, outside one) until dropped. A debug
/// build keeps the mark; a release build keeps nothing.
#[must_use]
pub struct Holding {
    #[cfg(debug_assertions)]
    mark: (Side, Holder),
}

/// Mark `side`'s lock held, for as long as the returned value lives.
pub fn holding(side: Side) -> Holding {
    #[cfg(debug_assertions)]
    {
        let mark = (side, Holder::current());
        marks::held().push(mark);
        Holding { mark }
    }
    #[cfg(not(debug_assertions))]
    {
        let _ = side;
        Holding {}
    }
}

/// Whether this task holds `side`'s lock. Always `false` in a release build: it is for
/// `debug_assert!`.
pub fn holds(side: Side) -> bool {
    #[cfg(debug_assertions)]
    {
        let me = Holder::current();
        marks::held().iter().any(|(s, h)| *s == side && *h == me)
    }
    #[cfg(not(debug_assertions))]
    {
        let _ = side;
        false
    }
}

#[cfg(debug_assertions)]
impl Drop for Holding {
    fn drop(&mut self) {
        let mut held = marks::held();
        if let Some(i) = held.iter().position(|m| *m == self.mark) {
            held.swap_remove(i);
        }
    }
}

/// Who holds a lock: a task, which may move between threads while it holds an async lock, or
/// the thread, outside any task.
#[cfg(debug_assertions)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Holder {
    Task(tokio::task::Id),
    Thread(std::thread::ThreadId),
}

#[cfg(debug_assertions)]
impl Holder {
    fn current() -> Self {
        match tokio::task::try_id() {
            Some(id) => Holder::Task(id),
            None => Holder::Thread(std::thread::current().id()),
        }
    }
}

#[cfg(debug_assertions)]
mod marks {
    use std::sync::{Mutex, MutexGuard};

    use super::{Holder, Side};

    static HELD: Mutex<Vec<(Side, Holder)>> = Mutex::new(Vec::new());

    pub(super) fn held() -> MutexGuard<'static, Vec<(Side, Holder)>> {
        harnix_contract::sync::lock(&HELD)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_mark_lasts_as_long_as_the_lock_and_belongs_to_its_holder() {
        assert!(!holds(Side::Hub));
        let held = holding(Side::Hub);
        assert!(holds(Side::Hub) == cfg!(debug_assertions));
        assert!(!holds(Side::Registry));
        std::thread::spawn(|| assert!(!holds(Side::Hub), "another thread holds nothing"))
            .join()
            .unwrap();
        drop(held);
        assert!(!holds(Side::Hub));
    }

    #[tokio::test]
    async fn a_task_holds_only_what_it_marked() {
        let held = holding(Side::Registry);
        assert!(holds(Side::Registry) == cfg!(debug_assertions));
        let other = tokio::spawn(async { holds(Side::Registry) }).await.unwrap();
        assert!(!other, "a spawned task is another holder");
        drop(held);
        assert!(!holds(Side::Registry));
    }
}
