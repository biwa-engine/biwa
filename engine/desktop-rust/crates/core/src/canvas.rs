//! canvas オブジェクトと遷移 (`engine/nodejs/src/engine/canvas/CanvasObjects.ts` の移植)。
//!
//! 規則は TypeScript 版と同じである (`docs/media-object-model.md`)。
//!
//! - biwa 側のパラメータ (`value` / `deviation`) を正とし、描くときに毎回 px に射影する。
//!   x / y / w / h は canvas の範囲が -50..50 の単位で持つので、canvas の大きさが変われば追従する。
//! - `add_transition` は積むだけ。`start_transitions` で、積まれた区間がある `(オブジェクト, パラメータ)` だけが置き換わる
//!   (触れていないパラメータは走り続ける)。
//! - 次の区間が始まると前の区間はそこで終わり、値を引き継ぐ。周期系は落ち着いた値を動かさず偏差だけを乗せる。
//! - sync 印の発火はテキストの進行と結びつく。クリックはまずそれを終端まで飛ばし、`sys_await_transitions` はそれを待つ。
//!   飛ばすのは時計ではなく区間の開始時刻 (独立に走っている演出を巻き込まない)。
//! - 大きさを指定しない (負の) `w` / `h` は、画像の元の大きさと canvas の大きさから 1 度だけ決める。
//!
//! 区間は番号で持つ表 ([`Canvas::segments`]) に置き、走っている列と sync の束はその番号で指す
//! (TypeScript 版では同じオブジェクトを両方から参照していた)。

use std::collections::{BTreeMap, HashMap, HashSet};

use crate::draw::DrawItem;
use crate::geometry::Rect;
use crate::image::ImageHandle;
use crate::syscall::UiId;
use crate::transition::{Curve, PARAM_COUNT, Param};

type SegmentId = u64;

/// 遷移の 1 区間 (`add_transition` 1 回)。
#[derive(Debug, Clone)]
struct Segment {
    owner: UiId,
    param: usize,
    kind: Curve,
    /// 一度限りの遷移では目標値、周期系では振幅。
    value: f64,
    /// `start_transitions` からの遅れ [ms]。
    after: f64,
    /// 一度限りの遷移では遷移時間、周期系では周期 [ms]。
    duration: f64,
    /// エンジン時計の上での開始時刻。発火したときに決まる。
    start_at: f64,
    /// 活性になった瞬間に捉えた基準値。
    baseline: f64,
}

impl Segment {
    /// 終わる時刻。周期系は終わらない。
    fn end(&self) -> f64 {
        if self.kind.is_periodic() {
            f64::INFINITY
        } else if self.kind == Curve::None {
            self.start_at
        } else {
            self.start_at + self.duration
        }
    }
}

#[derive(Debug)]
struct CanvasObject {
    canvas: UiId,
    image: ImageHandle,
    layer: i32,
    /// 作られた順。同じレイヤーの中の前後になる。
    order: u64,
    /// 画像の元の大きさ [px]。読み込みが終わるまで `None` (その間は描かない)。
    natural: Option<(u32, u32)>,
    /// 負の w / h を決め終えたか。
    auto_size_resolved: bool,
    /// パラメータごとの落ち着いた値 (周期系の偏差を含まない)。
    value: [f64; PARAM_COUNT],
    /// パラメータごとの、周期系が乗せている偏差。
    deviation: [f64; PARAM_COUNT],
    /// パラメータごとの、走っている区間の列。
    active: [Vec<SegmentId>; PARAM_COUNT],
    /// パラメータごとの、次の `start_transitions` を待つ区間。
    pending: [Vec<Segment>; PARAM_COUNT],
    /// `active` の中で今活性な区間の位置。`None` はまだどれも始まっていない。
    cursor: [Option<usize>; PARAM_COUNT],
    /// 消える予定の時刻。
    delete_at: Option<f64>,
}

impl CanvasObject {
    fn at(&self, param: Param) -> f64 {
        self.value[param as usize] + self.deviation[param as usize]
    }
}

/// canvas オブジェクトの全体。時刻はエンジン時計 [ms] を外から受け取る。
#[derive(Debug, Default)]
pub struct Canvas {
    objects: BTreeMap<UiId, CanvasObject>,
    segments: HashMap<SegmentId, Segment>,
    next_segment: SegmentId,
    next_order: u64,
    /// 完了していない sync 印の束 (区間の番号の列)。
    sync_batches: Vec<Vec<SegmentId>>,
}

impl Canvas {
    /// オブジェクトを置く。画像の読み込みは呼んだ側が依頼する。
    #[allow(clippy::too_many_arguments)]
    pub fn create(
        &mut self,
        id: UiId,
        canvas: UiId,
        image: ImageHandle,
        layer: i32,
        values: [f64; PARAM_COUNT],
        natural: Option<(u32, u32)>,
    ) -> Result<(), String> {
        if self.objects.contains_key(&id) {
            return Err(format!("canvas object {id} already exists"));
        }
        let order = self.next_order;
        self.next_order += 1;
        self.objects.insert(
            id,
            CanvasObject {
                canvas,
                image,
                layer,
                order,
                natural,
                auto_size_resolved: false,
                value: values,
                deviation: [0.0; PARAM_COUNT],
                active: Default::default(),
                pending: Default::default(),
                cursor: [None; PARAM_COUNT],
                delete_at: None,
            },
        );
        Ok(())
    }

    /// `after` ミリ秒後に消す。
    pub fn remove(&mut self, id: UiId, after: i32, now: f64) -> Result<(), String> {
        let object = self.objects.get_mut(&id).ok_or_else(|| format!("no such canvas object: {id}"))?;
        object.delete_at = Some(now + f64::from(after.max(0)));
        Ok(())
    }

    /// 次の `start` に備えて遷移を積む。
    pub fn add_transition(
        &mut self,
        id: UiId,
        param: i32,
        kind: i32,
        val_i: i32,
        val_f: f32,
        after: i32,
        duration: i32,
    ) -> Result<(), String> {
        let object = self.objects.get_mut(&id).ok_or_else(|| format!("no such canvas object: {id}"))?;
        let param = Param::from_raw(param).ok_or_else(|| format!("unknown transition param: {param}"))?;
        let kind = Curve::from_raw(kind).ok_or_else(|| format!("unknown transition kind: {kind}"))?;
        // 値は 2 枠のうちパラメータで決まる方を使う。
        let value = if param.is_integer() { f64::from(val_i) } else { f64::from(val_f) };
        object.pending[param as usize].push(Segment {
            owner: id,
            param: param as usize,
            kind,
            value,
            after: f64::from(after.max(0)),
            duration: f64::from(duration.max(0)),
            start_at: 0.0,
            baseline: 0.0,
        });
        Ok(())
    }

    /// 積まれた遷移を発火する。オブジェクトを跨いで一斉に始まる。
    pub fn start(&mut self, sync: bool, now: f64) {
        let mut fired = Vec::new();
        for object in self.objects.values_mut() {
            for param in 0..PARAM_COUNT {
                if object.pending[param].is_empty() {
                    continue;
                }
                let mut staged = std::mem::take(&mut object.pending[param]);
                staged.sort_by(|a, b| a.after.total_cmp(&b.after));
                let mut track = Vec::with_capacity(staged.len());
                for mut segment in staged {
                    segment.start_at = now + segment.after;
                    let id = self.next_segment;
                    self.next_segment += 1;
                    self.segments.insert(id, segment);
                    track.push(id);
                    fired.push(id);
                }
                object.active[param] = track;
                object.cursor[param] = None;
                // 前の周期系が乗せていた偏差は、置き換えた時点で消える。
                object.deviation[param] = 0.0;
            }
        }
        if sync && !fired.is_empty() {
            self.sync_batches.push(fired);
        }
    }

    /// 完了していない sync 印の演出があるか。
    pub fn has_sync(&self) -> bool {
        !self.sync_batches.is_empty()
    }

    /// 進行中の sync 印の演出を終端まで飛ばす。飛ばすものがあったかを返す。
    pub fn skip_sync(&mut self, now: f64) -> bool {
        let mut skipped = false;
        for batch in &self.sync_batches {
            let ends = batch
                .iter()
                .filter_map(|id| self.segments.get(id))
                .filter(|s| self.objects.contains_key(&s.owner) && !s.kind.is_periodic())
                .map(Segment::end)
                .fold(f64::NEG_INFINITY, f64::max);
            let delta = ends - now;
            if !delta.is_finite() || delta <= 0.0 {
                continue;
            }
            // 周期系も一緒にずらす。末尾の `None` がその過程で発火して、ゆらぎも意図どおりに終わる。
            for id in batch {
                if let Some(segment) = self.segments.get_mut(id) {
                    segment.start_at -= delta;
                }
            }
            skipped = true;
        }
        skipped
    }

    /// 時計を `now` まで進める。
    pub fn update(&mut self, now: f64) {
        let due: Vec<UiId> = self
            .objects
            .iter()
            .filter(|(_, o)| o.delete_at.is_some_and(|t| now >= t))
            .map(|(id, _)| *id)
            .collect();
        for id in due {
            self.objects.remove(&id);
        }
        for object in self.objects.values_mut() {
            advance(object, &mut self.segments, now);
        }
        self.collect(now);
    }

    /// 画像が読めた。その画像を使うオブジェクトが描けるようになる。
    pub fn image_loaded(&mut self, image: ImageHandle, width: u32, height: u32) {
        for object in self.objects.values_mut().filter(|o| o.image == image) {
            object.natural = Some((width, height));
        }
    }

    /// 時間で動くものがあるか。
    pub fn is_animating(&self, now: f64) -> bool {
        if !self.sync_batches.is_empty() {
            return true;
        }
        self.objects.values().any(|o| {
            o.delete_at.is_some()
                || (0..PARAM_COUNT).any(|p| {
                    let track = &o.active[p];
                    let Some(last) = track.last().and_then(|id| self.segments.get(id)) else {
                        return false;
                    };
                    o.cursor[p] != Some(track.len() - 1) || last.end() > now
                })
        })
    }

    /// `canvas` に置かれたオブジェクトを描く。`area` は canvas の範囲 (物理ピクセル)。
    pub fn draw(&mut self, canvas: UiId, area: Rect, out: &mut Vec<DrawItem>) {
        if area.width <= 0.0 || area.height <= 0.0 {
            return;
        }
        let mut objects: Vec<&mut CanvasObject> =
            self.objects.values_mut().filter(|o| o.canvas == canvas).collect();
        objects.sort_by_key(|o| (o.layer, o.order));

        let (unit_x, unit_y) = (f64::from(area.width) / 100.0, f64::from(area.height) / 100.0);
        let (cx, cy) = (
            f64::from(area.x) + f64::from(area.width) / 2.0,
            f64::from(area.y) + f64::from(area.height) / 2.0,
        );
        for object in objects {
            // 読み込み前は大きさが分からないので描かない。
            let Some(natural) = object.natural else { continue };
            if !object.auto_size_resolved {
                resolve_auto_size(object, natural, f64::from(area.width), f64::from(area.height));
            }
            let alpha = object.at(Param::Alpha).clamp(0.0, 255.0) / 255.0;
            out.push(DrawItem::Image {
                image: object.image,
                center_x: (cx + object.at(Param::X) * unit_x) as f32,
                // biwa の y は上が正、描画は下が正。
                center_y: (cy - object.at(Param::Y) * unit_y) as f32,
                width: (object.at(Param::W).max(0.0) * unit_x) as f32,
                height: (object.at(Param::H).max(0.0) * unit_y) as f32,
                // y を反転した分、回転の向きも反転させる (biwa の正 = 反時計回り)。
                rotation: (-object.at(Param::Theta)).to_radians() as f32,
                alpha: alpha as f32,
            });
        }
    }

    /// 終わった sync の束を外し、どこからも指されなくなった区間を捨てる。
    fn collect(&mut self, now: f64) {
        let segments = &self.segments;
        let objects = &self.objects;
        self.sync_batches.retain(|batch| {
            batch.iter().filter_map(|id| segments.get(id)).any(|s| {
                objects.contains_key(&s.owner) && !s.kind.is_periodic() && now < s.end()
            })
        });

        let mut live: HashSet<SegmentId> = self.sync_batches.iter().flatten().copied().collect();
        for object in self.objects.values() {
            live.extend(object.active.iter().flatten());
        }
        self.segments.retain(|id, _| live.contains(id));
    }
}

/// 各パラメータの活性な区間を進める。
fn advance(object: &mut CanvasObject, segments: &mut HashMap<SegmentId, Segment>, now: f64) {
    for param in 0..PARAM_COUNT {
        let track = object.active[param].clone();
        if track.is_empty() {
            continue;
        }
        let start_of = |i: usize, segments: &HashMap<SegmentId, Segment>| {
            segments.get(&track[i]).map_or(f64::INFINITY, |s| s.start_at)
        };

        let mut cursor = match object.cursor[param] {
            Some(c) => c,
            None => {
                if start_of(0, segments) > now {
                    continue;
                }
                activate(object, segments, track[0]);
                0
            }
        };
        // 次の区間が始まったら、前の区間はその時点で終わる。引き継ぐ値が正しくなるよう先に確定させる。
        while cursor + 1 < track.len() && start_of(cursor + 1, segments) <= now {
            let at = start_of(cursor + 1, segments);
            evaluate(object, segments, track[cursor], at);
            cursor += 1;
            activate(object, segments, track[cursor]);
        }
        object.cursor[param] = Some(cursor);
        evaluate(object, segments, track[cursor], now);
    }
}

/// 区間が活性になった瞬間に基準値を捉える。
fn activate(object: &mut CanvasObject, segments: &mut HashMap<SegmentId, Segment>, id: SegmentId) {
    if let Some(segment) = segments.get_mut(&id) {
        segment.baseline = object.value[segment.param];
        object.deviation[segment.param] = 0.0;
    }
}

fn evaluate(object: &mut CanvasObject, segments: &HashMap<SegmentId, Segment>, id: SegmentId, at: f64) {
    let Some(s) = segments.get(&id) else { return };
    let p = s.param;
    if s.kind == Curve::None {
        // 基準値で固定して終わる。
        object.value[p] = s.baseline;
        object.deviation[p] = 0.0;
    } else if s.kind.is_periodic() {
        // 落ち着いた値は動かさず、偏差だけを乗せる (区間が終われば基準値に戻る)。
        object.value[p] = s.baseline;
        let period = if s.duration > 0.0 { s.duration } else { 1.0 };
        let phase = ((at - s.start_at) / period).rem_euclid(1.0);
        object.deviation[p] = s.value * s.kind.wave(phase);
    } else {
        object.deviation[p] = 0.0;
        let progress = if s.duration > 0.0 { ((at - s.start_at) / s.duration).clamp(0.0, 1.0) } else { 1.0 };
        object.value[p] = s.baseline + (s.value - s.baseline) * s.kind.at(progress);
    }
}

/// 負の w / h (= 指定しない) を、画像の元の大きさと canvas の大きさ [px] から canvas の単位で決める。
///
/// - 両方負: 画像の元の大きさ (px) で出る
/// - 片方だけ負: 画像の縦横比を保つ (canvas の単位は縦横で尺度が違うので、一度 px に直して比を取る)
///
/// 決めるのは最初の 1 回だけで、以降は canvas の単位の値として遷移の起点にもなる。
fn resolve_auto_size(object: &mut CanvasObject, natural: (u32, u32), width: f64, height: f64) {
    let (nw, nh) = (f64::from(natural.0), f64::from(natural.1));
    if nw <= 0.0 || nh <= 0.0 {
        return;
    }
    object.auto_size_resolved = true;
    let w = object.value[Param::W as usize];
    let h = object.value[Param::H as usize];
    let to_x = |px: f64| px / width * 100.0;
    let to_y = |px: f64| px / height * 100.0;
    if w < 0.0 && h < 0.0 {
        object.value[Param::W as usize] = to_x(nw);
        object.value[Param::H as usize] = to_y(nh);
    } else if w < 0.0 {
        let h_px = h / 100.0 * height;
        object.value[Param::W as usize] = to_x(nw * (h_px / nh));
    } else if h < 0.0 {
        let w_px = w / 100.0 * width;
        object.value[Param::H as usize] = to_y(nh * (w_px / nw));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const IMG: ImageHandle = ImageHandle(1);

    fn canvas_with_object() -> Canvas {
        let mut c = Canvas::default();
        // x=0, y=0, w=10, h=10, alpha=255, theta=0
        c.create(1, 100, IMG, 0, [0.0, 0.0, 10.0, 10.0, 255.0, 0.0], Some((64, 64))).unwrap();
        c
    }

    fn x_of(c: &Canvas) -> f64 {
        c.objects[&1].at(Param::X)
    }

    #[test]
    fn linear_transition_reaches_target() {
        let mut c = canvas_with_object();
        c.add_transition(1, Param::X as i32, Curve::Linear as i32, 0, 20.0, 0, 1000).unwrap();
        c.start(false, 0.0);
        c.update(500.0);
        assert!((x_of(&c) - 10.0).abs() < 1e-6);
        c.update(1500.0);
        assert!((x_of(&c) - 20.0).abs() < 1e-6);
        assert!(!c.is_animating(1500.0));
    }

    #[test]
    fn next_segment_takes_over_from_current_value() {
        let mut c = canvas_with_object();
        c.add_transition(1, Param::X as i32, Curve::Linear as i32, 0, 100.0, 0, 1000).unwrap();
        // 500 ms で次の区間が始まり、その時点の値 (50) から 0 へ戻る。
        c.add_transition(1, Param::X as i32, Curve::Linear as i32, 0, 0.0, 500, 500).unwrap();
        c.start(false, 0.0);
        c.update(500.0);
        assert!((x_of(&c) - 50.0).abs() < 1e-6);
        c.update(750.0);
        assert!((x_of(&c) - 25.0).abs() < 1e-6);
    }

    #[test]
    fn periodic_adds_deviation_and_none_stops_it() {
        let mut c = canvas_with_object();
        c.add_transition(1, Param::Y as i32, Curve::Triangle as i32, 0, 4.0, 0, 1000).unwrap();
        c.add_transition(1, Param::Y as i32, Curve::None as i32, 0, 0.0, 2000, 0).unwrap();
        c.start(false, 0.0);
        c.update(250.0);
        assert!((c.objects[&1].at(Param::Y) - 4.0).abs() < 1e-6);
        assert!(c.is_animating(250.0));
        c.update(2100.0);
        assert!(c.objects[&1].at(Param::Y).abs() < 1e-6);
        assert!(!c.is_animating(2100.0));
    }

    #[test]
    fn untouched_params_keep_running() {
        let mut c = canvas_with_object();
        c.add_transition(1, Param::X as i32, Curve::Linear as i32, 0, 100.0, 0, 1000).unwrap();
        c.start(false, 0.0);
        c.update(500.0);
        c.add_transition(1, Param::Alpha as i32, Curve::Linear as i32, 0, 0.0, 0, 1000).unwrap();
        c.start(false, 500.0);
        c.update(1000.0);
        assert!((x_of(&c) - 100.0).abs() < 1e-6, "X was replaced");
    }

    #[test]
    fn sync_batches_are_awaited_and_skippable() {
        let mut c = canvas_with_object();
        c.add_transition(1, Param::X as i32, Curve::Linear as i32, 0, 20.0, 0, 1000).unwrap();
        c.start(true, 0.0);
        c.update(100.0);
        assert!(c.has_sync());
        assert!(c.skip_sync(100.0));
        c.update(100.0);
        assert!((x_of(&c) - 20.0).abs() < 1e-6);
        assert!(!c.has_sync());
        assert!(!c.skip_sync(100.0));
    }

    #[test]
    fn integer_param_uses_val_i() {
        let mut c = canvas_with_object();
        c.add_transition(1, Param::Alpha as i32, Curve::Linear as i32, 0, 999.0, 0, 0).unwrap();
        c.start(false, 0.0);
        c.update(0.0);
        assert_eq!(c.objects[&1].at(Param::Alpha), 0.0);
    }

    #[test]
    fn deletes_after_delay() {
        let mut c = canvas_with_object();
        c.remove(1, 300, 0.0).unwrap();
        c.update(200.0);
        assert!(c.objects.contains_key(&1));
        c.update(300.0);
        assert!(!c.objects.contains_key(&1));
        assert!(c.segments.is_empty());
    }

    #[test]
    fn auto_size_keeps_aspect_and_projects_to_pixels() {
        let mut c = Canvas::default();
        // 幅だけ 50 (canvas の半分) を指定。画像は 200x100 なので高さは px で幅の半分。
        c.create(1, 100, IMG, 0, [0.0, 10.0, 50.0, -1.0, 255.0, 90.0], Some((200, 100))).unwrap();
        let mut out = Vec::new();
        c.draw(100, Rect::new(0.0, 0.0, 1000.0, 500.0), &mut out);
        let DrawItem::Image { center_x, center_y, width, height, rotation, .. } = out[0] else {
            panic!("not an image")
        };
        assert_eq!((center_x, center_y), (500.0, 200.0));
        assert_eq!((width, height), (500.0, 250.0));
        assert!((rotation + std::f32::consts::FRAC_PI_2).abs() < 1e-6);
    }

    #[test]
    fn not_drawn_until_loaded_and_sorted_by_layer() {
        let mut c = Canvas::default();
        c.create(1, 100, ImageHandle(1), 2, [0.0, 0.0, 10.0, 10.0, 255.0, 0.0], None).unwrap();
        c.create(2, 100, ImageHandle(2), 0, [0.0, 0.0, 10.0, 10.0, 255.0, 0.0], Some((1, 1))).unwrap();
        let mut out = Vec::new();
        c.draw(100, Rect::new(0.0, 0.0, 100.0, 100.0), &mut out);
        assert_eq!(out.len(), 1);
        c.image_loaded(ImageHandle(1), 1, 1);
        out.clear();
        c.draw(100, Rect::new(0.0, 0.0, 100.0, 100.0), &mut out);
        let images: Vec<_> = out
            .iter()
            .map(|i| match i {
                DrawItem::Image { image, .. } => *image,
                _ => panic!(),
            })
            .collect();
        assert_eq!(images, vec![ImageHandle(2), ImageHandle(1)]);
    }
}
