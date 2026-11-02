//! 1 フレームに描くもの。エンジンが組み立て、描画 (`biwa-player-render`) が GPU に流す。
//!
//! 座標はすべて**物理ピクセル**で、原点は左上。描く順は配列の順 (後ろほど上) である。

use crate::geometry::{Color, Rect};
use crate::image::ImageHandle;
use crate::text::GlyphKey;

/// 描くもの 1 つ。
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum DrawItem {
    /// 塗りつぶした矩形。
    Rect { rect: Rect, color: Color },
    /// グリフ。`x` / `y` はベースライン上の位置。
    Glyph { key: GlyphKey, x: f32, y: f32, color: Color },
    /// 画像。中心と大きさで置き、中心のまわりに回す。画像がまだ無ければ描かない。
    Image {
        image: ImageHandle,
        center_x: f32,
        center_y: f32,
        width: f32,
        height: f32,
        /// ラジアン。画面の上で時計回りが正。
        rotation: f32,
        /// 0〜1。
        alpha: f32,
    },
    /// これ以降に描くものを `Some` の範囲の中だけに切る (`None` で切らない)。
    Clip(Option<Rect>),
}

/// 1 フレームぶんの描画の指示。
#[derive(Debug, Clone, Default, PartialEq)]
pub struct DrawList {
    /// 最初に塗る色。
    pub clear: Color,
    pub items: Vec<DrawItem>,
}
