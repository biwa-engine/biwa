use async_trait::async_trait;
use chrono::{DateTime, Utc};
use sqlx::PgPool;
use uuid::Uuid;

use biwa_hub_domain::{
    NewVersion, PackageId, PackageVersion, PackageVersionRecord, RepoError, VersionDependency,
    VersionRepository,
};

use crate::error::map_sqlx_error;

pub struct PgVersionRepository {
    pool: PgPool,
}

impl PgVersionRepository {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

#[derive(sqlx::FromRow)]
struct VersionRow {
    package_id: Uuid,
    major: i32,
    minor: i32,
    patch: i32,
    description: Option<String>,
    commit_hash: String,
    created_at: DateTime<Utc>,
}

#[derive(sqlx::FromRow)]
struct DependencyRow {
    #[allow(dead_code)]
    package_id: Uuid,
    major: i32,
    minor: i32,
    patch: i32,
    depends_on_package_id: Uuid,
    dep_major: i32,
    dep_minor: i32,
    dep_patch: i32,
}

fn version_key(major: i32, minor: i32, patch: i32) -> (i32, i32, i32) {
    (major, minor, patch)
}

#[async_trait]
impl VersionRepository for PgVersionRepository {
    async fn create(
        &self,
        package_id: PackageId,
        new_version: NewVersion,
    ) -> Result<PackageVersionRecord, RepoError> {
        let mut tx = self.pool.begin().await.map_err(map_sqlx_error)?;

        let major = new_version.version.major() as i32;
        let minor = new_version.version.minor() as i32;
        let patch = new_version.version.patch() as i32;

        let row = sqlx::query_as::<_, VersionRow>(
            r#"
            insert into package_versions
                (package_id, major, minor, patch, description, commit_hash)
            values ($1, $2, $3, $4, $5, $6)
            returning package_id, major, minor, patch, description, commit_hash, created_at
            "#,
        )
        .bind(package_id.value())
        .bind(major)
        .bind(minor)
        .bind(patch)
        .bind(&new_version.description)
        .bind(new_version.commit.value())
        .fetch_one(&mut *tx)
        .await
        .map_err(map_sqlx_error)?;

        for dep in &new_version.dependencies {
            sqlx::query(
                r#"
                insert into package_version_dependencies
                    (package_id, major, minor, patch,
                     depends_on_package_id, dep_major, dep_minor, dep_patch)
                values ($1, $2, $3, $4, $5, $6, $7, $8)
                "#,
            )
            .bind(package_id.value())
            .bind(major)
            .bind(minor)
            .bind(patch)
            .bind(dep.package_id.value())
            .bind(dep.version.major() as i32)
            .bind(dep.version.minor() as i32)
            .bind(dep.version.patch() as i32)
            .execute(&mut *tx)
            .await
            .map_err(map_sqlx_error)?;
        }

        tx.commit().await.map_err(map_sqlx_error)?;

        Ok(PackageVersionRecord {
            package_id: PackageId::new(row.package_id),
            version: PackageVersion::new(
                row.major as usize,
                row.minor as usize,
                row.patch as usize,
            ),
            description: row.description,
            commit: new_version.commit,
            dependencies: new_version.dependencies,
            created_at: row.created_at,
        })
    }

    async fn list_by_package(
        &self,
        package_id: PackageId,
    ) -> Result<Vec<PackageVersionRecord>, RepoError> {
        let version_rows = sqlx::query_as::<_, VersionRow>(
            r#"
            select package_id, major, minor, patch, description, commit_hash, created_at
            from package_versions
            where package_id = $1
            order by major, minor, patch
            "#,
        )
        .bind(package_id.value())
        .fetch_all(&self.pool)
        .await
        .map_err(map_sqlx_error)?;

        let dependency_rows = sqlx::query_as::<_, DependencyRow>(
            r#"
            select package_id, major, minor, patch,
                   depends_on_package_id, dep_major, dep_minor, dep_patch
            from package_version_dependencies
            where package_id = $1
            "#,
        )
        .bind(package_id.value())
        .fetch_all(&self.pool)
        .await
        .map_err(map_sqlx_error)?;

        version_rows
            .into_iter()
            .map(|row| {
                let key = version_key(row.major, row.minor, row.patch);
                let dependencies = dependency_rows
                    .iter()
                    .filter(|dep| version_key(dep.major, dep.minor, dep.patch) == key)
                    .map(|dep| VersionDependency {
                        package_id: PackageId::new(dep.depends_on_package_id),
                        version: PackageVersion::new(
                            dep.dep_major as usize,
                            dep.dep_minor as usize,
                            dep.dep_patch as usize,
                        ),
                    })
                    .collect();

                Ok(PackageVersionRecord {
                    package_id: PackageId::new(row.package_id),
                    version: PackageVersion::new(
                        row.major as usize,
                        row.minor as usize,
                        row.patch as usize,
                    ),
                    description: row.description,
                    commit: parse_commit_hash(&row.commit_hash)?,
                    dependencies,
                    created_at: row.created_at,
                })
            })
            .collect()
    }
}

/// `commit_hash` は登録時に検証済みの文字列なので、パース失敗はデータ破損とみなす。
fn parse_commit_hash(s: &str) -> Result<biwa_hub_domain::CommitHash, RepoError> {
    use std::str::FromStr;
    biwa_hub_domain::CommitHash::from_str(s)
        .map_err(|e| RepoError::Internal(format!("corrupt commit hash in db: {e}")))
}
