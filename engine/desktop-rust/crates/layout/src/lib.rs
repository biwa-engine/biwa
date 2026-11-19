//! UI のレイアウト (taffy)。
//!
//! 外から見えるのは [`TaffyLayouter`] が実装する `core::layout::Layouter` だけで、taffy の型はこの crate の外に出さない。
//!
//! `core` が作った素朴な木 (箱の並べ方・大きさ・余白・文字) を、TypeScript 版の DOM / CSS
//! (`engine/nodejs/src/engine/ui/UIObjects.ts` の `createDom`) と同じ意味の taffy のスタイルに写して計算する。
//!
//! | 並べ方 | CSS | taffy |
//! | --- | --- | --- |
//! | Block | `display: block` | Block |
//! | Row / Column | `display: flex; flex-direction: row / column` | Flex |
//! | Grid { n } | `display: grid; grid-template-columns: repeat(n, 1fr)` | Grid |
//! | Stack (Layers) | 1 マスの grid に子を重ね、子は `grid-area: 1 / 1; align-self: start` | Grid |
//! | Center (Link) | `display: inline-flex; align-items: center; justify-content: center` | Flex |
//!
//! - Page / ScenePage は `position: absolute; inset: 0` (親いっぱい)。
//! - `box-sizing` は CSS の既定の content-box で、MessageArea だけ border-box。
//! - 文字の葉は `core` の整形 (`TextShaper`) で測る。
//! - Link は CSS では inline-flex (行の中に並ぶ) だが、taffy に行の中の配置は無いので、block の中では 1 行を占める。

use biwa_player_core::geometry::{Rect, Viewport};
use biwa_player_core::layout::{BoxStyle, Flow, LayoutBox, LayoutNode, LayoutTree, Layouter};
use biwa_player_core::text::{DEFAULT_FONT_SIZE, ParagraphStyle, TextShaper, TextSpan, TextStyle};
use biwa_player_core::ui::UiSize;
use taffy::prelude::*;
use taffy::{BoxSizing, compute_leaf_layout};

/// taffy で UI をレイアウトする。木は毎回作り直す (UI が変わったときにしか呼ばれない)。
#[derive(Debug, Default)]
pub struct TaffyLayouter;

impl TaffyLayouter {
    pub fn new() -> Self {
        Self
    }
}

impl Layouter for TaffyLayouter {
    fn layout(&mut self, tree: &LayoutTree, viewport: Viewport, shaper: &mut dyn TextShaper) -> Vec<LayoutBox> {
        let mut boxes = vec![LayoutBox::default(); tree.nodes.len()];
        if tree.nodes.is_empty() {
            return boxes;
        }
        // 文字の葉は、木の中の添字を context に持たせて測る。
        let mut taffy: TaffyTree<usize> = TaffyTree::new();
        let mut ids = vec![None; tree.nodes.len()];
        let Some(root) = build(&mut taffy, tree, tree.root, viewport, true, &mut ids) else {
            return boxes;
        };
        let available = Size {
            width: AvailableSpace::Definite(viewport.width),
            height: AvailableSpace::Definite(viewport.height),
        };
        let result = taffy.compute_layout_with_measure(root, available, |inputs, _, context, style| {
            compute_leaf_layout(inputs, style, |_, _| 0.0, |known, available| {
                let Some(text) = context.and_then(|index| tree.nodes[*index].text.as_ref()) else {
                    return Size::ZERO;
                };
                let max_width = known.width.unwrap_or(match available.width {
                    AvailableSpace::Definite(w) => w,
                    // 折り返せるところでは全部折り返したときの幅。
                    AvailableSpace::MinContent => 0.0,
                    AvailableSpace::MaxContent => f32::INFINITY,
                });
                // 描くときと同じ大きさ (物理ピクセル) で整形して測る。論理ピクセルの大きさで測ると、
                // 行の高さの丸めや字幅の誤差が拡大率で変わり、描いた文字が箱からずれることがある。
                let scale = viewport.scale.max(f32::MIN_POSITIVE);
                let spans: Vec<TextSpan> = text
                    .spans
                    .iter()
                    .map(|span| TextSpan {
                        text: span.text.clone(),
                        style: TextStyle { size: span.style.size * scale, ..span.style.clone() },
                    })
                    .collect();
                let shaped = shaper.shape(
                    &spans,
                    ParagraphStyle {
                        max_width: max_width * scale,
                        line_height: text.line_height,
                        center: false,
                        base_size: DEFAULT_FONT_SIZE * scale,
                    },
                );
                Size {
                    width: known.width.unwrap_or((shaped.width / scale).ceil()),
                    height: known.height.unwrap_or((shaped.height / scale).ceil()),
                }
            })
        });
        if result.is_err() {
            return boxes;
        }
        place(&taffy, tree, tree.root, (0.0, 0.0), &ids, &mut boxes);
        boxes
    }
}

/// `index` の部分木を taffy の木にする。
fn build(
    taffy: &mut TaffyTree<usize>,
    tree: &LayoutTree,
    index: usize,
    viewport: Viewport,
    is_root: bool,
    ids: &mut [Option<NodeId>],
) -> Option<NodeId> {
    let node = &tree.nodes[index];
    let mut style = style_of(node, viewport);
    if is_root {
        style.size = Size { width: length(viewport.width), height: length(viewport.height) };
    }
    let id = if node.text.is_some() {
        taffy.new_leaf_with_context(style, index).ok()?
    } else {
        let children: Vec<NodeId> =
            node.children.iter().filter_map(|c| build(taffy, tree, *c, viewport, false, ids)).collect();
        taffy.new_with_children(style, &children).ok()?
    };
    ids[index] = Some(id);
    Some(id)
}

/// 計算した位置 (親からの相対) を画面の上の位置にして書き込む。
fn place(
    taffy: &TaffyTree<usize>,
    tree: &LayoutTree,
    index: usize,
    origin: (f32, f32),
    ids: &[Option<NodeId>],
    boxes: &mut [LayoutBox],
) {
    let Some(layout) = ids[index].and_then(|id| taffy.layout(id).ok()) else { return };
    let x = origin.0 + layout.location.x;
    let y = origin.1 + layout.location.y;
    let frame = Rect::new(x, y, layout.size.width, layout.size.height);
    let (p, b) = (layout.padding, layout.border);
    let content = frame.inset(p.left + b.left, p.right + b.right, p.top + b.top, p.bottom + b.bottom);
    boxes[index] = LayoutBox { frame, content };
    for child in &tree.nodes[index].children {
        place(taffy, tree, *child, (x, y), ids, boxes);
    }
}

/// `core` の書式を taffy のスタイルにする。
fn style_of(node: &LayoutNode, viewport: Viewport) -> Style {
    let s: &BoxStyle = &node.style;
    let mut style = Style {
        box_sizing: if s.border_box { BoxSizing::BorderBox } else { BoxSizing::ContentBox },
        size: Size { width: dimension(s.width, viewport), height: dimension(s.height, viewport) },
        margin: taffy::Rect {
            left: margin(s.margin[0], viewport),
            right: margin(s.margin[1], viewport),
            top: margin(s.margin[2], viewport),
            bottom: margin(s.margin[3], viewport),
        },
        padding: taffy::Rect {
            left: padding(s.padding[0], viewport),
            right: padding(s.padding[1], viewport),
            top: padding(s.padding[2], viewport),
            bottom: padding(s.padding[3], viewport),
        },
        ..Style::default()
    };

    match s.flow {
        Flow::Block => style.display = Display::Block,
        Flow::Row | Flow::Column => {
            style.display = Display::Flex;
            style.flex_direction = if s.flow == Flow::Row { FlexDirection::Row } else { FlexDirection::Column };
        }
        Flow::Grid { columns } => {
            style.display = Display::Grid;
            style.grid_template_columns = vec![repeat(columns.max(1), vec![fr(1.0)])];
        }
        Flow::Stack => {
            style.display = Display::Grid;
            style.grid_template_columns = vec![percent(1.0)];
            style.grid_template_rows = vec![percent(1.0)];
            // TypeScript 版の Layers は自分の大きさを 100 % にしている。
            style.size = Size { width: percent(1.0), height: percent(1.0) };
        }
        Flow::Center => {
            style.display = Display::Flex;
            style.align_items = AlignItems::CENTER;
            style.justify_content = JustifyContent::CENTER;
        }
    }
    if s.fill_parent {
        style.position = Position::Absolute;
        style.inset = taffy::Rect { left: length(0.0), right: length(0.0), top: length(0.0), bottom: length(0.0) };
    }
    if s.stacked {
        style.grid_row = Line { start: line(1), end: auto() };
        style.grid_column = Line { start: line(1), end: auto() };
        style.align_self = Some(AlignSelf::START);
    }
    if s.hidden {
        style.display = Display::None;
    }
    style
}

/// 画面の単位 (vw / vh) は px に解く。percent は taffy に渡して親に対して解かせる。
fn dimension(size: Option<UiSize>, viewport: Viewport) -> Dimension {
    match size {
        None => Dimension::auto(),
        Some(UiSize::Percent(v)) => Dimension::percent(v / 100.0),
        Some(s) => Dimension::length(s.resolve(viewport, 0.0)),
    }
}

fn margin(size: Option<UiSize>, viewport: Viewport) -> LengthPercentageAuto {
    match size {
        None => LengthPercentageAuto::length(0.0),
        Some(UiSize::Percent(v)) => LengthPercentageAuto::percent(v / 100.0),
        Some(s) => LengthPercentageAuto::length(s.resolve(viewport, 0.0)),
    }
}

fn padding(size: Option<UiSize>, viewport: Viewport) -> LengthPercentage {
    match size {
        None => LengthPercentage::length(0.0),
        Some(UiSize::Percent(v)) => LengthPercentage::percent(v / 100.0),
        Some(s) => LengthPercentage::length(s.resolve(viewport, 0.0)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use biwa_player_core::geometry::Color;
    use biwa_player_core::layout::LayoutText;
    use biwa_player_core::text::{LineHeight, ShapedText};

    /// 1 文字 10 px の等幅で、幅を超えたら折り返す整形 (行の高さ 20 px)。
    struct Mono;

    impl TextShaper for Mono {
        fn shape(&mut self, spans: &[TextSpan], p: ParagraphStyle) -> ShapedText {
            let chars: usize = spans.iter().map(|s| s.text.chars().count()).sum();
            let per_line = if p.max_width.is_finite() { ((p.max_width / 10.0).floor() as usize).max(1) } else { chars };
            let lines = chars.div_ceil(per_line.max(1)).max(1);
            ShapedText { glyphs: Vec::new(), width: (chars.min(per_line) * 10) as f32, height: (lines * 20) as f32 }
        }
    }

    const VP: Viewport = Viewport::new(1000.0, 500.0, 1.0);

    fn node(style: BoxStyle, children: Vec<usize>) -> LayoutNode {
        LayoutNode { style, text: None, children }
    }

    fn text(s: &str, center: bool) -> LayoutNode {
        LayoutNode {
            style: BoxStyle::new(Flow::Block),
            text: Some(LayoutText {
                spans: vec![TextSpan {
                    text: s.into(),
                    style: TextStyle { size: 16.0, weight: 400, color: Color::BLACK, family: None },
                }],
                line_height: LineHeight::Normal,
                center,
            }),
            children: Vec::new(),
        }
    }

    fn sized(flow: Flow, w: Option<UiSize>, h: Option<UiSize>) -> BoxStyle {
        BoxStyle { width: w, height: h, ..BoxStyle::new(flow) }
    }

    fn layout(nodes: Vec<LayoutNode>) -> Vec<LayoutBox> {
        TaffyLayouter.layout(&LayoutTree { nodes, root: 0 }, VP, &mut Mono)
    }

    #[test]
    fn page_fills_the_screen_and_vertical_stacks_children_with_margins() {
        let page = BoxStyle { fill_parent: true, ..BoxStyle::new(Flow::Block) };
        let mut vertical = sized(Flow::Column, Some(UiSize::Vw(40.0)), None);
        vertical.margin[0] = Some(UiSize::Vw(5.0));
        vertical.margin[2] = Some(UiSize::Vh(10.0));
        let a = sized(Flow::Block, None, Some(UiSize::Vh(10.0)));
        let mut b = sized(Flow::Block, Some(UiSize::Vw(30.0)), Some(UiSize::Vh(8.0)));
        b.margin[2] = Some(UiSize::Vh(2.0));
        let boxes = layout(vec![
            node(BoxStyle::new(Flow::Block), vec![1]),
            node(page, vec![2]),
            node(vertical, vec![3, 4]),
            node(a, vec![]),
            node(b, vec![]),
        ]);
        assert_eq!(boxes[1].frame, Rect::new(0.0, 0.0, 1000.0, 500.0));
        assert_eq!(boxes[2].frame, Rect::new(50.0, 50.0, 400.0, 100.0));
        // flex の column の子は幅いっぱいに伸びる。
        assert_eq!(boxes[3].frame, Rect::new(50.0, 50.0, 400.0, 50.0));
        assert_eq!(boxes[4].frame, Rect::new(50.0, 110.0, 300.0, 40.0));
    }

    #[test]
    fn layers_stack_children_in_one_cell() {
        let layers = BoxStyle::new(Flow::Stack);
        let canvas = BoxStyle {
            stacked: true,
            ..sized(Flow::Block, Some(UiSize::Percent(100.0)), Some(UiSize::Percent(100.0)))
        };
        let mut message = BoxStyle {
            stacked: true,
            border_box: true,
            ..sized(Flow::Block, Some(UiSize::Percent(100.0)), Some(UiSize::Percent(36.0)))
        };
        message.margin[2] = Some(UiSize::Vh(64.0));
        message.padding[0] = Some(UiSize::Percent(2.5));
        let page = BoxStyle { fill_parent: true, ..BoxStyle::new(Flow::Block) };
        let boxes = layout(vec![
            node(BoxStyle::new(Flow::Block), vec![1]),
            node(page, vec![2]),
            node(layers, vec![3, 4]),
            node(canvas, vec![]),
            node(message, vec![]),
        ]);
        assert_eq!(boxes[3].frame, Rect::new(0.0, 0.0, 1000.0, 500.0));
        assert_eq!(boxes[4].frame, Rect::new(0.0, 320.0, 1000.0, 180.0));
        // border-box なので padding は枠の内側に取る (percent は親の幅に対して)。
        assert_eq!(boxes[4].content, Rect::new(25.0, 320.0, 975.0, 180.0));
    }

    #[test]
    fn grid_splits_columns_evenly() {
        let grid = sized(Flow::Grid { columns: 4 }, Some(UiSize::Vw(80.0)), None);
        let cell = sized(Flow::Block, None, Some(UiSize::Vh(10.0)));
        let boxes = layout(vec![
            node(BoxStyle::new(Flow::Block), vec![1]),
            node(grid, vec![2, 3, 4, 5, 6]),
            node(cell, vec![]),
            node(cell, vec![]),
            node(cell, vec![]),
            node(cell, vec![]),
            node(cell, vec![]),
        ]);
        assert_eq!(boxes[2].frame, Rect::new(0.0, 0.0, 200.0, 50.0));
        assert_eq!(boxes[5].frame, Rect::new(600.0, 0.0, 200.0, 50.0));
        // 5 つ目は次の行。
        assert_eq!(boxes[6].frame, Rect::new(0.0, 50.0, 200.0, 50.0));
    }

    #[test]
    fn link_centers_its_text_and_box_text_wraps() {
        let link = sized(Flow::Center, Some(UiSize::Vw(30.0)), Some(UiSize::Vh(10.0)));
        let bx = sized(Flow::Block, Some(UiSize::Vw(5.0)), None);
        let boxes = layout(vec![
            node(BoxStyle::new(Flow::Block), vec![1, 3]),
            node(link, vec![2]),
            text("abcd", true),
            node(bx, vec![4]),
            text("abcdefghij", false),
        ]);
        // 300 x 50 の Link の中央に、幅 40 高さ 20 の文字。
        assert_eq!(boxes[2].frame, Rect::new(130.0, 15.0, 40.0, 20.0));
        // 幅 50 の Box の文字は 5 文字ずつ 2 行に折り返し、Box はその高さになる。
        assert_eq!(boxes[4].frame.height, 40.0);
        assert_eq!(boxes[3].frame.height, 40.0);
    }

    #[test]
    fn hidden_nodes_take_no_space() {
        let hidden = BoxStyle { hidden: true, ..sized(Flow::Block, None, Some(UiSize::Vh(50.0))) };
        let after = sized(Flow::Block, None, Some(UiSize::Vh(10.0)));
        let boxes = layout(vec![
            node(BoxStyle::new(Flow::Block), vec![1, 2]),
            node(hidden, vec![]),
            node(after, vec![]),
        ]);
        assert_eq!(boxes[2].frame.y, 0.0);
    }
}
