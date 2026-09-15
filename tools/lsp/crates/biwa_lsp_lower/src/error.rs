use biwac_span::Span;

/// CST から `biwac_ast` への lowering が構文要素を落とした箇所。
///
/// biwac_parser と違い、この lowering は `Result` で止まらない。
/// 表現できない構文に出会っても、その部分だけを諦めて残りの木を組み立て続ける
/// (rust-analyzer の `hir::lower` と同じ「salvage」方針)。
/// このリストは「壊れているのでコンパイルできない」ではなく
/// 「今の biwa-lsp-parser / biwac_ast の対応範囲の外にある」ことを表す。
#[derive(Debug, Clone)]
pub struct LowerError {
    pub message: String,
    pub span: Span,
}

impl LowerError {
    pub fn new(message: impl Into<String>, span: Span) -> Self {
        Self {
            message: message.into(),
            span,
        }
    }
}
