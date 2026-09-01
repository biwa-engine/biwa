import { Application, Container } from "pixi.js";

/**
 * 描画の層。
 *
 * - **canvas レイヤー**: PixiJS の Container。整数の index で識別する。
 *   `create_object` が受け取るのはこの index である。
 *   意味づけ (背景・立ち絵・前景) は std の仕事なので、ここには持たない。
 *   レイヤーは安いので、前後を細かく分けたければ index を分ければよい。
 * - **DOM レイヤー**: テキストや UI を載せる HTMLElement。名前で識別する。
 *
 * canvas は 1 枚の `<canvas>` の中に積まれ、その `<canvas>` は
 * すべての DOM レイヤーより下にある (`Renderer` が z-index 0 に置く)。
 * つまり canvas レイヤーの index が DOM レイヤーを追い越すことはない。
 */
export class LayerManager {
  private canvasLayers = new Map<number, Container>();
  private domLayers = new Map<string, HTMLElement>();
  private app: Application;
  private host: HTMLElement;

  constructor(app: Application, host: HTMLElement) {
    this.app = app;
    this.host = host;
  }

  /** canvas レイヤーを引く。無ければ作る。 */
  canvas(index: number): Container {
    const found = this.canvasLayers.get(index);
    if (found !== undefined) {
      return found;
    }

    const container = new Container();
    container.zIndex = index;
    this.app.stage.addChild(container);
    this.canvasLayers.set(index, container);
    return container;
  }

  defineDom(id: string, zIndex: number): void {
    if (this.domLayers.has(id)) return;

    const el = document.createElement("div");
    el.style.cssText = `
      position: absolute;
      inset: 0;
      z-index: ${zIndex};
      pointer-events: none;
    `;
    el.dataset.layerId = id;
    this.host.appendChild(el);
    this.domLayers.set(id, el);
  }

  dom(id: string): HTMLElement {
    const layer = this.domLayers.get(id);
    if (layer === undefined) {
      throw new Error(`[biwa] DOM layer "${id}" is not defined`);
    }
    return layer;
  }
}
