use biwa_lsp_highlight::{HighlightToken, TokenType};
use tower_lsp::lsp_types::SemanticToken;

/// LSP legend に渡すトークン型名の配列。インデックスが token type 番号になる。
pub const TOKEN_TYPES_LEGEND: &[&str] = &[
    "keyword",        // 0
    "type",           // 1
    "function",       // 2
    "variable",       // 3
    "parameter",      // 4
    "property",       // 5
    "number",         // 6
    "string",         // 7
    "comment",        // 8
    "operator",       // 9
    "namespace",      // 10
    "novelText",      // 11
    "novelCommand",   // 12
    "novelCharacter", // 13
    "method",         // 14
    "interface",      // 15
];

fn token_type_index(tt: TokenType) -> u32 {
    match tt {
        TokenType::Keyword => 0,
        TokenType::Type => 1,
        TokenType::Function => 2,
        TokenType::Variable => 3,
        TokenType::Parameter => 4,
        TokenType::Property => 5,
        TokenType::Number => 6,
        TokenType::String => 7,
        TokenType::Comment => 8,
        TokenType::Operator => 9,
        TokenType::Namespace => 10,
        TokenType::NovelText => 11,
        TokenType::NovelCommand => 12,
        TokenType::NovelCharacter => 13,
        TokenType::Method => 14,
        TokenType::Interface => 15,
    }
}

/// byte offset を (line, utf16_col) に変換するためのマッピングを構築する。
/// line は 0-based、col は utf-16 code unit 単位 (LSP の規約)。
///
/// `src` は改行を `\n` に揃えたもの (`normalize_document_text`) であること。
/// LSP の仕様では `\r\n`・`\r` も行の区切りだが、正規化で `\n` になっている。
pub(crate) fn build_line_index(src: &str) -> Vec<usize> {
    // 各行の開始 byte offset を格納
    let mut line_starts = vec![0usize];
    for (i, b) in src.bytes().enumerate() {
        if b == b'\n' {
            line_starts.push(i + 1);
        }
    }
    line_starts
}

pub(crate) fn offset_to_line_col(
    line_starts: &[usize],
    src: &str,
    byte_offset: usize,
) -> (u32, u32) {
    // binary search で行を特定
    let line = line_starts.partition_point(|&start| start <= byte_offset) - 1;
    let line_start_byte = line_starts[line];
    // utf-16 col: その行の先頭から byte_offset までの文字列を utf-16 で計算
    let col_bytes = &src[line_start_byte..byte_offset];
    let utf16_col: usize = col_bytes.chars().map(|c| c.len_utf16()).sum();
    (line as u32, utf16_col as u32)
}

/// `HighlightToken` のリストを LSP の SemanticToken delta encoding に変換する。
///
/// 入力は start の昇順にソート済みであること (highlight() はそれを保証する)。
/// 複数行にまたがるトークンは行ごとに分割する。
pub fn encode_semantic_tokens(src: &str, tokens: &[HighlightToken]) -> Vec<SemanticToken> {
    let line_starts = build_line_index(src);
    let mut result = Vec::with_capacity(tokens.len());
    let mut prev_line: u32 = 0;
    let mut prev_start_char: u32 = 0;

    for tok in tokens {
        let (line, start_char) = offset_to_line_col(&line_starts, src, tok.start);
        let length = {
            let text = &src[tok.start..tok.end];
            // 複数行にまたがるトークンは最初の行分だけ長さを取る
            let first_line_len = text.find('\n').unwrap_or(text.len());
            // utf-16 length
            text[..first_line_len]
                .chars()
                .map(|c| c.len_utf16())
                .sum::<usize>() as u32
        };

        let delta_line = line - prev_line;
        let delta_start = if delta_line == 0 {
            start_char - prev_start_char
        } else {
            start_char
        };

        result.push(SemanticToken {
            delta_line,
            delta_start,
            length,
            token_type: token_type_index(tok.token_type),
            token_modifiers_bitset: 0,
        });

        prev_line = line;
        prev_start_char = start_char;
    }

    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn single_token() {
        // "fn" は offset 0..2
        let src = "fn add() {}";
        let toks = biwa_lsp_highlight::highlight(src);
        let encoded = encode_semantic_tokens(src, &toks);
        // 最初のトークンの delta_line と delta_start は 0
        assert_eq!(encoded[0].delta_line, 0);
        assert_eq!(encoded[0].delta_start, 0);
    }

    #[test]
    fn multiline_tokens() {
        let src = "fn add(\n  x: Int\n) {}";
        let toks = biwa_lsp_highlight::highlight(src);
        let encoded = encode_semantic_tokens(src, &toks);
        // delta encoding: 2行目以降のトークンは delta_line > 0
        let has_multiline = encoded.iter().any(|t| t.delta_line > 0);
        assert!(
            has_multiline,
            "multiline source must produce delta_line > 0"
        );
    }

    /// LSP の仕様どおりに、元のテキスト (改行は `\r\n`・`\r`・`\n` のどれでもよい) の
    /// 各位置を (行, UTF-16 の桁) にする参照実装。
    ///
    /// 改行以外の文字 1 つと、改行 1 つ (`\r\n` は 2 文字で 1 つ) ごとに、
    /// その開始位置を返す。最後に末尾の位置を足す。
    fn reference_positions(raw: &str) -> Vec<(u32, u32)> {
        let mut out = Vec::new();
        let (mut line, mut col) = (0u32, 0u32);
        let mut chars = raw.chars().peekable();
        while let Some(c) = chars.next() {
            out.push((line, col));
            match c {
                '\r' | '\n' => {
                    if c == '\r' && chars.peek() == Some(&'\n') {
                        chars.next();
                    }
                    line += 1;
                    col = 0;
                }
                c => col += c.len_utf16() as u32,
            }
        }
        out.push((line, col));
        out
    }

    /// 正規化後のテキストで計算した位置が、元のテキストの位置と一致すること。
    ///
    /// サーバは正規化後のテキストだけを持ち、そこで位置を計算して返すので、
    /// これが崩れるとエディタ上の別の場所に診断やハイライトが出る。
    #[test]
    fn positions_on_normalized_text_match_the_raw_text() {
        // 改行の種類を混ぜ、`\r\r\n` (2 つの改行)、空行、
        // 多バイト文字・サロゲートペアになる文字も入れる。
        let raw = "fn a() {\r\n  琵琶\r🐟x\n\r\r\n\r\nend\r";
        let normalized = biwac_base::normalize_newlines(raw.to_string());

        let idx = build_line_index(&normalized);
        let mut actual: Vec<(u32, u32)> = normalized
            .char_indices()
            .map(|(i, _)| offset_to_line_col(&idx, &normalized, i))
            .collect();
        actual.push(offset_to_line_col(&idx, &normalized, normalized.len()));

        assert_eq!(reference_positions(raw), actual);
    }

    /// 改行コードが違っても、ハイライトの結果 (位置を含む) が変わらないこと。
    #[test]
    fn semantic_tokens_do_not_depend_on_line_endings() {
        let lf = r#"// コメント
fn add(x: Int,
  y: Int) -> Int {
  let s = "文字列";
  x + y
}

scene opening(g: MyGame) -> MyGame {{
  こんにちは $(add(1, 2)) さん。>>
  #let x = add(1,
    2)
  #endscene g
}}
"#;
        let encode = |src: &str| {
            let src = biwac_base::normalize_newlines(src.to_string());
            let toks = biwa_lsp_highlight::highlight(&src);
            encode_semantic_tokens(&src, &toks)
        };

        let expected = encode(lf);
        assert!(!expected.is_empty());
        assert_eq!(expected, encode(&lf.replace('\n', "\r\n")), "CRLF");
        assert_eq!(expected, encode(&lf.replace('\n', "\r")), "CR");
    }

    #[test]
    fn line_index_simple() {
        let src = "ab\ncd\nef";
        let idx = build_line_index(src);
        assert_eq!(idx, vec![0, 3, 6]);
        assert_eq!(offset_to_line_col(&idx, src, 0), (0, 0)); // 'a'
        assert_eq!(offset_to_line_col(&idx, src, 3), (1, 0)); // 'c'
        assert_eq!(offset_to_line_col(&idx, src, 5), (1, 2)); // 'd'
        assert_eq!(offset_to_line_col(&idx, src, 6), (2, 0)); // 'e'
    }
}
