use crate::grammar::{ParseResult, parse_root};
use biwa_lsp_lexer::{SyntaxKind, Token, lex};
use rowan::GreenNodeBuilder;

/// 構文エラー1件。`start`/`end` はソース全体に対する byte offset で、
/// LSP の `Range` (line/col) への変換は呼び出し側 (biwa_lsp_server) が行う。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParseError {
    pub message: String,
    pub start: usize,
    pub end: usize,
}

/// トークン列を保持しながら CST を構築するパーサ状態。
pub struct Parser<'src> {
    src: &'src str,
    pub(crate) tokens: Vec<Token>,
    pub(crate) pos: usize,
    pub(crate) builder: GreenNodeBuilder<'static>,
    pub(crate) errors: Vec<ParseError>,
    /// 構造体リテラルを式として認めない区間にいるか。
    ///
    /// `if`/`while`/`for .. in`/`match` の対象式は直後にブロックの `{` が来るため、
    /// `if flag {` の `{` を構造体リテラルの開始と読むと必ず誤る
    /// (biwac_parser の `TokenStream::no_struct_literal` と同じ)。
    pub(crate) no_struct_literal: bool,
}

impl<'src> Parser<'src> {
    fn new(src: &'src str) -> Self {
        let tokens = lex(src);
        Parser {
            src,
            tokens,
            pos: 0,
            builder: GreenNodeBuilder::new(),
            errors: Vec::new(),
            no_struct_literal: false,
        }
    }

    // ── トークン参照 ──────────────────────────────────────────────────────────

    /// 現在トークンの kind。EOF なら Eof を返す。
    pub(crate) fn current(&self) -> SyntaxKind {
        self.nth(0)
    }

    /// pos+n のトークンの kind (trivia をスキップしない)。
    pub(crate) fn nth(&self, n: usize) -> SyntaxKind {
        self.tokens
            .get(self.pos + n)
            .map(|t| t.kind)
            .unwrap_or(SyntaxKind::Eof)
    }

    /// trivia (Whitespace, Newline, LineComment) をスキップした先の kind。
    pub(crate) fn current_non_trivia(&self) -> SyntaxKind {
        let mut i = self.pos;
        loop {
            match self.tokens.get(i).map(|t| t.kind) {
                Some(SyntaxKind::Whitespace)
                | Some(SyntaxKind::Newline)
                | Some(SyntaxKind::LineComment) => i += 1,
                Some(k) => return k,
                None => return SyntaxKind::Eof,
            }
        }
    }

    pub(crate) fn at(&self, kind: SyntaxKind) -> bool {
        self.current_non_trivia() == kind
    }

    /// 現在位置のトークンの byte range。EOF ならソース末尾 (長さ0) を指す。
    pub(crate) fn current_span(&self) -> (usize, usize) {
        match self.tokens.get(self.pos) {
            Some(t) => (t.start, t.end),
            None => (self.src.len(), self.src.len()),
        }
    }

    /// 現在位置を指すエラーを記録する。
    pub(crate) fn push_error(&mut self, message: impl Into<String>) {
        let (start, end) = self.current_span();
        self.errors.push(ParseError {
            message: message.into(),
            start,
            end,
        });
    }

    // ── トークン消費 ──────────────────────────────────────────────────────────

    /// 1トークンを CST に追加して進む。
    pub(crate) fn bump(&mut self) {
        let tok = &self.tokens[self.pos];
        let text = &self.src[tok.start..tok.end];
        self.builder.token(tok.kind.into(), text);
        self.pos += 1;
    }

    /// trivia を CST に追加しながら読み飛ばす。
    pub(crate) fn skip_trivia(&mut self) {
        while matches!(
            self.current(),
            SyntaxKind::Whitespace | SyntaxKind::Newline | SyntaxKind::LineComment
        ) {
            self.bump();
        }
    }

    /// trivia をスキップし、期待 kind を消費。失敗ならエラートークンを挿入。
    pub(crate) fn expect(&mut self, kind: SyntaxKind) {
        self.skip_trivia();
        if self.current() == kind {
            self.bump();
        } else {
            let msg = format!("expected {:?} but found {:?}", kind, self.current());
            self.push_error(msg);
            // エラー回復: エラーノードとして現在トークンを消費
            if self.current() != SyntaxKind::Eof {
                self.builder.start_node(SyntaxKind::Error.into());
                self.bump();
                self.builder.finish_node();
            }
        }
    }

    // ── ノード管理 ────────────────────────────────────────────────────────────

    pub(crate) fn start_node(&mut self, kind: SyntaxKind) {
        self.builder.start_node(kind.into());
    }

    pub(crate) fn finish_node(&mut self) {
        self.builder.finish_node();
    }
}

/// ソース文字列をパースして ParseResult を返す。
pub fn parse(src: &str) -> ParseResult {
    let mut p = Parser::new(src);
    parse_root(&mut p);
    let green = p.builder.finish();
    ParseResult {
        green_node: green,
        errors: p.errors,
    }
}
