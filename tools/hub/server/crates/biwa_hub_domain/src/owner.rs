use std::fmt::Display;
use std::str::FromStr;

use uuid::Uuid;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct OwnerId(Uuid);

impl OwnerId {
    pub fn new(id: Uuid) -> Self {
        Self(id)
    }

    pub fn value(&self) -> Uuid {
        self.0
    }
}

/// Biwa Hub アカウントの表示 ID (`[a-zA-Z0-9-_]+`)。
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct VisibleId(String);

#[derive(Debug, thiserror::Error)]
#[error("invalid visible id: `{0}` (must match `[a-zA-Z0-9-_]+`)")]
pub struct VisibleIdError(String);

impl FromStr for VisibleId {
    type Err = VisibleIdError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let valid = !s.is_empty()
            && s.chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_');
        if valid {
            Ok(Self(s.to_string()))
        } else {
            Err(VisibleIdError(s.to_string()))
        }
    }
}

impl VisibleId {
    pub fn value(&self) -> &str {
        &self.0
    }
}

impl Display for VisibleId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// パッケージの所有者。
///
/// Phase 1 はユーザアカウント/認証を実装しないため、
/// [`crate::Package::owner`] は常に `None` になる。
/// スキーマとしては先に用意しておき、認証実装時にここへ差し込む。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Owner {
    pub id: OwnerId,
    pub visible_id: VisibleId,
}
