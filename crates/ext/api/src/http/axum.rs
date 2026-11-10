//! An `axum::Router` as an extension's route. **Not stable**: bound to axum 0.8.

use http_body_util::BodyExt;
use tower::ServiceExt;

use super::{Body, BoxError, Request, Response, RouteContext, RouteHandler};

/// Serve `router` as a route: it sees the request as the route handler does.
pub fn router(router: ::axum::Router) -> impl RouteHandler {
    AxumRoute(router)
}

struct AxumRoute(::axum::Router);

#[async_trait::async_trait]
impl RouteHandler for AxumRoute {
    async fn handle(&self, _cx: RouteContext, req: Request<Body>) -> Response<Body> {
        let req = req.map(::axum::body::Body::new);
        let response = match self.0.clone().oneshot(req).await {
            Ok(response) => response,
            Err(never) => match never {},
        };
        response.map(|body| body.map_err(|e| Box::new(e) as BoxError).boxed_unsync())
    }
}
