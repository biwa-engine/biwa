mod client;
mod error;

pub use client::HubClient;
pub use error::HubClientError;

// DTO はそのまま外に出す。コンパイラ/CLI 側で戻り値の中身 (バージョン一覧や
// 依存の名前など) を直接扱いたいことが多いため。
pub use biwa_hub_presentation::dto;
