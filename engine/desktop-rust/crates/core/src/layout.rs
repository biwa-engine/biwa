//! UI のレイアウトのインターフェース。
//!
//! エンジンは UI の木を、箱の並べ方・大きさ・余白・文字だけの素朴な木 ([`LayoutNode`]) にして渡し、
//! 各ノードの位置と大きさ ([`LayoutBox`]) を受け取る。CSS のどのレイアウト (block / flex / grid) で
//! 計算するか、それをどの crate がやるかは実装する側 (`biwa-player-layout`) の都合で、エンジンからは見えない。
//!
//! 写し方は TypeScript 版の DOM (`engine/nodejs/src/engine/ui/UIObjects.ts` の `createDom`) と同じで、
//! 大きさの単位も CSS と同じに解く (percent の基準は、幅・左右上下の余白・padding が親の幅、高さが親の高さ)。
//! 座標と大きさは**論理ピクセル**で、原点は画面の左上。

use crate::geometry::{Rect, Viewport};
use crate::text::{LineHeight, TextShaper, TextSpan};
use crate::ui::UiSize;

/// 子の並べ方。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Flow {
    /// 縦に積む (CSS の block)。
    Block,
    /// 横に並べる (flex の row)。`Horizontal`。
    Row,
    /// 縦に並べる (flex の column)。`Vertical`。
    Column,
    /// 列数の決まった格子 (`grid-template-columns: repeat(n, 1fr)`)。`HorizontalGrid`。
    Grid { columns: u16 },
    /// 子をすべて同じ 1 つのマスに重ねる。`Layers`。
    Stack,
    /// 中身を縦横とも中央に寄せる (inline-flex + center)。`Link` / `SceneStartButton`。
    Center,
}

/// 箱の書式。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BoxStyle {
    pub flow: Flow,
    pub width: Option<UiSize>,
    pub height: Option<UiSize>,
    /// left, right, top, bottom。
    pub margin: [Option<UiSize>; 4],
    pub padding: [Option<UiSize>; 4],
    /// 親いっぱいに広がる (`position: absolute; inset: 0`)。`Page` / `ScenePage`。
    pub fill_parent: bool,
    /// width / height が padding を含む (`box-sizing: border-box`)。`MessageArea`。
    pub border_box: bool,
    /// `Layers` の子。親のマスに重ね、縦は中身の高さ (`align-self: start`)。
    pub stacked: bool,
    /// 描かず、場所も取らない (`display: none`)。隠れている Page など。
    pub hidden: bool,
}

impl BoxStyle {
    pub const fn new(flow: Flow) -> Self {
        Self {
            flow,
            width: None,
            height: None,
            margin: [None; 4],
            padding: [None; 4],
            fill_parent: false,
            border_box: false,
            stacked: false,
            hidden: false,
        }
    }
}

/// レイアウトする木のノード。
#[derive(Debug, Clone, PartialEq)]
pub struct LayoutNode {
    pub style: BoxStyle,
    /// 文字だけの葉 (Box・Link の `text`)。大きさは文字を整形して測る (大きさは論理ピクセル)。
    pub text: Option<LayoutText>,
    /// 子の位置 (`LayoutTree::nodes` の添字)。
    pub children: Vec<usize>,
}

/// 文字の葉の中身。
#[derive(Debug, Clone, PartialEq)]
pub struct LayoutText {
    pub spans: Vec<TextSpan>,
    pub line_height: LineHeight,
    pub center: bool,
}

/// レイアウトする木。`nodes[root]` が根で、根は画面いっぱいの大きさになる。
#[derive(Debug, Clone, Default, PartialEq)]
pub struct LayoutTree {
    pub nodes: Vec<LayoutNode>,
    pub root: usize,
}

/// ノード 1 つのレイアウトの結果 (論理ピクセル、画面の左上が原点)。
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct LayoutBox {
    /// border box (背景を塗る範囲)。
    pub frame: Rect,
    /// content box (padding の内側。文字・子を置く範囲)。
    pub content: Rect,
}

/// レイアウトを計算する者。
pub trait Layouter {
    /// `tree` を `viewport` の大きさでレイアウトし、ノードごとの結果を `tree.nodes` と同じ順で返す。
    /// 文字の大きさは `shaper` で測る。
    fn layout(&mut self, tree: &LayoutTree, viewport: Viewport, shaper: &mut dyn TextShaper) -> Vec<LayoutBox>;
}
