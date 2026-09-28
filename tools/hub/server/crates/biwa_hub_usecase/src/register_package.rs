use std::str::FromStr;
use std::sync::Arc;

use biwa_hub_domain::{NewPackage, Package, PackageName, PackageRepository, RepositoryUrl};

use crate::error::UseCaseError;

pub struct RegisterPackageInput {
    pub name: String,
    pub repository: String,
}

pub struct RegisterPackageUseCase {
    package_repository: Arc<dyn PackageRepository>,
}

impl RegisterPackageUseCase {
    pub fn new(package_repository: Arc<dyn PackageRepository>) -> Self {
        Self { package_repository }
    }

    pub async fn execute(&self, input: RegisterPackageInput) -> Result<Package, UseCaseError> {
        let name = PackageName::from_str(&input.name)
            .map_err(|e| UseCaseError::Invalid(e.to_string()))?;
        let repository = RepositoryUrl::from_str(&input.repository)
            .map_err(|e| UseCaseError::Invalid(e.to_string()))?;

        if self.package_repository.find_by_name(&name).await?.is_some() {
            return Err(UseCaseError::Conflict(format!(
                "package `{name}` is already registered"
            )));
        }

        let package = self
            .package_repository
            .create(NewPackage { name, repository })
            .await?;

        Ok(package)
    }
}
