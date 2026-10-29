//! エンジン本体。メインスレッドで動き、ゲーム (vm) からの syscall を受けて状態を変え、
//! 1 フレームぶんの描画の指示 ([`DrawList`]) を組み立てる。
//!
//! ゲームを止める syscall (クリック待ち・sleep・演出待ち) は、エンジンが「まだ」と言っている間
//! 呼んだ側 (player) が返事を保留する。エンジンは返事の手段 (チャネル) を知らない。
//!
//! 段 3 までの範囲:
//! - Content API (Message Window と文字送り) は本物。
//! - Canvas API は受け取ってログに残すだけ (描画は段 4)。演出待ちはすぐ返る。
//! - UI は scene を始めるのに要る分と MessageArea の箱だけ (`ui.rs`)。

use std::collections::HashMap;

use crate::content::{Fragment, MessageContent, SizeUnit};
use crate::draw::{DrawItem, DrawList};
use crate::geometry::{Color, Rect, Viewport};
use crate::syscall::{SceneStart, Syscall, UiId};
use crate::text::{ParagraphStyle, ShapedText, TextShaper, TextSpan, TextStyle};
use crate::ui::{ElementKind, UiError, UiTree};

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
    Sleep { remaining_ms: f32 },
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
    call: Option<PendingCall>,
    call_finished: bool,
    /// 今見えている ScenePage。scene が始まると決まる。
    scene_page: Option<UiId>,
    /// 今見えている ScenePage の出力先の MessageArea。
    scene_message_area: Option<UiId>,
    shapes: HashMap<UiId, ShapeCache>,
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

            // canvas は段 4。受け取るだけ。
            Syscall::CreateObject { .. }
            | Syscall::DeleteObject { .. }
            | Syscall::AddTransition { .. }
            | Syscall::StartTransitions { .. } => {}

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
            Syscall::Sleep { ms } if ms > 0 => PendingCall::Sleep { remaining_ms: ms as f32 },
            // 演出 (canvas) は段 4 なので、待つものが無い。
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
        for content in self.contents.values_mut() {
            content.update(delta_ms);
        }
        if let Some(PendingCall::Sleep { remaining_ms }) = &mut self.call {
            *remaining_ms -= delta_ms;
            if *remaining_ms <= 0.0 {
                self.finish_call();
            }
        }
    }

    /// 画面がクリックされた。
    ///
    /// 進行中のもの (文字送り) があれば、まずそれを畳む。テキストを進めるにはもう一度クリックが要る
    /// (`engine/nodejs/src/engine/api/message.ts` の `waitForClick`)。
    pub fn click(&mut self) {
        let mut skipped = false;
        // 短絡させない。どの MessageArea の送りも畳む。
        for content in self.contents.values_mut() {
            skipped |= content.skip();
        }
        if skipped {
            return;
        }
        if self.call == Some(PendingCall::Wait) {
            self.finish_call();
        }
    }

    /// 時間で動くものがあるか (無ければ描き直さずに眠ってよい)。
    pub fn is_animating(&self) -> bool {
        matches!(self.call, Some(PendingCall::Sleep { .. }))
            || self.contents.values().any(MessageContent::is_revealing)
    }

    /// 最初の SceneStartButton で scene を始めるのに要るもの (段 3 までは UI を描かないので自動で押す)。
    pub fn first_scene_start(&self) -> Result<SceneStart, UiError> {
        self.ui.first_scene_start()
    }

    /// 今見えている Message Window に出し始めた文字列 (ウィンドウを持たない動かし方のログ用)。
    pub fn scene_text(&self) -> String {
        self.scene_message_area
            .and_then(|area| self.contents.get(&area))
            .map(|c| c.fragments().map(|f| f.text.as_str()).collect())
            .unwrap_or_default()
    }

    /// 1 フレームぶんの描画の指示を組み立てる。
    pub fn frame(&mut self, viewport: Viewport, shaper: &mut dyn TextShaper) -> DrawList {
        let mut list = DrawList { clear: Color::BLACK, items: Vec::new() };
        let Some(area) = self.scene_message_area else {
            return list;
        };
        let Some(area_box) = self.ui.message_area_box(area, viewport) else {
            return list;
        };
        let s = viewport.scale;

        if let Some(color) = area_box.background {
            list.items.push(DrawItem::Rect { rect: scaled(area_box.frame, s), color });
        }

        let Some(content) = self.contents.get(&area) else {
            return list;
        };
        let content_rect = scaled(area_box.content, s);
        let key = (
            viewport.width.to_bits(),
            viewport.height.to_bits(),
            viewport.scale.to_bits(),
        );
        let cache = self.shapes.get(&area);
        let fresh = cache.is_some_and(|c| {
            c.generation == content.generation()
                && c.width == content_rect.width
                && c.viewport == key
        });
        if !fresh {
            let spans: Vec<TextSpan> = content
                .fragments()
                .map(|f| TextSpan {
                    text: f.text.clone(),
                    style: TextStyle {
                        size: font_size(f, viewport) * s,
                        weight: f.weight,
                        color: f.color,
                    },
                })
                .collect();
            let shaped = shaper.shape(
                &spans,
                ParagraphStyle { max_width: content_rect.width, line_height: LINE_HEIGHT },
            );
            self.shapes.insert(
                area,
                ShapeCache {
                    generation: content.generation(),
                    width: content_rect.width,
                    viewport: key,
                    shaped,
                },
            );
        }

        let shaped = &self.shapes[&area].shaped;
        for glyph in &shaped.glyphs {
            // まだ出ていない文字は描かない (場所は整形で取ってあるので行は動かない)。
            if glyph.char_index >= content.revealed(glyph.span) {
                continue;
            }
            list.items.push(DrawItem::Glyph {
                key: glyph.key,
                x: content_rect.x + glyph.x,
                y: content_rect.y + glyph.y,
                color: glyph.color,
            });
        }
        list
    }

    fn content_mut(&mut self, ui: UiId) -> Result<&mut MessageContent, UiError> {
        self.contents
            .get_mut(&ui)
            .ok_or_else(|| UiError(format!("ui element {ui} is not a MessageArea")))
    }

    fn enter_scene_page(&mut self, window: UiId) {
        match self.ui.scene_outputs_of_window(window) {
            Some((page, message_area)) => {
                self.scene_page = Some(page);
                self.scene_message_area = Some(message_area);
            }
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

    #[test]
    fn content_to_non_message_area_is_an_error() {
        let mut e = engine_with_scene();
        assert!(e.apply(Syscall::ContentFlush { ui: 3 }).is_err());
    }
}
