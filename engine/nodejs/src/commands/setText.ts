import type { ComponentRegistry } from "../components/ComponentRegistry";
import type { Command } from "../types/Command";

export function setText(
  id: string,
  text: string,
  registry: ComponentRegistry,
): Command {
  return async () => {
    registry.getTextBox(id).setText(text);
  };
}
