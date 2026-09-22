#[derive(Debug, thiserror::Error)]
pub enum HubClientError {
    #[error("hub url is not configured (set `BIWA_HUB_URL` at build time, or pass one explicitly)")]
    MissingHubUrl,

    #[error("not found")]
    NotFound,

    #[error("conflict: {0}")]
    Conflict(String),

    #[error("invalid request: {0}")]
    Invalid(String),

    #[error("hub server error: {0}")]
    Server(String),

    #[error("failed to reach hub: {0}")]
    Transport(#[from] reqwest::Error),
}
