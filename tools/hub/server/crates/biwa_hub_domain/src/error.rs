/// リポジトリ層 (永続化) から usecase 層へ伝わるエラー。
///
/// sqlx など具体的な永続化技術には依存しない (domain は infrastructure を知らない)。
#[derive(Debug, thiserror::Error)]
pub enum RepoError {
    #[error("not found")]
    NotFound,
    #[error("conflict: {0}")]
    Conflict(String),
    #[error("internal repository error: {0}")]
    Internal(String),
}
