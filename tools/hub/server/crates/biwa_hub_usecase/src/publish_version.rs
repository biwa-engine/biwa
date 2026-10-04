use std::str::FromStr;
use std::sync::Arc;

use biwa_hub_domain::{
    CommitHash, NewVersion, PackageId, PackageName, PackageRepository, PackageVersion,
    VersionDependency, VersionRepository,
};

use crate::error::UseCaseError;
use crate::list_versions::resolve_views;
use crate::view::VersionView;

pub struct PublishVersionDependencyInput {
    pub id: uuid::Uuid,
    pub version: String,
}

pub struct PublishVersionInput {
    pub package_name: String,
    pub version: String,
    pub description: Option<String>,
    pub commit: String,
    pub dependencies: Vec<PublishVersionDependencyInput>,
}

pub struct PublishVersionUseCase {
    package_repository: Arc<dyn PackageRepository>,
    version_repository: Arc<dyn VersionRepository>,
}

impl PublishVersionUseCase {
    pub fn new(
        package_repository: Arc<dyn PackageRepository>,
        version_repository: Arc<dyn VersionRepository>,
    ) -> Self {
        Self {
            package_repository,
            version_repository,
        }
    }

    pub async fn execute(&self, input: PublishVersionInput) -> Result<VersionView, UseCaseError> {
        let package_name = PackageName::from_str(&input.package_name)
            .map_err(|e| UseCaseError::Invalid(e.to_string()))?;
        let package = self
            .package_repository
            .find_by_name(&package_name)
            .await?
            .ok_or(UseCaseError::NotFound)?;

        let version = PackageVersion::from_str(&input.version)
            .map_err(|e| UseCaseError::Invalid(e.to_string()))?;
        let commit = CommitHash::from_str(&input.commit)
            .map_err(|e| UseCaseError::Invalid(e.to_string()))?;

        let mut dependencies = Vec::with_capacity(input.dependencies.len());
        for dep in input.dependencies {
            let dep_version = PackageVersion::from_str(&dep.version)
                .map_err(|e| UseCaseError::Invalid(e.to_string()))?;
            let dep_id = PackageId::new(dep.id);
            // 依存先が実在しない uuid を指していないか確認する。
            // 実在しなければ後段の名前解決 (`resolve_views`) が失敗するより、
            // ここで早く分かるエラーにしたほうが公開者にとって分かりやすい。
            self.package_repository
                .find_by_id(dep_id)
                .await?
                .ok_or_else(|| {
                    UseCaseError::Invalid(format!("dependency package `{}` not found", dep.id))
                })?;
            dependencies.push(VersionDependency {
                package_id: dep_id,
                version: dep_version,
            });
        }

        let record = self
            .version_repository
            .create(
                package.id,
                NewVersion {
                    version,
                    description: input.description,
                    commit,
                    dependencies,
                },
            )
            .await?;

        let mut views = resolve_views(&self.package_repository, vec![record]).await?;
        Ok(views.remove(0))
    }
}
