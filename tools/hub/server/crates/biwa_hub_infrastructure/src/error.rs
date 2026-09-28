use biwa_hub_domain::RepoError;

/// `sqlx::Error` を domain の [`RepoError`] に写す。
///
/// domain 層は sqlx を知らない (依存性逆転) ので、ここでしか使わない。
pub(crate) fn map_sqlx_error(err: sqlx::Error) -> RepoError {
    match &err {
        sqlx::Error::RowNotFound => RepoError::NotFound,
        sqlx::Error::Database(db_err) if db_err.is_unique_violation() => {
            RepoError::Conflict(db_err.message().to_string())
        }
        _ => RepoError::Internal(err.to_string()),
    }
}
