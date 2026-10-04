/**
 * UI Element `Canvas` ごとの描画先。
 *
 * Canvas API (`sys_create_object`) は第一引数の ui_id で出力先の `Canvas` を指定する
 * (`docs/ui-api-impl-status.md` §14 S6)。ここはその ui_id から描画先
 * ({@link CanvasSurface}) を引く。
 *
 * 描画先は Canvas Element ごとに自分の PixiJS の `<canvas>` を持ち、それを
 * Element の DOM の中に置く。そのため
 *
 * - 位置・大きさは Element そのものであり、はみ出しは `<canvas>` の外なので描かれない。
 * - 前後は DOM の重なりに従う (`Layers` の中なら push 順)。
 * - Element が表示されていない (祖先の Page が隠れているなど) ときは描画先も見えない。
 *
 * 描画先の原点は Element の**中央**である (biwa の canvas 座標の規約そのまま)。
 * `<canvas>` は解像度 1 で Element と同じ大きさにする。biwa の座標・大きさ
 * (canvas の範囲が -50..50 の単位) を px に直すのは `CanvasObjects` の射影で、
 * そのために描画先は今の大きさ (`width` / `height` [px]) を持っている。
 */

import { Application, Container } from "pixi.js";
import type { UIObjects } from "../ui/UIObjects";

/** 1 つの `Canvas` Element の描画先。 */
export class CanvasSurface {
  private readonly app = new Application();
  private readonly element: HTMLElement;
  private readonly layers = new Map<number, Container>();
  private readonly observer: ResizeObserver;
  /** `init` が終わって描けるようになったか。 */
  private ready = false;
  private destroyed = false;
  /** Element の今の大きさ [px]。見えていなければ 0。 */
  private widthPx: number;
  private heightPx: number;

  constructor(element: HTMLElement) {
    this.element = element;
    // `init` の前から stage はあるので、オブジェクトは同期に積める
    // (`create_object` は積んで返る syscall なので待てない)。
    this.app.stage.sortableChildren = true;
    this.widthPx = element.clientWidth;
    this.heightPx = element.clientHeight;
    // 大きさは PixiJS の初期化を待たずに追う (射影が init の前から使うため)。
    this.observer = new ResizeObserver(() => this.fit());
    this.observer.observe(element);

    void this.app
      .init({
        width: Math.max(element.clientWidth, 1),
        height: Math.max(element.clientHeight, 1),
        // 下のレイヤー (DOM) が透けて見えるようにする。
        backgroundAlpha: 0,
        antialias: true,
        resolution: 1,
        // 描画は main の唯一の Ticker コールバックから `render()` で行う。
        autoStart: false,
      })
      .then(() => {
        if (this.destroyed) {
          this.app.destroy(true, { children: true });
          return;
        }
        this.app.canvas.style.cssText = `
          position: absolute;
          inset: 0;
          width: 100%;
          height: 100%;
          display: block;
          pointer-events: none;
        `;
        this.element.appendChild(this.app.canvas);
        this.ready = true;
        this.fit();
        this.applySize();
      })
      .catch((e: unknown) => {
        console.error("[biwa] failed to initialize a canvas renderer:", e);
      });
  }

  /**
   * レイヤーを引く。無ければ作る。
   *
   * `create_object` の layer index はこの描画先の中での前後である。
   * 意味づけ (背景・立ち絵・前景) は std の仕事なので、ここには持たない。
   */
  layer(index: number): Container {
    const found = this.layers.get(index);
    if (found !== undefined) return found;

    const container = new Container();
    container.zIndex = index;
    this.app.stage.addChild(container);
    this.layers.set(index, container);
    return container;
  }

  /** Element の今の幅 [px]。見えていなければ 0。 */
  get width(): number {
    return this.widthPx;
  }

  /** Element の今の高さ [px]。見えていなければ 0。 */
  get height(): number {
    return this.heightPx;
  }

  /** 1 フレーム描く。見えていない (大きさが 0 の) ときは描かない。 */
  render(): void {
    if (!this.ready || this.widthPx === 0 || this.heightPx === 0) return;
    this.app.render();
  }

  destroy(): void {
    this.destroyed = true;
    this.observer.disconnect();
    if (this.ready) {
      this.app.destroy(true, { children: true });
      this.ready = false;
    }
  }

  /** Element の大きさを読み直す。変わっていれば描画先にも反映する。 */
  private fit(): void {
    const width = this.element.clientWidth;
    const height = this.element.clientHeight;
    if (width === this.widthPx && height === this.heightPx) return;
    this.widthPx = width;
    this.heightPx = height;
    this.applySize();
  }

  /** `<canvas>` を Element の大きさにし、原点を中央に置く。 */
  private applySize(): void {
    if (!this.ready || this.widthPx === 0 || this.heightPx === 0) return;
    this.app.renderer.resize(this.widthPx, this.heightPx);
    this.app.stage.position.set(this.widthPx / 2, this.heightPx / 2);
  }
}

/** ui_id → 描画先。 */
export class CanvasSurfaces {
  private readonly surfaces = new Map<number, CanvasSurface>();
  private readonly ui: UIObjects;

  constructor(ui: UIObjects) {
    this.ui = ui;
  }

  /**
   * ui_id が指す `Canvas` の描画先を引く。初めてなら作る。
   *
   * Canvas でなければ名指しで叱って `null` (中断しない syscall なので投げても届かない)。
   */
  get(uiId: number): CanvasSurface | null {
    const element = this.ui.canvasElement(uiId);
    if (element === null) {
      console.error(`[biwa] ui element ${uiId} is not a Canvas`);
      return null;
    }

    const found = this.surfaces.get(uiId);
    if (found !== undefined) return found;

    const surface = new CanvasSurface(element);
    this.surfaces.set(uiId, surface);
    return surface;
  }

  /**
   * すべての描画先を 1 フレーム描く。
   *
   * Ticker から毎フレーム呼ばれる (`main.ts`)。Canvas Element が消えていたら、
   * その描画先 (WebGL コンテキスト) も捨てる。
   */
  render(): void {
    for (const [uiId, surface] of this.surfaces) {
      if (this.ui.canvasElement(uiId) === null) {
        surface.destroy();
        this.surfaces.delete(uiId);
        continue;
      }
      surface.render();
    }
  }
}
