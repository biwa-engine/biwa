import { Assets, Sprite, type Texture } from "pixi.js";
import { resolveAssetUrl } from "./assets";
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
 * `path` はパッケージの `assets/` を基準とした相対パスである
 * (解決は `api/assets.ts`)。
 *
 * テクスチャの読み込みは非同期だが、この syscall 自体は
 * 読み込みを積んで即座にリターンする (完了を待たない)。
 * したがって失敗してもゲームは止めず、ログを出すに留める。
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

  let url: string;
  try {
    url = resolveAssetUrl(path);
  } catch (e: unknown) {
    console.error(`[biwa] bad asset path "${path}":`, e);
    return;
  }

  void Assets.load(url)
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
      console.error(
        `[biwa] failed to load image "${path}" (looked for ${url}; ` +
        `paths are relative to the package's \`assets/\`):`,
        e,
      );
    });
}
