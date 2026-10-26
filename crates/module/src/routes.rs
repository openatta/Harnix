//! A module's own HTTP and WebSocket, under `/apps/<id>/` (design §7.7).
//!
//! Same origin as the interface, admitted by the same token or device
//! credential, decided in the authorization table as `route:<id>` and
//! audited. What it does **not** get is the Harnix channels' guarantees —
//! topology independence, `seq` catch-up. A module uses this for what those
//! channels do not fit: its own streaming protocol, a library that wants an
//! HTTP shape.

use harnix_authz::Principal;

pub struct RouteDecl {
    pub principals: Vec<Principal>,
    /// Paths are relative to `/apps/<id>/`.
    pub router: axum::Router,
}

#[derive(Default)]
pub struct RouteRegistry {
    decl: Option<RouteDecl>,
}

impl RouteRegistry {
    #[doc(hidden)]
    pub fn new() -> Self {
        Self::default()
    }

    /// Serve this router at `/apps/<id>/` to these principals. One router
    /// per module; a second call replaces the first.
    pub fn serve(&mut self, principals: &[Principal], router: axum::Router) {
        self.decl = Some(RouteDecl {
            principals: principals.to_vec(),
            router,
        });
    }

    #[doc(hidden)]
    pub fn into_decl(self) -> Option<RouteDecl> {
        self.decl
    }
}
