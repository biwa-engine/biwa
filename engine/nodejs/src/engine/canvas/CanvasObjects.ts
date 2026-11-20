import { Assets, Sprite, Texture } from "pixi.js";
import { resolveAssetUrl } from "../api/assets";
import { Param, isKnownKind, isKnownParam } from "../api/transition";
import { Automation } from "../automation/Automation";
import type { CanvasSurface, CanvasSurfaces } from "./CanvasSurfaces";

/**
 * canvas に置かれたオブジェクト 1 つ。
 *
 * biwa 側のパラメータ (`value` / `deviation`) を正とし、
 * PixiJS へは毎フレーム射影する。座標系も単位も両者で違うためである。
 * x / y / w / h は canvas の範囲が -50..50 の単位で持ち、射影のときに
 * 描画先の今の大きさで px に直す (canvas の大きさが変われば追従する)。
 */
class CanvasObject {
  readonly id: number;
  readonly sprite: Sprite;
  /** 置かれている描画先。座標・大きさを px に直すのに今の大きさを使う。 */
  readonly surface: CanvasSurface;
  /** テクスチャの元のサイズ [px]。ロードが終わるまで null。 */
  natural: { w: number; h: number } | null = null;
  /** 負の `w` / `h` (= 指定しない) を決め終えたか (`resolveAutoSize`)。 */
  autoSizeResolved = false;
  /** 削除予定時刻。null なら予定なし。 */
  deleteAt: number | null = null;
  alive = true;

  constructor(id: number, sprite: Sprite, surface: CanvasSurface) {
    this.id = id;
    this.sprite = sprite;
    this.surface = surface;
  }

  /** パラメータの表 (`Automation`) での持ち主の鍵。 */
  get key(): string {
    return `obj:${this.id}`;
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
   * パラメータ (x / y / w / h / alpha / theta) の値と遷移。sync 印の束もここが持つ。
   * 音のパラメータと同じ仕組みである (`automation/Automation.ts`)。
   */
  private readonly params = new Automation();
  private waiters: Array<() => void> = [];
  private timers: Timer[] = [];

  constructor(surfaces: CanvasSurfaces) {
    this.surfaces = surfaces;
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

    const object = new CanvasObject(id, sprite, surface);
    const initial = [x, y, w, h, alpha, theta];
    initial.forEach((value, param) => this.params.define(object.key, param, value));

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

    this.params.push(object.key, param, kind, value, after, duration);
  }

  /**
   * 積まれた遷移を発火する。オブジェクトを跨いで一斉に始まる。
   *
   * 置き換わるのは**積まれた区間がある `(オブジェクト, パラメータ)` だけ**で、
   * 触れられていないパラメータは走り続ける。
   * 背景のパンの最中にキャラクターが跳ねただけでパンが死ぬのを避けるためである。
   */
  start(sync: boolean): void {
    this.params.start(sync, this.now);
  }

  /** 完了していない sync 印の演出があるか。 */
  hasSync(): boolean {
    return this.params.hasSync();
  }

  /** sync 印の演出がすべて終わるまで待つ。 */
  awaitSync(): Promise<void> {
    if (!this.params.hasSync()) {
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
    return this.params.skipSync(this.now);
  }

  // --- Ticker から毎フレーム呼ばれる -----------------------------------

  update(deltaMs: number): void {
    this.now += deltaMs * this.timeScale;
    const now = this.now;

    for (const object of this.objects.values()) {
      // 出力先の Canvas Element ごと消えた (描画先が捨てられた) もの。
      if (object.sprite.destroyed) {
        object.alive = false;
        this.params.remove(object.key);
        this.objects.delete(object.id);
        continue;
      }
      if (object.deleteAt !== null && now >= object.deleteAt) {
        this.destroy(object);
        continue;
      }
    }
    this.params.update(now);
    for (const object of this.objects.values()) this.project(object);

    this.collect(now);
  }

  // --- 内部 -------------------------------------------------------------

  /** 画面に出る値。落ち着いた値に周期系の偏差を足したもの。 */
  private at(object: CanvasObject, param: number): number {
    return this.params.get(object.key, param) ?? 0;
  }

  /**
   * biwa のパラメータを PixiJS の Sprite に射影する。
   *
   * x / y / w / h は canvas の範囲が -50..50 の単位なので、描画先の今の大きさで px に直す
   * (x と w は幅の、y と h は高さの 1/100 が 1)。
   */
  private project(object: CanvasObject): void {
    // ロード前はサイズが分からないので触らない。
    if (object.natural === null) return;
    // 描画先が見えていない (大きさが 0) 間は px に直せないので触らない。
    const { width, height } = object.surface;
    if (width === 0 || height === 0) return;
    if (!object.autoSizeResolved) {
      this.resolveAutoSize(object, width, height);
    }

    const unitX = width / 100;
    const unitY = height / 100;
    const sprite = object.sprite;
    // 描画先 (`CanvasSurface`) の原点が Canvas Element の中央にある。
    sprite.x = this.at(object, Param.X) * unitX;
    // biwa の y は上が正。Pixi は下が正なので反転する。
    sprite.y = -this.at(object, Param.Y) * unitY;
    sprite.width = Math.max(this.at(object, Param.W), 0) * unitX;
    sprite.height = Math.max(this.at(object, Param.H), 0) * unitY;
    sprite.alpha = clamp(this.at(object, Param.Alpha), 0, 255) / 255;
    // y を反転した分、回転の向きも反転させる (正 = 反時計回り)。
    sprite.rotation = (-this.at(object, Param.Theta) * Math.PI) / 180;
  }

  /**
   * 負の `w` / `h` (= 指定しない) を、テクスチャの元のサイズ [px] と描画先の大きさ [px]
   * から canvas の単位で決める。
   *
   * - 両方負: 画像の元の大きさ (px) で出る
   * - 片方だけ負: 画像の縦横比を保つ。canvas の単位は縦横で尺度が違うので
   *   (幅は canvas の幅、高さは canvas の高さに対する割合)、一度 px に直して比を取る
   *
   * 決めるのは最初に描画先の大きさが分かったときの 1 回だけで、以降は canvas の単位の値として
   * 遷移の起点にもなる。そのため canvas の大きさが後で変わると、それに合わせて伸び縮みする。
   */
  private resolveAutoSize(
    object: CanvasObject,
    width: number,
    height: number,
  ): void {
    const natural = object.natural;
    if (natural === null || natural.w <= 0 || natural.h <= 0) return;
    object.autoSizeResolved = true;

    const w = this.at(object, Param.W);
    const h = this.at(object, Param.H);
    const toUnitX = (px: number): number => (px / width) * 100;
    const toUnitY = (px: number): number => (px / height) * 100;

    if (w < 0 && h < 0) {
      this.params.setSettled(object.key, Param.W, toUnitX(natural.w));
      this.params.setSettled(object.key, Param.H, toUnitY(natural.h));
    } else if (w < 0) {
      const hPx = (h / 100) * height;
      this.params.setSettled(object.key, Param.W, toUnitX(natural.w * (hPx / natural.h)));
    } else if (h < 0) {
      const wPx = (w / 100) * width;
      this.params.setSettled(object.key, Param.H, toUnitY(natural.h * (wPx / natural.w)));
    }
  }

  private destroy(object: CanvasObject): void {
    object.alive = false;
    this.params.remove(object.key);
    object.sprite.removeFromParent();
    object.sprite.destroy();
    this.objects.delete(object.id);
  }

  /** 終わった sync バッチと sleep を回収する。 */
  private collect(now: number): void {
    if (!this.params.hasSync() && this.waiters.length > 0) {
      const waiters = this.waiters;
      this.waiters = [];
      for (const resolve of waiters) resolve();
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

function clamp(value: number, min: number, max: number): number {
  return Math.min(Math.max(value, min), max);
}
