use reqwest::StatusCode;
use reqwest::blocking::{Client, Response};
use serde::de::DeserializeOwned;
use uuid::Uuid;

use biwa_hub_presentation::dto::{
    ErrorResponse, PackageDto, PublishVersionRequest, RegisterPackageRequest, VersionDto,
};

use crate::error::HubClientError;

/// ビルド時に取り込まれる既定のハブ URL。
///
/// 開発中に別のハブ (ローカルで立てたものなど) を向けたい場合は、
/// `BIWA_HUB_URL` を設定してビルドし直すか、[`HubClient::with_base_url`] を使う。
const BUILTIN_HUB_URL: Option<&str> = option_env!("BIWA_HUB_URL");

pub struct HubClient {
    base_url: String,
    http: Client,
}

impl HubClient {
    /// ビルド時に取り込まれた `BIWA_HUB_URL` を使う。
    ///
    /// 未設定なら [`HubClientError::MissingHubUrl`]。
    pub fn new() -> Result<Self, HubClientError> {
        let base_url = BUILTIN_HUB_URL.ok_or(HubClientError::MissingHubUrl)?;
        Ok(Self::with_base_url(base_url))
    }

    /// 明示的に URL を指定する (テストや、ビルド時設定を上書きしたい場合)。
    pub fn with_base_url(base_url: impl Into<String>) -> Self {
        let mut base_url = base_url.into();
        while base_url.ends_with('/') {
            base_url.pop();
        }
        Self {
            base_url,
            http: Client::new(),
        }
    }

    pub fn get_package_by_name(&self, name: &str) -> Result<PackageDto, HubClientError> {
        let url = format!("{}/v1/packages/{name}/", self.base_url);
        let resp = self.http.get(url).send()?;
        parse_response(resp)
    }

    pub fn get_package_by_uuid(&self, id: Uuid) -> Result<PackageDto, HubClientError> {
        let url = format!("{}/v1/packages/", self.base_url);
        let resp = self.http.get(url).query(&[("uuid", id.to_string())]).send()?;
        parse_response(resp)
    }

    pub fn list_versions(&self, name: &str) -> Result<Vec<VersionDto>, HubClientError> {
        let url = format!("{}/v1/packages/{name}/versions/", self.base_url);
        let resp = self.http.get(url).send()?;
        parse_response(resp)
    }

    pub fn register_package(
        &self,
        request: &RegisterPackageRequest,
    ) -> Result<PackageDto, HubClientError> {
        let url = format!("{}/v1/packages/", self.base_url);
        let resp = self.http.post(url).json(request).send()?;
        parse_response(resp)
    }

    pub fn publish_version(
        &self,
        package_name: &str,
        request: &PublishVersionRequest,
    ) -> Result<VersionDto, HubClientError> {
        let url = format!("{}/v1/packages/{package_name}/versions/", self.base_url);
        let resp = self.http.post(url).json(request).send()?;
        parse_response(resp)
    }
}

fn parse_response<T: DeserializeOwned>(resp: Response) -> Result<T, HubClientError> {
    let status = resp.status();
    if status.is_success() {
        return Ok(resp.json()?);
    }

    let message = resp
        .json::<ErrorResponse>()
        .map(|e| e.error)
        .unwrap_or_else(|_| status.to_string());

    Err(match status {
        StatusCode::NOT_FOUND => HubClientError::NotFound,
        StatusCode::CONFLICT => HubClientError::Conflict(message),
        StatusCode::BAD_REQUEST => HubClientError::Invalid(message),
        _ => HubClientError::Server(message),
    })
}
