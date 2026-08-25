import { Assets, Sprite, type Texture } from "pixi.js";
import { engine } from "./context";

/** 画像の位置に毎フレーム加算されるオフセット。`t` は表示開始からの経過秒。 */
export interface ImageMotion {
  dx: (t: number) => number;
  dy: (t: number) => number;
}

/**
 * 画像を canvas レイヤーに配置する。
 *
 * `std::game::base_engine::create_image` の実体。
 * テクスチャの読み込みは非同期だが、この syscall 自体は
 * 読み込みを積んで即座にリターンする (完了を待たない)。
 */
export function createImage(
  path: string,
  x: number,
  y: number,
  motion: ImageMotion,
): void {
  const { renderer, spriteLayerId } = engine();
  const layer = renderer.layers.getCanvas(spriteLayerId);
  const ticker = renderer.app.ticker;

  void Assets.load(path)
    .then((texture: Texture) => {
      const sprite = new Sprite(texture);
      sprite.x = x;
      sprite.y = y;
      layer.addChild(sprite);

      let elapsed = 0;
      ticker.add((t) => {
        elapsed += t.deltaMS / 1000;
        sprite.x = x + motion.dx(elapsed);
        sprite.y = y + motion.dy(elapsed);
      });
    })
    .catch((e: unknown) => {
      console.error(`[biwa] failed to load image "${path}":`, e);
    });
}
