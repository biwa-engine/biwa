//! canvas オブジェクトと遷移 (`engine/nodejs/src/engine/canvas/CanvasObjects.ts` の移植)。
//!
//! 規則は TypeScript 版と同じである (`docs/media-object-model.md`)。
//!
//! - biwa 側のパラメータを正とし、描くときに毎回 px に射影する。
//!   x / y / w / h は canvas の範囲が -50..50 の単位で持つので、canvas の大きさが変われば追従する。
//! - パラメータの遷移 (積む・発火・sync・飛ばす) は [`Automation`] に任せる。音のパラメータと同じ仕組みである。
//! - 大きさを指定しない (負の) `w` / `h` は、画像の元の大きさと canvas の大きさから 1 度だけ決める。

use std::collections::BTreeMap;

use crate::automation::Automation;
use crate::draw::DrawItem;
use crate::geometry::Rect;
use crate::image::ImageHandle;
use crate::syscall::UiId;
use crate::transition::{Curve, PARAM_COUNT, Param};

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
    /// 消える予定の時刻。
    delete_at: Option<f64>,
}

/// canvas オブジェクトの全体。時刻はエンジン時計 [ms] を外から受け取る。
#[derive(Debug, Default)]
pub struct Canvas {
    objects: BTreeMap<UiId, CanvasObject>,
    /// パラメータ (x / y / w / h / alpha / theta) の値と遷移。
    params: Automation<UiId>,
    next_order: u64,
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
            CanvasObject { canvas, image, layer, order, natural, auto_size_resolved: false, delete_at: None },
        );
        for (param, value) in values.into_iter().enumerate() {
            self.params.define(id, param as u8, value);
        }
        Ok(())
    }

    /// `after` ミリ秒後に消す。
    pub fn remove(&mut self, id: UiId, after: i32, now: f64) -> Result<(), String> {
        let object = self.objects.get_mut(&id).ok_or_else(|| format!("no such canvas object: {id}"))?;
        object.delete_at = Some(now + f64::from(after.max(0)));
        Ok(())
    }

    /// 次の `start` に備えて遷移を積む。
    #[allow(clippy::too_many_arguments)]
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
        if !self.objects.contains_key(&id) {
            return Err(format!("no such canvas object: {id}"));
        }
        let param = Param::from_raw(param).ok_or_else(|| format!("unknown transition param: {param}"))?;
        let kind = Curve::from_raw(kind).ok_or_else(|| format!("unknown transition kind: {kind}"))?;
        // 値は 2 枠のうちパラメータで決まる方を使う。
        let value = if param.is_integer() { f64::from(val_i) } else { f64::from(val_f) };
        self.params.push(id, param as u8, kind, value, f64::from(after), f64::from(duration));
        Ok(())
    }

    /// 積まれた遷移を発火する。オブジェクトを跨いで一斉に始まる。
    pub fn start(&mut self, sync: bool, now: f64) {
        self.params.start(sync, now);
    }

    /// 完了していない sync 印の演出があるか。
    pub fn has_sync(&self) -> bool {
        self.params.has_sync()
    }

    /// 進行中の sync 印の演出を終端まで飛ばす。飛ばすものがあったかを返す。
    pub fn skip_sync(&mut self, now: f64) -> bool {
        self.params.skip_sync(now)
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
            self.params.remove(id);
        }
        self.params.update(now);
    }

    /// 画像が読めた。その画像を使うオブジェクトが描けるようになる。
    pub fn image_loaded(&mut self, image: ImageHandle, width: u32, height: u32) {
        for object in self.objects.values_mut().filter(|o| o.image == image) {
            object.natural = Some((width, height));
        }
    }

    /// 時間で動くものがあるか。
    pub fn is_animating(&self, now: f64) -> bool {
        self.objects.values().any(|o| o.delete_at.is_some()) || self.params.is_animating(now)
    }

    fn at(&self, id: UiId, param: Param) -> f64 {
        self.params.get(id, param as u8).unwrap_or(0.0)
    }

    /// `canvas` に置かれたオブジェクトを描く。`area` は canvas の範囲 (物理ピクセル)。
    pub fn draw(&mut self, canvas: UiId, area: Rect, out: &mut Vec<DrawItem>) {
        if area.width <= 0.0 || area.height <= 0.0 {
            return;
        }
        let mut ids: Vec<UiId> =
            self.objects.iter().filter(|(_, o)| o.canvas == canvas).map(|(id, _)| *id).collect();
        ids.sort_by_key(|id| (self.objects[id].layer, self.objects[id].order));

        let (unit_x, unit_y) = (f64::from(area.width) / 100.0, f64::from(area.height) / 100.0);
        let (cx, cy) = (
            f64::from(area.x) + f64::from(area.width) / 2.0,
            f64::from(area.y) + f64::from(area.height) / 2.0,
        );
        for id in ids {
            // 読み込み前は大きさが分からないので描かない。
            let Some(natural) = self.objects[&id].natural else { continue };
            if !self.objects[&id].auto_size_resolved {
                self.resolve_auto_size(id, natural, f64::from(area.width), f64::from(area.height));
            }
            let alpha = self.at(id, Param::Alpha).clamp(0.0, 255.0) / 255.0;
            out.push(DrawItem::Image {
                image: self.objects[&id].image,
                center_x: (cx + self.at(id, Param::X) * unit_x) as f32,
                // biwa の y は上が正、描画は下が正。
                center_y: (cy - self.at(id, Param::Y) * unit_y) as f32,
                width: (self.at(id, Param::W).max(0.0) * unit_x) as f32,
                height: (self.at(id, Param::H).max(0.0) * unit_y) as f32,
                // y を反転した分、回転の向きも反転させる (biwa の正 = 反時計回り)。
                rotation: (-self.at(id, Param::Theta)).to_radians() as f32,
                alpha: alpha as f32,
            });
        }
    }

    /// 負の w / h (= 指定しない) を、画像の元の大きさと canvas の大きさ [px] から canvas の単位で決める。
    ///
    /// - 両方負: 画像の元の大きさ (px) で出る
    /// - 片方だけ負: 画像の縦横比を保つ (canvas の単位は縦横で尺度が違うので、一度 px に直して比を取る)
    ///
    /// 決めるのは最初の 1 回だけで、以降は canvas の単位の値として遷移の起点にもなる。
    fn resolve_auto_size(&mut self, id: UiId, natural: (u32, u32), width: f64, height: f64) {
        let (nw, nh) = (f64::from(natural.0), f64::from(natural.1));
        if nw <= 0.0 || nh <= 0.0 {
            return;
        }
        if let Some(object) = self.objects.get_mut(&id) {
            object.auto_size_resolved = true;
        }
        let w = self.at(id, Param::W);
        let h = self.at(id, Param::H);
        let to_x = |px: f64| px / width * 100.0;
        let to_y = |px: f64| px / height * 100.0;
        let (pw, ph) = (Param::W as u8, Param::H as u8);
        if w < 0.0 && h < 0.0 {
            self.params.set_settled(id, pw, to_x(nw));
            self.params.set_settled(id, ph, to_y(nh));
        } else if w < 0.0 {
            let h_px = h / 100.0 * height;
            self.params.set_settled(id, pw, to_x(nw * (h_px / nh)));
        } else if h < 0.0 {
            let w_px = w / 100.0 * width;
            self.params.set_settled(id, ph, to_y(nh * (w_px / nw)));
        }
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
        c.at(1, Param::X)
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
        assert!((c.at(1, Param::Y) - 4.0).abs() < 1e-6);
        assert!(c.is_animating(250.0));
        c.update(2100.0);
        assert!(c.at(1, Param::Y).abs() < 1e-6);
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
        assert_eq!(c.at(1, Param::Alpha), 0.0);
    }

    #[test]
    fn deletes_after_delay() {
        let mut c = canvas_with_object();
        c.remove(1, 300, 0.0).unwrap();
        c.update(200.0);
        assert!(c.objects.contains_key(&1));
        c.update(300.0);
        assert!(!c.objects.contains_key(&1));
        assert!(!c.params.contains(1, 0));
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
