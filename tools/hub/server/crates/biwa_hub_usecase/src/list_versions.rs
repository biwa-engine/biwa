use std::collections::HashMap;
use std::str::FromStr;
use std::sync::Arc;

use biwa_hub_domain::{
    PackageId, PackageName, PackageRepository, PackageVersionRecord, VersionDependency,
    VersionRepository,
};

use crate::error::UseCaseError;
use crate::view::{VersionDependencyView, VersionView};

pub struct ListVersionsUseCase {
    package_repository: Arc<dyn PackageRepository>,
    version_repository: Arc<dyn VersionRepository>,
}

impl ListVersionsUseCase {
    pub fn new(
        package_repository: Arc<dyn PackageRepository>,
        version_repository: Arc<dyn VersionRepository>,
    ) -> Self {
        Self {
            package_repository,
            version_repository,
        }
    }

    pub async fn execute(&self, package_name: &str) -> Result<Vec<VersionView>, UseCaseError> {
        let name = PackageName::from_str(package_name)
            .map_err(|e| UseCaseError::Invalid(e.to_string()))?;
        let package = self
            .package_repository
            .find_by_name(&name)
            .await?
            .ok_or(UseCaseError::NotFound)?;

        let versions = self.version_repository.list_by_package(package.id).await?;
        resolve_views(&self.package_repository, versions).await
    }
}

/// 複数バージョン分の依存 id をまとめて名前解決し、[`VersionView`] に組み立てる。
///
/// `list_versions` と `publish_version` (公開直後のレスポンス) の両方から使う。
pub(crate) async fn resolve_views(
    package_repository: &Arc<dyn PackageRepository>,
    versions: Vec<PackageVersionRecord>,
) -> Result<Vec<VersionView>, UseCaseError> {
    let all_dep_ids: Vec<PackageId> = versions
        .iter()
        .flat_map(|v| v.dependencies.iter().map(|d| d.package_id))
        .collect();
    let names = package_repository.find_names_by_ids(&all_dep_ids).await?;

    versions
        .into_iter()
        .map(|v| build_view(v, &names))
        .collect()
}

fn build_view(
    record: PackageVersionRecord,
    names: &HashMap<PackageId, PackageName>,
) -> Result<VersionView, UseCaseError> {
    let dependencies = record
        .dependencies
        .into_iter()
        .map(|dep: VersionDependency| {
            let name = names.get(&dep.package_id).cloned().ok_or_else(|| {
                UseCaseError::Internal(format!(
                    "dependency package {:?} referenced but not found",
                    dep.package_id
                ))
            })?;
            Ok(VersionDependencyView {
                package_id: dep.package_id,
                name,
                version: dep.version,
            })
        })
        .collect::<Result<_, UseCaseError>>()?;

    Ok(VersionView {
        version: record.version,
        description: record.description,
        commit: record.commit,
        dependencies,
        created_at: record.created_at,
    })
}
