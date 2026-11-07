//! エンジン本体。メインスレッドで動き、ゲーム (vm) からの syscall を受けて状態を変え、
//! 1 フレームぶんの描画の指示 ([`DrawList`]) を組み立てる。
//!
//! ゲームを止める syscall (クリック待ち・sleep・演出待ち) は、エンジンが「まだ」と言っている間
//! 呼んだ側 (player) が返事を保留する。エンジンは返事の手段 (チャネル) を知らない。
//! 画像も同じで、エンジンは番号を振って読み込みを依頼するだけである ([`Engine::take_image_requests`])。
//!
//! 画面は UI の木 (`ui.rs`) をレイアウトして (`Layouter`)、木の順に描く。
//! 背景・画像・文字のほか、Canvas の中に canvas オブジェクトを、MessageArea の中に Message Window を描く。

use std::collections::HashMap;

use crate::canvas::Canvas;
use crate::content::{Fragment, MessageContent, SizeUnit};
use crate::draw::{DrawItem, DrawList};
use crate::geometry::{Color, Rect, Viewport};
use crate::image::{ImageHandle, ImageRequest, normalize_asset_path};
use crate::layout::{LayoutBox, LayoutTree, Layouter};
use crate::syscall::{SceneStart, Syscall, UiId};
use crate::text::{LineHeight, ParagraphStyle, ShapedText, TextShaper, TextSpan, TextStyle};
use crate::ui::{Background, ElementKind, LayoutSlot, SceneOutputs, UiAction, UiError, UiTree};

/// Message Window の文字の行の高さ (文字の大きさに対する倍率)。TypeScript 版の `line-height: 1.8` と揃える。
const LINE_HEIGHT: LineHeight = LineHeight::Relative(1.8);

/// 止まる syscall の進み具合。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CallStatus {
    /// もう終わった。すぐに返事をしてよい。
    Done,
    /// まだ。終わったら [`Engine::take_finished_call`] が true を返す。
    Pending,
}

/// 画面を押した結果。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Click {
    /// 何も起きなかったか、エンジンの中で済んだ (Page の切り替え・文字送りの完了など)。
    Handled,
    /// SceneStartButton が押された。呼んだ側が vm に scene を始めさせる。
    StartScene(SceneStart),
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum PendingCall {
    /// クリック待ち。
    Wait,
    /// 残り時間 (ミリ秒) が 0 になるまで。
    Sleep { remaining_ms: f64 },
    /// sync 印の演出がすべて終わるまで。
    AwaitTransitions,
}

/// 整形の結果の使い回し。中身と大きさが同じなら整形し直さない。
#[derive(Debug)]
struct ShapeCache {
    generation: u64,
    width: u32,
    scale: u32,
    shaped: ShapedText,
}

/// レイアウトの結果の使い回し。UI と画面の大きさが同じなら計算し直さない。
#[derive(Debug)]
struct LayoutCache {
    generation: u64,
    viewport: (u32, u32, u32),
    tree: LayoutTree,
    slots: Vec<LayoutSlot>,
    boxes: Vec<LayoutBox>,
}

#[derive(Debug, Default)]
pub struct Engine {
    ui: UiTree,
    contents: HashMap<UiId, MessageContent>,
    canvas: Canvas,
    /// エンジン時計 [ms]。`update` の差分を積む (ポーズ・早送りはここに集める)。
    now: f64,
    call: Option<PendingCall>,
    call_finished: bool,
    /// 今見えている ScenePage とその出力先。scene が始まると決まる。
    scene: Option<SceneOutputs>,
    /// Message Window の整形の使い回し (MessageArea ごと)。
    message_shapes: HashMap<UiId, ShapeCache>,
    /// UI の文字の整形の使い回し (Element ごと)。
    text_shapes: HashMap<UiId, ShapeCache>,
    layout: Option<LayoutCache>,
    /// 最後に描いたフレームの、押せる Element とその範囲 (論理ピクセル、描いた順)。
    targets: Vec<(UiId, Rect)>,
    /// パス → 画像の番号。同じパスは 1 度だけ読む。
    images: HashMap<String, ImageHandle>,
    /// 読めた画像の元の大きさ。
    image_sizes: HashMap<ImageHandle, (u32, u32)>,
    /// まだ渡していない読み込みの依頼。
    image_requests: Vec<ImageRequest>,
}

impl Engine {
    pub fn new() -> Self {
        Self::default()
    }

    /// 止まらない syscall を当てる。契約違反はエラーで返す (呼んだ側はログに出して続ける)。
    pub fn apply(&mut self, syscall: Syscall) -> Result<(), UiError> {
        let result = self.apply_inner(syscall);
        // 消えた Element に紐づく状態を捨てる (Box に積み直したときなど)。
        for id in self.ui.take_destroyed() {
            self.contents.remove(&id);
            self.message_shapes.remove(&id);
            self.text_shapes.remove(&id);
        }
        result
    }

    fn apply_inner(&mut self, syscall: Syscall) -> Result<(), UiError> {
        match syscall {
            Syscall::ContentPushText {
                ui,
                text,
                speed,
                size_unit,
                size_value,
                weight,
                r,
                g,
                b,
                a,
            } => {
                self.content_mut(ui)?.push(Fragment {
                    text,
                    speed,
                    size_unit: SizeUnit::from_raw(size_unit),
                    size_value,
                    weight: weight.clamp(1, 1000) as u16,
                    color: Color::from_channels(r, g, b, a),
                });
            }
            Syscall::ContentFlush { ui } => self.content_mut(ui)?.flush(),
            Syscall::ContentClear { ui } => self.content_mut(ui)?.clear(),

            Syscall::CreateObject { id, canvas, path, layer, x, y, width, height, alpha, theta } => {
                if self.ui.kind(canvas) != Some(ElementKind::Canvas) {
                    return Err(UiError(format!("ui element {canvas} is not a Canvas")));
                }
                let image = register_image(&mut self.images, &mut self.image_requests, &path).map_err(UiError)?;
                let values = [
                    f64::from(x),
                    f64::from(y),
                    f64::from(width),
                    f64::from(height),
                    f64::from(alpha),
                    f64::from(theta),
                ];
                let natural = self.image_sizes.get(&image).copied();
                self.canvas.create(id, canvas, image, layer, values, natural).map_err(UiError)?;
            }
            Syscall::DeleteObject { id, after } => self.canvas.remove(id, after, self.now).map_err(UiError)?,
            Syscall::AddTransition { id, param, kind, val_i, val_f, after, duration } => self
                .canvas
                .add_transition(id, param, kind, val_i, val_f, after, duration)
                .map_err(UiError)?,
            Syscall::StartTransitions { sync } => self.canvas.start(sync != 0, self.now),

            Syscall::UiCreate { id, kind } => {
                self.ui.create(id, kind)?;
                if self.ui.kind(id) == Some(ElementKind::MessageArea) {
                    self.contents.insert(id, MessageContent::default());
                }
            }
            Syscall::UiSetProperty { id, kind, u1, u2, u3, u4, i1, i2, f } => {
                self.ui.set_property(id, kind, [u1, u2, u3, u4], [i1, i2], f)?;
            }
            Syscall::UiSetPropertyWithString { id, kind, s, .. } => {
                let (images, requests) = (&mut self.images, &mut self.image_requests);
                self.ui.set_string_property(id, kind, s, &mut |path| register_image(images, requests, path))?;
            }
            Syscall::UiPushChild { parent, child } => self.ui.push_child(parent, child)?,
            Syscall::UiSetHandler { id, kind, handle } => self.ui.set_handler(id, kind, handle)?,
            Syscall::EnterScenePage { window } => self.scene = Some(self.ui.enter_scene_page(window)?),

            Syscall::Wait | Syscall::AwaitTransitions | Syscall::Sleep { .. } => {
                return Err(UiError(format!(
                    "{} blocks and must go through begin_call",
                    syscall.name()
                )));
            }
        }
        Ok(())
    }

    /// 止まる syscall を始める。
    pub fn begin_call(&mut self, syscall: &Syscall) -> CallStatus {
        let call = match *syscall {
            Syscall::Wait => PendingCall::Wait,
            Syscall::Sleep { ms } if ms > 0 => PendingCall::Sleep { remaining_ms: f64::from(ms) },
            Syscall::AwaitTransitions if self.canvas.has_sync() => PendingCall::AwaitTransitions,
            _ => return CallStatus::Done,
        };
        self.call = Some(call);
        self.call_finished = false;
        CallStatus::Pending
    }

    /// 保留中の止まる syscall が終わったか。終わっていれば 1 度だけ true を返す。
    pub fn take_finished_call(&mut self) -> bool {
        std::mem::take(&mut self.call_finished)
    }

    /// エンジン時計を進める。
    pub fn update(&mut self, delta_ms: f32) {
        let delta = f64::from(delta_ms);
        self.now += delta;
        for content in self.contents.values_mut() {
            content.update(delta_ms);
        }
        self.canvas.update(self.now);
        match &mut self.call {
            Some(PendingCall::Sleep { remaining_ms }) => {
                *remaining_ms -= delta;
                if *remaining_ms <= 0.0 {
                    self.finish_call();
                }
            }
            Some(PendingCall::AwaitTransitions) if !self.canvas.has_sync() => self.finish_call(),
            _ => {}
        }
    }

    /// 画面の (`x`, `y`) (論理ピクセル) が押された。
    ///
    /// 押せる Element (Link・SceneStartButton) に当たればそれを押す。当たらなければ「進む」([`Engine::advance`])。
    /// Link・SceneStartButton のクリックは進むには使われない (TypeScript 版でクリックを host に伝えないのと同じ)。
    pub fn click_at(&mut self, x: f32, y: f32) -> Result<Click, UiError> {
        let hit = self
            .targets
            .iter()
            .rev()
            .find(|(_, r)| x >= r.x && y >= r.y && x < r.x + r.width && y < r.y + r.height)
            .map(|(id, _)| *id);
        let Some(id) = hit else {
            self.advance();
            return Ok(Click::Handled);
        };
        match self.ui.click(id)? {
            Some(UiAction::StartScene(start)) => Ok(Click::StartScene(start)),
            Some(UiAction::Navigated) | None => Ok(Click::Handled),
        }
    }

    /// 「進む」(画面のクリック・Enter など)。
    ///
    /// 進行中のもの (文字送り・sync 印の演出) があれば、まずそれを畳む。テキストを進めるにはもう一度要る
    /// (`engine/nodejs/src/engine/api/message.ts` の `waitForClick`)。
    pub fn advance(&mut self) {
        let mut skipped = false;
        // 短絡させない。どの MessageArea の送りも、sync の演出も畳む。
        for content in self.contents.values_mut() {
            skipped |= content.skip();
        }
        skipped |= self.canvas.skip_sync(self.now);
        if skipped {
            // 飛ばした演出を終端の値にしておく (次のフレームを待たずに)。
            self.update(0.0);
            return;
        }
        if self.call == Some(PendingCall::Wait) {
            self.finish_call();
        }
    }

    /// 時間で動くものがあるか (無ければ描き直さずに眠ってよい)。
    pub fn is_animating(&self) -> bool {
        matches!(self.call, Some(PendingCall::Sleep { .. } | PendingCall::AwaitTransitions))
            || self.contents.values().any(MessageContent::is_revealing)
            || self.canvas.is_animating(self.now)
    }

    /// まだ渡していない画像の読み込みの依頼を取り出す。
    pub fn take_image_requests(&mut self) -> Vec<ImageRequest> {
        std::mem::take(&mut self.image_requests)
    }

    /// 手放してよくなった関数の番号 (vm に伝える)。
    pub fn take_released_handlers(&mut self) -> Vec<u32> {
        self.ui.take_released_handlers()
    }

    /// 画像が読めた。
    pub fn image_loaded(&mut self, image: ImageHandle, width: u32, height: u32) {
        self.image_sizes.insert(image, (width, height));
        self.canvas.image_loaded(image, width, height);
    }

    /// 最初の SceneStartButton で scene を始めるのに要るもの (ウィンドウを持たない動かし方で自動で押す)。
    pub fn first_scene_start(&self) -> Result<SceneStart, UiError> {
        self.ui.first_scene_start()
    }

    /// 今見えている Message Window に出し始めた文字列 (ウィンドウを持たない動かし方のログ用)。
    pub fn scene_text(&self) -> String {
        self.scene
            .and_then(|s| s.message_area)
            .and_then(|area| self.contents.get(&area))
            .map(|c| c.fragments().map(|f| f.text.as_str()).collect())
            .unwrap_or_default()
    }

    /// 1 フレームぶんの描画の指示を組み立てる。
    pub fn frame(&mut self, viewport: Viewport, shaper: &mut dyn TextShaper, layouter: &mut dyn Layouter) -> DrawList {
        self.ensure_layout(viewport, shaper, layouter);
        let mut list = DrawList { clear: Color::BLACK, items: Vec::new() };
        self.targets.clear();
        let Some(layout) = self.layout.take() else {
            return list;
        };
        self.draw_layout_node(&layout, layout.tree.root, viewport, shaper, &mut list.items);
        self.layout = Some(layout);
        list
    }

    fn ensure_layout(&mut self, viewport: Viewport, shaper: &mut dyn TextShaper, layouter: &mut dyn Layouter) {
        // 文字は描くときの大きさ (物理ピクセル) で測るので、拡大率も鍵に入れる。
        let key = (viewport.width.to_bits(), viewport.height.to_bits(), viewport.scale.to_bits());
        if self.layout.as_ref().is_some_and(|l| l.generation == self.ui.generation() && l.viewport == key) {
            return;
        }
        let (tree, slots) = self.ui.layout_tree(viewport);
        let boxes = layouter.layout(&tree, viewport, shaper);
        self.layout = Some(LayoutCache { generation: self.ui.generation(), viewport: key, tree, slots, boxes });
    }

    fn draw_layout_node(
        &mut self,
        layout: &LayoutCache,
        index: usize,
        viewport: Viewport,
        shaper: &mut dyn TextShaper,
        out: &mut Vec<DrawItem>,
    ) {
        let node = &layout.tree.nodes[index];
        if node.style.hidden {
            return;
        }
        let s = viewport.scale;
        let b = layout.boxes.get(index).copied().unwrap_or_default();
        match layout.slots[index] {
            LayoutSlot::Root => {}
            LayoutSlot::Element(id) => {
                let Some(view) = self.ui.node_view(id) else { return };
                let frame = scaled(b.frame, s);
                match view.background {
                    Some(Background::Color(color)) => out.push(DrawItem::Rect { rect: frame, color }),
                    Some(Background::Image(image)) => self.draw_cover(image, frame, out),
                    None => {}
                }
                match view.kind {
                    ElementKind::Image => {
                        if let Some(image) = view.image {
                            self.draw_cover(image, frame, out);
                        }
                    }
                    ElementKind::Canvas => {
                        // canvas の外にはみ出した分は描かない。
                        out.push(DrawItem::Clip(Some(frame)));
                        self.canvas.draw(id, frame, out);
                        out.push(DrawItem::Clip(None));
                    }
                    ElementKind::MessageArea => self.draw_message_area(id, scaled(b.content, s), viewport, shaper, out),
                    _ => {}
                }
                if view.clickable {
                    self.targets.push((id, b.frame));
                }
            }
            LayoutSlot::Text(id) => {
                if let Some(text) = &node.text {
                    self.draw_text(id, text, scaled(b.content, s), s, shaper, out);
                }
            }
        }
        for child in &node.children {
            self.draw_layout_node(layout, *child, viewport, shaper, out);
        }
    }

    /// 画像を `area` いっぱいに、縦横比を保って敷く (`background-size: cover; background-position: center`)。
    fn draw_cover(&self, image: ImageHandle, area: Rect, out: &mut Vec<DrawItem>) {
        let Some(&(w, h)) = self.image_sizes.get(&image) else { return };
        if w == 0 || h == 0 || area.width <= 0.0 || area.height <= 0.0 {
            return;
        }
        let scale = (area.width / w as f32).max(area.height / h as f32);
        out.push(DrawItem::Clip(Some(area)));
        out.push(DrawItem::Image {
            image,
            center_x: area.x + area.width / 2.0,
            center_y: area.y + area.height / 2.0,
            width: w as f32 * scale,
            height: h as f32 * scale,
            rotation: 0.0,
            alpha: 1.0,
        });
        out.push(DrawItem::Clip(None));
    }

    /// UI の文字 (Box・Link の `text`) を `area` (物理ピクセル) に描く。
    fn draw_text(
        &mut self,
        id: UiId,
        text: &crate::layout::LayoutText,
        area: Rect,
        scale: f32,
        shaper: &mut dyn TextShaper,
        out: &mut Vec<DrawItem>,
    ) {
        let generation = self.ui.generation();
        let fresh = self.text_shapes.get(&id).is_some_and(|c| {
            c.generation == generation && c.width == area.width.to_bits() && c.scale == scale.to_bits()
        });
        if !fresh {
            let spans: Vec<TextSpan> = text
                .spans
                .iter()
                .map(|span| TextSpan {
                    text: span.text.clone(),
                    style: TextStyle { size: span.style.size * scale, ..span.style.clone() },
                })
                .collect();
            // レイアウトが測った幅で折り返す。レイアウトは位置と大きさを論理ピクセルの整数に丸めるので、
            // 箱が測った幅より最大 1 論理ピクセル狭くなりうる。そのぶん余計に折り返さないよう足す。
            let slack = scale.max(1.0);
            let shaped = shaper.shape(
                &spans,
                ParagraphStyle { max_width: area.width + slack, line_height: text.line_height, center: text.center },
            );
            self.text_shapes.insert(
                id,
                ShapeCache { generation, width: area.width.to_bits(), scale: scale.to_bits(), shaped },
            );
        }
        // 中央寄せは折り返しの幅 (`area.width + slack`) の中で行われるので、足した分の半分だけ戻す。
        let shift = if text.center { -scale.max(1.0) / 2.0 } else { 0.0 };
        for glyph in &self.text_shapes[&id].shaped.glyphs {
            out.push(DrawItem::Glyph {
                key: glyph.key,
                x: area.x + glyph.x + shift,
                y: area.y + glyph.y,
                color: glyph.color,
            });
        }
    }

    /// Message Window を `area` (MessageArea の padding の内側。物理ピクセル) に描く。
    fn draw_message_area(
        &mut self,
        id: UiId,
        area: Rect,
        viewport: Viewport,
        shaper: &mut dyn TextShaper,
        out: &mut Vec<DrawItem>,
    ) {
        let Some(content) = self.contents.get(&id) else { return };
        let s = viewport.scale;
        let fresh = self.message_shapes.get(&id).is_some_and(|c| {
            c.generation == content.generation() && c.width == area.width.to_bits() && c.scale == s.to_bits()
        });
        if !fresh {
            let spans: Vec<TextSpan> = content
                .fragments()
                .map(|f| TextSpan {
                    text: f.text.clone(),
                    style: TextStyle { size: font_size(f, viewport) * s, weight: f.weight, color: f.color, family: None },
                })
                .collect();
            let shaped =
                shaper.shape(&spans, ParagraphStyle { max_width: area.width, line_height: LINE_HEIGHT, center: false });
            self.message_shapes.insert(
                id,
                ShapeCache { generation: content.generation(), width: area.width.to_bits(), scale: s.to_bits(), shaped },
            );
        }
        for glyph in &self.message_shapes[&id].shaped.glyphs {
            // まだ出ていない文字は描かない (場所は整形で取ってあるので行は動かない)。
            if glyph.char_index >= content.revealed(glyph.span) {
                continue;
            }
            out.push(DrawItem::Glyph {
                key: glyph.key,
                x: area.x + glyph.x,
                y: area.y + glyph.y,
                color: glyph.color,
            });
        }
    }

    fn content_mut(&mut self, ui: UiId) -> Result<&mut MessageContent, UiError> {
        self.contents
            .get_mut(&ui)
            .ok_or_else(|| UiError(format!("ui element {ui} is not a MessageArea")))
    }

    fn finish_call(&mut self) {
        self.call = None;
        self.call_finished = true;
    }
}

/// パスの画像の番号。初めてのパスなら番号を振って読み込みを依頼する。
fn register_image(
    images: &mut HashMap<String, ImageHandle>,
    requests: &mut Vec<ImageRequest>,
    path: &str,
) -> Result<ImageHandle, String> {
    let path = normalize_asset_path(path)?;
    if let Some(image) = images.get(&path) {
        return Ok(*image);
    }
    let image = ImageHandle(images.len() as u32 + 1);
    images.insert(path.clone(), image);
    requests.push(ImageRequest { image, path });
    Ok(image)
}

fn font_size(fragment: &Fragment, viewport: Viewport) -> f32 {
    let base = match fragment.size_unit {
        SizeUnit::Vw => viewport.width,
        SizeUnit::Vh => viewport.height,
    };
    base * fragment.size_value / 100.0
}

fn scaled(rect: Rect, scale: f32) -> Rect {
    Rect::new(rect.x * scale, rect.y * scale, rect.width * scale, rect.height * scale)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::text::{GlyphKey, ShapedGlyph};

    /// 1 文字 1 グリフ、等幅で並べるだけの整形。
    struct Mono;

    impl TextShaper for Mono {
        fn shape(&mut self, spans: &[TextSpan], _: ParagraphStyle) -> ShapedText {
            let mut glyphs = Vec::new();
            for (span, s) in spans.iter().enumerate() {
                for (i, _) in s.text.chars().enumerate() {
                    glyphs.push(ShapedGlyph {
                        key: GlyphKey { font: 0, glyph: i as u32, size: 0, weight: 400 },
                        x: glyphs.len() as f32 * 10.0,
                        y: 10.0,
                        color: s.style.color,
                        span,
                        char_index: i,
                    });
                }
            }
            ShapedText { glyphs, width: 100.0, height: 20.0 }
        }
    }

    /// どのノードも画面全体に置くだけのレイアウト。
    struct Full;

    impl Layouter for Full {
        fn layout(&mut self, tree: &LayoutTree, viewport: Viewport, _: &mut dyn TextShaper) -> Vec<LayoutBox> {
            let r = Rect::new(0.0, 0.0, viewport.width, viewport.height);
            vec![LayoutBox { frame: r, content: r }; tree.nodes.len()]
        }
    }

    fn engine_with_scene() -> Engine {
        let mut e = Engine::new();
        let calls = [
            Syscall::UiCreate { id: 1, kind: 0 },
            Syscall::UiCreate { id: 2, kind: 11 },
            Syscall::UiCreate { id: 3, kind: 8 },
            Syscall::UiCreate { id: 4, kind: 9 },
            Syscall::UiPushChild { parent: 2, child: 3 },
            Syscall::UiPushChild { parent: 2, child: 4 },
            Syscall::UiSetProperty { id: 2, kind: 16, u1: 3, u2: 0, u3: 0, u4: 0, i1: 0, i2: 0, f: 0.0 },
            Syscall::UiSetProperty { id: 2, kind: 17, u1: 4, u2: 0, u3: 0, u4: 0, i1: 0, i2: 0, f: 0.0 },
            Syscall::UiSetProperty { id: 1, kind: 15, u1: 2, u2: 0, u3: 0, u4: 0, i1: 0, i2: 0, f: 0.0 },
            Syscall::EnterScenePage { window: 1 },
        ];
        for c in calls {
            e.apply(c).unwrap();
        }
        e
    }

    fn push(e: &mut Engine, text: &str, speed: f32) {
        e.apply(Syscall::ContentPushText {
            ui: 4,
            text: text.into(),
            speed,
            size_unit: 0,
            size_value: 2.0,
            weight: 400,
            r: 255,
            g: 255,
            b: 255,
            a: 255,
        })
        .unwrap();
    }

    fn glyph_count(e: &mut Engine) -> usize {
        let list = e.frame(Viewport::new(1280.0, 720.0, 1.0), &mut Mono, &mut Full);
        list.items.iter().filter(|i| matches!(i, DrawItem::Glyph { .. })).count()
    }

    #[test]
    fn reveals_glyphs_over_time() {
        let mut e = engine_with_scene();
        push(&mut e, "abcd", 10.0);
        e.apply(Syscall::ContentFlush { ui: 4 }).unwrap();
        assert_eq!(glyph_count(&mut e), 0);
        e.update(250.0);
        assert_eq!(glyph_count(&mut e), 2);
        assert!(e.is_animating());
    }

    #[test]
    fn draws_message_area_background_behind_text() {
        let mut e = engine_with_scene();
        e.apply(Syscall::UiSetProperty { id: 4, kind: 10, u1: 0, u2: 0, u3: 0, u4: 191, i1: 0, i2: 0, f: 0.0 })
            .unwrap();
        push(&mut e, "ab", 0.0);
        e.apply(Syscall::ContentFlush { ui: 4 }).unwrap();
        e.update(0.0);
        let list = e.frame(Viewport::new(1280.0, 720.0, 2.0), &mut Mono, &mut Full);
        // canvas (切り取りの範囲だけ) の上に、背景が先 (下)、文字が後 (上)。HiDPI では物理ピクセルに直っている。
        let full = Rect::new(0.0, 0.0, 2560.0, 1440.0);
        assert_eq!(list.items[0], DrawItem::Clip(Some(full)));
        assert_eq!(list.items[1], DrawItem::Clip(None));
        assert_eq!(list.items[2], DrawItem::Rect { rect: full, color: Color::rgba(0, 0, 0, 191) });
        assert!(matches!(list.items[3], DrawItem::Glyph { .. }));
    }

    #[test]
    fn click_skips_then_finishes_wait() {
        let mut e = engine_with_scene();
        push(&mut e, "abcd", 1.0);
        e.apply(Syscall::ContentFlush { ui: 4 }).unwrap();
        assert_eq!(e.begin_call(&Syscall::Wait), CallStatus::Pending);

        e.advance(); // 1 回目は送りを畳むだけ
        assert!(!e.take_finished_call());
        assert_eq!(glyph_count(&mut e), 4);

        e.advance(); // 2 回目で進む
        assert!(e.take_finished_call());
        assert!(!e.take_finished_call());
    }

    #[test]
    fn sleep_finishes_on_engine_clock() {
        let mut e = Engine::new();
        assert_eq!(e.begin_call(&Syscall::Sleep { ms: 100 }), CallStatus::Pending);
        e.update(60.0);
        assert!(!e.take_finished_call());
        e.update(60.0);
        assert!(e.take_finished_call());
    }

    fn create_object(e: &mut Engine, id: u32, path: &str) {
        e.apply(Syscall::CreateObject {
            id,
            canvas: 3,
            path: path.into(),
            layer: 0,
            x: 0.0,
            y: 0.0,
            width: 10.0,
            height: 10.0,
            alpha: 255,
            theta: 0.0,
        })
        .unwrap();
    }

    #[test]
    fn requests_each_image_once_and_draws_after_load() {
        let mut e = engine_with_scene();
        create_object(&mut e, 10, "a.png");
        create_object(&mut e, 11, "./a.png");
        let requests = e.take_image_requests();
        assert_eq!(requests.len(), 1);
        assert_eq!(requests[0].path, "a.png");
        let images = |e: &mut Engine| {
            e.frame(Viewport::new(100.0, 100.0, 1.0), &mut Mono, &mut Full)
                .items
                .iter()
                .filter(|i| matches!(i, DrawItem::Image { .. }))
                .count()
        };
        assert_eq!(images(&mut e), 0);
        e.image_loaded(requests[0].image, 4, 4);
        assert_eq!(images(&mut e), 2);
    }

    #[test]
    fn rejects_escaping_asset_paths_and_non_canvas_targets() {
        let mut e = engine_with_scene();
        let mut bad = |canvas: u32, path: &str| {
            e.apply(Syscall::CreateObject {
                id: 99,
                canvas,
                path: path.into(),
                layer: 0,
                x: 0.0,
                y: 0.0,
                width: 1.0,
                height: 1.0,
                alpha: 255,
                theta: 0.0,
            })
            .is_err()
        };
        assert!(bad(3, "../x.png"));
        assert!(bad(4, "x.png"));
    }

    #[test]
    fn await_transitions_waits_for_sync_and_click_skips_it() {
        let mut e = engine_with_scene();
        create_object(&mut e, 10, "a.png");
        e.apply(Syscall::AddTransition { id: 10, param: 0, kind: 1, val_i: 0, val_f: 20.0, after: 0, duration: 1000 })
            .unwrap();
        e.apply(Syscall::StartTransitions { sync: 1 }).unwrap();
        assert_eq!(e.begin_call(&Syscall::AwaitTransitions), CallStatus::Pending);
        e.update(500.0);
        assert!(!e.take_finished_call());
        e.update(600.0);
        assert!(e.take_finished_call());

        // 演出の途中のクリック待ち: 1 回目は演出を畳むだけ、2 回目で進む。
        e.apply(Syscall::AddTransition { id: 10, param: 0, kind: 1, val_i: 0, val_f: 0.0, after: 0, duration: 1000 })
            .unwrap();
        e.apply(Syscall::StartTransitions { sync: 1 }).unwrap();
        assert_eq!(e.begin_call(&Syscall::Wait), CallStatus::Pending);
        e.advance();
        assert!(!e.take_finished_call());
        assert!(!e.is_animating());
        e.advance();
        assert!(e.take_finished_call());
    }

    #[test]
    fn await_without_sync_returns_at_once() {
        let mut e = engine_with_scene();
        assert_eq!(e.begin_call(&Syscall::AwaitTransitions), CallStatus::Done);
    }

    /// 画面の右半分を SceneStartButton、ほかを画面全体に置くレイアウト。
    struct ButtonOnRight;

    impl Layouter for ButtonOnRight {
        fn layout(&mut self, tree: &LayoutTree, viewport: Viewport, _: &mut dyn TextShaper) -> Vec<LayoutBox> {
            let full = Rect::new(0.0, 0.0, viewport.width, viewport.height);
            let right = Rect::new(viewport.width / 2.0, 0.0, viewport.width / 2.0, viewport.height);
            (0..tree.nodes.len())
                .map(|i| {
                    let r = if i == tree.nodes.len() - 1 { right } else { full };
                    LayoutBox { frame: r, content: r }
                })
                .collect()
        }
    }

    #[test]
    fn clicks_hit_buttons_or_advance() {
        let mut e = engine_with_scene();
        // Window 1 に Page 5 と SceneStartButton 6 を足す (ScenePage は scene に入るまで隠れる)。
        let calls = [
            Syscall::UiCreate { id: 5, kind: 1 },
            Syscall::UiCreate { id: 6, kind: 12 },
            Syscall::UiSetHandler { id: 1, kind: 0, handle: 10 },
            Syscall::UiSetHandler { id: 6, kind: 1, handle: 11 },
            Syscall::UiPushChild { parent: 5, child: 6 },
            Syscall::UiPushChild { parent: 1, child: 5 },
        ];
        for c in calls {
            e.apply(c).unwrap();
        }
        e.frame(Viewport::new(100.0, 100.0, 1.0), &mut Mono, &mut ButtonOnRight);
        let Click::StartScene(start) = e.click_at(75.0, 50.0).unwrap() else { panic!("the button was not hit") };
        assert_eq!((start.on_click, start.main_scene, start.message_area), (11, 10, 4));

        // ボタンの外のクリックは「進む」(クリック待ちを終わらせる)。
        assert_eq!(e.begin_call(&Syscall::Wait), CallStatus::Pending);
        assert_eq!(e.click_at(25.0, 50.0).unwrap(), Click::Handled);
        assert!(e.take_finished_call());
    }

    #[test]
    fn content_to_non_message_area_is_an_error() {
        let mut e = engine_with_scene();
        assert!(e.apply(Syscall::ContentFlush { ui: 3 }).is_err());
    }
}
