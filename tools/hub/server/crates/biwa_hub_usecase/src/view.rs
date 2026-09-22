use chrono::{DateTime, Utc};

use biwa_hub_domain::{CommitHash, Package, PackageId, PackageName, PackageVersion};

/// パッケージ 1 件 + その最新バージョン。
///
/// `latest` はバージョンが 1 つも公開されていなければ `None`
/// (登録直後の状態。README の `latest` はここでは optional として扱う)。
pub struct PackageOverview {
    pub package: Package,
    pub latest: Option<PackageVersion>,
}

/// バージョンの依存 1 件。取得先ディレクトリを決めるのに要る名前まで解決済み。
pub struct VersionDependencyView {
    pub package_id: PackageId,
    pub name: PackageName,
    pub version: PackageVersion,
}

/// バージョン 1 件 (依存の名前解決済み)。
pub struct VersionView {
    pub version: PackageVersion,
    pub description: Option<String>,
    pub commit: CommitHash,
    pub dependencies: Vec<VersionDependencyView>,
    pub created_at: DateTime<Utc>,
}
