/**
 * UI Element `Canvas` ごとの描画先。
 *
 * Canvas API (`sys_create_object`) は第一引数の ui_id で出力先の `Canvas` を指定する
 * (`docs/ui-api-impl-status.md` §14 S6)。ここはその ui_id から PixiJS 上の描画先
 * ({@link CanvasSurface}) を引き、毎フレーム Element の DOM 矩形に合わせる。
 *
 * - 描画先の原点は Element の**中央**である (biwa の canvas 座標の規約そのまま)。
 * - Element の矩形からはみ出した部分は切り取る (マスク)。
 * - Element が表示されていない (祖先の Page が隠れているなど) ときは描画先も隠す。
 *
 * PixiJS の `<canvas>` は 1 枚で、すべての DOM レイヤーより下にある (`Renderer`)。
 * 描画先はその中の Container なので、canvas の中身は今までどおり UI より下に描かれる。
 * host は拡大縮小しないので (`Renderer.init`)、DOM の座標は PixiJS の座標と 1:1 で対応する。
 */

import { Container, Graphics } from "pixi.js";
import type { UIObjects } from "../ui/UIObjects";

/** 1 つの `Canvas` Element の描画先。 */
export class CanvasSurface {
  /** Element の中央に置かれる。子はレイヤー (index ごとの Container)。 */
  readonly root = new Container();
  private readonly mask = new Graphics();
  private readonly layers = new Map<number, Container>();
  private width = -1;
  private height = -1;

  constructor() {
    this.root.sortableChildren = true;
    this.root.addChild(this.mask);
    this.root.mask = this.mask;
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
    this.root.addChild(container);
    this.layers.set(index, container);
    return container;
  }

  /** Element の矩形 (host 基準の px) に合わせる。 */
  place(left: number, top: number, width: number, height: number): void {
    this.root.visible = true;
    this.root.position.set(left + width / 2, top + height / 2);
    if (width !== this.width || height !== this.height) {
      this.width = width;
      this.height = height;
      this.mask
        .clear()
        .rect(-width / 2, -height / 2, width, height)
        .fill(0xffffff);
    }
  }

  hide(): void {
    this.root.visible = false;
  }
}

/** ui_id → 描画先。 */
export class CanvasSurfaces {
  private readonly surfaces = new Map<number, CanvasSurface>();
  private readonly stage: Container;
  private readonly host: HTMLElement;
  private readonly ui: UIObjects;

  constructor(stage: Container, host: HTMLElement, ui: UIObjects) {
    this.stage = stage;
    this.host = host;
    this.ui = ui;
  }

  /**
   * ui_id が指す `Canvas` の描画先を引く。初めてなら作る。
   *
   * Canvas でなければ名指しで叱って `null` (中断しない syscall なので投げても届かない)。
   */
  get(uiId: number): CanvasSurface | null {
    if (this.ui.canvasElement(uiId) === null) {
      console.error(`[biwa] ui element ${uiId} is not a Canvas`);
      return null;
    }

    const found = this.surfaces.get(uiId);
    if (found !== undefined) return found;

    const surface = new CanvasSurface();
    this.stage.addChild(surface.root);
    this.surfaces.set(uiId, surface);
    this.syncOne(uiId, surface, this.host.getBoundingClientRect());
    return surface;
  }

  /**
   * すべての描画先を Element の今の矩形に合わせる。
   *
   * Ticker から毎フレーム呼ばれる (`main.ts`)。レイアウトは property の設定や
   * Page の切り替えで変わるので、変わったことを追いかけるより毎回読むほうが単純である。
   * 描画先は Canvas Element の数しか無いので、矩形を読む費用は小さい。
   */
  sync(): void {
    if (this.surfaces.size === 0) return;
    const hostRect = this.host.getBoundingClientRect();
    for (const [uiId, surface] of this.surfaces) {
      this.syncOne(uiId, surface, hostRect);
    }
  }

  private syncOne(uiId: number, surface: CanvasSurface, hostRect: DOMRect): void {
    const el = this.ui.canvasElement(uiId);
    // 消された Element や、祖先ごと `display: none` のものは矩形を持たない。
    if (el === null || !el.isConnected || el.getClientRects().length === 0) {
      surface.hide();
      return;
    }
    const rect = el.getBoundingClientRect();
    surface.place(
      rect.left - hostRect.left,
      rect.top - hostRect.top,
      rect.width,
      rect.height,
    );
  }
}
