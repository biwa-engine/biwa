/**
 * DOM の描画の層。テキストや UI を載せる HTMLElement で、名前で識別する。
 *
 * canvas に描くもの (`create_object`) の層はここには無い。出力先の UI Element
 * `Canvas` ごとの描画先が自分のレイヤーを持つ (`engine/canvas/CanvasSurfaces.ts`)。
 * PixiJS の `<canvas>` は 1 枚で、すべての DOM レイヤーより下にある
 * (`Renderer` が z-index 0 に置く)。つまり canvas レイヤーの index が
 * DOM レイヤーを追い越すことはない。
 */
export class LayerManager {
  private domLayers = new Map<string, HTMLElement>();
  private host: HTMLElement;

  constructor(host: HTMLElement) {
    this.host = host;
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
