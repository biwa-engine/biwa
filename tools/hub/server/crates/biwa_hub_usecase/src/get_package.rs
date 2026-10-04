use std::str::FromStr;
use std::sync::Arc;

use biwa_hub_domain::{
    PackageId, PackageName, PackageRepository, PackageVersion, VersionRepository,
};

use crate::error::UseCaseError;
use crate::view::PackageOverview;

pub struct GetPackageUseCase {
    package_repository: Arc<dyn PackageRepository>,
    version_repository: Arc<dyn VersionRepository>,
}

impl GetPackageUseCase {
    pub fn new(
        package_repository: Arc<dyn PackageRepository>,
        version_repository: Arc<dyn VersionRepository>,
    ) -> Self {
        Self {
            package_repository,
            version_repository,
        }
    }

    pub async fn by_name(&self, name: &str) -> Result<PackageOverview, UseCaseError> {
        let name = PackageName::from_str(name).map_err(|e| UseCaseError::Invalid(e.to_string()))?;
        let package = self
            .package_repository
            .find_by_name(&name)
            .await?
            .ok_or(UseCaseError::NotFound)?;
        self.with_latest(package).await
    }

    pub async fn by_id(&self, id: uuid::Uuid) -> Result<PackageOverview, UseCaseError> {
        let package = self
            .package_repository
            .find_by_id(PackageId::new(id))
            .await?
            .ok_or(UseCaseError::NotFound)?;
        self.with_latest(package).await
    }

    async fn with_latest(
        &self,
        package: biwa_hub_domain::Package,
    ) -> Result<PackageOverview, UseCaseError> {
        let versions = self.version_repository.list_by_package(package.id).await?;
        let latest = latest_version(&versions);
        Ok(PackageOverview { package, latest })
    }
}

/// 公開済みバージョンの中から最新 (semver 的な `>` の最大) を選ぶ。
/// 1 つも無ければ `None` (登録直後)。
pub(crate) fn latest_version(
    versions: &[biwa_hub_domain::PackageVersionRecord],
) -> Option<PackageVersion> {
    versions.iter().map(|v| v.version).max()
}
