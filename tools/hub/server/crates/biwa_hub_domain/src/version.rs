use chrono::{DateTime, Utc};

use crate::package::PackageId;
use crate::value::CommitHash;
use biwac_base::PackageVersion;

/// あるバージョンが直接依存するパッケージ 1 件。
///
/// 依存は id (UUID) で指定・保持する。公開 API のレスポンスでは
/// 依存先の名前 (取得ディレクトリを決めるのに要る) も付けて返すが、
/// それは usecase 層が [`crate::PackageRepository::find_names_by_ids`] で
/// 都度引く導出値であり、ここでは持たない。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VersionDependency {
    pub package_id: PackageId,
    pub version: PackageVersion,
}

#[derive(Debug, Clone)]
pub struct PackageVersionRecord {
    pub package_id: PackageId,
    pub version: PackageVersion,
    pub description: Option<String>,
    pub commit: CommitHash,
    pub dependencies: Vec<VersionDependency>,
    pub created_at: DateTime<Utc>,
}

/// バージョン公開の入力。
#[derive(Debug, Clone)]
pub struct NewVersion {
    pub version: PackageVersion,
    pub description: Option<String>,
    pub commit: CommitHash,
    pub dependencies: Vec<VersionDependency>,
}
