import { Assets, Sprite, Texture } from "pixi.js";
import { resolveAssetUrl } from "../api/assets";
import {
  Curve,
  PARAM_COUNT,
  Param,
  curveAt,
  isKnownKind,
  isKnownParam,
  isPeriodic,
  waveAt,
} from "../api/transition";
import type { CanvasSurfaces } from "./CanvasSurfaces";

/**
 * 遷移の 1 区間。
 *
 * `add_transition` 1 回がこれ 1 つになる。`start_transitions` まで
 * オブジェクトの `pending` に積まれ、発火して初めて `startAt` が決まる。
 */
interface Segment {
  owner: CanvasObject;
  param: number;
  kind: number;
  /** 一度限りの遷移では目標値、周期系では振幅。 */
  value: number;
  /** `start_transitions` からの遅れ [ms]。 */
  after: number;
  /** 一度限りの遷移では遷移時間、周期系では周期 [ms]。 */
  duration: number;
  /** エンジン時計の上での開始時刻。発火時に決まる。 */
  startAt: number;
  /** 活性になった瞬間に捉えた基準値。 */
  baseline: number;
}

/** 1 回の `start_transitions` で発火した区間のまとまり。 */
interface Batch {
  segments: Segment[];
}

/**
 * canvas に置かれたオブジェクト 1 つ。
 *
 * biwa 側のパラメータ (`value` / `deviation`) を正とし、
 * PixiJS へは毎フレーム射影する。座標系も単位も両者で違うためである。
 */
class CanvasObject {
  readonly id: number;
  readonly sprite: Sprite;
  /** テクスチャの元のサイズ。ロードが終わるまで null。 */
  natural: { w: number; h: number } | null = null;
  /** param ごとの落ち着いた値。周期系の偏差は含まない。 */
  readonly value = new Float64Array(PARAM_COUNT);
  /** param ごとの、活性な周期系が乗せている偏差。区間が終われば消える。 */
  readonly deviation = new Float64Array(PARAM_COUNT);
  /** param ごとの、走っている区間列。 */
  readonly active: Segment[][] = [];
  /** param ごとの、次の `start_transitions` を待つ区間列。 */
  readonly pending: Segment[][] = [];
  /** `active[param]` の中で今活性な区間の位置。-1 はまだどれも始まっていない。 */
  readonly cursor = new Int32Array(PARAM_COUNT).fill(-1);
  /** 削除予定時刻。null なら予定なし。 */
  deleteAt: number | null = null;
  alive = true;

  constructor(id: number, sprite: Sprite) {
    this.id = id;
    this.sprite = sprite;
    for (let param = 0; param < PARAM_COUNT; param++) {
      this.active.push([]);
      this.pending.push([]);
    }
  }

  /** 画面に出る値。落ち着いた値に周期系の偏差を足したもの。 */
  at(param: number): number {
    return this.value[param] + this.deviation[param];
  }
}

/** `sleep` で待っているもの。 */
interface Timer {
  at: number;
  resolve: () => void;
}

/**
 * canvas オブジェクトと遷移の管理。
 *
 * PixiJS の Ticker に登録するコールバックは **`update` の 1 つだけ**である。
 * オブジェクトごとに Ticker を生やさないのは、リークを避けるためでもあるし、
 * ポーズ・オート・スキップを 1 箇所の時間操作で効かせるためでもある。
 *
 * 設計は `docs/media-object-model.md` にある。
 */
export class CanvasObjects {
  /**
   * 出力先の `Canvas` Element ごとの描画先。
   *
   * 描画先の原点が Element の中央に置かれるので、ここでの射影は
   * 中央原点の biwa 座標をそのまま (y だけ反転して) 書けばよい。
   */
  private readonly surfaces: CanvasSurfaces;

  private readonly objects = new Map<number, CanvasObject>();
  private nextId = 1;

  /**
   * エンジン時計 [ms]。
   *
   * `performance.now()` ではなく Ticker の差分を積む。
   * ポーズは進めない、早送りは `timeScale` を上げる、
   * 演出のスキップは区間の `startAt` をずらす、という形で
   * 時間まわりの操作がここに集まる。
   */
  private now = 0;
  timeScale = 1;

  /**
   * 完了していない sync 印のバッチ。
   *
   * テキストの進行と結びついた演出だけがここに入る。
   * 独立に走らせるバッチは誰も待たないので記録しない。
   */
  private syncBatches: Batch[] = [];
  private waiters: Array<() => void> = [];
  private timers: Timer[] = [];

  constructor(surfaces: CanvasSurfaces) {
    this.surfaces = surfaces;
  }

  /**
   * 新しいオブジェクト id を採る。
   *
   * TypeScript ターゲット用。wasm ターゲットでは Worker が自分で採る
   * (メインスレッドと往復せずに id を返すため)。どちらか一方しか使われない。
   */
  allocId(): number {
    return this.nextId++;
  }

  // --- syscall の実体 ---------------------------------------------------

  /**
   * オブジェクトを作る。
   *
   * `canvasId` は出力先の UI Element `Canvas` の ui_id。Canvas でなければ叱って作らない。
   *
   * テクスチャのロードは非同期だが、**オブジェクトは同期に作る**。
   * `create` は積んで返る syscall なので、ロードが終わる前に
   * `add_transition` が届くのが普通だからである。
   */
  create(
    id: number,
    canvasId: number,
    path: string,
    layer: number,
    x: number,
    y: number,
    w: number,
    h: number,
    alpha: number,
    theta: number,
  ): void {
    if (this.objects.has(id)) {
      console.error(`[biwa] canvas object ${id} already exists`);
      return;
    }
    const surface = this.surfaces.get(canvasId);
    if (surface === null) return;

    const sprite = new Sprite(Texture.EMPTY);
    // 回転の中心を画像の中心にする。位置も中心で指定する規約である。
    sprite.anchor.set(0.5);
    // ロードが終わるまで出さない。Texture.EMPTY はサイズ 0 なので、
    // この間に width/height を書くと scale が壊れる。
    sprite.visible = false;

    const object = new CanvasObject(id, sprite);
    object.value[Param.X] = x;
    object.value[Param.Y] = y;
    object.value[Param.W] = w;
    object.value[Param.H] = h;
    object.value[Param.Alpha] = alpha;
    object.value[Param.Theta] = theta;

    this.objects.set(id, object);
    surface.layer(layer).addChild(sprite);

    let url: string;
    try {
      url = resolveAssetUrl(path);
    } catch (e: unknown) {
      console.error(`[biwa] bad asset path "${path}":`, e);
      return;
    }

    void Assets.load(url)
      .then((texture: Texture) => {
        if (!object.alive) return;
        object.sprite.texture = texture;
        object.natural = { w: texture.width, h: texture.height };
        this.resolveAutoSize(object);
        object.sprite.visible = true;
        this.project(object);
      })
      .catch((e: unknown) => {
        console.error(
          `[biwa] failed to load "${path}" (looked for ${url}; ` +
          "paths are relative to the package's `assets/`):",
          e,
        );
      });
  }

  /** `after` ミリ秒後にオブジェクトを消す。 */
  remove(id: number, after: number): void {
    const object = this.objects.get(id);
    if (object === undefined) {
      console.error(`[biwa] no such canvas object: ${id}`);
      return;
    }
    object.deleteAt = this.now + Math.max(after, 0);
  }

  /** 次の `start` に備えて遷移を積む。積むだけで何も起きない。 */
  addTransition(
    id: number,
    param: number,
    kind: number,
    value: number,
    after: number,
    duration: number,
  ): void {
    const object = this.objects.get(id);
    if (object === undefined) {
      console.error(`[biwa] no such canvas object: ${id}`);
      return;
    }
    if (!isKnownParam(param)) {
      console.error(`[biwa] unknown transition param: ${param}`);
      return;
    }
    if (!isKnownKind(kind)) {
      console.error(`[biwa] unknown transition kind: ${kind}`);
      return;
    }

    object.pending[param].push({
      owner: object,
      param,
      kind,
      value,
      after: Math.max(after, 0),
      duration: Math.max(duration, 0),
      startAt: 0,
      baseline: 0,
    });
  }

  /**
   * 積まれた遷移を発火する。オブジェクトを跨いで一斉に始まる。
   *
   * 置き換わるのは**積まれた区間がある `(オブジェクト, パラメータ)` だけ**で、
   * 触れられていないパラメータは走り続ける。
   * 背景のパンの最中にキャラクターが跳ねただけでパンが死ぬのを避けるためである。
   */
  start(sync: boolean): void {
    const segments: Segment[] = [];

    for (const object of this.objects.values()) {
      for (let param = 0; param < PARAM_COUNT; param++) {
        const staged = object.pending[param];
        if (staged.length === 0) continue;

        staged.sort((a, b) => a.after - b.after);
        for (const segment of staged) {
          segment.startAt = this.now + segment.after;
          segments.push(segment);
        }

        object.active[param] = staged;
        object.pending[param] = [];
        object.cursor[param] = -1;
        // 前の周期系が乗せていた偏差は、置き換えた時点で消える。
        object.deviation[param] = 0;
      }
    }

    if (sync && segments.length > 0) {
      this.syncBatches.push({ segments });
    }
  }

  /** sync 印の演出がすべて終わるまで待つ。 */
  awaitSync(): Promise<void> {
    if (this.syncBatches.length === 0) {
      return Promise.resolve();
    }
    return new Promise<void>((resolve) => {
      this.waiters.push(resolve);
    });
  }

  /** エンジン時計の上で `ms` ミリ秒待つ (ポーズ中は進まない)。 */
  sleep(ms: number): Promise<void> {
    if (ms <= 0) {
      return Promise.resolve();
    }
    return new Promise<void>((resolve) => {
      this.timers.push({ at: this.now + ms, resolve });
    });
  }

  /**
   * 進行中の sync 印の演出を終端まで飛ばす。
   *
   * クリックが来たときに、まずこれを試す。飛ばすものがあれば
   * そのクリックは演出の完了に使われ、テキストは進まない。
   *
   * 時計そのものは動かさない。動かすと独立に走っている演出まで早送りされる。
   * 代わりに対象のバッチの区間を過去へずらす。
   */
  skipSync(): boolean {
    let skipped = false;

    for (const batch of this.syncBatches) {
      let endsAt = -Infinity;
      for (const segment of batch.segments) {
        if (!segment.owner.alive || isPeriodic(segment.kind)) continue;
        endsAt = Math.max(endsAt, endOf(segment));
      }

      const delta = endsAt - this.now;
      if (!Number.isFinite(delta) || delta <= 0) continue;

      // 周期系も一緒にずらす。末尾に置かれた `None` がその過程で発火して、
      // ゆらぎもバッチの意図どおりに終わる。
      for (const segment of batch.segments) {
        segment.startAt -= delta;
      }
      skipped = true;
    }

    return skipped;
  }

  // --- Ticker から毎フレーム呼ばれる -----------------------------------

  update(deltaMs: number): void {
    this.now += deltaMs * this.timeScale;
    const now = this.now;

    for (const object of this.objects.values()) {
      // 出力先の Canvas Element ごと消えた (描画先が捨てられた) もの。
      if (object.sprite.destroyed) {
        object.alive = false;
        this.objects.delete(object.id);
        continue;
      }
      if (object.deleteAt !== null && now >= object.deleteAt) {
        this.destroy(object);
        continue;
      }
      this.advance(object, now);
      this.project(object);
    }

    this.collect(now);
  }

  // --- 内部 -------------------------------------------------------------

  /** 各パラメータの活性な区間を進める。 */
  private advance(object: CanvasObject, now: number): void {
    for (let param = 0; param < PARAM_COUNT; param++) {
      const track = object.active[param];
      if (track.length === 0) continue;

      let cursor = object.cursor[param];
      if (cursor < 0) {
        if (track[0].startAt > now) continue;
        cursor = 0;
        this.activate(object, track[0]);
      }

      // 次の区間が始まったら、前の区間はその時点で終わる。
      // 引き継ぐ値が正しくなるよう、終わる瞬間の値を先に確定させる。
      while (cursor + 1 < track.length && track[cursor + 1].startAt <= now) {
        this.evaluate(object, track[cursor], track[cursor + 1].startAt);
        cursor += 1;
        this.activate(object, track[cursor]);
      }

      object.cursor[param] = cursor;
      this.evaluate(object, track[cursor], now);
    }
  }

  /** 区間が活性になった瞬間に基準値を捉える。 */
  private activate(object: CanvasObject, segment: Segment): void {
    segment.baseline = object.value[segment.param];
    object.deviation[segment.param] = 0;
  }

  private evaluate(object: CanvasObject, segment: Segment, at: number): void {
    const param = segment.param;

    if (segment.kind === Curve.None) {
      // 基準値 (= 区間に入った時点の落ち着いた値) で固定して終わる。
      object.value[param] = segment.baseline;
      object.deviation[param] = 0;
      return;
    }

    if (isPeriodic(segment.kind)) {
      // 周期系は落ち着いた値を動かさず、偏差だけを乗せる。
      // だから区間が終われば値は基準値に戻る。
      object.value[param] = segment.baseline;
      const period = segment.duration > 0 ? segment.duration : 1;
      const phase = ((((at - segment.startAt) / period) % 1) + 1) % 1;
      object.deviation[param] = segment.value * waveAt(segment.kind, phase);
      return;
    }

    object.deviation[param] = 0;
    const progress =
      segment.duration > 0
        ? clamp((at - segment.startAt) / segment.duration, 0, 1)
        : 1;
    object.value[param] =
      segment.baseline +
      (segment.value - segment.baseline) * curveAt(segment.kind, progress);
  }

  /** biwa のパラメータを PixiJS の Sprite に射影する。 */
  private project(object: CanvasObject): void {
    // ロード前はサイズが分からないので触らない。
    if (object.natural === null) return;

    const sprite = object.sprite;
    // 描画先 (`CanvasSurface`) の原点が Canvas Element の中央にある。
    sprite.x = object.at(Param.X);
    // biwa の y は上が正。Pixi は下が正なので反転する。
    sprite.y = -object.at(Param.Y);
    sprite.width = Math.max(object.at(Param.W), 0);
    sprite.height = Math.max(object.at(Param.H), 0);
    sprite.alpha = clamp(object.at(Param.Alpha), 0, 255) / 255;
    // y を反転した分、回転の向きも反転させる (正 = 反時計回り)。
    sprite.rotation = (-object.at(Param.Theta) * Math.PI) / 180;
  }

  /** 負の `w` / `h` (= 指定しない) をテクスチャの元のサイズから決める。 */
  private resolveAutoSize(object: CanvasObject): void {
    const natural = object.natural;
    if (natural === null || natural.w <= 0 || natural.h <= 0) return;

    const w = object.value[Param.W];
    const h = object.value[Param.H];

    if (w < 0 && h < 0) {
      object.value[Param.W] = natural.w;
      object.value[Param.H] = natural.h;
    } else if (w < 0) {
      object.value[Param.W] = natural.w * (h / natural.h);
    } else if (h < 0) {
      object.value[Param.H] = natural.h * (w / natural.w);
    }
  }

  private destroy(object: CanvasObject): void {
    object.alive = false;
    object.sprite.removeFromParent();
    object.sprite.destroy();
    this.objects.delete(object.id);
  }

  /** 終わった sync バッチと sleep を回収する。 */
  private collect(now: number): void {
    if (this.syncBatches.length > 0) {
      this.syncBatches = this.syncBatches.filter(
        (batch) => !isBatchDone(batch, now),
      );
      if (this.syncBatches.length === 0 && this.waiters.length > 0) {
        const waiters = this.waiters;
        this.waiters = [];
        for (const resolve of waiters) resolve();
      }
    }

    if (this.timers.length > 0) {
      const due = this.timers.filter((timer) => now >= timer.at);
      if (due.length > 0) {
        this.timers = this.timers.filter((timer) => now < timer.at);
        for (const timer of due) timer.resolve();
      }
    }
  }
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

function isBatchDone(batch: Batch, now: number): boolean {
  for (const segment of batch.segments) {
    if (!segment.owner.alive || isPeriodic(segment.kind)) continue;
    if (now < endOf(segment)) return false;
  }
  return true;
}

function clamp(value: number, min: number, max: number): number {
  return Math.min(Math.max(value, min), max);
}
