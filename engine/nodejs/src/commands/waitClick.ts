import type { Command } from "../types/Command";

export function waitClick(target: HTMLElement): Command {
  return () =>
    new Promise<void>((resolve) => {
      target.addEventListener("click", () => resolve(), { once: true });
    });
}
