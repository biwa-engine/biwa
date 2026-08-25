import { Application, Container } from "pixi.js";

export type LayerType = "canvas" | "dom";

interface LayerDef {
  id: string;
  type: LayerType;
  zIndex: number;
}

interface CanvasLayer {
  type: "canvas";
  container: Container;
}

interface DomLayer {
  type: "dom";
  element: HTMLElement;
}

type Layer = CanvasLayer | DomLayer;

export class LayerManager {
  private layers = new Map<string, Layer>();
  private app: Application;
  private host: HTMLElement;

  constructor(app: Application, host: HTMLElement) {
    this.app = app;
    this.host = host;
  }

  defineLayer(def: LayerDef): void {
    if (this.layers.has(def.id)) return;

    if (def.type === "canvas") {
      const container = new Container();
      container.zIndex = def.zIndex;
      this.app.stage.addChild(container);
      this.layers.set(def.id, { type: "canvas", container });
    } else {
      const el = document.createElement("div");
      el.style.cssText = `
        position: absolute;
        inset: 0;
        z-index: ${def.zIndex};
        pointer-events: none;
      `;
      el.dataset.layerId = def.id;
      this.host.appendChild(el);
      this.layers.set(def.id, { type: "dom", element: el });
    }
  }

  getCanvas(id: string): Container {
    const layer = this.layers.get(id);
    if (!layer || layer.type !== "canvas") {
      throw new Error(`Canvas layer "${id}" not found`);
    }
    return layer.container;
  }

  getDom(id: string): HTMLElement {
    const layer = this.layers.get(id);
    if (!layer || layer.type !== "dom") {
      throw new Error(`DOM layer "${id}" not found`);
    }
    return layer.element;
  }
}

