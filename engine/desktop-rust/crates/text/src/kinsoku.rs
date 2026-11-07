//! 禁則処理 (JIS X 4051 の行頭禁則・行末禁則の基本)。
//!
//! parley の改行位置は ICU4X (UAX #14) に従う。日本語のロケールを渡しているので、
//! 閉じ括弧・句読点・小書きの仮名・長音符・繰り返し記号の前では、もともと改行しない
//! (2026-11 に実測。ICU だけでは全角のチルダ「～」が行頭に来た)。
//! この表は、その取りこぼしを拾う安全網である。ノベルの本文では行頭に来ると目立つので、
//! JIS X 4051 の行頭禁則・行末禁則の文字を並べて止める。

/// 行頭に来てはいけない文字 (この文字の前で改行しない)。
fn no_break_before(c: char) -> bool {
    matches!(
        c,
        // 閉じ括弧類
        '）' | ')' | '」' | '』' | '］' | ']' | '｝' | '}' | '〕' | '〉' | '》' | '】' | '〙' | '〗' | '〟' | '’' | '”' | '｠' | '»'
        // 区切り約物・中点・句読点
        | '、' | '。' | '，' | '．' | ',' | '.' | '・' | '：' | '；' | ':' | ';' | '？' | '！' | '?' | '!' | '‼' | '⁇' | '⁈' | '⁉'
        // 小書きの仮名
        | 'ぁ' | 'ぃ' | 'ぅ' | 'ぇ' | 'ぉ' | 'っ' | 'ゃ' | 'ゅ' | 'ょ' | 'ゎ' | 'ゕ' | 'ゖ'
        | 'ァ' | 'ィ' | 'ゥ' | 'ェ' | 'ォ' | 'ッ' | 'ャ' | 'ュ' | 'ョ' | 'ヮ' | 'ヵ' | 'ヶ'
        | 'ㇰ'..='ㇿ'
        // 長音符・繰り返し記号・ハイフン類
        | 'ー' | 'ｰ' | '々' | 'ゝ' | 'ゞ' | 'ヽ' | 'ヾ' | '〻' | '‐' | '゠' | '–' | '〜' | '～'
    )
}

/// 行末に来てはいけない文字 (この文字の後で改行しない)。
fn no_break_after(c: char) -> bool {
    matches!(
        c,
        '（' | '(' | '「' | '『' | '［' | '[' | '｛' | '{' | '〔' | '〈' | '《' | '【' | '〘' | '〖' | '〝' | '‘' | '“' | '｟' | '«'
    )
}

/// parley の改行位置の上書き。止めるときだけ `Some(false)`、それ以外は既定に任せる。
pub(crate) fn line_break_override(before: char, after: char) -> Option<bool> {
    if no_break_before(after) || no_break_after(before) {
        Some(false)
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn suppresses_breaks_around_kinsoku_characters() {
        assert_eq!(line_break_override('あ', '。'), Some(false));
        assert_eq!(line_break_override('か', 'っ'), Some(false));
        assert_eq!(line_break_override('ラ', 'ー'), Some(false));
        assert_eq!(line_break_override('「', 'あ'), Some(false));
        assert_eq!(line_break_override('あ', 'い'), None);
    }
}
