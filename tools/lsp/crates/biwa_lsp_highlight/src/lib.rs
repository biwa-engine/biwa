//! Biwa 言語のシンタックスハイライト。
//!
//! CST (rowan の SyntaxNode) を走査し、LSP の semantic tokens 互換の
//! `HighlightToken` リストを生成する。

pub mod highlight;

pub use highlight::{HighlightToken, TokenType, highlight};

use biwa_lsp_parser::ParseResult;

/// LSP の diagnostics 用にパース結果(エラーリスト付き)を返す。
pub fn parse_for_diagnostics(src: &str) -> ParseResult {
    biwa_lsp_parser::parse(src)
}
