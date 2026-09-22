use chrono::{DateTime, Utc};
use uuid::Uuid;

use crate::owner::Owner;
use crate::value::RepositoryUrl;
use biwac_base::PackageName;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct PackageId(Uuid);

impl PackageId {
    pub fn new(id: Uuid) -> Self {
        Self(id)
    }

    pub fn generate() -> Self {
        Self(Uuid::new_v4())
    }

    pub fn value(&self) -> Uuid {
        self.0
    }
}

/// ハブに登録されたパッケージ。
///
/// `latest` (README 記載) はここには無い。バージョンの集合から導出する値であり、
/// 保持すると公開のたびに整合性を取り続ける必要が生まれるため、
/// usecase 層が [`crate::VersionRepository`] の結果から都度計算する。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Package {
    pub id: PackageId,
    pub name: PackageName,
    pub repository: RepositoryUrl,
    /// Phase 1 では常に `None` (認証未実装のため)。
    pub owner: Option<Owner>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// パッケージ新規登録の入力。
#[derive(Debug, Clone)]
pub struct NewPackage {
    pub name: PackageName,
    pub repository: RepositoryUrl,
}
