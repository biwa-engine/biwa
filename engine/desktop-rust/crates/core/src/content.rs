//! Message Window の中身 (Content API) と文字送り。
//!
//! `engine/nodejs/src/components/TextBox.ts` と同じ規則で動く (`docs/content-api.md`)。
//!
//! - content は**断片の列**として持つ。断片ごとに色・大きさ・太さ・速度が違いうる。
//! - `push` は積むだけで、`flush` で初めて文字送りが始まる。`clear` も std からしか来ない。
//! - 文字送りは列に対するカーソル 1 本と、1 文字に満たない端数 (`carry`) で持つ。
//!   断片を出し切って余った時間は、次の断片の速度で文字数に直して持ち越す。
//! - **出ていない文字は「消す」のではなく「隠す」。** 段落は flush のたびに 1 度だけ整形し、
//!   送りの間は「先頭から何文字目まで描くか」だけが変わる。行が動かない。
//!
//! 文字数は `char` (Unicode のスカラー値) で数える。TypeScript 版は UTF-16 の単位で数えている
//! (サロゲートペアの途中で切れうる)。

use crate::geometry::Color;

/// 断片の大きさの単位。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SizeUnit {
    /// 画面の幅の 1/100。
    Vw,
    /// 画面の高さの 1/100。
    Vh,
}

impl SizeUnit {
    /// syscall の番号から。知らない番号は vw として扱う。
    pub fn from_raw(raw: i32) -> Self {
        if raw == 1 { Self::Vh } else { Self::Vw }
    }
}

/// 1 つの content。値はすべて std が解決済みの絶対値である。
#[derive(Debug, Clone, PartialEq)]
pub struct Fragment {
    pub text: String,
    /// 秒間何文字出るか。0 以下なら送らずに一度に出す。
    pub speed: f32,
    pub size_unit: SizeUnit,
    pub size_value: f32,
    /// 100〜900。
    pub weight: u16,
    pub color: Color,
}

/// 文字送り中の断片。
#[derive(Debug, Clone, PartialEq)]
struct Revealing {
    fragment: Fragment,
    /// `char` の数。
    len: usize,
    /// 見えている文字数。
    revealed: usize,
}

/// Message Window 1 つ分の中身。
#[derive(Debug, Default)]
pub struct MessageContent {
    /// まだ `flush` されていない断片。
    pending: Vec<Fragment>,
    /// 出し始めた断片。
    revealing: Vec<Revealing>,
    cursor: usize,
    /// 1 文字に満たない端数 (文字数)。
    carry: f32,
    /// 段落の中身 (文字列と書式) が変わるたびに増える。整形の結果を使い回す鍵になる。
    generation: u64,
}

impl MessageContent {
    /// 断片を 1 つ積む。まだ描かない。
    pub fn push(&mut self, fragment: Fragment) {
        self.pending.push(fragment);
    }

    /// 積まれた断片を出し始める。
    ///
    /// カーソルは触らない。前の送りが終わっていなければ、続きとしてそのまま繋がる。
    pub fn flush(&mut self) {
        if self.pending.is_empty() {
            return;
        }
        for fragment in self.pending.drain(..) {
            let len = fragment.text.chars().count();
            self.revealing.push(Revealing { fragment, len, revealed: 0 });
        }
        self.generation += 1;
    }

    /// 枠を空にする。積んだだけでまだ出していないものも捨てる。
    pub fn clear(&mut self) {
        self.pending.clear();
        self.revealing.clear();
        self.cursor = 0;
        self.carry = 0.0;
        self.generation += 1;
    }

    /// 残りを全部出す。出すものがあったかを返す。
    ///
    /// クリックはまずこれを試す。出すものがあれば、そのクリックは送りの完了に使われる。
    pub fn skip(&mut self) -> bool {
        let skipped = self.remaining() > 0;
        for r in &mut self.revealing[self.cursor..] {
            r.revealed = r.len;
        }
        self.cursor = self.revealing.len();
        self.carry = 0.0;
        skipped
    }

    /// 文字送りを進める。`delta_ms` はエンジン時計の差分。
    pub fn update(&mut self, delta_ms: f32) {
        let mut seconds = delta_ms / 1000.0;

        while self.cursor < self.revealing.len() {
            let r = &mut self.revealing[self.cursor];
            let remaining = r.len - r.revealed;

            // 空の断片、出し切った断片、送らない断片は時間を使わない。
            if remaining == 0 {
                self.cursor += 1;
                continue;
            }
            if r.fragment.speed <= 0.0 {
                r.revealed = r.len;
                self.cursor += 1;
                continue;
            }
            if seconds <= 0.0 {
                break;
            }

            self.carry += seconds * r.fragment.speed;
            let step = (self.carry.floor() as usize).min(remaining);
            self.carry -= step as f32;
            r.revealed += step;

            if step < remaining {
                // まだ途中。このフレームの時間は使い切った。
                seconds = 0.0;
            } else {
                // 出し切った。余った端数を次の断片の時間として戻す。
                seconds = self.carry / r.fragment.speed;
                self.carry = 0.0;
                self.cursor += 1;
            }
        }
    }

    /// まだ出ていない文字があるか (描き続ける必要があるか)。
    pub fn is_revealing(&self) -> bool {
        self.remaining() > 0
    }

    /// 出し始めた断片。描くのはこれだけである (`pending` は描かない)。
    pub fn fragments(&self) -> impl Iterator<Item = &Fragment> {
        self.revealing.iter().map(|r| &r.fragment)
    }

    /// `index` 番目の断片の、見えている文字数。
    pub fn revealed(&self, index: usize) -> usize {
        self.revealing.get(index).map_or(0, |r| r.revealed)
    }

    pub fn generation(&self) -> u64 {
        self.generation
    }

    fn remaining(&self) -> usize {
        self.revealing[self.cursor.min(self.revealing.len())..]
            .iter()
            .map(|r| r.len - r.revealed)
            .sum()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fragment(text: &str, speed: f32) -> Fragment {
        Fragment {
            text: text.to_string(),
            speed,
            size_unit: SizeUnit::Vw,
            size_value: 2.0,
            weight: 400,
            color: Color::WHITE,
        }
    }

    fn revealed(content: &MessageContent) -> Vec<usize> {
        (0..content.revealing.len()).map(|i| content.revealed(i)).collect()
    }

    #[test]
    fn push_alone_does_not_reveal() {
        let mut c = MessageContent::default();
        c.push(fragment("abc", 10.0));
        c.update(1000.0);
        assert_eq!(c.fragments().count(), 0);
        c.flush();
        assert_eq!(revealed(&c), vec![0]);
    }

    #[test]
    fn reveals_by_speed_and_carries_over_fragments() {
        let mut c = MessageContent::default();
        c.push(fragment("あいう", 10.0)); // 0.3 秒
        c.push(fragment("abcd", 20.0)); // 0.2 秒
        c.flush();

        c.update(150.0);
        assert_eq!(revealed(&c), vec![1, 0]);
        // 0.3 秒で 1 つ目を出し切り、余り 0.05 秒が 2 つ目の 1 文字になる。
        c.update(200.0);
        assert_eq!(revealed(&c), vec![3, 1]);
        c.update(1000.0);
        assert_eq!(revealed(&c), vec![3, 4]);
        assert!(!c.is_revealing());
    }

    #[test]
    fn zero_speed_shows_at_once() {
        let mut c = MessageContent::default();
        c.push(fragment("ab", 0.0));
        c.push(fragment("cd", 10.0));
        c.flush();
        c.update(0.0);
        assert_eq!(revealed(&c), vec![2, 0]);
    }

    #[test]
    fn skip_reveals_rest_once() {
        let mut c = MessageContent::default();
        c.push(fragment("abc", 1.0));
        c.flush();
        assert!(c.skip());
        assert_eq!(revealed(&c), vec![3]);
        assert!(!c.skip());
    }

    #[test]
    fn flush_continues_after_previous_text() {
        let mut c = MessageContent::default();
        c.push(fragment("ab", 0.0));
        c.flush();
        c.update(0.0);
        c.push(fragment("cd", 10.0));
        let before = c.generation();
        c.flush();
        assert!(c.generation() > before);
        c.update(100.0);
        assert_eq!(revealed(&c), vec![2, 1]);
    }

    #[test]
    fn clear_drops_everything() {
        let mut c = MessageContent::default();
        c.push(fragment("ab", 1.0));
        c.flush();
        c.push(fragment("cd", 1.0));
        c.clear();
        c.flush();
        assert_eq!(c.fragments().count(), 0);
        assert!(!c.is_revealing());
    }
}
