use biwa_lsp_lexer::SyntaxKind;
use biwa_lsp_parser::{BiwaLanguage, parse};
use rowan::SyntaxNode;

/// LSP の SemanticTokenTypes に対応するトークン種別。
/// https://microsoft.github.io/language-server-protocol/specifications/lsp/3.17/specification/#semanticTokenTypes
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TokenType {
    Keyword,
    Type,
    Function,
    Variable,
    Parameter,
    Property,
    Number,
    String,
    Comment,
    Operator,
    Namespace,
    // ノベルモード固有
    NovelText,
    NovelCommand,
    NovelCharacter,
}

impl TokenType {
    /// LSP の `textDocument/semanticTokens` legend 用の文字列名。
    pub fn as_str(self) -> &'static str {
        match self {
            TokenType::Keyword => "keyword",
            TokenType::Type => "type",
            TokenType::Function => "function",
            TokenType::Variable => "variable",
            TokenType::Parameter => "parameter",
            TokenType::Property => "property",
            TokenType::Number => "number",
            TokenType::String => "string",
            TokenType::Comment => "comment",
            TokenType::Operator => "operator",
            TokenType::Namespace => "namespace",
            TokenType::NovelText => "novelText",
            TokenType::NovelCommand => "novelCommand",
            TokenType::NovelCharacter => "novelCharacter",
        }
    }
}

/// ソース上のハイライト範囲1つ。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HighlightToken {
    /// byte offset (開始)
    pub start: usize,
    /// byte offset (終端、exclusive)
    pub end: usize,
    pub token_type: TokenType,
}

/// ソース文字列をパースしてハイライトトークン列を返す。
///
/// 返るリストはソース上の出現順 (start の昇順)。
pub fn highlight(src: &str) -> Vec<HighlightToken> {
    let parse_result = parse(src);
    let root = parse_result.syntax();
    let mut tokens = Vec::new();
    walk(&root, &mut tokens);
    tokens.sort_by_key(|t| t.start);
    tokens
}

fn walk(node: &SyntaxNode<BiwaLanguage>, out: &mut Vec<HighlightToken>) {
    for child in node.children_with_tokens() {
        match child {
            rowan::NodeOrToken::Token(tok) => {
                if let Some(tt) = token_type_for(tok.kind(), tok.parent().map(|n| n.kind())) {
                    let range = tok.text_range();
                    out.push(HighlightToken {
                        start: usize::from(range.start()),
                        end: usize::from(range.end()),
                        token_type: tt,
                    });
                }
            }
            rowan::NodeOrToken::Node(child_node) => {
                walk(&child_node, out);
            }
        }
    }
}

/// トークン kind と親ノード kind からハイライト種別を決定する。
/// `None` を返すとそのトークンはハイライト対象外 (trivia など)。
fn token_type_for(kind: SyntaxKind, parent: Option<SyntaxKind>) -> Option<TokenType> {
    match kind {
        // ── trivia ────────────────────────────────────────────────────────────
        SyntaxKind::Whitespace | SyntaxKind::Newline => None,

        SyntaxKind::LineComment => Some(TokenType::Comment),

        // ── keywords ─────────────────────────────────────────────────────────
        SyntaxKind::KwImport
        | SyntaxKind::KwAs
        | SyntaxKind::KwFn
        | SyntaxKind::KwStruct
        | SyntaxKind::KwType
        | SyntaxKind::KwImpl
        | SyntaxKind::KwScene
        | SyntaxKind::KwLet
        | SyntaxKind::KwIf
        | SyntaxKind::KwElse
        | SyntaxKind::KwWhile
        | SyntaxKind::KwFor
        | SyntaxKind::KwIn
        | SyntaxKind::KwSelf
        | SyntaxKind::KwReturn
        | SyntaxKind::KwPackage => Some(TokenType::Keyword),

        // ── built-in types ────────────────────────────────────────────────────
        SyntaxKind::KwVoid
        | SyntaxKind::KwInt
        | SyntaxKind::KwUint
        | SyntaxKind::KwFloat
        | SyntaxKind::KwBool => Some(TokenType::Type),

        // ── literals ──────────────────────────────────────────────────────────
        SyntaxKind::TrueLiteral | SyntaxKind::FalseLiteral | SyntaxKind::NoneLiteral => {
            Some(TokenType::Keyword)
        }

        SyntaxKind::IntLiteral | SyntaxKind::FloatLiteral => Some(TokenType::Number),

        SyntaxKind::StringLiteral => Some(TokenType::String),

        // ── identifiers: 親ノードによって種別を変える ─────────────────────────
        SyntaxKind::Ident => match parent {
            Some(SyntaxKind::FunctionDef) | Some(SyntaxKind::MethodDef) => {
                Some(TokenType::Function)
            }

            Some(SyntaxKind::FunctionArgDecl) | Some(SyntaxKind::MethodArgDecl) => {
                Some(TokenType::Parameter)
            }

            Some(SyntaxKind::StructDef)
            | Some(SyntaxKind::TypeAliasDef)
            | Some(SyntaxKind::ImplBlock) => Some(TokenType::Type),

            Some(SyntaxKind::StructLiteralField) => Some(TokenType::Property),

            Some(SyntaxKind::PostfixExpr) => Some(TokenType::Property),

            Some(SyntaxKind::ImportDecl) => Some(TokenType::Namespace),

            _ => Some(TokenType::Variable),
        },

        // ── operators / punctuation ───────────────────────────────────────────
        SyntaxKind::Plus
        | SyntaxKind::Minus
        | SyntaxKind::Star
        | SyntaxKind::Slash
        | SyntaxKind::Percent
        | SyntaxKind::Eq
        | SyntaxKind::EqEq
        | SyntaxKind::BangEq
        | SyntaxKind::Lt
        | SyntaxKind::LtEq
        | SyntaxKind::Gt
        | SyntaxKind::GtEq
        | SyntaxKind::AmpAmp
        | SyntaxKind::PipePipe
        | SyntaxKind::Bang
        | SyntaxKind::Arrow
        | SyntaxKind::ColonColon => Some(TokenType::Operator),

        // ── novel mode ────────────────────────────────────────────────────────
        SyntaxKind::DoubleLBrace | SyntaxKind::DoubleRBrace => Some(TokenType::Keyword),
        SyntaxKind::NovelText => Some(TokenType::NovelText),
        SyntaxKind::NovelAt => Some(TokenType::NovelCharacter),
        SyntaxKind::NovelHash => Some(TokenType::NovelCommand),
        SyntaxKind::NovelDollarBrace | SyntaxKind::NovelCloseBrace => Some(TokenType::Operator),

        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn types(src: &str) -> Vec<TokenType> {
        highlight(src).into_iter().map(|t| t.token_type).collect()
    }

    #[test]
    fn highlight_fn_keyword() {
        let src = "fn add(x: Int) -> Int { x }";
        let ts = types(src);
        assert!(ts.contains(&TokenType::Keyword));
        assert!(ts.contains(&TokenType::Type));
        assert!(ts.contains(&TokenType::Function));
        assert!(ts.contains(&TokenType::Parameter));
    }

    #[test]
    fn highlight_comment() {
        let src = "// hello\nfn f() -> Void {}";
        let ts = types(src);
        assert!(ts.contains(&TokenType::Comment));
    }

    #[test]
    fn highlight_numbers() {
        let src = "fn f() -> Int { 42 }";
        let ts = types(src);
        assert!(ts.contains(&TokenType::Number));
    }

    #[test]
    fn highlight_string() {
        let src = r#"fn f() -> Void { let s = "hello"; }"#;
        let ts = types(src);
        assert!(ts.contains(&TokenType::String));
    }

    #[test]
    fn highlight_novel_mode() {
        let src = "scene s(g: G) -> G {{\n  @biwa\n  Hello!\n}}\n";
        let ts = types(src);
        assert!(ts.contains(&TokenType::NovelCharacter));
        assert!(ts.contains(&TokenType::NovelText));
    }

    #[test]
    fn highlight_novel_command() {
        let src = "scene s(g: G) -> G {{\n  #play_se(se1)\n}}\n";
        let ts = types(src);
        assert!(ts.contains(&TokenType::NovelCommand));
    }

    #[test]
    fn sorted_by_start() {
        let src = "fn add(x: Int) -> Int { x }";
        let hs = highlight(src);
        let starts: Vec<usize> = hs.iter().map(|t| t.start).collect();
        let mut sorted = starts.clone();
        sorted.sort();
        assert_eq!(
            starts, sorted,
            "highlight tokens must be sorted by start offset"
        );
    }
}
