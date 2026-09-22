use std::fmt::Display;
use std::str::FromStr;

/// Git リポジトリの URL。
///
/// Phase 1 はソースを持たずここへの参照だけを保持するため、
/// 形式チェックはごく緩い ("空でなく、scheme らしきものを持つ") ものに留める。
/// GitHub 以外 (自前の git サーバなど) も受け付けられるようにするため。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RepositoryUrl(String);

#[derive(Debug, thiserror::Error)]
pub enum RepositoryUrlError {
    #[error("repository url must not be empty")]
    Empty,
    #[error(
        "invalid repository url: `{0}` (expected e.g. `https://github.com/owner/repo`, `git@host:owner/repo.git`)"
    )]
    Invalid(String),
}

impl FromStr for RepositoryUrl {
    type Err = RepositoryUrlError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        if s.trim().is_empty() {
            return Err(RepositoryUrlError::Empty);
        }
        let looks_like_url = s.starts_with("https://")
            || s.starts_with("http://")
            || s.starts_with("git@")
            || s.starts_with("ssh://")
            || s.starts_with("git://")
            || s.starts_with("file://");
        if !looks_like_url {
            return Err(RepositoryUrlError::Invalid(s.to_string()));
        }
        Ok(Self(s.to_string()))
    }
}

impl RepositoryUrl {
    pub fn value(&self) -> &str {
        &self.0
    }
}

impl Display for RepositoryUrl {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// Git のコミットハッシュ (SHA-1、将来 SHA-256 も見越して長さは幅を持たせる)。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommitHash(String);

#[derive(Debug, thiserror::Error)]
#[error("invalid commit hash: `{0}` (expected a hex string, 7 to 64 characters)")]
pub struct CommitHashError(String);

impl FromStr for CommitHash {
    type Err = CommitHashError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let valid_len = (7..=64).contains(&s.len());
        let valid_chars = !s.is_empty() && s.chars().all(|c| c.is_ascii_hexdigit());
        if valid_len && valid_chars {
            Ok(Self(s.to_lowercase()))
        } else {
            Err(CommitHashError(s.to_string()))
        }
    }
}

impl CommitHash {
    pub fn value(&self) -> &str {
        &self.0
    }
}

impl Display for CommitHash {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}
