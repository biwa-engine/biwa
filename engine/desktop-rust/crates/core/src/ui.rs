//! UI Element の木 (`docs/ui-api.md`)。`engine/nodejs/src/engine/ui/UIObjects.ts` の移植。
//!
//! 規則は TypeScript 版と同じである。
//!
//! - `Window` は作った時点で画面に出る (ルートに付く)。他の Element は push_child されるまでどこにも繋がらない。
//! - 親が子をいくつ持てるか: Window / Page / ScenePage / Layers / Horizontal / Vertical / HorizontalGrid は多数、
//!   Box は 1 つ (積むと前の子は消える)、Link / SceneStartButton / Image / Canvas / MessageArea は持てない (積んだ子は消える)。
//!   消えた Element に預けてあった関数は手放す ([`UiTree::take_released_handlers`])。
//! - Window の Page は、最初に積まれたものが見える。Link を押すと `on_click_link` の page_id の Page に替わる。
//!   ScenePage は Link では遷移できず、scene が始まるときに見える ([`UiTree::enter_scene_page`])。
//! - Window は Page を積む前に ScenePage と `main_scene` を持っていなければならない。
//!   Canvas / MessageArea を 2 か所に置くことはできない。これらは契約違反としてエラーにする。
//! - 文字の書式 (大きさ・太さ・色・フォント) は親から受け継ぐ (CSS と同じ)。既定は 16 px・400・黒・sans-serif。
//!
//! 見た目は DOM の代わりに、レイアウト用の木 ([`UiTree::layout_tree`]) と描画の情報 ([`UiTree::node_view`]) にして渡す。
//! kind の番号は `engine/nodejs/src/engine/api/ui.ts` が正で、std はそれを `.wat` に直書きしている。

use std::collections::HashMap;

use crate::geometry::{Color, Viewport};
use crate::image::ImageHandle;
use crate::layout::{BoxStyle, Flow, LayoutNode, LayoutText, LayoutTree};
use crate::syscall::{SceneStart, UiId};
use crate::text::{LineHeight, TextSpan, TextStyle};

/// 文字の大きさの既定 (論理ピクセル。ブラウザの既定と同じ)。
const DEFAULT_TEXT_SIZE: f32 = 16.0;

/// UI Element の種類。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ElementKind {
    Window,
    Page,
    Box,
    Link,
    Horizontal,
    Vertical,
    HorizontalGrid,
    Image,
    Canvas,
    MessageArea,
    Layers,
    ScenePage,
    SceneStartButton,
}

impl ElementKind {
    pub fn from_raw(raw: i32) -> Option<Self> {
        Some(match raw {
            0 => Self::Window,
            1 => Self::Page,
            2 => Self::Box,
            3 => Self::Link,
            4 => Self::Horizontal,
            5 => Self::Vertical,
            6 => Self::HorizontalGrid,
            7 => Self::Image,
            8 => Self::Canvas,
            9 => Self::MessageArea,
            10 => Self::Layers,
            11 => Self::ScenePage,
            12 => Self::SceneStartButton,
            _ => return None,
        })
    }

    fn capacity(self) -> Capacity {
        match self {
            Self::Window
            | Self::Page
            | Self::ScenePage
            | Self::Layers
            | Self::Horizontal
            | Self::Vertical
            | Self::HorizontalGrid => Capacity::Many,
            Self::Box => Capacity::Single,
            Self::Link | Self::SceneStartButton | Self::Image | Self::Canvas | Self::MessageArea => Capacity::None,
        }
    }

    fn flow(self) -> Flow {
        match self {
            Self::Horizontal => Flow::Row,
            Self::Vertical => Flow::Column,
            Self::HorizontalGrid => Flow::Grid { columns: 1 },
            Self::Layers => Flow::Stack,
            Self::Link | Self::SceneStartButton => Flow::Center,
            _ => Flow::Block,
        }
    }
}

/// 親が子をいくつ持てるか。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Capacity {
    Many,
    Single,
    None,
}

/// property の番号 (`api/ui.ts` の `PropertyKind`)。
///
/// 範囲で受けるもの (margin・padding) の途中の番号も、表として残しておく。
#[allow(dead_code)]
mod property {
    pub const WIDTH: i32 = 0;
    pub const HEIGHT: i32 = 1;
    pub const MARGIN_LEFT: i32 = 2;
    pub const MARGIN_RIGHT: i32 = 3;
    pub const MARGIN_TOP: i32 = 4;
    pub const MARGIN_BOTTOM: i32 = 5;
    pub const PADDING_LEFT: i32 = 6;
    pub const PADDING_RIGHT: i32 = 7;
    pub const PADDING_TOP: i32 = 8;
    pub const PADDING_BOTTOM: i32 = 9;
    pub const BACKGROUND_COLOR: i32 = 10;
    pub const COLUMN: i32 = 11;
    pub const TEXT_SIZE: i32 = 12;
    pub const TEXT_WEIGHT: i32 = 13;
    pub const TEXT_COLOR: i32 = 14;
    pub const WINDOW_SCENE_PAGE: i32 = 15;
    pub const SCENE_PAGE_CANVAS: i32 = 16;
    pub const SCENE_PAGE_MESSAGE_AREA: i32 = 17;

    pub const PAGE_ID: i32 = 100;
    pub const ON_CLICK_LINK: i32 = 101;
    pub const TEXT: i32 = 102;
    pub const TEXT_FONT: i32 = 103;
    pub const BACKGROUND_IMAGE: i32 = 104;
    pub const IMAGE: i32 = 105;
    pub const ID: i32 = 106;
}

/// ハンドラの種類 (`api/ui.ts` の `HandlerKind`)。
mod handler {
    /// Window の `main_scene`。
    pub const WINDOW_MAIN_SCENE: i32 = 0;
    /// SceneStartButton の `on_click`。
    pub const SCENE_START_BUTTON_ON_CLICK: i32 = 1;
}

/// UI の大きさ。
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum UiSize {
    /// 画面の幅の 1/100。
    Vw(f32),
    /// 画面の高さの 1/100。
    Vh(f32),
    /// 親の大きさに対する割合 (100 で 100 %)。どの軸の親かは property が決める。
    Percent(f32),
}

impl UiSize {
    fn from_raw(unit: i32, value: f32) -> Option<Self> {
        match unit {
            0 => Some(Self::Vw(value)),
            1 => Some(Self::Vh(value)),
            2 => Some(Self::Percent(value)),
            _ => None,
        }
    }

    /// 論理ピクセルにする。`parent` は percent の基準になる親の大きさ。
    pub fn resolve(self, viewport: Viewport, parent: f32) -> f32 {
        match self {
            Self::Vw(v) => viewport.width * v / 100.0,
            Self::Vh(v) => viewport.height * v / 100.0,
            Self::Percent(v) => parent * v / 100.0,
        }
    }
}

/// 背景。色と画像は排他 (後から設定した方が勝つ。std も排他にしている)。
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Background {
    Color(Color),
    /// `background-size: cover; background-position: center` で敷く。
    Image(ImageHandle),
}

/// 自分に設定された文字の書式 (設定されていないものは親から受け継ぐ)。
#[derive(Debug, Clone, Default, PartialEq)]
struct TextProps {
    size: Option<UiSize>,
    weight: Option<u16>,
    color: Option<Color>,
    family: Option<String>,
}

#[derive(Debug)]
struct Node {
    kind: ElementKind,
    parent: Option<UiId>,
    children: Vec<UiId>,
    handlers: HashMap<i32, u32>,
    style: BoxStyle,
    background: Option<Background>,
    /// `Image` Element の画像。
    image: Option<ImageHandle>,
    text: Option<String>,
    text_props: TextProps,
    page_id: Option<String>,
    on_click_link: Option<String>,
    name: Option<String>,
    /// Page / ScenePage が見えているか。
    visible: bool,
    /// Window の ScenePage。
    scene_page: Option<UiId>,
    /// ScenePage の出力先。
    scene_canvas: Option<UiId>,
    scene_message_area: Option<UiId>,
}

/// UI の契約違反 (std とエンジンの食い違い)。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UiError(pub String);

impl std::fmt::Display for UiError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

/// ScenePage と、その scene の出力先。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SceneOutputs {
    pub page: UiId,
    pub canvas: Option<UiId>,
    pub message_area: Option<UiId>,
}

/// Element を押したときに起きること。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UiAction {
    /// Link で Page を替えた。
    Navigated,
    /// SceneStartButton が押された。scene を始める。
    StartScene(SceneStart),
}

/// 描くのに要る、Element 1 つの情報。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct NodeView {
    pub kind: ElementKind,
    pub background: Option<Background>,
    /// `Image` Element の画像。
    pub image: Option<ImageHandle>,
    /// 押せる Element か (Link・SceneStartButton)。
    pub clickable: bool,
}

/// レイアウト用の木の、ノード 1 つが何を表すか。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LayoutSlot {
    /// 画面全体 (根)。
    Root,
    /// Element。
    Element(UiId),
    /// Element の `text` (文字だけの葉)。
    Text(UiId),
}

#[derive(Debug, Default)]
pub struct UiTree {
    nodes: HashMap<UiId, Node>,
    /// 画面に出ている Window (作った順)。
    windows: Vec<UiId>,
    names: HashMap<String, UiId>,
    /// 見た目に効く変更のたびに増える。レイアウトと文字の整形を使い回す鍵になる。
    generation: u64,
    released: Vec<u32>,
    destroyed: Vec<UiId>,
}

impl UiTree {
    pub fn generation(&self) -> u64 {
        self.generation
    }

    pub fn kind(&self, id: UiId) -> Option<ElementKind> {
        self.nodes.get(&id).map(|n| n.kind)
    }

    /// `id` property から ui_id を引く。
    pub fn resolve_name(&self, name: &str) -> Option<UiId> {
        self.names.get(name).copied()
    }

    pub fn create(&mut self, id: UiId, kind: i32) -> Result<(), UiError> {
        if self.nodes.contains_key(&id) {
            return Err(UiError(format!("ui element {id} already exists")));
        }
        let kind = ElementKind::from_raw(kind).ok_or_else(|| UiError(format!("unknown ui element kind {kind}")))?;
        let mut style = BoxStyle::new(kind.flow());
        style.fill_parent = matches!(kind, ElementKind::Window | ElementKind::Page | ElementKind::ScenePage);
        style.border_box = kind == ElementKind::MessageArea;
        self.nodes.insert(
            id,
            Node {
                kind,
                parent: None,
                children: Vec::new(),
                handlers: HashMap::new(),
                style,
                background: None,
                image: None,
                text: None,
                text_props: TextProps::default(),
                page_id: None,
                on_click_link: None,
                name: None,
                // Page / ScenePage は最初は隠れている。
                visible: !matches!(kind, ElementKind::Page | ElementKind::ScenePage),
                scene_page: None,
                scene_canvas: None,
                scene_message_area: None,
            },
        );
        if kind == ElementKind::Window {
            self.windows.push(id);
        }
        self.generation += 1;
        Ok(())
    }

    pub fn set_property(&mut self, id: UiId, kind: i32, u: [i32; 4], _i: [i32; 2], f: f32) -> Result<(), UiError> {
        match kind {
            property::WINDOW_SCENE_PAGE => return self.set_scene_page(id, u[0] as UiId),
            property::SCENE_PAGE_CANVAS | property::SCENE_PAGE_MESSAGE_AREA => {
                return self.set_scene_output(id, kind, u[0] as UiId);
            }
            _ => {}
        }
        let node = self.node_mut(id)?;
        let size = || {
            UiSize::from_raw(u[0], f).ok_or_else(|| UiError(format!("unknown size unit {} on ui element {id}", u[0])))
        };
        match kind {
            property::WIDTH => node.style.width = Some(size()?),
            property::HEIGHT => node.style.height = Some(size()?),
            property::MARGIN_LEFT..=property::MARGIN_BOTTOM => {
                node.style.margin[(kind - property::MARGIN_LEFT) as usize] = Some(size()?);
            }
            property::PADDING_LEFT..=property::PADDING_BOTTOM => {
                node.style.padding[(kind - property::PADDING_LEFT) as usize] = Some(size()?);
            }
            property::BACKGROUND_COLOR => {
                node.background = Some(Background::Color(Color::from_channels(u[0], u[1], u[2], u[3])));
            }
            property::COLUMN => {
                if node.kind != ElementKind::HorizontalGrid {
                    return Err(UiError(format!(
                        "\"column\" property is only meaningful on HorizontalGrid (ui element {id})"
                    )));
                }
                node.style.flow = Flow::Grid { columns: u[0].max(1) as u16 };
            }
            property::TEXT_SIZE => node.text_props.size = Some(size()?),
            property::TEXT_WEIGHT => node.text_props.weight = Some(u[0].clamp(1, 1000) as u16),
            property::TEXT_COLOR => node.text_props.color = Some(Color::from_channels(u[0], u[1], u[2], u[3])),
            _ => return Err(UiError(format!("unknown or non-numeric ui property: {kind}"))),
        }
        self.generation += 1;
        Ok(())
    }

    /// 文字列を伴う property を設定する。画像のパスは `image` で番号にする (読み込みの依頼はそちらの仕事)。
    pub fn set_string_property(
        &mut self,
        id: UiId,
        kind: i32,
        value: String,
        image: &mut dyn FnMut(&str) -> Result<ImageHandle, String>,
    ) -> Result<(), UiError> {
        if kind == property::ID {
            return self.set_name(id, value);
        }
        let node = self.node_mut(id)?;
        match kind {
            property::PAGE_ID => node.page_id = Some(value),
            property::ON_CLICK_LINK => node.on_click_link = Some(value),
            property::TEXT => {
                if !matches!(node.kind, ElementKind::Box | ElementKind::Link | ElementKind::SceneStartButton) {
                    return Err(UiError(format!("\"text\" property is not meaningful on ui element {id}")));
                }
                node.text = Some(value);
            }
            property::TEXT_FONT => node.text_props.family = Some(value),
            property::BACKGROUND_IMAGE => node.background = Some(Background::Image(image(&value).map_err(UiError)?)),
            property::IMAGE => {
                if node.kind != ElementKind::Image {
                    return Err(UiError(format!("\"image\" property is only meaningful on Image (ui element {id})")));
                }
                node.image = Some(image(&value).map_err(UiError)?);
            }
            _ => return Err(UiError(format!("unknown or non-string ui property: {kind}"))),
        }
        self.generation += 1;
        Ok(())
    }

    /// ハンドラを設定する。同じ種類のハンドラが既にあれば置き換え、古い番号は手放す。
    pub fn set_handler(&mut self, id: UiId, kind: i32, handle: u32) -> Result<(), UiError> {
        let target = match kind {
            handler::WINDOW_MAIN_SCENE => ElementKind::Window,
            handler::SCENE_START_BUTTON_ON_CLICK => ElementKind::SceneStartButton,
            _ => {
                self.released.push(handle);
                return Err(UiError(format!("unknown ui handler kind: {kind}")));
            }
        };
        let Some(node) = self.nodes.get_mut(&id) else {
            self.released.push(handle);
            return Err(UiError(format!("unknown ui element {id}")));
        };
        if node.kind != target {
            self.released.push(handle);
            return Err(UiError(format!("ui handler kind {kind} cannot be set on ui element {id}")));
        }
        if let Some(previous) = node.handlers.insert(kind, handle) {
            self.released.push(previous);
        }
        Ok(())
    }

    /// 子を親に積む (`docs/ui-api.md` の push_child の規則)。
    pub fn push_child(&mut self, parent: UiId, child: UiId) -> Result<(), UiError> {
        let parent_kind = self.node(parent)?.kind;
        let child_node = self.node(child)?;
        if parent_kind == ElementKind::Window {
            self.require_scene_settings(parent)?;
        }
        // Canvas / MessageArea は std が `new()` の時点で作るので、同じ値を 2 か所に置くと同じ Element が 2 回積まれる。
        if let Some(placed) = child_node.parent
            && matches!(child_node.kind, ElementKind::Canvas | ElementKind::MessageArea)
        {
            return Err(UiError(format!(
                "ui element {child} is already placed in ui element {placed}; the same element cannot be placed twice"
            )));
        }
        match parent_kind.capacity() {
            Capacity::None => self.destroy(child),
            Capacity::Single => {
                for existing in self.node(parent)?.children.clone() {
                    self.destroy(existing);
                }
                self.attach(parent, child);
            }
            Capacity::Many => self.attach(parent, child),
        }
        self.generation += 1;
        Ok(())
    }

    /// scene を始めるので、Window の ScenePage を見せて他の Page を隠す。
    pub fn enter_scene_page(&mut self, window: UiId) -> Result<SceneOutputs, UiError> {
        let outputs = self
            .scene_outputs_of_window(window)
            .ok_or_else(|| UiError(format!("Window {window} has no ScenePage to enter")))?;
        for child in self.node(window)?.children.clone() {
            if let Some(node) = self.nodes.get_mut(&child) {
                if node.kind == ElementKind::Page {
                    node.visible = false;
                } else if child == outputs.page {
                    node.visible = true;
                }
            }
        }
        self.generation += 1;
        Ok(outputs)
    }

    /// Element `id` が押された。
    pub fn click(&mut self, id: UiId) -> Result<Option<UiAction>, UiError> {
        let node = self.node(id)?;
        match node.kind {
            ElementKind::Link => {
                let Some(target) = node.on_click_link.clone() else { return Ok(None) };
                let window = self
                    .owner_window(id)
                    .ok_or_else(|| UiError(format!("a Link outside of any Window was clicked (ui element {id})")))?;
                self.show_page(window, &target)?;
                Ok(Some(UiAction::Navigated))
            }
            ElementKind::SceneStartButton => Ok(Some(UiAction::StartScene(self.scene_start(id)?))),
            _ => Ok(None),
        }
    }

    /// 最初の SceneStartButton を押したときに scene を始めるのに要るもの (ウィンドウを持たない動かし方で使う)。
    pub fn first_scene_start(&self) -> Result<SceneStart, UiError> {
        let mut buttons: Vec<_> = self
            .nodes
            .iter()
            .filter(|(_, n)| n.kind == ElementKind::SceneStartButton)
            .map(|(id, _)| *id)
            .collect();
        buttons.sort_unstable();
        let button = *buttons.first().ok_or_else(|| UiError("the game has no SceneStartButton".into()))?;
        self.scene_start(button)
    }

    /// SceneStartButton `button` が押されたときに scene を始めるのに要るもの。
    pub fn scene_start(&self, button: UiId) -> Result<SceneStart, UiError> {
        let node = self.node(button)?;
        let on_click = *node
            .handlers
            .get(&handler::SCENE_START_BUTTON_ON_CLICK)
            .ok_or_else(|| UiError(format!("SceneStartButton {button} has no on_click")))?;
        let window = self
            .owner_window(button)
            .ok_or_else(|| UiError(format!("SceneStartButton {button} is outside of any Window")))?;
        let win = self.node(window)?;
        let main_scene = *win
            .handlers
            .get(&handler::WINDOW_MAIN_SCENE)
            .ok_or_else(|| UiError(format!("Window {window} has no main_scene")))?;
        let page = win.scene_page.ok_or_else(|| UiError(format!("Window {window} has no ScenePage")))?;
        let (canvas, message_area) = self.scene_outputs_checked(page)?;
        Ok(SceneStart { window, on_click, main_scene, canvas, message_area })
    }

    /// Window の ScenePage の出力先。
    pub fn scene_outputs_of_window(&self, window: UiId) -> Option<SceneOutputs> {
        let page = self.nodes.get(&window)?.scene_page?;
        let node = self.nodes.get(&page)?;
        Some(SceneOutputs { page, canvas: node.scene_canvas, message_area: node.scene_message_area })
    }

    /// 手放してよくなった関数の番号を取り出す。
    pub fn take_released_handlers(&mut self) -> Vec<u32> {
        std::mem::take(&mut self.released)
    }

    /// 消えた Element を取り出す (エンジンがそれに紐づく状態を捨てる)。
    pub fn take_destroyed(&mut self) -> Vec<UiId> {
        std::mem::take(&mut self.destroyed)
    }

    /// 描くのに要る、Element 1 つの情報。
    pub fn node_view(&self, id: UiId) -> Option<NodeView> {
        let n = self.nodes.get(&id)?;
        let clickable = match n.kind {
            ElementKind::Link => n.on_click_link.is_some(),
            ElementKind::SceneStartButton => true,
            _ => false,
        };
        Some(NodeView { kind: n.kind, background: n.background, image: n.image, clickable })
    }

    /// レイアウト用の木にする。返す `Vec` は、木のノードがどの Element (の文字) かの対応 (`tree.nodes` と同じ順)。
    ///
    /// 根は画面全体で、その子が画面に出ている Window (作った順)。見えていない Page は `hidden` になる。
    /// 文字の大きさは論理ピクセルに解いて入れる (書式は親から受け継ぐ)。
    pub fn layout_tree(&self, viewport: Viewport) -> (LayoutTree, Vec<LayoutSlot>) {
        let mut tree = LayoutTree::default();
        let mut slots = Vec::new();
        let root_style = TextStyle { size: DEFAULT_TEXT_SIZE, weight: 400, color: Color::BLACK, family: None };
        tree.nodes.push(LayoutNode { style: BoxStyle::new(Flow::Block), text: None, children: Vec::new() });
        slots.push(LayoutSlot::Root);
        for window in &self.windows {
            if let Some(index) = self.push_layout(*window, false, &root_style, viewport, &mut tree, &mut slots) {
                tree.nodes[0].children.push(index);
            }
        }
        (tree, slots)
    }

    fn push_layout(
        &self,
        id: UiId,
        stacked: bool,
        inherited: &TextStyle,
        viewport: Viewport,
        tree: &mut LayoutTree,
        slots: &mut Vec<LayoutSlot>,
    ) -> Option<usize> {
        let node = self.nodes.get(&id)?;
        let text_style = self.text_style(node, inherited, viewport);
        let mut style = node.style;
        style.stacked = stacked;
        style.hidden = !node.visible;
        let index = tree.nodes.len();
        tree.nodes.push(LayoutNode { style, text: None, children: Vec::new() });
        slots.push(LayoutSlot::Element(id));

        // 文字は、子より前に置いた文字だけの葉にする (Box では子と分けて持つ。Link では中央に寄せる)。
        if let Some(text) = &node.text {
            let center = node.kind != ElementKind::Box;
            let leaf = tree.nodes.len();
            tree.nodes.push(LayoutNode {
                style: BoxStyle::new(Flow::Block),
                text: Some(LayoutText {
                    spans: vec![TextSpan { text: text.clone(), style: text_style.clone() }],
                    // CSS の既定 (`line-height: normal`) と同じく、フォント自身の値で決める。
                    line_height: LineHeight::Normal,
                    center,
                }),
                children: Vec::new(),
            });
            slots.push(LayoutSlot::Text(id));
            tree.nodes[index].children.push(leaf);
        }
        let stack_children = node.kind == ElementKind::Layers;
        for child in &node.children {
            if let Some(c) = self.push_layout(*child, stack_children, &text_style, viewport, tree, slots) {
                tree.nodes[index].children.push(c);
            }
        }
        Some(index)
    }

    /// 自分の文字の書式 (設定されていないものは親から受け継ぐ)。大きさは論理ピクセル。
    fn text_style(&self, node: &Node, inherited: &TextStyle, viewport: Viewport) -> TextStyle {
        let p = &node.text_props;
        TextStyle {
            // percent は親の文字の大きさに対する割合 (CSS の font-size と同じ)。
            size: p.size.map_or(inherited.size, |s| s.resolve(viewport, inherited.size)),
            weight: p.weight.unwrap_or(inherited.weight),
            color: p.color.unwrap_or(inherited.color),
            family: p.family.clone().or_else(|| inherited.family.clone()),
        }
    }

    fn attach(&mut self, parent: UiId, child: UiId) {
        self.detach(child);
        if let Some(p) = self.nodes.get_mut(&parent) {
            p.children.push(child);
        }
        if let Some(c) = self.nodes.get_mut(&child) {
            c.parent = Some(parent);
        }
        if self.kind(child) == Some(ElementKind::Page) {
            self.show_first_page_if_none_visible(parent, child);
        }
    }

    fn detach(&mut self, id: UiId) {
        let Some(parent) = self.nodes.get(&id).and_then(|n| n.parent) else { return };
        if let Some(p) = self.nodes.get_mut(&parent) {
            p.children.retain(|c| *c != id);
            if p.scene_page == Some(id) {
                p.scene_page = None;
            }
        }
        if let Some(n) = self.nodes.get_mut(&id) {
            n.parent = None;
        }
    }

    /// 木から消す。子も道連れにする。
    fn destroy(&mut self, id: UiId) {
        self.detach(id);
        let Some(node) = self.nodes.remove(&id) else { return };
        for child in node.children {
            if let Some(c) = self.nodes.get_mut(&child) {
                c.parent = None;
            }
            self.destroy(child);
        }
        if let Some(name) = node.name
            && self.names.get(&name) == Some(&id)
        {
            self.names.remove(&name);
        }
        self.released.extend(node.handlers.into_values());
        self.windows.retain(|w| *w != id);
        self.destroyed.push(id);
    }

    fn set_name(&mut self, id: UiId, name: String) -> Result<(), UiError> {
        let node = self.node_mut(id)?;
        if let Some(old) = node.name.replace(name.clone())
            && old != name
            && self.names.get(&old) == Some(&id)
        {
            self.names.remove(&old);
        }
        // 同じ名前が別の Element に使われていても上書きする (TypeScript 版はログを出して上書き)。
        self.names.insert(name, id);
        Ok(())
    }

    fn set_scene_page(&mut self, window: UiId, page: UiId) -> Result<(), UiError> {
        if self.node(window)?.kind != ElementKind::Window {
            return Err(UiError(format!("\"scene_page\" property is not meaningful on ui element {window}")));
        }
        if self.kind(page) != Some(ElementKind::ScenePage) {
            return Err(UiError(format!(
                "\"scene_page\" of ui element {window} must be a ScenePage, but got ui element {page}"
            )));
        }
        // ScenePage の部分木はこの時点で組み上がっているので、出力先が揃っていることをここで確かめる。
        self.scene_outputs_checked(page)?;
        let previous = self.node(window)?.scene_page;
        if previous == Some(page) {
            return Ok(());
        }
        if let Some(previous) = previous {
            self.destroy(previous);
        }
        self.attach(window, page);
        self.node_mut(window)?.scene_page = Some(page);
        self.generation += 1;
        Ok(())
    }

    fn set_scene_output(&mut self, page: UiId, kind: i32, target: UiId) -> Result<(), UiError> {
        let (what, expected) = if kind == property::SCENE_PAGE_CANVAS {
            ("canvas", ElementKind::Canvas)
        } else {
            ("message_area", ElementKind::MessageArea)
        };
        if self.node(page)?.kind != ElementKind::ScenePage {
            return Err(UiError(format!("\"{what}\" property is not meaningful on ui element {page}")));
        }
        if self.kind(target) != Some(expected) {
            return Err(UiError(format!(
                "\"{what}\" of ui element {page} (ScenePage) must be a {expected:?}, but got ui element {target}"
            )));
        }
        let node = self.node_mut(page)?;
        if expected == ElementKind::Canvas {
            node.scene_canvas = Some(target);
        } else {
            node.scene_message_area = Some(target);
        }
        Ok(())
    }

    /// ScenePage の出力先。どちらも設定されていて、ScenePage の子孫に置かれていなければならない。
    fn scene_outputs_checked(&self, page: UiId) -> Result<(UiId, UiId), UiError> {
        let node = self.node(page)?;
        let output = |id: Option<UiId>, what: &str| -> Result<UiId, UiError> {
            let id = id.ok_or_else(|| UiError(format!("ui element {page} (ScenePage) must have its \"{what}\" set")))?;
            if !self.is_descendant_of(id, page) {
                return Err(UiError(format!(
                    "the \"{what}\" of ui element {page} (ScenePage) is ui element {id}, which must be placed inside the ScenePage"
                )));
            }
            Ok(id)
        };
        Ok((output(node.scene_canvas, "canvas")?, output(node.scene_message_area, "message_area")?))
    }

    /// Window がノベルゲームとして scene を始められることを確かめる (Page を積む前に要る)。
    fn require_scene_settings(&self, window: UiId) -> Result<(), UiError> {
        let node = self.node(window)?;
        let mut missing = Vec::new();
        if node.scene_page.is_none() {
            missing.push("`scene_page`");
        }
        if !node.handlers.contains_key(&handler::WINDOW_MAIN_SCENE) {
            missing.push("`main_scene`");
        }
        if missing.is_empty() {
            Ok(())
        } else {
            Err(UiError(format!(
                "ui element {window} (Window) must have {} set before its pages are pushed",
                missing.join(" and ")
            )))
        }
    }

    fn show_first_page_if_none_visible(&mut self, window: UiId, page: UiId) {
        let Some(win) = self.nodes.get(&window) else { return };
        let any_visible = win
            .children
            .iter()
            .filter(|c| **c != page)
            .filter_map(|c| self.nodes.get(c))
            .any(|n| n.kind == ElementKind::Page && n.visible);
        if !any_visible && let Some(n) = self.nodes.get_mut(&page) {
            n.visible = true;
        }
    }

    /// Window 直下の Page を、page_id が一致するものだけ見せる (ScenePage は隠す)。
    fn show_page(&mut self, window: UiId, page_id: &str) -> Result<(), UiError> {
        let mut found = false;
        for child in self.node(window)?.children.clone() {
            let Some(node) = self.nodes.get_mut(&child) else { continue };
            match node.kind {
                ElementKind::ScenePage => node.visible = false,
                ElementKind::Page => {
                    node.visible = node.page_id.as_deref() == Some(page_id);
                    found |= node.visible;
                }
                _ => {}
            }
        }
        self.generation += 1;
        if found {
            Ok(())
        } else {
            Err(UiError(format!("no page \"{page_id}\" in ui element {window} (Window)")))
        }
    }

    fn owner_window(&self, id: UiId) -> Option<UiId> {
        let mut cur = Some(id);
        while let Some(here) = cur {
            if self.kind(here) == Some(ElementKind::Window) {
                return Some(here);
            }
            cur = self.nodes.get(&here).and_then(|n| n.parent);
        }
        None
    }

    fn is_descendant_of(&self, id: UiId, ancestor: UiId) -> bool {
        let mut cur = self.nodes.get(&id).and_then(|n| n.parent);
        while let Some(here) = cur {
            if here == ancestor {
                return true;
            }
            cur = self.nodes.get(&here).and_then(|n| n.parent);
        }
        false
    }

    fn node(&self, id: UiId) -> Result<&Node, UiError> {
        self.nodes.get(&id).ok_or_else(|| UiError(format!("unknown ui element {id}")))
    }

    fn node_mut(&mut self, id: UiId) -> Result<&mut Node, UiError> {
        self.nodes.get_mut(&id).ok_or_else(|| UiError(format!("unknown ui element {id}")))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn no_images(_: &str) -> Result<ImageHandle, String> {
        Ok(ImageHandle(1))
    }

    fn set_str(t: &mut UiTree, id: UiId, kind: i32, value: &str) {
        t.set_string_property(id, kind, value.into(), &mut no_images).unwrap();
    }

    /// test1 と同じ形の木: Window → (ScenePage → Layers → {Canvas, MessageArea}), Page "main" → SceneStartButton,
    /// Page "second" → Link "main"。
    fn sample() -> UiTree {
        let mut t = UiTree::default();
        for (id, kind) in [(1, 0), (2, 11), (3, 10), (4, 8), (5, 9), (6, 1), (7, 12), (8, 1), (9, 3)] {
            t.create(id, kind).unwrap();
        }
        t.push_child(3, 4).unwrap();
        t.push_child(3, 5).unwrap();
        t.push_child(2, 3).unwrap();
        t.set_property(2, property::SCENE_PAGE_CANVAS, [4, 0, 0, 0], [0, 0], 0.0).unwrap();
        t.set_property(2, property::SCENE_PAGE_MESSAGE_AREA, [5, 0, 0, 0], [0, 0], 0.0).unwrap();
        t.set_property(1, property::WINDOW_SCENE_PAGE, [2, 0, 0, 0], [0, 0], 0.0).unwrap();
        t.set_handler(1, handler::WINDOW_MAIN_SCENE, 10).unwrap();
        set_str(&mut t, 6, property::PAGE_ID, "main");
        set_str(&mut t, 8, property::PAGE_ID, "second");
        set_str(&mut t, 9, property::ON_CLICK_LINK, "main");
        t.set_handler(7, handler::SCENE_START_BUTTON_ON_CLICK, 11).unwrap();
        t.push_child(6, 7).unwrap();
        t.push_child(8, 9).unwrap();
        t.push_child(1, 6).unwrap();
        t.push_child(1, 8).unwrap();
        t
    }

    fn visible(t: &UiTree, id: UiId) -> bool {
        t.nodes[&id].visible
    }

    #[test]
    fn finds_scene_start() {
        let start = sample().first_scene_start().unwrap();
        assert_eq!(start, SceneStart { window: 1, on_click: 11, main_scene: 10, canvas: 4, message_area: 5 });
    }

    #[test]
    fn output_must_be_inside_scene_page() {
        let mut t = sample();
        t.create(20, 9).unwrap(); // ScenePage の外の MessageArea
        t.set_property(2, property::SCENE_PAGE_MESSAGE_AREA, [20, 0, 0, 0], [0, 0], 0.0).unwrap();
        assert!(t.first_scene_start().is_err());
    }

    #[test]
    fn first_page_is_visible_and_links_navigate() {
        let mut t = sample();
        assert!(visible(&t, 6) && !visible(&t, 8) && !visible(&t, 2));
        // "second" を見せてから、Link で "main" に戻る。
        t.show_page(1, "second").unwrap();
        assert!(!visible(&t, 6) && visible(&t, 8));
        assert_eq!(t.click(9).unwrap(), Some(UiAction::Navigated));
        assert!(visible(&t, 6) && !visible(&t, 8));
    }

    #[test]
    fn clicking_scene_start_button_starts_the_scene() {
        let mut t = sample();
        let Some(UiAction::StartScene(start)) = t.click(7).unwrap() else { panic!("no scene start") };
        assert_eq!(start.on_click, 11);
        let outputs = t.enter_scene_page(1).unwrap();
        assert_eq!(outputs.message_area, Some(5));
        assert!(!visible(&t, 6) && visible(&t, 2));
    }

    #[test]
    fn pages_need_scene_settings_first() {
        let mut t = UiTree::default();
        t.create(1, 0).unwrap();
        t.create(2, 1).unwrap();
        assert!(t.push_child(1, 2).is_err());
    }

    #[test]
    fn box_keeps_one_child_and_releases_handlers_of_the_replaced_one() {
        let mut t = sample();
        t.create(30, 2).unwrap(); // Box
        t.create(31, 12).unwrap(); // SceneStartButton
        t.set_handler(31, handler::SCENE_START_BUTTON_ON_CLICK, 40).unwrap();
        t.create(32, 2).unwrap();
        t.push_child(30, 31).unwrap();
        t.push_child(30, 32).unwrap();
        assert_eq!(t.nodes[&30].children, vec![32]);
        assert_eq!(t.take_released_handlers(), vec![40]);
        assert_eq!(t.take_destroyed(), vec![31]);
    }

    #[test]
    fn leaf_elements_drop_pushed_children() {
        let mut t = sample();
        t.create(30, 3).unwrap(); // Link
        t.create(31, 2).unwrap();
        t.push_child(30, 31).unwrap();
        assert!(t.nodes[&30].children.is_empty());
        assert!(!t.nodes.contains_key(&31));
    }

    #[test]
    fn canvas_cannot_be_placed_twice() {
        let mut t = sample();
        t.create(30, 4).unwrap();
        assert!(t.push_child(30, 4).is_err());
    }

    #[test]
    fn text_style_is_inherited_and_resolved() {
        let mut t = sample();
        // "main" の Page に文字の色と大きさ (vh) を付け、その中のボタンに文字を置く。
        t.set_property(6, property::TEXT_COLOR, [255, 255, 255, 255], [0, 0], 0.0).unwrap();
        t.set_property(6, property::TEXT_SIZE, [1, 0, 0, 0], [0, 0], 5.0).unwrap();
        t.set_property(7, property::TEXT_WEIGHT, [700, 0, 0, 0], [0, 0], 0.0).unwrap();
        set_str(&mut t, 7, property::TEXT, "start");
        let (tree, slots) = t.layout_tree(Viewport::new(1000.0, 400.0, 1.0));
        let leaf = slots.iter().position(|s| *s == LayoutSlot::Text(7)).unwrap();
        let style = &tree.nodes[leaf].text.as_ref().unwrap().spans[0].style;
        assert_eq!((style.size, style.weight, style.color), (20.0, 700, Color::WHITE));
        assert!(tree.nodes[leaf].text.as_ref().unwrap().center);
    }

    #[test]
    fn hidden_pages_are_hidden_in_layout_and_layers_children_are_stacked() {
        let t = sample();
        let (tree, slots) = t.layout_tree(Viewport::new(100.0, 100.0, 1.0));
        let at = |id| slots.iter().position(|s| *s == LayoutSlot::Element(id)).unwrap();
        assert!(!tree.nodes[at(6)].style.hidden);
        assert!(tree.nodes[at(8)].style.hidden);
        assert!(tree.nodes[at(2)].style.hidden);
        assert!(tree.nodes[at(4)].style.stacked && tree.nodes[at(5)].style.stacked);
        assert!(!tree.nodes[at(3)].style.stacked);
    }
}
