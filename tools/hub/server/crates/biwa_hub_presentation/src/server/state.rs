use std::sync::Arc;

use biwa_hub_domain::{PackageRepository, VersionRepository};
use biwa_hub_usecase::{
    GetPackageUseCase, ListVersionsUseCase, PublishVersionUseCase, RegisterPackageUseCase,
};

#[derive(Clone)]
pub struct AppState {
    pub register_package: Arc<RegisterPackageUseCase>,
    pub get_package: Arc<GetPackageUseCase>,
    pub list_versions: Arc<ListVersionsUseCase>,
    pub publish_version: Arc<PublishVersionUseCase>,
}

impl AppState {
    pub fn new(
        package_repository: Arc<dyn PackageRepository>,
        version_repository: Arc<dyn VersionRepository>,
    ) -> Self {
        Self {
            register_package: Arc::new(RegisterPackageUseCase::new(package_repository.clone())),
            get_package: Arc::new(GetPackageUseCase::new(
                package_repository.clone(),
                version_repository.clone(),
            )),
            list_versions: Arc::new(ListVersionsUseCase::new(
                package_repository.clone(),
                version_repository.clone(),
            )),
            publish_version: Arc::new(PublishVersionUseCase::new(
                package_repository,
                version_repository,
            )),
        }
    }
}
