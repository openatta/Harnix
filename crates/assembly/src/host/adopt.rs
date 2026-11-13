//! Adopting an extension process's own engine for a host component (`HostApi::adopt`): the
//! instance layer's adoption, behind the host component interface's words.
//!
//! The registry lists the engine as `proc.<id>`, an extension's (`InstanceTrust::Extension`), and
//! hands the hub its link; the carrier keeps what comes back here, and hands it each run's stream.

use std::sync::Arc;

use harnix_contract::s1::ExtId;
use harnix_contract::ChainLink;
use harnix_host_api::{AdoptSpec, AdoptedEngine, EngineNote, Waker};
use harnix_instance::registry::{Adopting, Adoption, EngineNote as Note, Wake};
use harnix_instance::InstanceRegistry;
use harnix_kernel::Rpc;
use tokio::sync::mpsc;

/// Adopt `id`'s engine as `spec` says.
pub(crate) async fn adopt(registry: &InstanceRegistry, id: &ExtId, spec: AdoptSpec) -> Rpc<Arc<dyn AdoptedEngine>> {
    let adopting = Adopting {
        ext: id.clone(),
        data_root: spec.data_root().clone(),
        inherit_providers: spec.inherits_providers(),
        wake: Arc::new(ByWaker(spec.waker().clone())),
    };
    let adoption = registry.adopt(adopting).await?;
    Ok(Arc::new(Adopted(adoption)))
}

/// The carrier's waker, as the instance layer calls it.
struct ByWaker(Arc<dyn Waker>);

#[async_trait::async_trait]
impl Wake for ByWaker {
    async fn wake(&self) -> Rpc<()> {
        self.0.wake().await
    }
}

struct Adopted(Arc<Adoption>);

#[async_trait::async_trait]
impl AdoptedEngine for Adopted {
    fn instance(&self) -> &str {
        self.0.instance()
    }

    async fn attach(&self, stream: std::os::unix::net::UnixStream) -> Rpc<()> {
        self.0.attach(stream).await
    }

    async fn detach(&self, why: &str) {
        self.0.detach(why).await
    }

    async fn hibernate(&self) {
        self.0.hibernate().await
    }

    fn hibernating(&self) -> bool {
        self.0.asleep().is_some()
    }

    async fn engaged(&self) -> bool {
        self.0.engaged().await
    }

    fn turn_chain(&self, session: &str) -> Vec<ChainLink> {
        self.0.turn_chain(session)
    }

    fn notes(&self) -> Option<mpsc::UnboundedReceiver<EngineNote>> {
        let mut from = self.0.notes()?;
        let (to, notes) = mpsc::unbounded_channel();
        tokio::spawn(async move {
            while let Some(note) = from.recv().await {
                let note = match note {
                    Note::TurnStarted { session_id, chain } => EngineNote::TurnStarted { session_id, chain },
                    Note::TurnSettled { session_id } => EngineNote::TurnSettled { session_id },
                    Note::Flooded => EngineNote::Flooded,
                };
                if to.send(note).is_err() {
                    break;
                }
            }
        });
        Some(notes)
    }
}
