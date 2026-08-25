import type { Ticker } from "pixi.js";

type TweenTarget = Record<string, number>;

export function tween(
  ticker: Ticker,
  target: TweenTarget,
  to: TweenTarget,
  duration: number, // seconds
): Promise<void> {
  return new Promise((resolve) => {
    const from: TweenTarget = {};
    for (const key of Object.keys(to)) {
      from[key] = target[key];
    }

    let elapsed = 0;

    const onTick = (t: Ticker) => {
      elapsed += t.deltaMS / 1000;
      const progress = Math.min(elapsed / duration, 1);

      for (const key of Object.keys(to)) {
        target[key] = from[key] + (to[key] - from[key]) * progress;
      }

      if (progress >= 1) {
        ticker.remove(onTick);
        resolve();
      }
    };

    ticker.add(onTick);
  });
}
