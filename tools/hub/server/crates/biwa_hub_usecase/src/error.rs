use biwa_hub_domain::RepoError;

#[derive(Debug, thiserror::Error)]
pub enum UseCaseError {
    #[error("not found")]
    NotFound,
    #[error("conflict: {0}")]
    Conflict(String),
    /// リクエストの形式は正しいが、値そのものが不正 (パッケージ名の形式違反など)。
    #[error("invalid input: {0}")]
    Invalid(String),
    #[error("internal error: {0}")]
    Internal(String),
}

impl From<RepoError> for UseCaseError {
    fn from(err: RepoError) -> Self {
        match err {
            RepoError::NotFound => Self::NotFound,
            RepoError::Conflict(msg) => Self::Conflict(msg),
            RepoError::Internal(msg) => Self::Internal(msg),
        }
    }
}
