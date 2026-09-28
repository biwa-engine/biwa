mod error;
mod package_repository;
mod version_repository;

pub use package_repository::PgPackageRepository;
pub use version_repository::PgVersionRepository;

/// `migrations/` を埋め込んだマイグレータ。起動時に `run(&pool)` する。
pub fn migrator() -> &'static sqlx::migrate::Migrator {
    static MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate!("./migrations");
    &MIGRATOR
}
