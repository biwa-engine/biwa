import { Assets, Sprite } from "pixi.js";
import type { LayerManager } from "../engine/LayerManager";
import type { Command } from "../types/Command";

const sprites = new Map<string, Sprite>();

export function showImage(
  x: number,
  y: number,
  width: number,
  height: number,
  id: string,
  layerId: string,
  layers: LayerManager,
): Command {
  return async () => {
    const texture = await Assets.load(id);
    const sprite = new Sprite(texture);
    sprite.x = x;
    sprite.y = y;
    sprite.width = width;
    sprite.height = height;
    sprite.label = id;

    if (sprites.has(id)) {
      sprites.get(id)!.destroy();
    }
    sprites.set(id, sprite);
    layers.getCanvas(layerId).addChild(sprite);
  };
}

export { sprites };

