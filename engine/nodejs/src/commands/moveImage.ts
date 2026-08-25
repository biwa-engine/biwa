import type { Ticker } from "pixi.js";
import { tween } from "../engine/tween";
import { sprites } from "./showImage";
import type { Command } from "../types/Command";

export function moveImage(
  xStart: number,
  yStart: number,
  xEnd: number,
  yEnd: number,
  id: string,
  ticker: Ticker,
  duration = 0.5,
): Command {
  return async () => {
    const sprite = sprites.get(id);
    if (!sprite) {
      console.warn(`moveImage: sprite "${id}" not found`);
      return;
    }
    sprite.x = xStart;
    sprite.y = yStart;
    await tween(ticker, sprite as unknown as Record<string, number>, { x: xEnd, y: yEnd }, duration);
  };
}
