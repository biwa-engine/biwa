//! 時間で動かせるパラメータ (音の分野で言うオートメーション)。
//!
//! canvas オブジェクトの x や alpha も、音の音量やフィルタの周波数も、同じ規則で動く
//! (`docs/media-object-model.md`、`docs/sound.md`):
//!
//! - `push` は積むだけ。`start` で、積まれた区間がある `(持ち主, パラメータ)` だけが置き換わる
//!   (触れていないパラメータは走り続ける)。
//! - 次の区間が始まると前の区間はそこで終わり、値を引き継ぐ。基準値は区間が活性になった瞬間に捉える。
//! - 周期系は落ち着いた値を動かさず、偏差だけを乗せる。
//! - sync 印の発火 (`start(true, ..)`) は束として覚え、[`Automation::has_sync`] で待たれ、
//!   [`Automation::skip_sync`] で終端まで飛ばされる。飛ばすのは時計ではなく区間の開始時刻である。
//!
//! どのパラメータがあるか、値の単位や補間の空間 (Hz を対数で動かすなど) は持ち主が決める。
//! ここは「1 つの値を時間で動かす区間」だけを知っていて、値は線形に補間する。
//! 対数で動かしたいものは、持ち主が対数に直してから渡す。
//!
//! `engine/nodejs/src/engine/automation/Automation.ts` と同じ規則である。

use std::collections::{BTreeMap, HashMap, HashSet};

use crate::transition::Curve;

type SegmentId = u64;

/// 遷移の 1 区間 (`add_transition` 1 回)。
#[derive(Debug, Clone)]
struct Segment<K> {
    owner: K,
    param: u8,
    kind: Curve,
    /// 一度限りの遷移では目標値、周期系では振幅。
    value: f64,
    /// 一度限りの遷移では遷移時間、周期系では周期 [ms]。
    duration: f64,
    /// エンジン時計の上での開始時刻。発火したときに決まる。
    start_at: f64,
    /// 活性になった瞬間に捉えた基準値。
    baseline: f64,
}

impl<K> Segment<K> {
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

/// 1 つのパラメータの状態。
#[derive(Debug, Default)]
struct Track {
    /// 落ち着いた値 (周期系の偏差を含まない)。
    value: f64,
    /// 周期系が乗せている偏差。
    deviation: f64,
    /// 走っている区間の列。
    active: Vec<SegmentId>,
    /// 次の `start` を待つ区間。
    pending: Vec<(Curve, f64, f64, f64)>,
    /// `active` の中で今活性な区間の位置。`None` はまだどれも始まっていない。
    cursor: Option<usize>,
}

/// 持ち主 `K` ごと・パラメータごとの、時間で動く値の全体。時刻はエンジン時計 [ms] を外から受け取る。
#[derive(Debug)]
pub struct Automation<K> {
    tracks: BTreeMap<(K, u8), Track>,
    segments: HashMap<SegmentId, Segment<K>>,
    next_segment: SegmentId,
    /// 完了していない sync 印の束 (区間の番号の列)。
    sync_batches: Vec<Vec<SegmentId>>,
}

impl<K> Default for Automation<K> {
    fn default() -> Self {
        Self { tracks: BTreeMap::new(), segments: HashMap::new(), next_segment: 0, sync_batches: Vec::new() }
    }
}

impl<K: Ord + Copy> Automation<K> {
    /// パラメータを初期値で置く (既にあれば値だけ置き換える)。
    pub fn define(&mut self, owner: K, param: u8, value: f64) {
        self.tracks.entry((owner, param)).or_default().value = value;
    }

    /// パラメータがあるか。
    pub fn contains(&self, owner: K, param: u8) -> bool {
        self.tracks.contains_key(&(owner, param))
    }

    /// 今の値 (落ち着いた値 + 周期系の偏差)。無いパラメータは `None`。
    pub fn get(&self, owner: K, param: u8) -> Option<f64> {
        self.tracks.get(&(owner, param)).map(|t| t.value + t.deviation)
    }

    /// 落ち着いた値を直接書き換える (遷移の起点を決め直すときなど)。走っている区間はそのまま。
    pub fn set_settled(&mut self, owner: K, param: u8, value: f64) {
        if let Some(track) = self.tracks.get_mut(&(owner, param)) {
            track.value = value;
        }
    }

    /// 即座に値を設定する。走っている区間と、まだ発火していない区間は捨てる。
    pub fn set(&mut self, owner: K, param: u8, value: f64) {
        let track = self.tracks.entry((owner, param)).or_default();
        track.value = value;
        track.deviation = 0.0;
        track.active.clear();
        track.cursor = None;
    }

    /// 次の `start` に備えて区間を積む。パラメータが無ければ何もしない (`false`)。
    pub fn push(&mut self, owner: K, param: u8, kind: Curve, value: f64, after: f64, duration: f64) -> bool {
        let Some(track) = self.tracks.get_mut(&(owner, param)) else {
            return false;
        };
        track.pending.push((kind, value, after.max(0.0), duration.max(0.0)));
        true
    }

    /// 積まれた区間を発火する。持ち主を跨いで一斉に始まる。発火した区間があったかを返す。
    pub fn start(&mut self, sync: bool, now: f64) -> bool {
        let mut fired = Vec::new();
        for (&(owner, param), track) in &mut self.tracks {
            if track.pending.is_empty() {
                continue;
            }
            let mut staged = std::mem::take(&mut track.pending);
            staged.sort_by(|a, b| a.2.total_cmp(&b.2));
            let mut ids = Vec::with_capacity(staged.len());
            for (kind, value, after, duration) in staged {
                let id = self.next_segment;
                self.next_segment += 1;
                self.segments.insert(
                    id,
                    Segment { owner, param, kind, value, duration, start_at: now + after, baseline: 0.0 },
                );
                ids.push(id);
                fired.push(id);
            }
            track.active = ids;
            track.cursor = None;
            // 前の周期系が乗せていた偏差は、置き換えた時点で消える。
            track.deviation = 0.0;
        }
        let any = !fired.is_empty();
        if sync && any {
            self.sync_batches.push(fired);
        }
        any
    }

    /// 持ち主のパラメータをすべて捨てる。
    pub fn remove(&mut self, owner: K) {
        self.tracks.retain(|(o, _), _| *o != owner);
    }

    /// 完了していない sync 印の区間があるか。
    pub fn has_sync(&self) -> bool {
        !self.sync_batches.is_empty()
    }

    /// 進行中の sync 印の区間を終端まで飛ばす。飛ばすものがあったかを返す。
    pub fn skip_sync(&mut self, now: f64) -> bool {
        let mut skipped = false;
        for batch in &self.sync_batches {
            let ends = batch
                .iter()
                .filter_map(|id| self.segments.get(id))
                .filter(|s| self.tracks.contains_key(&(s.owner, s.param)) && !s.kind.is_periodic())
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
        for track in self.tracks.values_mut() {
            advance(track, &mut self.segments, now);
        }
        self.collect(now);
    }

    /// 時間で動いているものがあるか。
    pub fn is_animating(&self, now: f64) -> bool {
        if !self.sync_batches.is_empty() {
            return true;
        }
        self.tracks.values().any(|t| {
            let Some(last) = t.active.last().and_then(|id| self.segments.get(id)) else {
                return false;
            };
            t.cursor != Some(t.active.len() - 1) || last.end() > now
        })
    }

    /// 終わった sync の束を外し、どこからも指されなくなった区間を捨てる。
    fn collect(&mut self, now: f64) {
        let segments = &self.segments;
        let tracks = &self.tracks;
        self.sync_batches.retain(|batch| {
            batch.iter().filter_map(|id| segments.get(id)).any(|s| {
                tracks.contains_key(&(s.owner, s.param)) && !s.kind.is_periodic() && now < s.end()
            })
        });

        let mut live: HashSet<SegmentId> = self.sync_batches.iter().flatten().copied().collect();
        for track in self.tracks.values() {
            live.extend(track.active.iter());
        }
        self.segments.retain(|id, _| live.contains(id));
    }

    #[cfg(test)]
    pub(crate) fn segment_count(&self) -> usize {
        self.segments.len()
    }
}

/// 活性な区間を進める。
fn advance<K>(track: &mut Track, segments: &mut HashMap<SegmentId, Segment<K>>, now: f64) {
    if track.active.is_empty() {
        return;
    }
    let ids = track.active.clone();
    let start_of =
        |i: usize, segments: &HashMap<SegmentId, Segment<K>>| segments.get(&ids[i]).map_or(f64::INFINITY, |s| s.start_at);

    let mut cursor = match track.cursor {
        Some(c) => c,
        None => {
            if start_of(0, segments) > now {
                return;
            }
            activate(track, segments, ids[0]);
            0
        }
    };
    // 次の区間が始まったら、前の区間はその時点で終わる。引き継ぐ値が正しくなるよう先に確定させる。
    while cursor + 1 < ids.len() && start_of(cursor + 1, segments) <= now {
        let at = start_of(cursor + 1, segments);
        evaluate(track, segments, ids[cursor], at);
        cursor += 1;
        activate(track, segments, ids[cursor]);
    }
    track.cursor = Some(cursor);
    evaluate(track, segments, ids[cursor], now);
}

/// 区間が活性になった瞬間に基準値を捉える。
fn activate<K>(track: &mut Track, segments: &mut HashMap<SegmentId, Segment<K>>, id: SegmentId) {
    if let Some(segment) = segments.get_mut(&id) {
        segment.baseline = track.value;
        track.deviation = 0.0;
    }
}

fn evaluate<K>(track: &mut Track, segments: &HashMap<SegmentId, Segment<K>>, id: SegmentId, at: f64) {
    let Some(s) = segments.get(&id) else { return };
    if s.kind == Curve::None {
        // 基準値で固定して終わる。
        track.value = s.baseline;
        track.deviation = 0.0;
    } else if s.kind.is_periodic() {
        // 落ち着いた値は動かさず、偏差だけを乗せる (区間が終われば基準値に戻る)。
        track.value = s.baseline;
        let period = if s.duration > 0.0 { s.duration } else { 1.0 };
        let phase = ((at - s.start_at) / period).rem_euclid(1.0);
        track.deviation = s.value * s.kind.wave(phase);
    } else {
        track.deviation = 0.0;
        let progress = if s.duration > 0.0 { ((at - s.start_at) / s.duration).clamp(0.0, 1.0) } else { 1.0 };
        track.value = s.baseline + (s.value - s.baseline) * s.kind.at(progress);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn auto() -> Automation<u32> {
        let mut a = Automation::default();
        a.define(1, 0, 0.0);
        a
    }

    #[test]
    fn linear_reaches_target_and_next_segment_takes_over() {
        let mut a = auto();
        a.push(1, 0, Curve::Linear, 100.0, 0.0, 1000.0);
        a.push(1, 0, Curve::Linear, 0.0, 500.0, 500.0);
        a.start(false, 0.0);
        a.update(500.0);
        assert!((a.get(1, 0).unwrap() - 50.0).abs() < 1e-6);
        a.update(750.0);
        assert!((a.get(1, 0).unwrap() - 25.0).abs() < 1e-6);
        a.update(2000.0);
        assert!(!a.is_animating(2000.0));
    }

    #[test]
    fn set_cancels_running_segments() {
        let mut a = auto();
        a.push(1, 0, Curve::Linear, 100.0, 0.0, 1000.0);
        a.start(false, 0.0);
        a.update(100.0);
        a.set(1, 0, -5.0);
        a.update(900.0);
        assert_eq!(a.get(1, 0), Some(-5.0));
        assert!(!a.is_animating(900.0));
    }

    #[test]
    fn sync_is_skippable_and_removed_owners_do_not_block() {
        let mut a = auto();
        a.define(2, 0, 0.0);
        a.push(1, 0, Curve::Linear, 10.0, 0.0, 1000.0);
        a.push(2, 0, Curve::Linear, 10.0, 0.0, 5000.0);
        a.start(true, 0.0);
        a.update(10.0);
        assert!(a.has_sync());
        a.remove(2);
        assert!(a.skip_sync(10.0));
        a.update(10.0);
        assert_eq!(a.get(1, 0), Some(10.0));
        assert!(!a.has_sync());
        assert_eq!(a.segment_count(), 1);
    }

    #[test]
    fn push_to_unknown_param_is_refused() {
        let mut a = auto();
        assert!(!a.push(1, 9, Curve::Linear, 1.0, 0.0, 0.0));
        assert!(!a.start(false, 0.0));
    }
}
