//! An extension's routes, as the transport serves them: one router per extension, under its id,
//! after the transport admitted the request and the router allowed it.

use std::sync::Arc;

use harnix_ext::http::{Body, BoxError};
use harnix_ext::RouteDecl;
use http_body_util::BodyExt;

/// The router for one extension's routes: each request to the route whose prefix it is under,
/// the longest such prefix.
pub(crate) fn router(routes: Vec<RouteDecl>) -> axum::Router {
    let routes = Arc::new(routes);
    axum::Router::new().fallback(move |req: axum::extract::Request| {
        let routes = routes.clone();
        async move { serve(&routes, req).await }
    })
}

/// Whether `path` (what follows `/<id>`) is under `prefix`, by whole segments.
fn under(prefix: &str, path: &str) -> bool {
    let prefix = prefix.trim_matches('/');
    if prefix.is_empty() {
        return true;
    }
    let path = path.trim_start_matches('/');
    path == prefix || path.strip_prefix(prefix).is_some_and(|rest| rest.starts_with('/'))
}

pub(crate) async fn serve(routes: &[RouteDecl], req: axum::extract::Request) -> axum::response::Response {
    let path = req.uri().path().to_string();
    let Some(route) = routes.iter().filter(|r| under(r.prefix(), &path)).max_by_key(|r| r.prefix().len()) else {
        let mut response = axum::response::Response::new(axum::body::Body::from("no route here"));
        *response.status_mut() = axum::http::StatusCode::NOT_FOUND;
        return response;
    };
    let req = req.map(|body| -> Body { body.map_err(|e| Box::new(e) as BoxError).boxed_unsync() });
    route.handle(req).await.map(axum::body::Body::new)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_route_is_under_its_prefix_by_whole_segments() {
        assert!(under("", "/anything"));
        assert!(under("ui", "/ui"));
        assert!(under("/ui/", "/ui/index.html"));
        assert!(!under("ui", "/uix/index.html"));
        assert!(!under("ui", "/api"));
    }
}
