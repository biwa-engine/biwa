//! 改行コードの正規化。
//!
//! ソースの改行は `\r\n` (Windows)・`\r` 単独・`\n` のどれで書かれていてもよい。
//! 3 種類とも改行として扱うのは LSP の仕様
//! (`TextDocuments` の行の区切り) に合わせたもので、
//! コンパイラと biwa-lsp で行の数え方が食い違わないようにするためである。
//!
//! 字句解析器やパーサは `\n` だけを改行として扱う。
//! 3 種類の改行を各所で見分けるのではなく、**ソースを読み込んだ直後に
//! [`normalize_newlines`] で `\n` に揃え、それ以降は正規化済みの文字列だけを扱う**。
//! span はすべて正規化後の文字列のバイトオフセットである。
//!
//! # 位置について
//!
//! 正規化しても、どの文字の (行, 桁) も変わらない。
//! `\r\n` → `\n` は行末の `\r` が消えるだけで、
//! `\r` → `\n` は 1 バイトを 1 バイトに置き換えるだけだからである。
//! そのため、正規化後の span を正規化後の文字列で (行, 桁) に直せば、
//! 元のファイル (エディタ上のバッファ) の位置としてそのまま正しい。
//! 一方バイトオフセットは、手前にある `\r\n` の数だけ元のファイルより小さくなる。
//! **正規化後のオフセットを元の文字列に当てはめてはいけない。**

/// 改行を `\n` に揃える。
///
/// - `\r\n` → `\n`
/// - `\r` (後ろに `\n` が続かないもの) → `\n`
/// - `\n` → そのまま
///
/// `\r\r\n` は `\r` と `\r\n` の 2 つの改行である (LSP の行の数え方と同じ)。
///
/// `\r` を含まなければ、受け取った文字列をそのまま返す (再確保しない)。
/// 正規化済みの文字列に掛けても何も変わらない。
pub fn normalize_newlines(src: String) -> String {
    if !src.contains('\r') {
        return src;
    }

    let mut out = String::with_capacity(src.len());
    let mut rest = src.as_str();
    // `\r` も `\n` も ASCII なので、UTF-8 の多バイト文字の途中に現れることはない。
    // バイト位置で切っても文字境界を壊さない。
    while let Some(cr) = rest.find('\r') {
        out.push_str(&rest[..cr]);
        out.push('\n');
        rest = &rest[cr + 1..];
        if let Some(after_crlf) = rest.strip_prefix('\n') {
            rest = after_crlf;
        }
    }
    out.push_str(rest);
    out
}

/// 改行が `\n` に揃っているか ([`normalize_newlines`] を通した後か)。
///
/// 正規化済みの文字列だけを受け取る関数の入口で、前提の確認に使う。
pub fn is_newline_normalized(src: &str) -> bool {
    !src.contains('\r')
}

#[cfg(test)]
mod tests {
    use super::*;

    fn norm(s: &str) -> String {
        normalize_newlines(s.to_string())
    }

    #[test]
    fn lf_is_kept() {
        assert_eq!(norm("a\nb\n"), "a\nb\n");
    }

    #[test]
    fn crlf_becomes_lf() {
        assert_eq!(norm("a\r\nb\r\n"), "a\nb\n");
    }

    #[test]
    fn lone_cr_becomes_lf() {
        assert_eq!(norm("a\rb\r"), "a\nb\n");
    }

    #[test]
    fn mixed_line_endings() {
        assert_eq!(norm("a\r\nb\rc\nd"), "a\nb\nc\nd");
    }

    #[test]
    fn cr_before_crlf_is_two_line_breaks() {
        // `\r` + `\r\n`。LSP でも 2 行として数える。
        assert_eq!(norm("a\r\r\nb"), "a\n\nb");
        // `\n` + `\r` も 2 つ (`\n\r` という改行は無い)。
        assert_eq!(norm("a\n\rb"), "a\n\nb");
    }

    #[test]
    fn line_breaks_at_both_ends() {
        assert_eq!(norm("\r\n"), "\n");
        assert_eq!(norm("\r"), "\n");
        assert_eq!(norm("\r\na\r"), "\na\n");
        assert_eq!(norm(""), "");
    }

    #[test]
    fn multibyte_characters_around_line_breaks() {
        assert_eq!(norm("琵琶\r\n湖\r🐟"), "琵琶\n湖\n🐟");
    }

    #[test]
    fn idempotent() {
        let once = norm("a\r\r\nb\rc\r\n");
        assert_eq!(normalize_newlines(once.clone()), once);
        assert!(is_newline_normalized(&once));
    }

    #[test]
    fn lf_only_source_is_not_reallocated() {
        let src = "fn main() {\n}\n".to_string();
        let ptr = src.as_ptr();
        let out = normalize_newlines(src);
        assert_eq!(out.as_ptr(), ptr);
    }

    /// 正規化しても、どの文字の (行, 桁) も変わらないこと。
    ///
    /// 元の文字列は 3 種類の改行で、正規化後は `\n` だけで行を数える。
    #[test]
    fn line_and_column_are_preserved() {
        let raw = "ab\r\nc\rd\n\r\r\n琵琶\re\r\n";
        let normalized = norm(raw);

        // 改行以外の文字の (行, 桁) を、改行の種類を見分けながら数える。
        fn positions(s: &str) -> Vec<(char, usize, usize)> {
            let mut out = Vec::new();
            let (mut line, mut col) = (0, 0);
            let mut chars = s.chars().peekable();
            while let Some(c) = chars.next() {
                match c {
                    '\r' => {
                        if chars.peek() == Some(&'\n') {
                            chars.next();
                        }
                        line += 1;
                        col = 0;
                    }
                    '\n' => {
                        line += 1;
                        col = 0;
                    }
                    c => {
                        out.push((c, line, col));
                        col += 1;
                    }
                }
            }
            out
        }

        assert_eq!(positions(raw), positions(&normalized));

        // 改行の数も変わらない (`\r\n` は `\r` と `\n` の両方に数えられているので 1 回引く)。
        let raw_line_breaks =
            raw.matches('\r').count() + raw.matches('\n').count() - raw.matches("\r\n").count();
        assert_eq!(raw_line_breaks, normalized.matches('\n').count());
    }
}
