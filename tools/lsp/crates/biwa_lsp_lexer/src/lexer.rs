use crate::pre_scan::{Segment, pre_scan};
use crate::syntax_kind::SyntaxKind;
use logos::Logos;

/// ソース上のトークン1つ。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Token {
    pub kind: SyntaxKind,
    /// ソース上の byte offset (開始)
    pub start: usize,
    /// ソース上の byte offset (終端、exclusive)
    pub end: usize,
}

impl Token {
    pub fn text<'s>(&self, src: &'s str) -> &'s str {
        &src[self.start..self.end]
    }
}

// ────────────────────────────────────────────────────────────────────────────
// 通常モード用ロゴスレキサ
// ────────────────────────────────────────────────────────────────────────────

#[derive(Logos, Debug, Clone, PartialEq)]
#[logos(skip r"[ \t\r]+")] // 水平空白は skip (Whitespace として後で保持したい場合は変更)
enum CodeToken {
    // trivia
    #[token("\n")]
    Newline,
    #[regex(r"//[^\n]*")]
    LineComment,

    // literals
    #[regex(r"0x[0-9a-fA-F]+")]
    #[regex(r"0b[01]+")]
    #[regex(r"[0-9]+u?")]
    IntLiteral,
    #[regex(r"[0-9]+\.[0-9]+")]
    FloatLiteral,
    #[regex(r#""[^"\n]*""#)]
    StringLiteral,

    // keywords / built-in types
    #[token("import")]
    KwImport,
    #[token("as")]
    KwAs,
    #[token("fn")]
    KwFn,
    #[token("struct")]
    KwStruct,
    #[token("type")]
    KwType,
    #[token("impl")]
    KwImpl,
    #[token("scene")]
    KwScene,
    #[token("let")]
    KwLet,
    #[token("if")]
    KwIf,
    #[token("else")]
    KwElse,
    #[token("while")]
    KwWhile,
    #[token("for")]
    KwFor,
    #[token("in")]
    KwIn,
    #[token("self")]
    KwSelf,
    #[token("return")]
    KwReturn,
    #[token("package")]
    KwPackage,
    #[token("Void")]
    KwVoid,
    #[token("Int")]
    KwInt,
    #[token("Uint")]
    KwUint,
    #[token("Float")]
    KwFloat,
    #[token("Bool")]
    KwBool,
    #[token("TRUE")]
    TrueLiteral,
    #[token("FALSE")]
    FalseLiteral,
    #[token("NONE")]
    NoneLiteral,

    // identifier
    #[regex(r"_*[a-zA-Z][0-9a-zA-Z_]*")]
    Ident,

    // multi-char punctuation (longer must come first)
    #[token("::")]
    ColonColon,
    #[token("->")]
    Arrow,
    #[token("=>")]
    FatArrow,
    #[token("==")]
    EqEq,
    #[token("!=")]
    BangEq,
    #[token("<=")]
    LtEq,
    #[token(">=")]
    GtEq,
    #[token("&&")]
    AmpAmp,
    #[token("||")]
    PipePipe,
    #[token("{{")]
    DoubleLBrace,
    #[token("}}")]
    DoubleRBrace,

    // single-char punctuation
    #[token(";")]
    Semi,
    #[token(":")]
    Colon,
    #[token(",")]
    Comma,
    #[token(".")]
    Dot,
    #[token("=")]
    Eq,
    #[token("!")]
    Bang,
    #[token("+")]
    Plus,
    #[token("-")]
    Minus,
    #[token("*")]
    Star,
    #[token("/")]
    Slash,
    #[token("%")]
    Percent,
    #[token("<")]
    Lt,
    #[token(">")]
    Gt,
    #[token("(")]
    LParen,
    #[token(")")]
    RParen,
    #[token("{")]
    LBrace,
    #[token("}")]
    RBrace,
    #[token("[")]
    LBracket,
    #[token("]")]
    RBracket,
}

fn code_token_to_syntax_kind(t: &CodeToken) -> SyntaxKind {
    match t {
        CodeToken::Newline => SyntaxKind::Newline,
        CodeToken::LineComment => SyntaxKind::LineComment,
        CodeToken::IntLiteral => SyntaxKind::IntLiteral,
        CodeToken::FloatLiteral => SyntaxKind::FloatLiteral,
        CodeToken::StringLiteral => SyntaxKind::StringLiteral,
        CodeToken::TrueLiteral => SyntaxKind::TrueLiteral,
        CodeToken::FalseLiteral => SyntaxKind::FalseLiteral,
        CodeToken::NoneLiteral => SyntaxKind::NoneLiteral,
        CodeToken::KwImport => SyntaxKind::KwImport,
        CodeToken::KwAs => SyntaxKind::KwAs,
        CodeToken::KwFn => SyntaxKind::KwFn,
        CodeToken::KwStruct => SyntaxKind::KwStruct,
        CodeToken::KwType => SyntaxKind::KwType,
        CodeToken::KwImpl => SyntaxKind::KwImpl,
        CodeToken::KwScene => SyntaxKind::KwScene,
        CodeToken::KwLet => SyntaxKind::KwLet,
        CodeToken::KwIf => SyntaxKind::KwIf,
        CodeToken::KwElse => SyntaxKind::KwElse,
        CodeToken::KwWhile => SyntaxKind::KwWhile,
        CodeToken::KwFor => SyntaxKind::KwFor,
        CodeToken::KwIn => SyntaxKind::KwIn,
        CodeToken::KwSelf => SyntaxKind::KwSelf,
        CodeToken::KwReturn => SyntaxKind::KwReturn,
        CodeToken::KwPackage => SyntaxKind::KwPackage,
        CodeToken::KwVoid => SyntaxKind::KwVoid,
        CodeToken::KwInt => SyntaxKind::KwInt,
        CodeToken::KwUint => SyntaxKind::KwUint,
        CodeToken::KwFloat => SyntaxKind::KwFloat,
        CodeToken::KwBool => SyntaxKind::KwBool,
        CodeToken::Ident => SyntaxKind::Ident,
        CodeToken::ColonColon => SyntaxKind::ColonColon,
        CodeToken::Arrow => SyntaxKind::Arrow,
        CodeToken::FatArrow => SyntaxKind::FatArrow,
        CodeToken::EqEq => SyntaxKind::EqEq,
        CodeToken::BangEq => SyntaxKind::BangEq,
        CodeToken::LtEq => SyntaxKind::LtEq,
        CodeToken::GtEq => SyntaxKind::GtEq,
        CodeToken::AmpAmp => SyntaxKind::AmpAmp,
        CodeToken::PipePipe => SyntaxKind::PipePipe,
        CodeToken::DoubleLBrace => SyntaxKind::DoubleLBrace,
        CodeToken::DoubleRBrace => SyntaxKind::DoubleRBrace,
        CodeToken::Semi => SyntaxKind::Semi,
        CodeToken::Colon => SyntaxKind::Colon,
        CodeToken::Comma => SyntaxKind::Comma,
        CodeToken::Dot => SyntaxKind::Dot,
        CodeToken::Eq => SyntaxKind::Eq,
        CodeToken::Bang => SyntaxKind::Bang,
        CodeToken::Plus => SyntaxKind::Plus,
        CodeToken::Minus => SyntaxKind::Minus,
        CodeToken::Star => SyntaxKind::Star,
        CodeToken::Slash => SyntaxKind::Slash,
        CodeToken::Percent => SyntaxKind::Percent,
        CodeToken::Lt => SyntaxKind::Lt,
        CodeToken::Gt => SyntaxKind::Gt,
        CodeToken::LParen => SyntaxKind::LParen,
        CodeToken::RParen => SyntaxKind::RParen,
        CodeToken::LBrace => SyntaxKind::LBrace,
        CodeToken::RBrace => SyntaxKind::RBrace,
        CodeToken::LBracket => SyntaxKind::LBracket,
        CodeToken::RBracket => SyntaxKind::RBracket,
    }
}

// ────────────────────────────────────────────────────────────────────────────
// ノベルモード用ロゴスレキサ
// ────────────────────────────────────────────────────────────────────────────

/// ノベルモード区間の字句解析。
/// `{{` と行頭 `}}` トークンも含む (pre_scan が渡す区間には両端が入っている)。
#[derive(Logos, Debug, Clone, PartialEq)]
enum NovelToken {
    /// {{ (ノベルモード開始)
    #[token("{{")]
    DoubleLBrace,

    /// }} (ノベルモード終了; pre_scan により必ず行頭に来る)
    #[token("}}")]
    DoubleRBrace,

    /// @identifier (キャラクター指定行)
    #[regex(r"@[^\n]*")]
    AtLine,

    /// #... (コマンド行) - 行全体を1トークンとして扱う
    #[regex(r"#[^\n]*")]
    HashLine,

    /// ${ (値埋め込み開始)
    #[token("${")]
    DollarBrace,

    /// } (値埋め込み終了)
    #[token("}")]
    CloseBrace,

    /// 改行
    #[token("\n")]
    Newline,

    /// // コメント行
    #[regex(r"//[^\n]*")]
    LineComment,

    /// プレーンなテキスト (改行・特殊文字以外)
    #[regex(r"[^\n@#${}]+")]
    Text,
}

fn lex_novel_segment(src: &str, offset: usize, out: &mut Vec<Token>) {
    let mut lexer = NovelToken::lexer(src);
    while let Some(result) = lexer.next() {
        let span = lexer.span();
        let start = offset + span.start;
        let end = offset + span.end;
        let kind = match result {
            Ok(NovelToken::DoubleLBrace) => SyntaxKind::DoubleLBrace,
            Ok(NovelToken::DoubleRBrace) => SyntaxKind::DoubleRBrace,
            Ok(NovelToken::AtLine) => SyntaxKind::NovelAt,
            Ok(NovelToken::HashLine) => SyntaxKind::NovelHash,
            Ok(NovelToken::DollarBrace) => SyntaxKind::NovelDollarBrace,
            Ok(NovelToken::CloseBrace) => SyntaxKind::NovelCloseBrace,
            Ok(NovelToken::Newline) => SyntaxKind::Newline,
            Ok(NovelToken::LineComment) => SyntaxKind::LineComment,
            Ok(NovelToken::Text) => SyntaxKind::NovelText,
            Err(_) => SyntaxKind::Error,
        };
        out.push(Token { kind, start, end });
    }
}

// ────────────────────────────────────────────────────────────────────────────
// 公開 API
// ────────────────────────────────────────────────────────────────────────────

/// ソース文字列全体をトークン列に変換する。
/// 水平空白(スペース・タブ)は Whitespace トークンとして保持する。
pub fn lex(src: &str) -> Vec<Token> {
    let segments = pre_scan(src);
    let mut tokens = Vec::new();

    for seg in segments {
        match seg {
            Segment::Code { start, end } => {
                lex_code_segment(&src[start..end], start, &mut tokens);
            }
            Segment::Novel { start, end } => {
                lex_novel_segment(&src[start..end], start, &mut tokens);
            }
        }
    }

    tokens.push(Token {
        kind: SyntaxKind::Eof,
        start: src.len(),
        end: src.len(),
    });
    tokens
}

fn lex_code_segment(src: &str, offset: usize, out: &mut Vec<Token>) {
    // logos の skip で水平空白を捨てているが、CST では保持したいので
    // 手動でスペースを走査する
    let bytes = src.as_bytes();
    let len = bytes.len();
    let mut pos = 0;

    while pos < len {
        // 先に水平空白をまとめてWhitespaceトークンへ
        if bytes[pos] == b' ' || bytes[pos] == b'\t' {
            let ws_start = pos;
            while pos < len && (bytes[pos] == b' ' || bytes[pos] == b'\t') {
                pos += 1;
            }
            out.push(Token {
                kind: SyntaxKind::Whitespace,
                start: offset + ws_start,
                end: offset + pos,
            });
            continue;
        }

        // logos でトークナイズ (残りの部分を渡してスパン0から取る)
        let slice = &src[pos..];
        let mut lexer = CodeToken::lexer(slice);
        if let Some(result) = lexer.next() {
            let span = lexer.span();
            let tok_start = offset + pos + span.start;
            let tok_end = offset + pos + span.end;
            let kind = match result {
                Ok(ref t) => code_token_to_syntax_kind(t),
                Err(_) => SyntaxKind::Error,
            };
            out.push(Token {
                kind,
                start: tok_start,
                end: tok_end,
            });
            pos += span.end;
        } else {
            // logos が何もマッチしない (ありえないはずだが防衛的に)
            out.push(Token {
                kind: SyntaxKind::Error,
                start: offset + pos,
                end: offset + pos + 1,
            });
            pos += 1;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kinds(src: &str) -> Vec<SyntaxKind> {
        lex(src).into_iter().map(|t| t.kind).collect()
    }

    #[test]
    fn lex_fn_decl() {
        let src = "fn add(x: Int) -> Int { x }";
        let ks = kinds(src);
        assert!(ks.contains(&SyntaxKind::KwFn));
        assert!(ks.contains(&SyntaxKind::Ident));
        assert!(ks.contains(&SyntaxKind::KwInt));
        assert!(ks.contains(&SyntaxKind::Arrow));
        assert_eq!(*ks.last().unwrap(), SyntaxKind::Eof);
    }

    #[test]
    fn lex_novel_mode() {
        let src = "scene s(g: G) -> G {{\n  Hello!\n}}\n";
        let ks = kinds(src);
        assert!(ks.contains(&SyntaxKind::KwScene));
        assert!(ks.contains(&SyntaxKind::DoubleLBrace));
        assert!(ks.contains(&SyntaxKind::NovelText));
        assert!(ks.contains(&SyntaxKind::DoubleRBrace));
    }

    #[test]
    fn lex_novel_at() {
        let src = "scene s(g: G) -> G {{\n@biwa\nこんにちは\n}}\n";
        let ks = kinds(src);
        assert!(ks.contains(&SyntaxKind::NovelAt));
        assert!(ks.contains(&SyntaxKind::NovelText));
    }

    #[test]
    fn lex_novel_hash() {
        let src = "scene s(g: G) -> G {{\n#play_se(se1)\n}}\n";
        let ks = kinds(src);
        assert!(ks.contains(&SyntaxKind::NovelHash));
    }
}
