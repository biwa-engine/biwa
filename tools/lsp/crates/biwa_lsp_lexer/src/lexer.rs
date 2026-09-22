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
    // `\"` を正しく飛ばさないと `"\"foo\""` のようなエスケープされた `"` を
    // 含む文字列が途中で閉じたことになってしまう (`[^"\n]*` は `\` の次の
    // 文字を特別扱いしないため)。biwac_lexer と同じ対応表・走査規則
    // (`biwac_base::string_body_end`) を共有し、コールバックで手動走査する。
    #[token("\"", lex_string_literal)]
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
    #[token("Self")]
    KwSelfType,
    #[token("return")]
    KwReturn,
    #[token("endscene")]
    KwEndScene,
    #[token("package")]
    KwPackage,
    #[token("enum")]
    KwEnum,
    #[token("match")]
    KwMatch,
    #[token("trait")]
    KwTrait,
    #[token("_")]
    KwUnderscore,
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

/// 開き `"` (トークンとしてはすでに消費済み) から続きを走査し、
/// エスケープを正しく飛ばしながら閉じ `"` まで読み進める。
///
/// biwa の文字列はコード中では改行をまたがないので (`docs/lexical.md`)、
/// 閉じが見つからない場合も行末までで打ち切る
/// (`biwac_lexer::divide_regions` が改行に対してエラーにするのと同じ規則。
/// ただしこちらはエラー耐性のため、行末までを「壊れた文字列」として
/// そのままトークンにする)。
fn lex_string_literal(lex: &mut logos::Lexer<CodeToken>) -> bool {
    let rest = lex.remainder();
    let line_end = rest.find('\n').unwrap_or(rest.len());
    let window = &rest[..line_end];
    match biwac_base::string_body_end(window) {
        Some(body_len) => lex.bump(body_len + 1), // +1: 閉じ `"` 自身
        None => lex.bump(window.len()),
    }
    true
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
        CodeToken::KwSelfType => SyntaxKind::KwSelfType,
        CodeToken::KwReturn => SyntaxKind::KwReturn,
        CodeToken::KwEndScene => SyntaxKind::KwEndScene,
        CodeToken::KwPackage => SyntaxKind::KwPackage,
        CodeToken::KwEnum => SyntaxKind::KwEnum,
        CodeToken::KwMatch => SyntaxKind::KwMatch,
        CodeToken::KwTrait => SyntaxKind::KwTrait,
        CodeToken::KwUnderscore => SyntaxKind::KwUnderscore,
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
// ノベルモード用の手書きスキャナ
// ────────────────────────────────────────────────────────────────────────────
//
// `biwac_novel_parser` (`scan.rs`, `token.rs`) と同じく、ノベル DSL は
// 正規表現 1 発では読めない (行ごとに「これはコード行か地の文か」が決まり、
// `#` コマンド行は `(`/`[`/`,`/`.`/`::` で終わると次の行へ継続する)。
// そのため logos ではなく手書きの行指向スキャナにしてある。
//
// `docs/lexical.md`, `docs/content-api.md` を参照。

/// `#` コマンド行の継続判定に使う。行の最後の非トリビアトークンがこれらなら、
/// 次の行もまだ同じコマンドの続きである。
fn continues_over_line(kind: SyntaxKind) -> bool {
    matches!(
        kind,
        SyntaxKind::LParen
            | SyntaxKind::LBracket
            | SyntaxKind::Comma
            | SyntaxKind::Dot
            | SyntaxKind::ColonColon
    )
}

fn is_trivia_kind(kind: SyntaxKind) -> bool {
    matches!(
        kind,
        SyntaxKind::Whitespace | SyntaxKind::Newline | SyntaxKind::LineComment
    )
}

/// `src[..limit]` の中で `begin` から続く識別子の終わり。識別子でなければ `begin`。
fn novel_ident_end(src: &str, begin: usize, limit: usize) -> usize {
    let mut i = begin;
    for (off, c) in src[begin..limit].char_indices() {
        let ok = if off == 0 {
            c.is_ascii_alphabetic() || c == '_'
        } else {
            c.is_ascii_alphanumeric() || c == '_'
        };
        if !ok {
            break;
        }
        i = begin + off + c.len_utf8();
    }
    i
}

/// 開き `"` の位置から、閉じ `"` の次の位置を返す。閉じが無ければ `limit`。
/// エスケープの対応表は biwac_base のものを使う (`docs/lexical.md`)。
fn skip_string_literal(src: &str, quote_pos: usize, limit: usize) -> usize {
    let body_start = quote_pos + 1;
    match biwac_base::string_body_end(&src[body_start..limit]) {
        Some(body_len) => body_start + body_len + 1,
        None => limit,
    }
}

/// `(` から対応する `)` の次の位置。文字列リテラルの中の括弧は数えない。
fn balanced_paren_end(src: &str, begin: usize, limit: usize) -> Option<usize> {
    let mut depth = 0usize;
    let mut i = begin;
    while i < limit {
        let c = src[i..limit].chars().next().expect("in range");
        match c {
            '"' => {
                i = skip_string_literal(src, i, limit);
                continue;
            }
            '(' => depth += 1,
            ')' => {
                depth -= 1;
                if depth == 0 {
                    return Some(i + c.len_utf8());
                }
            }
            _ => {}
        }
        i += c.len_utf8();
    }
    None
}

/// `$` の直後 `begin` から、埋め込み式の終端を探す。見つかれば
/// `$` そのものと式の中身 (通常コードのトークン列として) を `out` に積み、
/// 終端位置を返す。文法が壊れていれば `None` (呼び出し側が `$` だけを
/// エラーとして処理し、残りは地の文として読み進める)。
///
/// ```ebnf
/// <embeded-expression> ::= `$` `(` <expression> `)`
///   | `$` <identifier> ( <argument-list> | <member-access-or-method-calling>* <method-calling> )
/// ```
/// (`docs/content-api.md`)
fn scan_embedded_expr(
    src: &str,
    offset: usize,
    dollar_pos: usize,
    limit: usize,
    out: &mut Vec<Token>,
) -> Option<usize> {
    let after_dollar = dollar_pos + 1;
    let mut i = after_dollar;
    let mut ends_with_call;

    match src[i..limit].chars().next() {
        Some('(') => {
            i = balanced_paren_end(src, i, limit)?;
            ends_with_call = true;
        }
        Some(c) if c.is_ascii_alphabetic() || c == '_' => {
            i = novel_ident_end(src, i, limit);
            if src[i..limit].starts_with('(') {
                i = balanced_paren_end(src, i, limit)?;
                ends_with_call = true;
            } else {
                ends_with_call = false;
            }
        }
        _ => return None,
    }

    while src[i..limit].starts_with('.') {
        let next = novel_ident_end(src, i + 1, limit);
        if next == i + 1 {
            return None;
        }
        i = next;
        if src[i..limit].starts_with('(') {
            i = balanced_paren_end(src, i, limit)?;
            ends_with_call = true;
        } else {
            ends_with_call = false;
        }
    }

    if !ends_with_call {
        return None;
    }

    out.push(Token {
        kind: SyntaxKind::NovelDollar,
        start: offset + dollar_pos,
        end: offset + after_dollar,
    });
    // 埋め込み式の中身は通常コードそのものなので、通常コードのレキサに委ねる。
    lex_code_segment(&src[after_dollar..i], offset + after_dollar, out);

    Some(i)
}

fn push_text(out: &mut Vec<Token>, offset: usize, from: usize, to: usize) {
    if to > from {
        out.push(Token {
            kind: SyntaxKind::NovelText,
            start: offset + from,
            end: offset + to,
        });
    }
}

/// 地の文 (`@` 行の識別子より後ろも含む) を、`//` コメントと `$` 埋め込み式を
/// 見分けながら `[start, limit)` の範囲で読む。
fn scan_novel_text(src: &str, offset: usize, start: usize, limit: usize, out: &mut Vec<Token>) {
    // `>>` (「待ち」コマンド。積んだ内容をまとめて出してクリックを待つ)。
    // 行 (trailing の空白を除く) の末尾がこれなら、そこで地の文を打ち切る。
    // `biwac_novel_parser` の `WAIT_COMMAND` / `consume_raw_novel_line` と同じ規則
    // (`docs/content-api.md`)。
    let trimmed_end = src[start..limit].trim_end_matches([' ', '\t']).len();
    let wait_at = if trimmed_end >= 2 && src[start..start + trimmed_end].ends_with(">>") {
        Some(start + trimmed_end - 2)
    } else {
        None
    };
    let text_limit = wait_at.unwrap_or(limit);

    let mut i = start;
    let mut text_start = start;

    while i < text_limit {
        let rest = &src[i..text_limit];
        let c = rest.chars().next().expect("in range");

        if rest.starts_with("//") {
            push_text(out, offset, text_start, i);
            out.push(Token {
                kind: SyntaxKind::LineComment,
                start: offset + i,
                end: offset + text_limit,
            });
            if let Some(wait_at) = wait_at {
                out.push(Token {
                    kind: SyntaxKind::NovelWait,
                    start: offset + wait_at,
                    end: offset + wait_at + 2,
                });
                push_text(out, offset, wait_at + 2, limit);
            }
            return;
        }

        // `\$` は `$` そのもの。埋め込み式を開かない (`docs/lexical.md`)。
        if c == '\\' && rest[c.len_utf8()..].starts_with('$') {
            i += c.len_utf8() + 1;
            continue;
        }

        if c == '$' {
            push_text(out, offset, text_start, i);
            match scan_embedded_expr(src, offset, i, text_limit, out) {
                Some(end) => {
                    i = end;
                }
                None => {
                    // 形が壊れている: `$` だけをエラーにして、残りは地の文として読み進める。
                    // (本文の診断は上位のパーサが span 付きで出す。)
                    out.push(Token {
                        kind: SyntaxKind::Error,
                        start: offset + i,
                        end: offset + i + c.len_utf8(),
                    });
                    i += c.len_utf8();
                }
            }
            text_start = i;
            continue;
        }

        i += c.len_utf8();
    }
    push_text(out, offset, text_start, i);

    if let Some(wait_at) = wait_at {
        out.push(Token {
            kind: SyntaxKind::NovelWait,
            start: offset + wait_at,
            end: offset + wait_at + 2,
        });
        push_text(out, offset, wait_at + 2, limit);
    }
}

/// `#` の位置から、継続する限り複数行にまたがるコマンドを読む。
/// 戻り値は「論理的なコマンドが終わった位置」(まだ改行は消費していない)。
fn scan_hash_command(
    src: &str,
    offset: usize,
    hash_pos: usize,
    len: usize,
    out: &mut Vec<Token>,
) -> usize {
    out.push(Token {
        kind: SyntaxKind::NovelHash,
        start: offset + hash_pos,
        end: offset + hash_pos + 1,
    });
    let mut pos = hash_pos + 1;

    loop {
        let mut line_end = pos;
        while line_end < len && src.as_bytes()[line_end] != b'\n' {
            line_end += 1;
        }

        let before = out.len();
        lex_code_segment(&src[pos..line_end], offset + pos, out);
        pos = line_end;

        let continues = out[before..]
            .iter()
            .rev()
            .find(|t| !is_trivia_kind(t.kind))
            .is_some_and(|t| continues_over_line(t.kind));

        if !continues || pos >= len {
            return pos;
        }

        // 継続する: 改行だけ消費して次の行も同じコマンドとして読み続ける。
        out.push(Token {
            kind: SyntaxKind::Newline,
            start: offset + pos,
            end: offset + pos + 1,
        });
        pos += 1;
    }
}

fn lex_novel_segment(src: &str, offset: usize, out: &mut Vec<Token>) {
    let bytes = src.as_bytes();
    let len = bytes.len();

    out.push(Token {
        kind: SyntaxKind::DoubleLBrace,
        start: offset,
        end: offset + 2,
    });
    let mut pos = 2;

    while pos < len {
        let line_start = pos;
        let mut line_end = pos;
        while line_end < len && bytes[line_end] != b'\n' {
            line_end += 1;
        }
        let has_newline = line_end < len;

        // 行頭の空白 (インデント) は `#`/`@`/`}}` 行の種別判定では読み飛ばすが、
        // rowan の CST は lossless (ソースの全バイトがどれかのトークンに属する)
        // でなければならないので、判定用に読み飛ばした分も必ず別のトークンとして
        // 出す (`Whitespace`)。生地の文では表示に反映される内容なので、
        // こちらは読み飛ばさず `line_start` からそのままテキストとして読む
        // (下の `else` 分岐参照)。
        let mut content_start = line_start;
        while content_start < line_end
            && (bytes[content_start] == b' ' || bytes[content_start] == b'\t')
        {
            content_start += 1;
        }
        let push_indent = |out: &mut Vec<Token>| {
            if content_start > line_start {
                out.push(Token {
                    kind: SyntaxKind::Whitespace,
                    start: offset + line_start,
                    end: offset + content_start,
                });
            }
        };

        let is_end_brace = content_start + 1 < len
            && bytes[content_start] == b'}'
            && bytes[content_start + 1] == b'}';
        // 単独の `}` 行。`#if cond {` のように `#` コマンド行の末尾に置かれた
        // `{` (ordinary code token として lex_code_segment 側で `LBrace` になる)
        // に対応するブロック終端で、`}}` (scene 全体の終端) とは別物。
        // 同じ `RBrace` トークンにしておくと、対応する `LBrace` と種類が揃う。
        let is_single_brace_close = !is_end_brace
            && content_start < line_end
            && bytes[content_start] == b'}';

        if is_end_brace {
            push_indent(out);
            out.push(Token {
                kind: SyntaxKind::DoubleRBrace,
                start: offset + content_start,
                end: offset + content_start + 2,
            });
            let after = content_start + 2;
            if after < line_end {
                // `}}` の後ろに何か残っていれば (通常は起きない)、そのまま読み捨てる。
                out.push(Token {
                    kind: SyntaxKind::Error,
                    start: offset + after,
                    end: offset + line_end,
                });
            }
            pos = line_end;
        } else if is_single_brace_close {
            push_indent(out);
            out.push(Token {
                kind: SyntaxKind::RBrace,
                start: offset + content_start,
                end: offset + content_start + 1,
            });
            let after = content_start + 1;
            if after < line_end {
                // `}` の後ろに何か残っていれば (通常は起きない)、そのまま読み捨てる。
                out.push(Token {
                    kind: SyntaxKind::Error,
                    start: offset + after,
                    end: offset + line_end,
                });
            }
            pos = line_end;
        } else if content_start >= line_end {
            // 空行 (空白のみ、または完全に空)。
            if line_end > line_start {
                out.push(Token {
                    kind: SyntaxKind::Whitespace,
                    start: offset + line_start,
                    end: offset + line_end,
                });
            }
            pos = line_end;
        } else if bytes[content_start] == b'#' {
            push_indent(out);
            pos = scan_hash_command(src, offset, content_start, len, out);
        } else if bytes[content_start] == b'@' {
            push_indent(out);
            out.push(Token {
                kind: SyntaxKind::NovelAt,
                start: offset + content_start,
                end: offset + content_start + 1,
            });
            scan_novel_text(src, offset, content_start + 1, line_end, out);
            pos = line_end;
        } else {
            // 生の地の文。行頭の空白も表示に反映される内容なので `line_start` から読む。
            scan_novel_text(src, offset, line_start, line_end, out);
            pos = line_end;
        }

        if has_newline && pos == line_end {
            out.push(Token {
                kind: SyntaxKind::Newline,
                start: offset + pos,
                end: offset + pos + 1,
            });
            pos += 1;
        }
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

    #[test]
    fn lex_novel_hash_command_contents_as_code_tokens() {
        let src = "scene s(g: G) -> G {{\n#play_se(se1, 2)\n}}\n";
        let ks = kinds(src);
        assert!(ks.contains(&SyntaxKind::NovelHash));
        assert!(ks.contains(&SyntaxKind::Ident));
        assert!(ks.contains(&SyntaxKind::LParen));
        assert!(ks.contains(&SyntaxKind::Comma));
        assert!(ks.contains(&SyntaxKind::IntLiteral));
        assert!(ks.contains(&SyntaxKind::RParen));
        assert!(!ks.contains(&SyntaxKind::Error));
    }

    #[test]
    fn lex_novel_hash_command_continues_over_line_on_open_paren() {
        // `(` で終わっているので次の行も同じコマンドの続きとして読まれ、
        // 2 行目の `foo` が (改めて `#` を待たずに) Ident として出る。
        let src = "scene s(g: G) -> G {{\n#play_se(\n  foo\n)\n}}\n";
        let toks = lex(src);
        let hash_count = toks
            .iter()
            .filter(|t| t.kind == SyntaxKind::NovelHash)
            .count();
        assert_eq!(hash_count, 1, "continuation must not start a new command");
        let kinds: Vec<SyntaxKind> = toks.iter().map(|t| t.kind).collect();
        assert!(kinds.contains(&SyntaxKind::Ident));
        assert!(!kinds.contains(&SyntaxKind::Error));
    }

    #[test]
    fn lex_novel_hash_command_does_not_continue_without_trailing_mark() {
        // 1行目が `)` で終わる (継続対象ではない) ので、2行目の `#` は
        // 新しいコマンドとして数えられる。
        let src = "scene s(g: G) -> G {{\n#foo()\n#bar()\n}}\n";
        let toks = lex(src);
        let hash_count = toks
            .iter()
            .filter(|t| t.kind == SyntaxKind::NovelHash)
            .count();
        assert_eq!(hash_count, 2);
    }

    #[test]
    fn lex_embedded_expr_paren_call() {
        let src = "scene s(g: G) -> G {{\n$blue(bold(\"a\"))text\n}}\n";
        let toks = lex(src);
        let kinds: Vec<SyntaxKind> = toks.iter().map(|t| t.kind).collect();
        assert!(kinds.contains(&SyntaxKind::NovelDollar));
        assert!(kinds.contains(&SyntaxKind::Ident));
        assert!(kinds.contains(&SyntaxKind::StringLiteral));
        assert!(kinds.contains(&SyntaxKind::NovelText)); // "text" のあと
        assert!(!kinds.contains(&SyntaxKind::Error));
    }

    #[test]
    fn lex_embedded_expr_bare_paren_expr() {
        let src = "scene s(g: G) -> G {{\n$(player.hp)\n}}\n";
        let toks = lex(src);
        let kinds: Vec<SyntaxKind> = toks.iter().map(|t| t.kind).collect();
        assert!(kinds.contains(&SyntaxKind::NovelDollar));
        assert!(kinds.contains(&SyntaxKind::LParen));
        assert!(kinds.contains(&SyntaxKind::Dot));
        assert!(!kinds.contains(&SyntaxKind::Error));
    }

    #[test]
    fn lex_embedded_expr_must_end_with_a_call() {
        // `$foo.bar` はメソッド呼び出しで終わっていないので式として読めない。
        // `$` だけがエラーになり、残りは地の文として読み進められる。
        let src = "scene s(g: G) -> G {{\n$foo.bar\n}}\n";
        let toks = lex(src);
        let kinds: Vec<SyntaxKind> = toks.iter().map(|t| t.kind).collect();
        assert!(kinds.contains(&SyntaxKind::Error));
        assert!(kinds.contains(&SyntaxKind::NovelText));
    }

    #[test]
    fn lex_escaped_dollar_is_plain_text() {
        let src = "scene s(g: G) -> G {{\n100\\$\n}}\n";
        let toks = lex(src);
        let kinds: Vec<SyntaxKind> = toks.iter().map(|t| t.kind).collect();
        assert!(!kinds.contains(&SyntaxKind::NovelDollar));
        assert!(!kinds.contains(&SyntaxKind::Error));
        assert!(kinds.contains(&SyntaxKind::NovelText));
    }

    #[test]
    fn lex_new_keywords() {
        let ks = kinds("enum match trait _");
        assert!(ks.contains(&SyntaxKind::KwEnum));
        assert!(ks.contains(&SyntaxKind::KwMatch));
        assert!(ks.contains(&SyntaxKind::KwTrait));
        assert!(ks.contains(&SyntaxKind::KwUnderscore));
    }

    #[test]
    fn underscore_prefixed_ident_is_still_an_ident() {
        let ks = kinds("_probe");
        assert!(ks.contains(&SyntaxKind::Ident));
        assert!(!ks.contains(&SyntaxKind::KwUnderscore));
    }

    /// トークン列を素朴に連結したものが元のソースと一致すること
    /// (lossless: すべてのバイトがどれかのトークンに属する)。
    /// rowan の CST はこれが成り立っていないと、あるノード以降の
    /// 位置がすべてずれて壊れる。
    fn assert_lossless(src: &str) {
        let toks = lex(src);
        let mut rebuilt = String::new();
        for t in &toks {
            if t.kind == SyntaxKind::Eof {
                continue;
            }
            rebuilt.push_str(&src[t.start..t.end]);
        }
        assert_eq!(
            rebuilt, src,
            "token concatenation must reproduce the source exactly"
        );
    }

    #[test]
    fn indented_hash_command_is_lossless() {
        // 回帰テスト: `#`/`@`/`}}` 行の行頭インデントがトークン化されず、
        // それ以降のノベルモード全体の位置がずれるバグがあった。
        assert_lossless("scene s(g: G) -> G {{\n    #play_se(se1)\n}}\n");
    }

    #[test]
    fn indented_at_line_is_lossless() {
        assert_lossless("scene s(g: G) -> G {{\n    @biwa\n    こんにちは\n}}\n");
    }

    #[test]
    fn indented_end_brace_is_lossless() {
        assert_lossless("scene s(g: G) -> G {{\n  text\n    }}\n");
    }

    #[test]
    fn blank_line_with_only_whitespace_is_lossless() {
        assert_lossless("scene s(g: G) -> G {{\n text\n    \nmore\n}}\n");
    }

    #[test]
    fn multiline_scene_with_continuation_and_comments_is_lossless() {
        assert_lossless(
            "scene s(g: G) -> G {{\n    #let x = f(\n        1,\n        2, // comment\n    )\n    text $foo(1).bar() more >>\n}}\n",
        );
    }

    #[test]
    fn lex_wait_command() {
        let src = "scene s(g: G) -> G {{\nHello! >>\n}}\n";
        let toks = lex(src);
        let wait: Vec<_> = toks
            .iter()
            .filter(|t| t.kind == SyntaxKind::NovelWait)
            .collect();
        assert_eq!(
            wait.len(),
            1,
            "expected exactly one NovelWait token, got {toks:?}"
        );
        assert_eq!(&src[wait[0].start..wait[0].end], ">>");
        assert_lossless(src);
    }

    #[test]
    fn wait_command_after_embedded_expr_is_lossless() {
        let src = "scene s(g: G) -> G {{\ntext $blue(bold(\"a\")) more >>\n}}\n";
        assert_lossless(src);
        let toks = lex(src);
        assert!(toks.iter().any(|t| t.kind == SyntaxKind::NovelWait));
    }

    #[test]
    fn wait_command_without_preceding_text_produces_no_empty_error_tokens() {
        let src = "scene s(g: G) -> G {{\n>>\n}}\n";
        assert_lossless(src);
        let toks = lex(src);
        assert!(!toks.iter().any(|t| t.kind == SyntaxKind::Error));
    }

    #[test]
    fn lex_endscene_keyword() {
        let ks = kinds("scene s(g: G) -> G {{\n#endscene g\n}}\n");
        assert!(ks.contains(&SyntaxKind::KwEndScene));
        assert!(ks.contains(&SyntaxKind::Ident), "`g` should still lex as Ident");
    }

    #[test]
    fn lex_single_brace_line_closes_as_rbrace_not_double_rbrace() {
        // `#if cond {` の `{` に対応する、`#if` ブロックだけを閉じる単独の `}`。
        // 行全体の終端 (`}}`) と区別できなければならない。
        let src = "scene s(g: G) -> G {{\n#if x {\ntext\n}\n}}\n";
        let toks = lex(src);
        let kinds: Vec<SyntaxKind> = toks.iter().map(|t| t.kind).collect();
        assert!(kinds.contains(&SyntaxKind::LBrace));
        let rbrace_count = kinds.iter().filter(|k| **k == SyntaxKind::RBrace).count();
        assert_eq!(rbrace_count, 1, "expected exactly one single-brace close, got {toks:?}");
        let double_rbrace_count = kinds
            .iter()
            .filter(|k| **k == SyntaxKind::DoubleRBrace)
            .count();
        assert_eq!(double_rbrace_count, 1);
        assert!(!kinds.contains(&SyntaxKind::Error));
        assert_lossless(src);
    }

    #[test]
    fn string_literal_with_escaped_quote_is_a_single_token() {
        // `[^"\n]*` ベースの旧正規表現は `\` の次の `"` を特別扱いせず、
        // `"\"` の時点で閉じたことにしてしまっていた。
        let src = r#"let s = "a\"b";"#;
        let toks = lex(src);
        let strings: Vec<&Token> = toks
            .iter()
            .filter(|t| t.kind == SyntaxKind::StringLiteral)
            .collect();
        assert_eq!(strings.len(), 1, "expected exactly one string token, got {toks:?}");
        assert_eq!(strings[0].text(src), r#""a\"b""#);
        assert!(!toks.iter().any(|t| t.kind == SyntaxKind::Error));
        assert_lossless(src);
    }

    #[test]
    fn string_literal_with_escaped_quote_inside_novel_embedded_expr_is_a_single_token() {
        // test1/src/main.biwa 118行目付近と同じ形: 地の文の `$expr(...)` の中で
        // エスケープされた `"` を含む文字列リテラルを渡す。
        let src = "scene s(g: G) -> G {{\n$green(\"\\\"foo\\\"\")\n>>\n}}\n";
        let toks = lex(src);
        assert!(
            !toks.iter().any(|t| t.kind == SyntaxKind::Error),
            "unexpected error token(s): {toks:?}"
        );
        let strings: Vec<&Token> = toks
            .iter()
            .filter(|t| t.kind == SyntaxKind::StringLiteral)
            .collect();
        assert_eq!(strings.len(), 1, "expected exactly one string token, got {toks:?}");
        assert_eq!(strings[0].text(src), "\"\\\"foo\\\"\"");
        assert_lossless(src);
    }
}
