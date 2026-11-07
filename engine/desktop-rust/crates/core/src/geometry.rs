//! 位置・大きさ・色。どの層からも使う素朴な値である。

/// 論理ピクセルの矩形。原点は左上、y は下向き。
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

impl Rect {
    pub const fn new(x: f32, y: f32, width: f32, height: f32) -> Self {
        Self { x, y, width, height }
    }

    /// 4 辺を内側へ縮めた矩形。縮めすぎたら大きさは 0 に留める。
    pub fn inset(self, left: f32, right: f32, top: f32, bottom: f32) -> Self {
        Self {
            x: self.x + left,
            y: self.y + top,
            width: (self.width - left - right).max(0.0),
            height: (self.height - top - bottom).max(0.0),
        }
    }
}

/// 8 bit の RGBA。biwa の色の表し方 (0〜255) をそのまま持つ。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct Color {
    pub r: u8,
    pub g: u8,
    pub b: u8,
    pub a: u8,
}

impl Color {
    pub const BLACK: Self = Self::rgba(0, 0, 0, 255);
    pub const WHITE: Self = Self::rgba(255, 255, 255, 255);
    pub const TRANSPARENT: Self = Self::rgba(0, 0, 0, 0);

    pub const fn rgba(r: u8, g: u8, b: u8, a: u8) -> Self {
        Self { r, g, b, a }
    }

    /// syscall の i32 の 4 成分から作る。範囲外は 0〜255 に丸める。
    pub fn from_channels(r: i32, g: i32, b: i32, a: i32) -> Self {
        let c = |v: i32| v.clamp(0, 255) as u8;
        Self::rgba(c(r), c(g), c(b), c(a))
    }
}

/// 描画先の大きさ。
///
/// UI の大きさの単位 (`vw` / `vh`) はこの論理ピクセルの大きさを 100 とする。
/// `scale` は論理ピクセル 1 つが物理ピクセルいくつか (HiDPI)。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Viewport {
    pub width: f32,
    pub height: f32,
    pub scale: f32,
}

impl Viewport {
    pub const fn new(width: f32, height: f32, scale: f32) -> Self {
        Self { width, height, scale }
    }
}
