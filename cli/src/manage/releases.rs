//! 配布されているツールチェインの一覧 (GitHub Releases)。
//!
//! release のタグとツールチェインの名前の対応:
//!   - `v0.1.0`         → `0.1.0` (版。チャンネル stable)
//!   - `nightly-<hash>` → `<hash>` (コミットの先頭 12 文字。チャンネル nightly)
//!
//! release の資産 (`.github/workflows/release.yml` が上げる):
//!   - `biwa-<name>-<host>.tar.gz` (Windows は `.zip`): `~/.biwa` にそのまま展開できる形
//!   - `biwa-cli-<name>-<host>[.exe]`: CLI だけ (`biwa self update` が使う)
//!   - `SHA256SUMS`: 上のすべての SHA-256
//!
//! 取得先は環境変数 `BIWA_RELEASES_URL` で差し替えられる (試験用。GitHub の API と同じ形の JSON を返すこと)。

use anyhow::{Context, Result, bail};
use serde::Deserialize;

use super::name::Version;

const DEFAULT_RELEASES_URL: &str = "https://api.github.com/repos/biwa-engine/biwa/releases";
const NIGHTLY_TAG_PREFIX: &str = "nightly-";
pub const CHECKSUMS: &str = "SHA256SUMS";

#[derive(Debug, Clone, Deserialize)]
pub struct Release {
    pub tag_name: String,
    #[serde(default)]
    pub draft: bool,
    #[serde(default)]
    pub published_at: Option<String>,
    #[serde(default)]
    pub assets: Vec<Asset>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Asset {
    pub name: String,
    pub browser_download_url: String,
    #[serde(default)]
    pub size: u64,
}

/// 更新の流れ。版の release を追うか、nightly を追うか。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Channel {
    Stable,
    Nightly,
}

impl Channel {
    /// そのツールチェインが属するチャンネル (版なら stable、それ以外は nightly)。
    pub fn of(name: &str) -> Self {
        if name.parse::<Version>().is_ok() {
            Self::Stable
        } else {
            Self::Nightly
        }
    }
}

impl Release {
    /// この release のツールチェインの名前。ツールチェインの release でなければ `None`。
    pub fn toolchain_name(&self) -> Option<String> {
        if let Some(hash) = self.tag_name.strip_prefix(NIGHTLY_TAG_PREFIX) {
            return (!hash.is_empty()).then(|| hash.to_string());
        }
        self.version().map(|v| v.to_string())
    }

    fn version(&self) -> Option<Version> {
        self.tag_name.strip_prefix('v')?.parse().ok()
    }

    pub fn asset(&self, name: &str) -> Result<&Asset> {
        self.assets
            .iter()
            .find(|a| a.name == name)
            .with_context(|| {
                format!(
                    "the release `{}` has no `{name}` (is this host supported?)",
                    self.tag_name
                )
            })
    }
}

/// HTTP の client。
pub fn client() -> Result<reqwest::blocking::Client> {
    let mut headers = reqwest::header::HeaderMap::new();
    // GitHub の API は認証が無いと 1 時間に 60 回までなので、トークンがあれば使う。
    if let Some(token) = std::env::var("GITHUB_TOKEN").ok().filter(|t| !t.is_empty()) {
        headers.insert(
            reqwest::header::AUTHORIZATION,
            format!("Bearer {token}")
                .parse()
                .context("GITHUB_TOKEN is not a valid header value")?,
        );
    }
    reqwest::blocking::Client::builder()
        .user_agent(concat!("biwa-cli/", env!("CARGO_PKG_VERSION")))
        .default_headers(headers)
        .build()
        .context("failed to set up the HTTP client")
}

/// 配布されている release (下書きを除く)。
pub fn list(client: &reqwest::blocking::Client) -> Result<Vec<Release>> {
    let base = std::env::var("BIWA_RELEASES_URL")
        .ok()
        .filter(|v| !v.is_empty())
        .unwrap_or_else(|| DEFAULT_RELEASES_URL.to_string());
    let url = format!("{base}?per_page=100");
    let response = client
        .get(&url)
        .header(reqwest::header::ACCEPT, "application/vnd.github+json")
        .send()
        .with_context(|| format!("failed to fetch the list of Biwa releases ({url})"))?;
    if !response.status().is_success() {
        bail!(
            "failed to fetch the list of Biwa releases ({url}): {}",
            response.status()
        );
    }
    let releases: Vec<Release> = response
        .json()
        .with_context(|| format!("unexpected response from {url}"))?;
    Ok(releases
        .into_iter()
        .filter(|r| !r.draft && r.toolchain_name().is_some())
        .collect())
}

/// チャンネルの最新の release。
pub fn latest(releases: &[Release], channel: Channel) -> Option<&Release> {
    match channel {
        Channel::Stable => releases
            .iter()
            .filter_map(|r| Some((r.version()?, r)))
            .max_by_key(|(v, _)| *v)
            .map(|(_, r)| r),
        // nightly の新旧は公開した日時で決める (ハッシュでは比べられない)。ISO 8601 なので文字列で比べてよい。
        Channel::Nightly => releases
            .iter()
            .filter(|r| r.tag_name.starts_with(NIGHTLY_TAG_PREFIX))
            .max_by(|a, b| a.published_at.cmp(&b.published_at)),
    }
}

/// `biwa toolchain install <SPEC>` の SPEC に当たる release。
///
/// SPEC: `stable` / `nightly` (それぞれの最新)、`0.1.0`・`v0.1.0` (版)、それ以外は nightly の名前 (ハッシュ)。
/// 省略したら stable、stable がまだ無ければ nightly。
pub fn find<'a>(releases: &'a [Release], spec: Option<&str>) -> Result<&'a Release> {
    let found = match spec {
        None => latest(releases, Channel::Stable).or_else(|| latest(releases, Channel::Nightly)),
        Some("stable") => latest(releases, Channel::Stable),
        Some("nightly") => latest(releases, Channel::Nightly),
        Some(name) => match name.parse::<Version>() {
            Ok(v) => releases.iter().find(|r| r.version() == Some(v)),
            Err(()) => releases
                .iter()
                .find(|r| r.toolchain_name().as_deref() == Some(name)),
        },
    };
    found.with_context(|| match spec {
        None => "no Biwa toolchain has been released yet".to_string(),
        Some(s) => format!("no released Biwa toolchain matches `{s}`"),
    })
}

/// この OS 向けの配布物の名前に付く `<host>`。
pub fn host() -> Result<&'static str> {
    match (std::env::consts::OS, std::env::consts::ARCH) {
        ("windows", "x86_64") => Ok("windows-x86_64"),
        // Apple Silicon と Intel の両方で動く universal バイナリ。
        ("macos", _) => Ok("macos-universal"),
        ("linux", "x86_64") => Ok("linux-x86_64"),
        (os, arch) => bail!("Biwa toolchains are not released for this host ({os} {arch}) yet"),
    }
}

/// ツールチェインの圧縮ファイルの資産名。
pub fn archive_asset(name: &str, host: &str) -> String {
    let ext = if host.starts_with("windows") {
        "zip"
    } else {
        "tar.gz"
    };
    format!("biwa-{name}-{host}.{ext}")
}

/// CLI だけの資産名。
pub fn cli_asset(name: &str, host: &str) -> String {
    let ext = if host.starts_with("windows") {
        ".exe"
    } else {
        ""
    };
    format!("biwa-cli-{name}-{host}{ext}")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn release(tag: &str, published: &str) -> Release {
        Release {
            tag_name: tag.into(),
            draft: false,
            published_at: Some(published.into()),
            assets: vec![],
        }
    }

    #[test]
    fn finds_releases_by_spec() {
        let rs = vec![
            release("nightly-aaaaaaaaaaaa", "2026-11-01T00:00:00Z"),
            release("v0.2.0", "2026-10-01T00:00:00Z"),
            release("nightly-bbbbbbbbbbbb", "2026-11-02T00:00:00Z"),
            release("v0.10.0", "2026-09-01T00:00:00Z"),
            release("something-else", "2026-11-03T00:00:00Z"),
        ];
        let tag = |spec| find(&rs, spec).map(|r| r.tag_name.clone()).ok();
        assert_eq!(tag(Some("stable")).as_deref(), Some("v0.10.0"));
        assert_eq!(
            tag(Some("nightly")).as_deref(),
            Some("nightly-bbbbbbbbbbbb")
        );
        assert_eq!(tag(Some("0.2.0")).as_deref(), Some("v0.2.0"));
        assert_eq!(tag(Some("v0.2.0")).as_deref(), Some("v0.2.0"));
        assert_eq!(
            tag(Some("aaaaaaaaaaaa")).as_deref(),
            Some("nightly-aaaaaaaaaaaa")
        );
        assert_eq!(tag(Some("0.3.0")), None);
        assert_eq!(tag(None).as_deref(), Some("v0.10.0"));
        assert_eq!(rs[4].toolchain_name(), None);

        let nightly_only = vec![release("nightly-aaaaaaaaaaaa", "2026-11-01T00:00:00Z")];
        assert_eq!(
            find(&nightly_only, None).unwrap().tag_name,
            "nightly-aaaaaaaaaaaa"
        );
    }

    #[test]
    fn names_assets() {
        assert_eq!(
            archive_asset("0.1.0", "linux-x86_64"),
            "biwa-0.1.0-linux-x86_64.tar.gz"
        );
        assert_eq!(
            archive_asset("abc", "windows-x86_64"),
            "biwa-abc-windows-x86_64.zip"
        );
        assert_eq!(
            cli_asset("abc", "windows-x86_64"),
            "biwa-cli-abc-windows-x86_64.exe"
        );
        assert_eq!(
            cli_asset("abc", "macos-universal"),
            "biwa-cli-abc-macos-universal"
        );
    }
}
