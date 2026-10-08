import { Ticker } from "pixi.js";
import { LayerManager } from "./LayerManager";

/**
 * 画面 (host) と、エンジンの時計。
 *
 * canvas に描くものは出力先の UI Element `Canvas` ごとの描画先が持つ
 * (`engine/canvas/CanvasSurfaces.ts`)。ここが持つのは host の大きさと、
 * エンジン全体を駆動する唯一の Ticker である。
 */
export class Renderer {
  /** エンジンの時計。コールバックは `main.ts` の 1 つだけを登録する。 */
  readonly ticker = new Ticker();
  readonly layers: LayerManager;
  readonly host: HTMLElement;

  constructor(host: HTMLElement) {
    this.host = host;
    this.layers = new LayerManager(host);
  }

  /**
   * host の大きさを決め、時計を動かす。
   *
   * `width` / `height` は px。省略すると画面全体 (`100vw` / `100vh`) になり、
   * Biwa Language 側 (`fn app()` が `show()` する `Window`) が制御する UI の範囲が画面全体になる。
   */
  init(width?: number, height?: number): void {
    // host は拡大縮小しない。Canvas Element の中の座標は CSS の px と 1:1 で対応する。
    // 何も置かれていない所は黒く見せる (UI が出るまでも含めて)。
    this.host.style.cssText = `
      position: relative;
      width: ${width === undefined ? "100vw" : `${width}px`};
      height: ${height === undefined ? "100vh" : `${height}px`};
      overflow: hidden;
      background: #000;
    `;
    this.ticker.start();
  }
}
