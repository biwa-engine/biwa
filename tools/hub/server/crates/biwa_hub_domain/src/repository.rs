use std::collections::HashMap;

use async_trait::async_trait;
use biwac_base::PackageName;

use crate::error::RepoError;
use crate::package::{NewPackage, Package, PackageId};
use crate::version::{NewVersion, PackageVersionRecord};

/// パッケージ (名前・リポジトリ URL・所有者) の永続化。
///
/// usecase 層はこの trait だけを知り、実装 (Postgres など) は
/// `biwa_hub_infrastructure` が提供する (依存性逆転)。
#[async_trait]
pub trait PackageRepository: Send + Sync {
    async fn create(&self, new_package: NewPackage) -> Result<Package, RepoError>;

    async fn find_by_name(&self, name: &PackageName) -> Result<Option<Package>, RepoError>;

    async fn find_by_id(&self, id: PackageId) -> Result<Option<Package>, RepoError>;

    /// 複数 id から名前をまとめて引く。
    ///
    /// バージョンの依存一覧 (id のみ保持) をレスポンスに載せるとき、
    /// 依存先ごとに 1 回ずつ問い合わせると N+1 になるためまとめて解決する。
    async fn find_names_by_ids(
        &self,
        ids: &[PackageId],
    ) -> Result<HashMap<PackageId, PackageName>, RepoError>;
}

/// パッケージバージョンの永続化。
#[async_trait]
pub trait VersionRepository: Send + Sync {
    /// 公開は取り消せない前提なので更新・削除は無い。
    async fn create(
        &self,
        package_id: PackageId,
        new_version: NewVersion,
    ) -> Result<PackageVersionRecord, RepoError>;

    /// あるパッケージの全バージョンを公開順に返す。
    async fn list_by_package(
        &self,
        package_id: PackageId,
    ) -> Result<Vec<PackageVersionRecord>, RepoError>;
}
