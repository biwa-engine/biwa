//! Biwa Package Hub の Web API サーバ本体。
//!
//! 環境変数:
//! - `DATABASE_URL` (必須): Postgres への接続文字列。
//! - `BIND_ADDR` (省略時 `0.0.0.0:8080`): listen するアドレス。

use std::sync::Arc;

use anyhow::{Context, Result};
use sqlx::postgres::PgPoolOptions;

use biwa_hub_domain::{PackageRepository, VersionRepository};
use biwa_hub_infrastructure::{PgPackageRepository, PgVersionRepository};
use biwa_hub_presentation::AppState;

const DEFAULT_BIND_ADDR: &str = "0.0.0.0:8080";

#[tokio::main]
async fn main() -> Result<()> {
    let database_url =
        std::env::var("DATABASE_URL").context("DATABASE_URL environment variable is not set")?;
    let bind_addr = std::env::var("BIND_ADDR").unwrap_or_else(|_| DEFAULT_BIND_ADDR.to_string());

    let pool = PgPoolOptions::new()
        .max_connections(10)
        .connect(&database_url)
        .await
        .context("failed to connect to the database")?;

    biwa_hub_infrastructure::migrator()
        .run(&pool)
        .await
        .context("failed to run database migrations")?;

    let package_repository: Arc<dyn PackageRepository> =
        Arc::new(PgPackageRepository::new(pool.clone()));
    let version_repository: Arc<dyn VersionRepository> =
        Arc::new(PgVersionRepository::new(pool));

    let state = AppState::new(package_repository, version_repository);
    let router = biwa_hub_presentation::router(state);

    let listener = tokio::net::TcpListener::bind(&bind_addr)
        .await
        .with_context(|| format!("failed to bind {bind_addr}"))?;
    println!("Biwa Package Hub listening on {bind_addr}");

    axum::serve(listener, router)
        .await
        .context("server error")?;

    Ok(())
}
