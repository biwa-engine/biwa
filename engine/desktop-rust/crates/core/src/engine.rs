//! エンジン本体。メインスレッドで動き、ゲーム (vm) からの syscall を受けて状態を変え、
//! 1 フレームぶんの描画の指示 ([`DrawList`]) を組み立てる。
//!
//! ゲームを止める syscall (クリック待ち・sleep・演出待ち) は、エンジンが「まだ」と言っている間
//! 呼んだ側 (player) が返事を保留する。エンジンは返事の手段 (チャネル) を知らない。
//! 画像も同じで、エンジンは番号を振って読み込みを依頼するだけである ([`Engine::take_image_requests`])。
//!
//! 段 4 までの範囲:
//! - Content API (Message Window と文字送り) と Canvas API (オブジェクトと遷移) は本物。
//! - UI は scene を始めるのに要る分と、scene の出力先 (Canvas・MessageArea) の箱だけ (`ui.rs`)。

use std::collections::HashMap;

use crate::canvas::Canvas;
use crate::content::{Fragment, MessageContent, SizeUnit};
use crate::draw::{DrawItem, DrawList};
use crate::geometry::{Color, Rect, Viewport};
use crate::image::{ImageHandle, ImageRequest, normalize_asset_path};
use crate::syscall::{SceneStart, Syscall, UiId};
use crate::text::{ParagraphStyle, ShapedText, TextShaper, TextSpan, TextStyle};
use crate::ui::{ElementKind, SceneOutputs, UiError, UiTree};

/// 文字の行の高さ (文字の大きさに対する倍率)。TypeScript 版の `line-height: 1.8` と揃える。
const LINE_HEIGHT: f32 = 1.8;

/// 止まる syscall の進み具合。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CallStatus {
    /// もう終わった。すぐに返事をしてよい。
    Done,
    /// まだ。終わったら [`Engine::take_finished_call`] が true を返す。
    Pending,
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
    width: f32,
    viewport: (u32, u32, u32),
    shaped: ShapedText,
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
    shapes: HashMap<UiId, ShapeCache>,
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
                let path = normalize_asset_path(&path).map_err(UiError)?;
                let image = self.image_handle(path);
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
            // 文字列の property (Text・画像・id ...) は段 5 で扱う。
            Syscall::UiSetPropertyWithString { .. } => {}
            Syscall::UiPushChild { parent, child } => self.ui.push_child(parent, child)?,
            Syscall::UiSetHandler { id, kind, handle } => self.ui.set_handler(id, kind, handle)?,
            Syscall::EnterScenePage { window } => self.enter_scene_page(window),

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

    /// 画面がクリックされた。
    ///
    /// 進行中のもの (文字送り・sync 印の演出) があれば、まずそれを畳む。テキストを進めるにはもう一度クリックが要る
    /// (`engine/nodejs/src/engine/api/message.ts` の `waitForClick`)。
    pub fn click(&mut self) {
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

    /// 画像が読めた。
    pub fn image_loaded(&mut self, image: ImageHandle, width: u32, height: u32) {
        self.image_sizes.insert(image, (width, height));
        self.canvas.image_loaded(image, width, height);
    }

    /// 最初の SceneStartButton で scene を始めるのに要るもの (段 5 までは UI を描かないので自動で押す)。
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
    ///
    /// scene の Canvas を描いてから Message Window を重ねる (test1 などの `Layers` の push 順。
    /// 本物の重なりの順は UI のレイアウトと一緒に段 5 で決める)。
    pub fn frame(&mut self, viewport: Viewport, shaper: &mut dyn TextShaper) -> DrawList {
        let mut list = DrawList { clear: Color::BLACK, items: Vec::new() };
        let Some(scene) = self.scene else {
            return list;
        };
        let s = viewport.scale;

        if let Some(canvas) = scene.canvas
            && let Some(canvas_box) = self.ui.element_box(canvas, viewport)
        {
            let area = scaled(canvas_box.frame, s);
            if let Some(color) = canvas_box.background {
                list.items.push(DrawItem::Rect { rect: area, color });
            }
            // canvas の外にはみ出した分は描かない。
            list.items.push(DrawItem::Clip(Some(area)));
            self.canvas.draw(canvas, area, &mut list.items);
            list.items.push(DrawItem::Clip(None));
        }

        if let Some(area) = scene.message_area {
            self.draw_message_area(area, viewport, shaper, &mut list.items);
        }
        list
    }

    fn draw_message_area(
        &mut self,
        area: UiId,
        viewport: Viewport,
        shaper: &mut dyn TextShaper,
        out: &mut Vec<DrawItem>,
    ) {
        let Some(area_box) = self.ui.element_box(area, viewport) else {
            return;
        };
        let s = viewport.scale;
        if let Some(color) = area_box.background {
            out.push(DrawItem::Rect { rect: scaled(area_box.frame, s), color });
        }

        let Some(content) = self.contents.get(&area) else {
            return;
        };
        let content_rect = scaled(area_box.content, s);
        let key = (
            viewport.width.to_bits(),
            viewport.height.to_bits(),
            viewport.scale.to_bits(),
        );
        let fresh = self.shapes.get(&area).is_some_and(|c| {
            c.generation == content.generation() && c.width == content_rect.width && c.viewport == key
        });
        if !fresh {
            let spans: Vec<TextSpan> = content
                .fragments()
                .map(|f| TextSpan {
                    text: f.text.clone(),
                    style: TextStyle { size: font_size(f, viewport) * s, weight: f.weight, color: f.color },
                })
                .collect();
            let shaped = shaper.shape(
                &spans,
                ParagraphStyle { max_width: content_rect.width, line_height: LINE_HEIGHT },
            );
            self.shapes.insert(
                area,
                ShapeCache { generation: content.generation(), width: content_rect.width, viewport: key, shaped },
            );
        }

        for glyph in &self.shapes[&area].shaped.glyphs {
            // まだ出ていない文字は描かない (場所は整形で取ってあるので行は動かない)。
            if glyph.char_index >= content.revealed(glyph.span) {
                continue;
            }
            out.push(DrawItem::Glyph {
                key: glyph.key,
                x: content_rect.x + glyph.x,
                y: content_rect.y + glyph.y,
                color: glyph.color,
            });
        }
    }

    fn content_mut(&mut self, ui: UiId) -> Result<&mut MessageContent, UiError> {
        self.contents
            .get_mut(&ui)
            .ok_or_else(|| UiError(format!("ui element {ui} is not a MessageArea")))
    }

    /// パスの画像の番号。初めてのパスなら番号を振って読み込みを依頼する。
    fn image_handle(&mut self, path: String) -> ImageHandle {
        if let Some(image) = self.images.get(&path) {
            return *image;
        }
        let image = ImageHandle(self.images.len() as u32 + 1);
        self.images.insert(path.clone(), image);
        self.image_requests.push(ImageRequest { image, path });
        image
    }

    fn enter_scene_page(&mut self, window: UiId) {
        match self.ui.scene_outputs_of_window(window) {
            Some(outputs) => self.scene = Some(outputs),
            None => eprintln!("[biwa] Window {window} has no ScenePage to enter"),
        }
    }

    fn finish_call(&mut self) {
        self.call = None;
        self.call_finished = true;
    }
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
            ShapedText { glyphs, height: 20.0 }
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
        let list = e.frame(Viewport::new(1280.0, 720.0, 1.0), &mut Mono);
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
        let list = e.frame(Viewport::new(1280.0, 720.0, 2.0), &mut Mono);
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

        e.click(); // 1 回目は送りを畳むだけ
        assert!(!e.take_finished_call());
        assert_eq!(glyph_count(&mut e), 4);

        e.click(); // 2 回目で進む
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
            e.frame(Viewport::new(100.0, 100.0, 1.0), &mut Mono)
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
        e.click();
        assert!(!e.take_finished_call());
        assert!(!e.is_animating());
        e.click();
        assert!(e.take_finished_call());
    }

    #[test]
    fn await_without_sync_returns_at_once() {
        let mut e = engine_with_scene();
        assert_eq!(e.begin_call(&Syscall::AwaitTransitions), CallStatus::Done);
    }

    #[test]
    fn content_to_non_message_area_is_an_error() {
        let mut e = engine_with_scene();
        assert!(e.apply(Syscall::ContentFlush { ui: 3 }).is_err());
    }
}
