import { Application } from "pixi.js";
import { LayerManager } from "./LayerManager";

export class Renderer {
  app: Application;
  layers: LayerManager;
  readonly host: HTMLElement;

  constructor(host: HTMLElement) {
    this.host = host;
    this.app = new Application();
    this.layers = new LayerManager(this.app, host);
  }

  async init(width: number, height: number): Promise<void> {
    await this.app.init({
      width,
      height,
      backgroundColor: 0x000000,
      antialias: true,
    });

    this.host.style.cssText = `
      position: relative;
      width: ${width}px;
      height: ${height}px;
      overflow: hidden;
    `;

    const canvas = this.app.canvas;
    (canvas as HTMLCanvasElement).style.cssText = `
      position: absolute;
      top: 0;
      left: 0;
      z-index: 0;
      pointer-events: none;
    `;
    this.host.appendChild(canvas);

    this.app.stage.sortableChildren = true;
  }
}

