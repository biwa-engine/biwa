use axum::Json;
use axum::extract::{Path, Query, State};

use biwa_hub_usecase::{
    PackageOverview, PublishVersionDependencyInput, PublishVersionInput, RegisterPackageInput,
};

use crate::dto::{
    GetByUuidQuery, PackageDto, PublishVersionRequest, RegisterPackageRequest, VersionDto,
};
use crate::server::error::ApiError;
use crate::server::state::AppState;

pub async fn register_package(
    State(state): State<AppState>,
    Json(body): Json<RegisterPackageRequest>,
) -> Result<Json<PackageDto>, ApiError> {
    let package = state
        .register_package
        .execute(RegisterPackageInput {
            name: body.name,
            repository: body.repository,
        })
        .await?;

    // 登録直後はバージョンが無いので `latest` は必ず `None`。
    let overview = PackageOverview {
        package,
        latest: None,
    };
    Ok(Json(overview.into()))
}

pub async fn get_package_by_uuid(
    State(state): State<AppState>,
    Query(query): Query<GetByUuidQuery>,
) -> Result<Json<PackageDto>, ApiError> {
    let overview = state.get_package.by_id(query.uuid).await?;
    Ok(Json(overview.into()))
}

pub async fn get_package_by_name(
    State(state): State<AppState>,
    Path(name): Path<String>,
) -> Result<Json<PackageDto>, ApiError> {
    let overview = state.get_package.by_name(&name).await?;
    Ok(Json(overview.into()))
}

pub async fn list_versions(
    State(state): State<AppState>,
    Path(name): Path<String>,
) -> Result<Json<Vec<VersionDto>>, ApiError> {
    let views = state.list_versions.execute(&name).await?;
    Ok(Json(views.into_iter().map(Into::into).collect()))
}

pub async fn publish_version(
    State(state): State<AppState>,
    Path(name): Path<String>,
    Json(body): Json<PublishVersionRequest>,
) -> Result<Json<VersionDto>, ApiError> {
    let view = state
        .publish_version
        .execute(PublishVersionInput {
            package_name: name,
            version: body.version,
            description: body.description,
            commit: body.commit,
            dependencies: body
                .dependencies
                .into_iter()
                .map(|d| PublishVersionDependencyInput {
                    id: d.id,
                    version: d.version,
                })
                .collect(),
        })
        .await?;

    Ok(Json(view.into()))
}
