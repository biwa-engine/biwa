use std::collections::HashMap;
use std::str::FromStr;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use sqlx::PgPool;
use uuid::Uuid;

use biwa_hub_domain::{
    NewPackage, Package, PackageId, PackageName, PackageRepository, RepoError, RepositoryUrl,
};

use crate::error::map_sqlx_error;

pub struct PgPackageRepository {
    pool: PgPool,
}

impl PgPackageRepository {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

/// `packages` テーブル 1 行分。ドメイン型は sqlx の `Decode` を知らないので、
/// 一旦プレーンな型で受けてから [`Row::into_domain`] で変換する。
#[derive(sqlx::FromRow)]
struct Row {
    id: Uuid,
    name: String,
    repository: String,
    owner_id: Option<Uuid>,
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
}

impl Row {
    /// 保存されている文字列は登録時に一度検証済みなので、
    /// パース失敗はデータ破損に等しく `Internal` として扱う。
    fn into_domain(self) -> Result<Package, RepoError> {
        // Phase 1 はユーザアカウントを実装しないため owner_id は書き込まれない。
        // 将来 users テーブルと usecase 層が揃ったらここで owner を解決する。
        let _ = self.owner_id;

        Ok(Package {
            id: PackageId::new(self.id),
            name: PackageName::from_str(&self.name)
                .map_err(|e| RepoError::Internal(format!("corrupt package name in db: {e}")))?,
            repository: RepositoryUrl::from_str(&self.repository)
                .map_err(|e| RepoError::Internal(format!("corrupt repository url in db: {e}")))?,
            owner: None,
            created_at: self.created_at,
            updated_at: self.updated_at,
        })
    }
}

#[async_trait]
impl PackageRepository for PgPackageRepository {
    async fn create(&self, new_package: NewPackage) -> Result<Package, RepoError> {
        let id = Uuid::new_v4();
        let row = sqlx::query_as::<_, Row>(
            r#"
            insert into packages (id, name, repository, owner_id)
            values ($1, $2, $3, null)
            returning id, name, repository, owner_id, created_at, updated_at
            "#,
        )
        .bind(id)
        .bind(new_package.name.value())
        .bind(new_package.repository.value())
        .fetch_one(&self.pool)
        .await
        .map_err(map_sqlx_error)?;

        row.into_domain()
    }

    async fn find_by_name(&self, name: &PackageName) -> Result<Option<Package>, RepoError> {
        let row = sqlx::query_as::<_, Row>(
            r#"
            select id, name, repository, owner_id, created_at, updated_at
            from packages
            where name = $1
            "#,
        )
        .bind(name.value())
        .fetch_optional(&self.pool)
        .await
        .map_err(map_sqlx_error)?;

        row.map(Row::into_domain).transpose()
    }

    async fn find_by_id(&self, id: PackageId) -> Result<Option<Package>, RepoError> {
        let row = sqlx::query_as::<_, Row>(
            r#"
            select id, name, repository, owner_id, created_at, updated_at
            from packages
            where id = $1
            "#,
        )
        .bind(id.value())
        .fetch_optional(&self.pool)
        .await
        .map_err(map_sqlx_error)?;

        row.map(Row::into_domain).transpose()
    }

    async fn find_names_by_ids(
        &self,
        ids: &[PackageId],
    ) -> Result<HashMap<PackageId, PackageName>, RepoError> {
        if ids.is_empty() {
            return Ok(HashMap::new());
        }
        let raw_ids: Vec<Uuid> = ids.iter().map(|id| id.value()).collect();

        #[derive(sqlx::FromRow)]
        struct NameRow {
            id: Uuid,
            name: String,
        }

        let rows = sqlx::query_as::<_, NameRow>(
            r#"
            select id, name
            from packages
            where id = any($1)
            "#,
        )
        .bind(&raw_ids)
        .fetch_all(&self.pool)
        .await
        .map_err(map_sqlx_error)?;

        rows.into_iter()
            .map(|row| {
                let name = PackageName::from_str(&row.name)
                    .map_err(|e| RepoError::Internal(format!("corrupt package name in db: {e}")))?;
                Ok((PackageId::new(row.id), name))
            })
            .collect()
    }
}
