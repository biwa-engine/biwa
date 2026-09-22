use axum::Router;
use axum::routing::get;

use crate::server::handlers;
use crate::server::state::AppState;

pub fn router(state: AppState) -> Router {
    Router::new()
        .route(
            "/v1/packages/",
            get(handlers::get_package_by_uuid).post(handlers::register_package),
        )
        .route("/v1/packages/{name}/", get(handlers::get_package_by_name))
        .route(
            "/v1/packages/{name}/versions/",
            get(handlers::list_versions).post(handlers::publish_version),
        )
        .with_state(state)
}
