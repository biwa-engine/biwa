import { Curve, curveAt, isPeriodic, waveAt } from "../api/transition";

/**
 * 時間で動かせるパラメータ (音の分野で言うオートメーション)。
 *
 * canvas オブジェクトの x や alpha も、音の音量やフィルタの周波数も、同じ規則で動く
 * (`docs/media-object-model.md`、`docs/sound.md`):
 *
 * - `push` は積むだけ。`start` で、積まれた区間がある `(持ち主, パラメータ)` だけが置き換わる
 *   (触れていないパラメータは走り続ける)。
 * - 次の区間が始まると前の区間はそこで終わり、値を引き継ぐ。基準値は区間が活性になった瞬間に捉える。
 * - 周期系は落ち着いた値を動かさず、偏差だけを乗せる。
 * - sync 印の発火 (`start(true, ..)`) は束として覚え、`hasSync` で待たれ、
 *   `skipSync` で終端まで飛ばされる。飛ばすのは時計ではなく区間の開始時刻である。
 *
 * どのパラメータがあるか、値の単位や補間の空間 (Hz を対数で動かすなど) は持ち主が決める。
 * ここは「1 つの値を時間で動かす区間」だけを知っていて、値は線形に補間する。
 *
 * ネイティブの `engine/desktop-rust/crates/core/src/automation.rs` と同じ規則である。
 * 持ち主は文字列の鍵で指す (`"obj:3"` など。種類の違う id 空間を 1 つの表に置けるように)。
 */

/** 遷移の 1 区間 (`add_transition` 1 回)。 */
interface Segment {
  track: Track;
  kind: number;
  /** 一度限りの遷移では目標値、周期系では振幅。 */
  value: number;
  /** `start` からの遅れ [ms]。 */
  after: number;
  /** 一度限りの遷移では遷移時間、周期系では周期 [ms]。 */
  duration: number;
  /** エンジン時計の上での開始時刻。発火時に決まる。 */
  startAt: number;
  /** 活性になった瞬間に捉えた基準値。 */
  baseline: number;
}

/** 1 つのパラメータの状態。 */
interface Track {
  /** 落ち着いた値。周期系の偏差は含まない。 */
  value: number;
  /** 活性な周期系が乗せている偏差。区間が終われば消える。 */
  deviation: number;
  /** 走っている区間列。 */
  active: Segment[];
  /** 次の `start` を待つ区間列。 */
  pending: Segment[];
  /** `active` の中で今活性な区間の位置。-1 はまだどれも始まっていない。 */
  cursor: number;
  alive: boolean;
}

export class Automation {
  private readonly tracks = new Map<string, Track>();
  /** 完了していない sync 印のバッチ。 */
  private syncBatches: Segment[][] = [];

  /** パラメータを初期値で置く (既にあれば値だけ置き換える)。 */
  define(owner: string, param: number, value: number): void {
    const key = keyOf(owner, param);
    const track = this.tracks.get(key);
    if (track !== undefined) {
      track.value = value;
      return;
    }
    this.tracks.set(key, {
      value,
      deviation: 0,
      active: [],
      pending: [],
      cursor: -1,
      alive: true,
    });
  }

  has(owner: string, param: number): boolean {
    return this.tracks.has(keyOf(owner, param));
  }

  /** 今の値 (落ち着いた値 + 周期系の偏差)。無いパラメータは `undefined`。 */
  get(owner: string, param: number): number | undefined {
    const track = this.tracks.get(keyOf(owner, param));
    return track === undefined ? undefined : track.value + track.deviation;
  }

  /** 落ち着いた値を直接書き換える。走っている区間はそのまま。 */
  setSettled(owner: string, param: number, value: number): void {
    const track = this.tracks.get(keyOf(owner, param));
    if (track !== undefined) track.value = value;
  }

  /** 即座に値を設定する。走っている区間は捨てる。 */
  set(owner: string, param: number, value: number): void {
    this.define(owner, param, value);
    const track = this.tracks.get(keyOf(owner, param))!;
    track.deviation = 0;
    track.active = [];
    track.cursor = -1;
  }

  /** 次の `start` に備えて区間を積む。パラメータが無ければ何もしない (`false`)。 */
  push(
    owner: string,
    param: number,
    kind: number,
    value: number,
    after: number,
    duration: number,
  ): boolean {
    const track = this.tracks.get(keyOf(owner, param));
    if (track === undefined) return false;
    track.pending.push({
      track,
      kind,
      value,
      after: Math.max(after, 0),
      duration: Math.max(duration, 0),
      startAt: 0,
      baseline: 0,
    });
    return true;
  }

  /** 積まれた区間を発火する。持ち主を跨いで一斉に始まる。発火した区間があったかを返す。 */
  start(sync: boolean, now: number): boolean {
    const fired: Segment[] = [];
    for (const track of this.tracks.values()) {
      if (track.pending.length === 0) continue;
      const staged = track.pending;
      staged.sort((a, b) => a.after - b.after);
      for (const segment of staged) {
        segment.startAt = now + segment.after;
        fired.push(segment);
      }
      track.active = staged;
      track.pending = [];
      track.cursor = -1;
      // 前の周期系が乗せていた偏差は、置き換えた時点で消える。
      track.deviation = 0;
    }
    if (sync && fired.length > 0) this.syncBatches.push(fired);
    return fired.length > 0;
  }

  /** 持ち主のパラメータをすべて捨てる。 */
  remove(owner: string): void {
    const prefix = `${owner}#`;
    for (const [key, track] of this.tracks) {
      if (key.startsWith(prefix)) {
        track.alive = false;
        this.tracks.delete(key);
      }
    }
  }

  hasSync(): boolean {
    return this.syncBatches.length > 0;
  }

  /** 進行中の sync 印の区間を終端まで飛ばす。飛ばすものがあったかを返す。 */
  skipSync(now: number): boolean {
    let skipped = false;
    for (const batch of this.syncBatches) {
      let endsAt = -Infinity;
      for (const segment of batch) {
        if (!segment.track.alive || isPeriodic(segment.kind)) continue;
        endsAt = Math.max(endsAt, endOf(segment));
      }
      const delta = endsAt - now;
      if (!Number.isFinite(delta) || delta <= 0) continue;
      // 周期系も一緒にずらす。末尾の `None` がその過程で発火して、ゆらぎも意図どおりに終わる。
      for (const segment of batch) segment.startAt -= delta;
      skipped = true;
    }
    return skipped;
  }

  /** 時計を `now` まで進める。 */
  update(now: number): void {
    for (const track of this.tracks.values()) advance(track, now);
    if (this.syncBatches.length > 0) {
      this.syncBatches = this.syncBatches.filter((batch) => !isBatchDone(batch, now));
    }
  }

  /** 時間で動いているものがあるか。 */
  isAnimating(now: number): boolean {
    if (this.syncBatches.length > 0) return true;
    for (const track of this.tracks.values()) {
      const last = track.active[track.active.length - 1];
      if (last === undefined) continue;
      if (
        track.cursor !== track.active.length - 1 ||
        isPeriodic(last.kind) ||
        endOf(last) > now
      ) {
        return true;
      }
    }
    return false;
  }
}

function keyOf(owner: string, param: number): string {
  return `${owner}#${param}`;
}

/** 活性な区間を進める。 */
function advance(track: Track, now: number): void {
  const segments = track.active;
  if (segments.length === 0) return;

  let cursor = track.cursor;
  if (cursor < 0) {
    if (segments[0].startAt > now) return;
    cursor = 0;
    activate(track, segments[0]);
  }
  // 次の区間が始まったら、前の区間はその時点で終わる。
  // 引き継ぐ値が正しくなるよう、終わる瞬間の値を先に確定させる。
  while (cursor + 1 < segments.length && segments[cursor + 1].startAt <= now) {
    evaluate(track, segments[cursor], segments[cursor + 1].startAt);
    cursor += 1;
    activate(track, segments[cursor]);
  }
  track.cursor = cursor;
  evaluate(track, segments[cursor], now);
}

/** 区間が活性になった瞬間に基準値を捉える。 */
function activate(track: Track, segment: Segment): void {
  segment.baseline = track.value;
  track.deviation = 0;
}

function evaluate(track: Track, segment: Segment, at: number): void {
  if (segment.kind === Curve.None) {
    // 基準値 (= 区間に入った時点の落ち着いた値) で固定して終わる。
    track.value = segment.baseline;
    track.deviation = 0;
    return;
  }
  if (isPeriodic(segment.kind)) {
    // 周期系は落ち着いた値を動かさず、偏差だけを乗せる。だから区間が終われば値は基準値に戻る。
    track.value = segment.baseline;
    const period = segment.duration > 0 ? segment.duration : 1;
    const phase = ((((at - segment.startAt) / period) % 1) + 1) % 1;
    track.deviation = segment.value * waveAt(segment.kind, phase);
    return;
  }
  track.deviation = 0;
  const progress =
    segment.duration > 0
      ? Math.min(Math.max((at - segment.startAt) / segment.duration, 0), 1)
      : 1;
  track.value =
    segment.baseline + (segment.value - segment.baseline) * curveAt(segment.kind, progress);
}

/**
 * 区間が終わる時刻。
 *
 * 周期系は終わらないので `-Infinity` を返す (完了の判定に数えない)。
 * これがあるので、ゆらぎの混じった sync バッチでも待ちが固まらない。
 */
function endOf(segment: Segment): number {
  if (isPeriodic(segment.kind)) return -Infinity;
  if (segment.kind === Curve.None) return segment.startAt;
  return segment.startAt + segment.duration;
}

function isBatchDone(batch: Segment[], now: number): boolean {
  for (const segment of batch) {
    if (!segment.track.alive || isPeriodic(segment.kind)) continue;
    if (now < endOf(segment)) return false;
  }
  return true;
}
