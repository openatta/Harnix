//! What a package serves over HTTP, under `/ext/<id>/`: its interface's files. The route is the
//! carriers' shared one ([`UiFiles`](harnix_carrier_shared::UiFiles)); what is this carrier's is
//! which packages run.

use std::sync::Arc;

use harnix_carrier_shared::{Hosted, Loaded};

use crate::registry::Registry;

impl Hosted for Registry {
    fn running(&self, id: &str) -> Option<Arc<Loaded>> {
        self.get(id)
    }
}
