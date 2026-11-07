//! 文字の整形とラスタライズのインターフェース。
//!
//! エンジンは文字を「書式付きの断片の列」として渡し、グリフの並びを受け取る。
//! 整形・行分割・フォントの選択・グリフの画像化をどの crate がやるかは、
//! このトレイトを実装する側 (`biwa-player-text`) の都合であって、エンジンからは見えない。
//!
//! 座標と大きさはすべて**物理ピクセル**である (HiDPI でも文字をぼかさないため)。
//! 論理ピクセルからの換算はエンジンが済ませてから渡す。

use crate::geometry::Color;

/// 断片の書式。
#[derive(Debug, Clone, PartialEq)]
pub struct TextStyle {
    /// 文字の大きさ (物理ピクセル。レイアウトで大きさを測るときは論理ピクセル)。
    pub size: f32,
    /// 100〜900。
    pub weight: u16,
    pub color: Color,
    /// フォント名 (CSS の font-family と同じく、カンマ区切りで候補を並べてよい)。`None` はシステムの sans-serif。
    pub family: Option<String>,
}

/// 書式付きの断片。
#[derive(Debug, Clone, PartialEq)]
pub struct TextSpan {
    pub text: String,
    pub style: TextStyle,
}

/// 整形の条件。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ParagraphStyle {
    /// 折り返す幅 (物理ピクセル)。`f32::INFINITY` なら折り返さない。
    pub max_width: f32,
    pub line_height: LineHeight,
    /// 各行を `max_width` の中で中央に寄せる (Link・ボタンの文字)。
    pub center: bool,
    /// 段落そのものの文字の大きさ (物理ピクセル)。CSS の親要素の font-size にあたり、
    /// 行の高さはこの大きさの行の高さを下回らない (CSS の strut と同じ)。
    pub base_size: f32,
}

/// CSS の既定の文字の大きさ (論理ピクセル)。[`ParagraphStyle::base_size`] は普通これに拡大率を掛けたもの。
/// TypeScript 版は文字の書式を断片にだけ付け、段落 (要素) の font-size はブラウザの既定のままなので、それに合わせる。
pub const DEFAULT_FONT_SIZE: f32 = 16.0;

/// 行の高さ。
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum LineHeight {
    /// フォント自身の値 (上下の高さと行間) で決める。CSS の `line-height: normal`。
    Normal,
    /// 文字の大きさに対する倍率。CSS の単位の無い `line-height`。
    Relative(f32),
}

/// グリフを指す鍵。同じ鍵のグリフは同じ画像になる (グリフアトラスの鍵に使う)。
///
/// 中身の意味 (どのフォントの何番か) は整形する側が決める。エンジンと描画は比べるだけである。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct GlyphKey {
    /// 整形する側が振ったフォントの番号。
    pub font: u32,
    /// フォントの中のグリフ番号。
    pub glyph: u32,
    /// 大きさ (物理ピクセルの 1/64 単位)。
    pub size: u32,
    /// 太さ (可変フォントの `wght`。100〜900)。
    pub weight: u16,
}

/// 整形済みのグリフ 1 つ。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ShapedGlyph {
    pub key: GlyphKey,
    /// 段落の左上を原点とするベースライン上の位置 (物理ピクセル)。
    pub x: f32,
    pub y: f32,
    pub color: Color,
    /// 何番目の断片の文字か。
    pub span: usize,
    /// 断片の中で何文字目 (`char` の数え方) から始まる字形か。文字送りがこれで隠す。
    pub char_index: usize,
}

/// 整形した段落。
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ShapedText {
    pub glyphs: Vec<ShapedGlyph>,
    /// いちばん長い行の幅 (物理ピクセル)。内容に合わせて箱の大きさを決めるのに使う。
    pub width: f32,
    /// 段落全体の高さ (物理ピクセル)。
    pub height: f32,
}

/// 文字を整形する者。
pub trait TextShaper {
    fn shape(&mut self, spans: &[TextSpan], paragraph: ParagraphStyle) -> ShapedText;
}

/// グリフの画像。アルファだけの 8 bit (色は描くときに掛ける)。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GlyphImage {
    pub width: u32,
    pub height: u32,
    /// ベースライン上の位置から見た、画像の左上の位置 (物理ピクセル。y は下向き)。
    pub left: i32,
    pub top: i32,
    /// `width * height` バイト。
    pub alpha: Vec<u8>,
}

/// グリフを画像にする者。描画が初めて見る鍵のときだけ呼ばれる。
pub trait GlyphRasterizer {
    /// 画像が無い (空白など) なら `None`。
    fn rasterize(&mut self, key: GlyphKey) -> Option<GlyphImage>;
}
