//! UI Element の木 (`docs/ui-api.md`)。
//!
//! 今は**段 4 までの仮の実装**で、レイアウトをしない (taffy による本物のレイアウトは段 5)。
//! 持っているのは次の 2 つだけである。
//!
//! - scene を始めるのに要るもの: Element の親子、預けたハンドラの番号、Window の ScenePage と
//!   ScenePage の出力先 (`engine/nodejs/src/engine/ui/UIObjects.ts` の `handleSceneStartClick`)。
//! - 箱の property (大きさ・余白・背景)。scene の MessageArea と Canvas を描くのに使う。
//!   位置は親を画面全体とみなして決める ([`UiTree::element_box`])。
//!
//! kind の番号は `engine/nodejs/src/engine/api/ui.ts` が正で、std はそれを `.wat` に直書きしている。

use std::collections::HashMap;

use crate::geometry::{Color, Rect, Viewport};
use crate::syscall::{SceneStart, UiId};

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
}

/// 数値の property の番号 (`api/ui.ts` の `PropertyKind`)。
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
    pub const WINDOW_SCENE_PAGE: i32 = 15;
    pub const SCENE_PAGE_CANVAS: i32 = 16;
    pub const SCENE_PAGE_MESSAGE_AREA: i32 = 17;
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

/// 箱の property。
#[derive(Debug, Clone, Copy, Default, PartialEq)]
struct BoxProperties {
    width: Option<UiSize>,
    height: Option<UiSize>,
    /// left, right, top, bottom。
    margin: [Option<UiSize>; 4],
    padding: [Option<UiSize>; 4],
    background: Option<Color>,
}

#[derive(Debug)]
struct Node {
    kind: ElementKind,
    parent: Option<UiId>,
    children: Vec<UiId>,
    handlers: HashMap<i32, u32>,
    props: BoxProperties,
    /// Window の ScenePage。
    scene_page: Option<UiId>,
    /// ScenePage の出力先。
    scene_canvas: Option<UiId>,
    scene_message_area: Option<UiId>,
}

/// ScenePage と、その scene の出力先。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SceneOutputs {
    pub page: UiId,
    pub canvas: Option<UiId>,
    pub message_area: Option<UiId>,
}

/// UI の契約違反 (std とエンジンの食い違い)。止まらない syscall から来るので、ログに出して捨てる。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UiError(pub String);

impl std::fmt::Display for UiError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

/// Element を描くための箱。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ElementBox {
    /// 背景を塗る範囲 (論理ピクセル)。
    pub frame: Rect,
    /// 文字を置く範囲 (padding の内側)。
    pub content: Rect,
    pub background: Option<Color>,
}

#[derive(Debug, Default)]
pub struct UiTree {
    nodes: HashMap<UiId, Node>,
}

impl UiTree {
    pub fn create(&mut self, id: UiId, kind: i32) -> Result<(), UiError> {
        let kind = ElementKind::from_raw(kind)
            .ok_or_else(|| UiError(format!("unknown ui element kind {kind}")))?;
        self.nodes.insert(
            id,
            Node {
                kind,
                parent: None,
                children: Vec::new(),
                handlers: HashMap::new(),
                props: BoxProperties::default(),
                scene_page: None,
                scene_canvas: None,
                scene_message_area: None,
            },
        );
        Ok(())
    }

    pub fn kind(&self, id: UiId) -> Option<ElementKind> {
        self.nodes.get(&id).map(|n| n.kind)
    }

    pub fn push_child(&mut self, parent: UiId, child: UiId) -> Result<(), UiError> {
        if !self.nodes.contains_key(&parent) {
            return Err(UiError(format!("unknown ui element {parent} (parent)")));
        }
        let node = self
            .nodes
            .get_mut(&child)
            .ok_or_else(|| UiError(format!("unknown ui element {child} (child)")))?;
        node.parent = Some(parent);
        self.node_mut(parent)?.children.push(child);
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    pub fn set_property(
        &mut self,
        id: UiId,
        kind: i32,
        u: [i32; 4],
        _i: [i32; 2],
        f: f32,
    ) -> Result<(), UiError> {
        let node = self.node_mut(id)?;
        let size = || {
            UiSize::from_raw(u[0], f)
                .ok_or_else(|| UiError(format!("unknown size unit {} on ui element {id}", u[0])))
        };
        match kind {
            property::WIDTH => node.props.width = Some(size()?),
            property::HEIGHT => node.props.height = Some(size()?),
            property::MARGIN_LEFT..=property::MARGIN_BOTTOM => {
                node.props.margin[(kind - property::MARGIN_LEFT) as usize] = Some(size()?);
            }
            property::PADDING_LEFT..=property::PADDING_BOTTOM => {
                node.props.padding[(kind - property::PADDING_LEFT) as usize] = Some(size()?);
            }
            property::BACKGROUND_COLOR => {
                node.props.background = Some(Color::from_channels(u[0], u[1], u[2], u[3]));
            }
            property::WINDOW_SCENE_PAGE => node.scene_page = Some(u[0] as UiId),
            property::SCENE_PAGE_CANVAS => node.scene_canvas = Some(u[0] as UiId),
            property::SCENE_PAGE_MESSAGE_AREA => node.scene_message_area = Some(u[0] as UiId),
            // 残り (列数・文字の書式など) はレイアウトと一緒に段 5 で扱う。
            _ => {}
        }
        Ok(())
    }

    pub fn set_handler(&mut self, id: UiId, kind: i32, handle: u32) -> Result<(), UiError> {
        self.node_mut(id)?.handlers.insert(kind, handle);
        Ok(())
    }

    /// 最初の SceneStartButton を押したときに scene を始めるのに要るもの。
    ///
    /// 段 3 までは UI を描かないので、ボタンを押す代わりにこれでそのまま scene を始める。
    pub fn first_scene_start(&self) -> Result<SceneStart, UiError> {
        let mut buttons: Vec<_> = self
            .nodes
            .iter()
            .filter(|(_, n)| n.kind == ElementKind::SceneStartButton)
            .map(|(id, _)| *id)
            .collect();
        buttons.sort_unstable();
        let button = *buttons
            .first()
            .ok_or_else(|| UiError("the game has no SceneStartButton".into()))?;
        self.scene_start(button)
    }

    /// SceneStartButton `button` が押された。
    pub fn scene_start(&self, button: UiId) -> Result<SceneStart, UiError> {
        let node = self.node(button)?;
        let on_click = *node
            .handlers
            .get(&handler::SCENE_START_BUTTON_ON_CLICK)
            .ok_or_else(|| UiError(format!("SceneStartButton {button} has no on_click")))?;
        let window = self
            .ancestors(button)
            .find(|id| self.kind(*id) == Some(ElementKind::Window))
            .ok_or_else(|| UiError(format!("SceneStartButton {button} is outside of any Window")))?;
        let win = self.node(window)?;
        let main_scene = *win
            .handlers
            .get(&handler::WINDOW_MAIN_SCENE)
            .ok_or_else(|| UiError(format!("Window {window} has no main_scene")))?;
        let page = win
            .scene_page
            .ok_or_else(|| UiError(format!("Window {window} has no ScenePage")))?;
        let page_node = self.node(page)?;
        let output = |id: Option<UiId>, what: &str| -> Result<UiId, UiError> {
            let id = id.ok_or_else(|| UiError(format!("ScenePage {page} has no {what}")))?;
            if !self.ancestors(id).any(|a| a == page) {
                return Err(UiError(format!(
                    "the {what} of ScenePage {page} is ui element {id}, which must be placed inside the ScenePage"
                )));
            }
            Ok(id)
        };
        Ok(SceneStart {
            window,
            on_click,
            main_scene,
            canvas: output(page_node.scene_canvas, "canvas")?,
            message_area: output(page_node.scene_message_area, "message_area")?,
        })
    }

    /// Window の ScenePage の出力先。
    pub fn scene_outputs_of_window(&self, window: UiId) -> Option<SceneOutputs> {
        let page = self.nodes.get(&window)?.scene_page?;
        let node = self.nodes.get(&page)?;
        Some(SceneOutputs { page, canvas: node.scene_canvas, message_area: node.scene_message_area })
    }

    /// Element `id` の箱。
    ///
    /// **段 4 までの仮の配置。** 親を画面全体とみなし、自分の大きさ・margin (左と上)・padding だけで決める。
    /// percent の基準は CSS と同じく、幅・左右・上下の余白はどれも親の幅、高さは親の高さ。
    pub fn element_box(&self, id: UiId, viewport: Viewport) -> Option<ElementBox> {
        let node = self.nodes.get(&id)?;
        let p = &node.props;
        let (pw, ph) = (viewport.width, viewport.height);
        let w = p.width.map_or(pw, |s| s.resolve(viewport, pw));
        let h = p.height.map_or(ph, |s| s.resolve(viewport, ph));
        let side = |s: Option<UiSize>| s.map_or(0.0, |s| s.resolve(viewport, pw));
        let frame = Rect::new(side(p.margin[0]), side(p.margin[2]), w, h);
        let content = frame.inset(
            side(p.padding[0]),
            side(p.padding[1]),
            side(p.padding[2]),
            side(p.padding[3]),
        );
        Some(ElementBox { frame, content, background: p.background })
    }

    fn node(&self, id: UiId) -> Result<&Node, UiError> {
        self.nodes.get(&id).ok_or_else(|| UiError(format!("unknown ui element {id}")))
    }

    fn node_mut(&mut self, id: UiId) -> Result<&mut Node, UiError> {
        self.nodes.get_mut(&id).ok_or_else(|| UiError(format!("unknown ui element {id}")))
    }

    /// `id` から根へ向かう祖先 (`id` 自身は含まない)。ScenePage は Window の子として扱う。
    fn ancestors(&self, id: UiId) -> impl Iterator<Item = UiId> + '_ {
        let mut cur = Some(id);
        std::iter::from_fn(move || {
            let here = cur?;
            let parent = self.nodes.get(&here).and_then(|n| n.parent).or_else(|| {
                // ScenePage は push_child されず、Window の property で結びつく。
                self.nodes
                    .iter()
                    .find(|(_, n)| n.scene_page == Some(here))
                    .map(|(id, _)| *id)
            });
            cur = parent;
            parent
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// test1 と同じ形の木: Window → (ScenePage → Layers → {Canvas, MessageArea}), Page → SceneStartButton。
    fn sample() -> UiTree {
        let mut t = UiTree::default();
        t.create(1, 0).unwrap(); // Window
        t.create(2, 11).unwrap(); // ScenePage
        t.create(3, 10).unwrap(); // Layers
        t.create(4, 8).unwrap(); // Canvas
        t.create(5, 9).unwrap(); // MessageArea
        t.create(6, 1).unwrap(); // Page
        t.create(7, 12).unwrap(); // SceneStartButton
        t.push_child(3, 4).unwrap();
        t.push_child(3, 5).unwrap();
        t.push_child(2, 3).unwrap();
        t.set_property(2, property::SCENE_PAGE_CANVAS, [4, 0, 0, 0], [0, 0], 0.0).unwrap();
        t.set_property(2, property::SCENE_PAGE_MESSAGE_AREA, [5, 0, 0, 0], [0, 0], 0.0).unwrap();
        t.set_property(1, property::WINDOW_SCENE_PAGE, [2, 0, 0, 0], [0, 0], 0.0).unwrap();
        t.set_handler(1, handler::WINDOW_MAIN_SCENE, 10).unwrap();
        t.push_child(6, 7).unwrap();
        t.push_child(1, 6).unwrap();
        t.set_handler(7, handler::SCENE_START_BUTTON_ON_CLICK, 11).unwrap();
        t
    }

    #[test]
    fn finds_scene_start() {
        let start = sample().first_scene_start().unwrap();
        assert_eq!(
            start,
            SceneStart { window: 1, on_click: 11, main_scene: 10, canvas: 4, message_area: 5 }
        );
    }

    #[test]
    fn output_must_be_inside_scene_page() {
        let mut t = sample();
        t.create(8, 9).unwrap(); // ScenePage の外の MessageArea
        t.set_property(2, property::SCENE_PAGE_MESSAGE_AREA, [8, 0, 0, 0], [0, 0], 0.0).unwrap();
        assert!(t.first_scene_start().is_err());
    }

    #[test]
    fn message_area_box_resolves_sizes() {
        let mut t = sample();
        let vp = Viewport::new(1280.0, 720.0, 1.0);
        t.set_property(5, property::WIDTH, [2, 0, 0, 0], [0, 0], 100.0).unwrap();
        t.set_property(5, property::HEIGHT, [2, 0, 0, 0], [0, 0], 36.111_1).unwrap();
        t.set_property(5, property::MARGIN_TOP, [1, 0, 0, 0], [0, 0], 63.888_9).unwrap();
        t.set_property(5, property::PADDING_LEFT, [2, 0, 0, 0], [0, 0], 2.5).unwrap();
        t.set_property(5, property::PADDING_TOP, [2, 0, 0, 0], [0, 0], 1.875).unwrap();
        let b = t.element_box(5, vp).unwrap();
        assert!((b.frame.y - 460.0).abs() < 0.01);
        assert!((b.frame.height - 260.0).abs() < 0.01);
        assert!((b.content.x - 32.0).abs() < 0.01);
        assert!((b.content.y - 484.0).abs() < 0.01);
    }
}
