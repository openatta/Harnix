//! Where a tool call learns which calls its session's turn is on behalf of.
//!
//! A turn is started by a call — a device's, or an extension's that is waiting on it — and every
//! tool the turn's session calls carries that call's chain
//! ([`ToolCx::chain`](harnix_engine_api::ToolCx::chain)), so a tool that calls back into an
//! extension still holding the turn's call is refused at once rather than left waiting for
//! itself. The instance layer hands each engine it builds in this process one of these; the
//! engine's tool adapter asks it once per call.

use harnix_contract::s1::{ChainLink, SessionRef};

/// The chain of the turn running in a session, outermost first; none when no turn runs, or it
/// runs on behalf of no extension.
pub trait ChainSource: Send + Sync {
    fn chain(&self, session: &SessionRef) -> Vec<ChainLink>;
}

/// A source with no turns: an engine whose tools nothing waits on.
pub struct NoChains;

impl ChainSource for NoChains {
    fn chain(&self, _session: &SessionRef) -> Vec<ChainLink> {
        Vec::new()
    }
}
