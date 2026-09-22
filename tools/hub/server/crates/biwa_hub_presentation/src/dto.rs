//! README に記載の JSON スキーマそのもの。
//!
//! `server` (axum ハンドラ) と `client` (レスポンスのパース) の両方から、
//! feature 無しで参照できる (このモジュールは axum に依存しない)。

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use biwa_hub_usecase::{PackageOverview, VersionDependencyView, VersionView};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OwnerDto {
    pub id: Uuid,
    pub visible_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PackageDto {
    pub id: Uuid,
    pub name: String,
    pub repository: String,
    /// Phase 1 は認証未実装のため常に `null`。
    pub owner: Option<OwnerDto>,
    /// バージョンが 1 つも公開されていなければ `null`。
    pub latest: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl From<PackageOverview> for PackageDto {
    fn from(overview: PackageOverview) -> Self {
        Self {
            id: overview.package.id.value(),
            name: overview.package.name.to_string(),
            repository: overview.package.repository.to_string(),
            owner: overview.package.owner.map(|o| OwnerDto {
                id: o.id.value(),
                visible_id: o.visible_id.to_string(),
            }),
            latest: overview.latest.map(|v| v.to_string()),
            created_at: overview.package.created_at,
            updated_at: overview.package.updated_at,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VersionDependencyDto {
    pub id: Uuid,
    pub name: String,
    pub version: String,
}

impl From<VersionDependencyView> for VersionDependencyDto {
    fn from(dep: VersionDependencyView) -> Self {
        Self {
            id: dep.package_id.value(),
            name: dep.name.to_string(),
            version: dep.version.to_string(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VersionDto {
    pub version: String,
    pub description: Option<String>,
    pub commit: String,
    pub dependencies: Vec<VersionDependencyDto>,
    pub created_at: DateTime<Utc>,
}

impl From<VersionView> for VersionDto {
    fn from(view: VersionView) -> Self {
        Self {
            version: view.version.to_string(),
            description: view.description,
            commit: view.commit.to_string(),
            dependencies: view.dependencies.into_iter().map(Into::into).collect(),
            created_at: view.created_at,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RegisterPackageRequest {
    pub name: String,
    pub repository: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PublishVersionDependencyRequest {
    pub id: Uuid,
    pub version: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PublishVersionRequest {
    pub version: String,
    pub description: Option<String>,
    pub commit: String,
    #[serde(default)]
    pub dependencies: Vec<PublishVersionDependencyRequest>,
}

/// `GET /v1/packages/?uuid=<uuid>` のクエリパラメータ。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GetByUuidQuery {
    pub uuid: Uuid,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ErrorResponse {
    pub error: String,
}
